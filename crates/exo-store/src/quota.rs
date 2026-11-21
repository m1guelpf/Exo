//! Spending Gmail's per-user quota ahead of time, so calls stay under it instead of finding it
//! with a 429, and what the user is waiting on goes first.
//!
//! Every Gmail call spends its method's units ([`gmail::quota::units`]) before it goes out,
//! under two limits: no rolling minute may see more than 90 % of the quota (Gmail counts per
//! minute), and no burst more than 45 % of it, a token bucket refilling at that 90 % a minute
//! (a screenful of threads at once, but not a minute's quota in a second). Calls wait in
//! [`Lane`] order, the cheapest first within a lane: a call of an earlier lane always goes
//! first, a star doesn't queue behind thread fetches, and the later lanes keep part of both the
//! minute and the bucket for the earlier ones, so a click finds budget at once even while sync
//! spends all it may. When Gmail pushes back anyway (another client of the same project, a
//! limit it doesn't document), nothing goes out until the time it asked for, and the minute's
//! limit and the refill halve, then recover over two minutes.

use std::collections::{BTreeSet, VecDeque};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use tokio::sync::Notify;
use tokio::time::Instant;

/// The longest a call waits for Gmail to stop rate limiting; asked to wait longer, it fails.
pub(crate) const MAX_WAIT: Duration = Duration::from_secs(32);
/// The most of a minute's quota any rolling minute spends.
const MINUTE: f64 = 0.9;
/// How much of a minute's quota the bucket holds.
const BURST: f64 = 0.45;
/// What [`Lane::Show`] and [`Lane::Sync`] keep for earlier lanes, of the minute and of the
/// bucket, as shares of a minute's quota: a few attachments' worth for a click, and a page of
/// search results more for what's on screen.
const SHOW_KEEPS: f64 = 0.05;
const SYNC_KEEPS: f64 = 0.2;
/// Gmail's quota window.
const WINDOW: Duration = Duration::from_secs(60);
/// The lowest the minute's limit and the refill drop to after Gmail pushes back, and how long
/// they take to recover from there.
const MIN_SHARE: f64 = 0.125;
const RECOVERY: Duration = Duration::from_secs(120);
/// Throttling is over once a call succeeds a whole quota window after Gmail last pushed back.
const QUIET: Duration = WINDOW;

/// Gmail's per-user quota, which the store paces its calls under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quota {
    /// Units the account may spend per minute.
    pub per_minute: u32,
}

impl Quota {
    /// What Gmail grants a user of a Cloud project created since May 2026.
    pub const GMAIL: Self = Self {
        per_minute: gmail::quota::PER_USER_PER_MINUTE,
    };
}

/// Whom a Gmail call is for, most urgent first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Lane {
    /// Something the user just did and is waiting on: an attachment, a triage, a draft, a send,
    /// a search's first results.
    Act,
    /// Filling what's on screen: the list's first rows, a list opened short, more search
    /// results, a search run ahead of being asked for, a contact's conversations.
    Show,
    /// Background sync: history, backfill, bulk relabels.
    Sync,
}

/// The account's quota and who is waiting for it. Shared by every call the store makes.
pub(crate) struct Budget {
    pace: Option<Pace>,
    state: Mutex<State>,
    /// Ticks whenever a waiting call may now go: another one went or gave up, or Gmail paused.
    changed: Notify,
}

/// The limits for a quota, in units.
#[derive(Debug, Clone, Copy)]
struct Pace {
    quota: f64,
}

impl Pace {
    fn new(quota: Quota) -> Self {
        Self {
            quota: f64::from(quota.per_minute),
        }
    }

    fn burst(&self) -> f64 {
        self.quota * BURST
    }

    /// The bucket's refill, per second.
    fn rate(&self) -> f64 {
        self.quota * MINUTE / WINDOW.as_secs_f64()
    }

