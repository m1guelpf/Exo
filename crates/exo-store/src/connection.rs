//! Whether Exo can reach Gmail, as the offline bar tells it.
//!
//! A service is marked down when a call to it finally fails, and up again when a check of it
//! gets through: every Gmail call reports how it ended, one that gets through counts as a check,
//! [`Store::check_connection`] is the poller's check and [`Store::network_changed`] the OS's
//! word that the network changed. A service isn't believed back until it has been down a while,
//! nor soon after it last came back ([`Debounce`]), so a flaky network doesn't flicker the bar.
//!
//! [`Store::check_connection`]: crate::Store::check_connection
//! [`Store::network_changed`]: crate::Store::network_changed

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Instant;

/// What's keeping Exo from Gmail: the service the offline bar names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outage {
    /// The network is down: a check after the OS reported a change got no answer, or several
    /// things are down at once.
    Offline,
    /// Google's sign-in didn't answer a token refresh.
    Auth,
    /// Gmail didn't answer.
    Gmail,
    /// Gmail is rate limiting the account for longer than a call waits.
    Throttled,
    /// Gmail says the account has no mail service.
    GmailDisabled,
}

impl Outage {
    const ALL: [Self; 5] = [
        Self::Offline,
        Self::Auth,
        Self::Gmail,
        Self::Throttled,
        Self::GmailDisabled,
    ];
}

/// The connection to Gmail as the offline bar shows it. `Default` is online.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Connection {
    /// What's down, if anything; when several things are, the one the bar names.
    pub outage: Option<Outage>,
    /// The disk holding the store had less than 10 MB free when the connection last changed
    /// while down.
    pub disk_low: bool,
}

impl Connection {
    /// Nothing is down.
    #[must_use]
    pub fn is_online(&self) -> bool {
        self.outage.is_none()
    }
}

/// How sure Exo must be that something that went down is back: it has been down at least
/// `down`, and it didn't come back less than `bounce` ago: 10 s and 60 s unless tests shorten
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Debounce {
    /// The least an outage lasts.
    pub down: Duration,
    /// How long after coming back something must stay up before it may come back again.
    pub bounce: Duration,
}

impl Default for Debounce {
    fn default() -> Self {
        Self {
            down: Duration::from_secs(10),
            bounce: Duration::from_secs(60),
        }
    }
}

/// Less free space than this, and the bar blames the disk.
const LOW_DISK: u64 = 10 << 20;

/// The account's connection: what's down, shared with whoever watches it.
pub(crate) struct Network {
    state: Mutex<State>,
    sender: watch::Sender<Connection>,
    debounce: Debounce,
    /// Where the store keeps its files, whose disk the bar may blame.
    dir: PathBuf,
}

impl Network {
    pub fn new(debounce: Debounce, dir: &Path) -> Self {
        Self {
            state: Mutex::new(State::default()),
            sender: watch::Sender::new(Connection::default()),
            debounce,
            dir: dir.to_owned(),
        }
    }

    pub fn subscribe(&self) -> watch::Receiver<Connection> {
        self.sender.subscribe()
    }

    /// A call finally failed this way.
    pub fn down(&self, outage: Outage) {
        let mut state = self.lock();
        if state.down(outage, Instant::now()) {
            self.publish(&state);
        }
    }

    /// A call got through: what's been down long enough is back. A Gmail call (`gmail`) answers
    /// for every service; another Google API's only for the network and Google's sign-in.
    pub fn reached(&self, gmail: bool) {
        let services: &[Outage] = if gmail {
            &Outage::ALL
        } else {
            &[Outage::Offline, Outage::Auth]
        };
        let mut state = self.lock();
        if state.reached(services, Instant::now(), self.debounce) {
            self.publish(&state);
        }
    }

    fn publish(&self, state: &State) {
        let outage = state.shown();
        let connection = Connection {
            outage,
            disk_low: outage.is_some() && disk_low(&self.dir),
        };
        self.sender
            .send_if_modified(|current| std::mem::replace(current, connection) != connection);
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .expect("connection lock is never poisoned")
    }
}

/// Each outage: since when it's down, and when it last came back.
#[derive(Debug, Default)]
struct State {
    services: [Service; Outage::ALL.len()],
}

#[derive(Debug, Default, Clone, Copy)]
struct Service {
    down_since: Option<Instant>,
    back_at: Option<Instant>,
}

impl State {
    /// Marks `outage` down at `now`; whether that changed anything. Already down, it stays down
    /// since then.
    fn down(&mut self, outage: Outage, now: Instant) -> bool {
        let service = &mut self.services[outage as usize];
        let changed = service.down_since.is_none();
        service.down_since.get_or_insert(now);
        changed
    }

    /// Brings back, at `now`, those of `services` that have been down for `debounce.down` and
    /// didn't come back in the last `debounce.bounce`; whether any did.
    fn reached(&mut self, services: &[Outage], now: Instant, debounce: Debounce) -> bool {
        let mut changed = false;
        for outage in services {
            let service = &mut self.services[*outage as usize];
            let Some(since) = service.down_since else {
                continue;
            };
            let bounced = service
                .back_at
                .is_some_and(|back| now.duration_since(back) < debounce.bounce);
            if now.duration_since(since) >= debounce.down && !bounced {
                *service = Service {
                    down_since: None,
                    back_at: Some(now),
                };
                changed = true;
            }
        }
        changed
    }