    /// What `lane` keeps for earlier lanes, of the minute and of the bucket.
    fn keeps(&self, lane: Lane) -> f64 {
        self.quota
            * match lane {
                Lane::Act => 0.0,
                Lane::Show => SHOW_KEEPS,
                Lane::Sync => SYNC_KEEPS,
            }
    }
}

struct State {
    tokens: f64,
    /// When `tokens` was last refilled (or, during a pause, when refilling starts again).
    at: Instant,
    /// What went out in the last minute: when, and how many units.
    spent: VecDeque<(Instant, f64)>,
    /// The share of the minute's limit and the refill in use: below 1 after Gmail pushed back.
    share: f64,
    /// Calls waiting to spend, in the order they go: by lane, then cost, then arrival.
    waiting: BTreeSet<Ticket>,
    next: u64,
    /// Gmail asked for no calls before then.
    pause: Option<(Instant, Timestamp)>,
    episode: Option<Episode>,
}

/// Gmail rate limiting the account, from its first pushback until calls flow again: what the
/// log's summary reports.
#[derive(Debug, Clone, PartialEq)]
struct Episode {
    started: Timestamp,
    /// When the latest pause ends.
    resumes: (Instant, Timestamp),
    pushbacks: u32,
    /// Calls that waited for a pause.
    waited: u32,
    /// Calls that gave up, the pause being longer than [`MAX_WAIT`].
    failed: u32,
}

/// What a call waiting to spend does next.
#[derive(Debug, PartialEq)]
enum Turn {
    Go,
    /// Check again after this long, or when another call goes (`None`: only then).
    Wait(Option<Duration>),
    /// Gmail is paused until then, longer than a call waits.
    GiveUp(Timestamp),
}

impl Budget {
    /// A budget for `quota`; `None` paces nothing but still waits out Gmail's pauses.
    pub fn new(quota: Option<Quota>) -> Self {
        Self {
            pace: quota.map(Pace::new),
            state: Mutex::new(State {
                tokens: quota.map_or(0.0, |quota| Pace::new(quota).burst()),
                at: Instant::now(),
                spent: VecDeque::new(),
                share: 1.0,
                waiting: BTreeSet::new(),
                next: 0,
                pause: None,
                episode: None,
            }),
            changed: Notify::new(),
        }
    }

    /// Waits until a call of `units` may go out for `lane`, and spends them. Fails with when
    /// Gmail will take calls again if it has paused them for longer than [`MAX_WAIT`].
    pub async fn spend(&self, lane: Lane, units: u32) -> Result<(), Timestamp> {
        let key = {
            let mut state = self.lock();
            state.next += 1;
            let key = (lane, units, state.next);
            state.waiting.insert(key);
            key
        };
        let _leave = Leave { budget: self, key };
        let mut counted = false;
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let turn = self
                .lock()
                .turn(self.pace, key, units, Instant::now(), &mut counted);
            match turn {
                Turn::Go => return Ok(()),
                Turn::GiveUp(until) => return Err(until),
                Turn::Wait(Some(wait)) => tokio::select! {
                    () = tokio::time::sleep(wait) => {}
                    () = &mut changed => {}
                },
                Turn::Wait(None) => changed.await,
            }
        }
    }

    /// Gmail rate limited a call and asked for `wait`: nothing goes out until then, and the
    /// refill slows. Returns when calls resume.
    pub fn pushback(&self, wait: Duration) -> Timestamp {
        let until = self.lock().pushback(Instant::now(), Timestamp::now(), wait);
        self.changed.notify_waiters();
        until
    }

    /// A call succeeded: once Gmail has been quiet for a while, the throttling is over.
    pub fn settled(&self) {
        if let Some(episode) = self.lock().settle(Instant::now()) {
            let lasted = episode.resumes.1.duration_since(episode.started);
            log::info!(
                "Gmail stopped rate limiting after {:.1} s: it pushed back {} times, {} calls waited and {} gave up",
                lasted.as_secs_f64(),
                episode.pushbacks,
                episode.waited,
                episode.failed
            );
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("budget lock is never poisoned")
    }
}