    /// The outage the bar names: the only one down, or with several, a disabled Gmail over
    /// everything else, which together read as being offline.
    fn shown(&self) -> Option<Outage> {
        let mut down = Outage::ALL
            .into_iter()
            .filter(|outage| self.services[*outage as usize].down_since.is_some());
        let first = down.next()?;
        Some(match down.next() {
            None => first,
            Some(_)
                if self.services[Outage::GmailDisabled as usize]
                    .down_since
                    .is_some() =>
            {
                Outage::GmailDisabled
            }
            Some(_) => Outage::Offline,
        })
    }
}

/// Whether the disk holding `dir` has less than [`LOW_DISK`] free.
#[cfg(unix)]
fn disk_low(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;

    let Ok(path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` is NUL-terminated and `stats` is written in full when the call succeeds.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return false;
    }
    // SAFETY: `statvfs` returned 0, so it filled `stats`.
    let stats = unsafe { stats.assume_init() };
    // Widened past either field, whose widths differ between platforms.
    u128::from(stats.f_bavail) * u128::from(stats.f_frsize) < u128::from(LOW_DISK)
}

#[cfg(not(unix))]
fn disk_low(_dir: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEBOUNCE: Debounce = Debounce {
        down: Duration::from_secs(10),
        bounce: Duration::from_secs(60),
    };

    fn at(start: Instant, seconds: u64) -> Instant {
        start + Duration::from_secs(seconds)
    }

    #[test]
    fn one_outage_shows_itself_and_several_read_as_offline() {
        let now = Instant::now();
        let mut state = State::default();
        assert_eq!(state.shown(), None);
        assert!(state.down(Outage::Throttled, now));
        assert!(!state.down(Outage::Throttled, now), "already down");
        assert_eq!(state.shown(), Some(Outage::Throttled));
        state.down(Outage::Gmail, now);
        assert_eq!(state.shown(), Some(Outage::Offline));
        state.down(Outage::GmailDisabled, now);
        assert_eq!(
            state.shown(),
            Some(Outage::GmailDisabled),
            "a disabled account wins"
        );
    }

    #[test]
    fn an_outage_lasts_ten_seconds_and_a_bounce_a_minute() {
        let start = Instant::now();
        let mut state = State::default();
        state.down(Outage::Gmail, start);
        assert!(
            !state.reached(&Outage::ALL, at(start, 3), DEBOUNCE),
            "down too recently"
        );
        assert_eq!(state.shown(), Some(Outage::Gmail));
        assert!(state.reached(&Outage::ALL, at(start, 10), DEBOUNCE));
        assert_eq!(state.shown(), None);

        // Down again 20 s later and answering 10 s after that: it came back too recently.
        state.down(Outage::Gmail, at(start, 30));
        assert!(
            !state.reached(&Outage::ALL, at(start, 45), DEBOUNCE),
            "bounced too recently"
        );
        assert!(!state.reached(&Outage::ALL, at(start, 69), DEBOUNCE));
        assert!(
            state.reached(&Outage::ALL, at(start, 70), DEBOUNCE),
            "a minute after it last came back"
        );
    }

    #[test]
    fn going_down_again_while_down_keeps_the_first_time() {
        let start = Instant::now();
        let mut state = State::default();
        state.down(Outage::Offline, start);
        state.down(Outage::Offline, at(start, 8));
        assert!(state.reached(&Outage::ALL, at(start, 10), DEBOUNCE));
    }

    #[test]
    fn what_answers_brings_back_everything_down_long_enough() {
        let start = Instant::now();
        let mut state = State::default();
        state.down(Outage::Offline, start);
        state.down(Outage::Gmail, at(start, 5));
        assert!(
            state.reached(&Outage::ALL, at(start, 12), DEBOUNCE),
            "the network's back"
        );
        assert_eq!(state.shown(), Some(Outage::Gmail), "Gmail went down later");
        assert!(state.reached(&Outage::ALL, at(start, 15), DEBOUNCE));
        assert_eq!(state.shown(), None);
    }

    #[test]
    fn another_apis_answer_says_only_the_network_and_sign_in_are_back() {
        let start = Instant::now();
        let mut state = State::default();
        state.down(Outage::Offline, start);
        state.down(Outage::Gmail, start);
        assert!(
            state.reached(&[Outage::Offline, Outage::Auth], at(start, 10), DEBOUNCE),
            "Calendar answered"
        );
        assert_eq!(
            state.shown(),
            Some(Outage::Gmail),
            "Gmail itself still doesn't"
        );
    }

    #[tokio::test]
    async fn the_connection_says_what_changed() {
        let network = Network::new(
            Debounce {
                down: Duration::ZERO,
                bounce: Duration::ZERO,
            },
            &std::env::temp_dir(),
        );
        let mut connection = network.subscribe();
        network.down(Outage::Throttled);
        assert!(connection.has_changed().expect("open"));
        let shown = *connection.borrow_and_update();
        assert_eq!(shown.outage, Some(Outage::Throttled));
        assert!(!shown.disk_low, "the temp directory's disk has room");
        network.down(Outage::Throttled);
        assert!(!connection.has_changed().expect("open"), "nothing new");
        network.reached(true);
        assert!(connection.borrow_and_update().is_online());
    }
}