/// Takes a call out of the waiting line however it ends (sent, given up, cancelled), letting
/// the next one check.
struct Leave<'a> {
    budget: &'a Budget,
    key: Ticket,
}

/// A waiting call: its lane, its cost and when it came.
type Ticket = (Lane, u32, u64);

impl Drop for Leave<'_> {
    fn drop(&mut self) {
        self.budget.lock().waiting.remove(&self.key);
        self.budget.changed.notify_waiters();
    }
}

impl State {
    /// Whether the waiting call `key` may spend `units` at `now`; `counted` records that it
    /// was counted as waiting for a pause.
    fn turn(
        &mut self,
        pace: Option<Pace>,
        key: Ticket,
        units: u32,
        now: Instant,
        counted: &mut bool,
    ) -> Turn {
        if let Some((resumes, until)) = self.pause.filter(|(resumes, _)| *resumes > now) {
            let wait = resumes - now;
            if let Some(episode) = &mut self.episode {
                episode.waited += u32::from(!*counted);
                episode.failed += u32::from(wait > MAX_WAIT);
            }
            *counted = true;
            return if wait > MAX_WAIT {
                Turn::GiveUp(until)
            } else {
                Turn::Wait(Some(wait))
            };
        }
        let Some(pace) = pace else { return Turn::Go };
        self.refill(pace, now);
        while self
            .spent
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) >= WINDOW)
        {
            self.spent.pop_front();
        }
        if self.waiting.first() != Some(&key) {
            return Turn::Wait(None);
        }
        let units = f64::from(units);
        let keeps = pace.keeps(key.0);
        // A call dearer than the whole bucket goes once it's full.
        let need = (units + keeps).min(pace.burst());
        let bucket = (need - self.tokens).max(0.0) / (pace.rate() * self.share);
        let minute = self.minute_wait(units, (pace.quota * MINUTE - keeps) * self.share, now);
        if bucket <= 0.0 && minute.is_zero() {
            self.tokens -= units;
            self.spent.push_back((now, units));
            return Turn::Go;
        }
        Turn::Wait(Some(Duration::from_secs_f64(bucket).max(minute)))
    }

    /// How long until `units` more fit in the last minute's spending under `limit`: once
    /// enough of it is a minute old. A call dearer than the limit goes into an empty minute.
    fn minute_wait(&self, units: f64, limit: f64, now: Instant) -> Duration {
        let used: f64 = self.spent.iter().map(|(_, units)| units).sum();
        let mut over = used + units - limit.max(units);
        if over <= 0.0 {
            return Duration::ZERO;
        }
        for (at, spent) in &self.spent {
            over -= spent;
            if over <= 0.0 {
                return (*at + WINDOW).saturating_duration_since(now);
            }
        }
        WINDOW
    }

    fn refill(&mut self, pace: Pace, now: Instant) {
        if now <= self.at {
            return;
        }
        let elapsed = (now - self.at).as_secs_f64();
        self.tokens = (self.tokens + elapsed * pace.rate() * self.share).min(pace.burst());
        self.share = (self.share + elapsed / RECOVERY.as_secs_f64()).min(1.0);
        self.at = now;
    }

    fn pushback(&mut self, now: Instant, at: Timestamp, wait: Duration) -> Timestamp {
        let pause = (
            now + wait,
            at.checked_add(SignedDuration::try_from(wait).unwrap_or(SignedDuration::MAX))
                .unwrap_or(Timestamp::MAX),
        );
        let paused = self.pause.is_some_and(|(resumes, _)| resumes > now);
        // Calls sent together are refused together: one slowdown per pause.
        if !paused {
            self.share = (self.share / 2.0).max(MIN_SHARE);
        }
        // An answer may ask for less than an earlier one did.
        let pause = self
            .pause
            .filter(|(resumes, _)| *resumes >= pause.0)
            .unwrap_or(pause);
        self.pause = Some(pause);
        self.tokens = self.tokens.min(0.0);
        self.at = self.at.max(pause.0);
        match &mut self.episode {
            Some(episode) => {
                episode.pushbacks += 1;
                episode.resumes = pause;
            }
            None => {
                log::warn!(
                    "Gmail is rate limiting this account; calls wait for it ({wait:.1?} first) and go slower until it stops"
                );
                self.episode = Some(Episode {
                    started: at,
                    resumes: pause,
                    pushbacks: 1,
                    waited: 0,
                    failed: 0,
                });
            }
        }
        pause.1
    }

    /// Ends the episode, if Gmail has been quiet long enough by `now`.
    fn settle(&mut self, now: Instant) -> Option<Episode> {
        if self
            .episode
            .as_ref()
            .is_some_and(|episode| now >= episode.resumes.0 + QUIET)
        {
            return self.episode.take();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUOTA: Quota = Quota { per_minute: 6_000 };

    fn state(tokens: f64, at: Instant) -> State {
        State {
            tokens,
            at,
            spent: VecDeque::new(),
            share: 1.0,
            waiting: BTreeSet::new(),
            next: 0,
            pause: None,
            episode: None,
        }
    }

    fn wait(lane: Lane, state: &mut State, units: u32, now: Instant) -> (Ticket, Turn) {
        state.next += 1;
        let key = (lane, units, state.next);
        state.waiting.insert(key);
        (
            key,
            state.turn(Some(Pace::new(QUOTA)), key, units, now, &mut false),
        )
    }

    #[test]
    fn a_rolling_minute_never_sees_more_than_ninety_percent_and_clicks_go_at_once() {
        let start = Instant::now();
        let pace = Pace::new(QUOTA);
        let mut bucket = state(pace.burst(), start);
        let (mut now, mut click, mut spent, mut slowest_click) =
            (start, start, Vec::new(), Duration::ZERO);
        // Sync fetches full threads back to back for five minutes; the user clicks every 7 s.
        while now < start + Duration::from_secs(300) {
            let (lane, units) = if now >= click {
                (Lane::Act, 20)
            } else {
                (Lane::Sync, 120)
            };
            let (key, turn) = wait(lane, &mut bucket, units, now);
            bucket.waiting.remove(&key);
            match turn {
                Turn::Go => {
                    spent.push((now, lane, units));
                    if lane == Lane::Act {
                        slowest_click = slowest_click.max(now - click);
                        click = now + Duration::from_secs(7);
                    }
                }
                Turn::Wait(Some(wait)) => {
                    now += wait
                        .min(click.saturating_duration_since(now))
                        .max(Duration::from_millis(1))
                }
                other => panic!("{other:?}"),
            }
        }
        for (at, _, _) in &spent {
            let minute: u32 = spent
                .iter()
                .filter(|(then, _, _)| then >= at && *then < *at + WINDOW)
                .map(|(_, _, units)| units)
                .sum();
            assert!(
                f64::from(minute) <= 0.9 * 6_000.0,
                "{minute} units in the minute from {:?}",
                *at - start
            );
        }
        assert_eq!(slowest_click, Duration::ZERO, "a click never waits");
        // Sync gets what it may: 70 % of every minute, after the first.
        let sync: u32 = spent
            .iter()
            .filter(|(at, lane, _)| *lane == Lane::Sync && *at >= start + WINDOW)
            .map(|(_, _, units)| units)
            .sum();
        assert!(
            f64::from(sync) / 4.0 > 0.65 * 6_000.0,
            "{sync} units in four minutes"
        );
    }

    #[test]
    fn earlier_lanes_go_first_and_later_ones_leave_some_behind() {
        let now = Instant::now();
        let mut bucket = state(1_000.0, now);
        // Sync must leave 1,200 units: a full thread (120) doesn't fit in 1,000.
        let (_, turn) = wait(Lane::Sync, &mut bucket, 120, now);
        assert_eq!(
            turn,
            Turn::Wait(Some(Duration::from_secs_f64(320.0 / 90.0)))
        );
        // A click arriving after it goes first, at once.
        let (_, turn) = wait(Lane::Act, &mut bucket, 20, now);
        assert_eq!(turn, Turn::Go);
        assert_eq!(bucket.tokens, 980.0);
        // While a Show call waits its turn, Sync waits behind it.
        bucket.waiting.retain(|(lane, _, _)| *lane != Lane::Act);
        let sync = *bucket.waiting.first().unwrap();
        bucket.tokens = 1_400.0;
        let (show, turn) = wait(Lane::Show, &mut bucket, 1_200, now);
        assert!(matches!(turn, Turn::Wait(Some(_))), "{turn:?}");
        assert_eq!(
            bucket.turn(Some(Pace::new(QUOTA)), sync, 120, now, &mut false),
            Turn::Wait(None)
        );
        bucket.waiting.remove(&show);
        assert_eq!(
            bucket.turn(Some(Pace::new(QUOTA)), sync, 120, now, &mut false),
            Turn::Go
        );
    }

    #[test]
    fn within_a_lane_the_cheaper_call_goes_first() {
        let now = Instant::now();
        let mut bucket = state(100.0, now);
        let (_, fetch) = wait(Lane::Act, &mut bucket, 120, now);
        assert!(matches!(fetch, Turn::Wait(Some(_))), "{fetch:?}");
        let (_, star) = wait(Lane::Act, &mut bucket, 5, now);
        assert_eq!(star, Turn::Go);
    }

    #[test]
    fn a_pushback_pauses_drains_and_slows_once_per_pause() {
        let now = Instant::now();
        let at = Timestamp::now();
        let mut bucket = state(2_700.0, now);
        let until = bucket.pushback(now, at, Duration::from_secs(2));
        // Calls sent together all come back refused: one slowdown.
        assert_eq!(
            bucket.pushback(now, at, Duration::from_secs(1)),
            until,
            "the longer pause stands"
        );
        assert_eq!((bucket.tokens, bucket.share), (0.0, 0.5));
        assert_eq!(bucket.episode.as_ref().map(|e| e.pushbacks), Some(2));

        let (_, turn) = wait(Lane::Act, &mut bucket, 20, now + Duration::from_secs(1));
        assert_eq!(turn, Turn::Wait(Some(Duration::from_secs(1))));
        assert_eq!(bucket.episode.as_ref().map(|e| e.waited), Some(1));
        // After the pause the bucket refills at half the rate, from empty.
        let later = now + Duration::from_secs(4);
        bucket.refill(Pace::new(QUOTA), later);
        assert!(
            (bucket.tokens - 2.0 * 45.0).abs() < 1.0,
            "{}",
            bucket.tokens
        );

        // A pause longer than a call waits fails it.
        bucket.pushback(later, at, Duration::from_secs(120));
        let (_, turn) = wait(Lane::Sync, &mut bucket, 120, later);
        assert!(matches!(turn, Turn::GiveUp(_)));
        assert_eq!(
            bucket.episode.as_ref().map(|e| (e.waited, e.failed)),
            Some((2, 1))
        );
    }

    #[test]
    fn throttling_ends_after_a_quiet_minute() {
        let now = Instant::now();
        let mut bucket = state(0.0, now);
        bucket.pushback(now, Timestamp::now(), Duration::from_secs(2));
        assert_eq!(bucket.settle(now + Duration::from_secs(30)), None);
        assert!(bucket.settle(now + Duration::from_secs(63)).is_some());
        assert_eq!(bucket.episode, None);
    }

    #[tokio::test]
    async fn unpaced_calls_go_at_once_but_wait_out_a_pause() {
        let budget = Budget::new(None);
        budget.spend(Lane::Sync, 120).await.expect("goes");
        budget.pushback(Duration::from_millis(200));
        let started = Instant::now();
        budget
            .spend(Lane::Act, 20)
            .await
            .expect("goes after the pause");
        assert!(
            started.elapsed() >= Duration::from_millis(150),
            "{:?}",
            started.elapsed()
        );
        budget.pushback(Duration::from_secs(60));
        assert!(budget.spend(Lane::Act, 20).await.is_err());
    }
}
