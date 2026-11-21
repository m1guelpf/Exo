//! Achievements, all counted on this Mac: conversations marked done, messages sent, Get Me To
//! Zero's clearing and Inbox Zero's days and weekly streak; the popups when a count reaches a
//! level, the streak's `Week N` animation and the Achievements dialog.

mod app;
mod copy;
mod dialog;
mod popup;
mod streak;

use exo_commands::{Command, Commands, Region};
use exo_store::{Achievements, Statistics};
use jiff::civil::Date;

/// The achievements on screen: the popups, the streak animation and the dialog.
#[derive(Default)]
pub struct State {
    popups: popup::Popups,
    animation: Option<streak::Animation>,
    /// The dialog's view, while it's open.
    dialog: Option<View>,
    /// Get Me To Zero's progress counted so far, by bulk action.
    cleared: std::collections::HashMap<crate::account::UndoToken, usize>,
    /// The window had the keyboard last frame.
    focused: bool,
}

impl State {
    /// The dialog's view, while it's open.
    pub fn dialog(&self) -> Option<View> {
        self.dialog
    }

    /// The streak animation has the window: the header, Inbox Zero's button and the footer are
    /// hidden.
    pub fn hides_chrome(&self, now: f64) -> bool {
        self.animation
            .is_some_and(|animation| animation.hides_chrome(now))
    }

    /// The popups' titles, oldest first. For tests.
    pub fn popups(&self) -> Vec<String> {
        self.popups.titles()
    }
}

/// What the palette and the dialog's keys do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open the dialog on this view.
    Show(View),
    /// Escape in the dialog.
    Close,
}

/// Registers the palette's `Achievements` and its categories' rows, and while the dialog is open
/// (`open`), its Escape.
pub(crate) fn register(commands: &mut Commands<crate::commands::Action>, open: bool) {
    let act = crate::commands::Action::Achievements;
    let show = |view| act(Action::Show(view));
    if open {
        const DIALOG: &[Region] = &[Region::Dialog];
        commands.add(
            Command::shortcut("ACHIEVEMENTS_ESCAPE", act(Action::Close))
                .keys(["esc"])
                .regions(DIALOG),
        );
    }
    commands.add(
        Command::new("SHOW_ACHIEVEMENTS", "Achievements", show(View::Main))
            .aliases([
                "Inbox Zero Statistics",
                "Productivity Statistics",
                "Inbox Zero Stats",
                "Productivity Stats",
                "Inbox Zero Streak",
            ])
            .default_score(0.0)
            .icon("data/diamond"),
    );
    for (id, title, view) in [
        (
            "SHOW_MARK_DONE_ACHIEVEMENTS",
            "Show Mark Done Achievements",
            View::MarkDone,
        ),
        (
            "SHOW_SEND_ACHIEVEMENTS",
            "Show Send Achievements",
            View::Send,
        ),
        (
            "SHOW_IZ_ACHIEVEMENTS",
            "Show IZ Achievements",
            View::InboxZero,
        ),
    ] {
        commands.add(
            Command::new(id, title, show(view))
                .default_score(0.0)
                .icon("data/trophy"),
        );
    }
}

/// What a popup celebrates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `markDone`.
    MarkDone,
    /// `send`.
    Send,
    /// `inboxZero`: the weekly streak.
    InboxZero,
    /// `inboxZeroDays`.
    InboxZeroDays,
    /// `gmtz`: Get Me To Zero.
    Gmtz,
}

impl Kind {
    /// Its key in `seen-thresholds`.
    fn key(self) -> &'static str {
        match self {
            Self::MarkDone => "markDone",
            Self::Send => "send",
            Self::InboxZero => "inboxZero",
            Self::InboxZeroDays => "inboxZeroDays",
            Self::Gmtz => "gmtz",
        }
    }

    /// The card it marks `New`.
    fn badge(self) -> Option<Card> {
        match self {
            Self::MarkDone => Some(Card::MarkedDone),
            Self::Send => Some(Card::SentMessages),
            Self::InboxZero => Some(Card::InboxZero),
            Self::Gmtz => Some(Card::GetMeToZero),
            Self::InboxZeroDays => None,
        }
    }

    /// The dialog's view a click on its popup opens.
    pub fn view(self) -> View {
        match self {
            Self::MarkDone | Self::Gmtz => View::MarkDone,
            Self::Send => View::Send,
            Self::InboxZero | Self::InboxZeroDays => View::InboxZero,
        }
    }
}

/// The Achievements dialog's views.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// `ACHIEVEMENTS`: one card per category.
    Main,
    /// `SHOW_IZ_ACHIEVEMENTS`.
    InboxZero,
    /// `SHOW_MARK_DONE_ACHIEVEMENTS`.
    MarkDone,
    /// `SHOW_SEND_ACHIEVEMENTS`.
    Send,
}

/// A statistic card, by its key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Card {
    InboxZero,
    CurrentStreak,
    LongestStreak,
    MarkedDone,
    GetMeToZero,
    SentMessages,
    SendsLastWeek,
    WeeklySendRecord,
}

impl Card {
    /// Its key (`INBOX_ZERO`…), which names its `New` badge.
    fn key(self) -> &'static str {
        match self {
            Self::InboxZero => "INBOX_ZERO",
            Self::CurrentStreak => "CURRENT_STREAK",
            Self::LongestStreak => "MAX_STREAK",
            Self::MarkedDone => "MARKED_DONE",
            Self::GetMeToZero => "GET_ME_TO_ZERO",
            Self::SentMessages => "SENT_MESSAGES",
            Self::SendsLastWeek => "SENDS_LAST_WEEK",
            Self::WeeklySendRecord => "WEEKLY_SENDS",
        }
    }
}

impl View {
    /// Its cards, top to bottom.
    pub fn cards(self) -> &'static [Card] {
        match self {
            Self::Main => &[Card::InboxZero, Card::MarkedDone, Card::SentMessages],
            Self::InboxZero => &[Card::InboxZero, Card::CurrentStreak, Card::LongestStreak],
            Self::MarkDone => &[Card::MarkedDone, Card::GetMeToZero],
            Self::Send => &[
                Card::SentMessages,
                Card::SendsLastWeek,
                Card::WeeklySendRecord,
            ],
        }
    }

    /// The `New` badges opening it clears.
    fn clears(self) -> &'static [Card] {
        match self {
            Self::Main => &[],
            Self::InboxZero => &[Card::InboxZero],
            Self::MarkDone => &[Card::MarkedDone, Card::GetMeToZero],
            Self::Send => &[Card::SentMessages],
        }
    }
}

/// Clears the `New` badges of the view the dialog leaves.
pub fn leave(view: View, seen: &mut Achievements) {
    for card in view.clears() {
        seen.new_badges.remove(card.key());
    }
}

/// Whether `card` shows `New`.
pub fn is_new(card: Card, seen: &Achievements) -> bool {
    seen.new_badges.contains(card.key())
}

/// A level reached: a popup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reached {
    pub kind: Kind,
    /// The count reached.
    pub completed: u64,
    /// How many more to the level after.
    pub next: u64,
}

/// A count's levels: the listed thresholds, then one every `step`.
struct Ladder {
    levels: &'static [u64],
    step: u64,
}

/// `dh`: conversations marked done.
const MARK_DONE: Ladder = Ladder {
    levels: &[50, 100, 300, 400, 1_000, 3_000, 5_000],
    step: 2_500,
};
/// `dm`: the Mark Done counts with a popup of their own.
const MARK_DONE_FUN: &[u64] = &[
    150, 260, 300, 321, 451, 747, 1_111, 1_352, 1_440, 1_543, 1_618, 1_800, 1_984, 2_001, 2_222,
    2_600, 2_909, 3_000, 3_600, 4_047, 5_555, 7_777, 8_080, 10_000, 10_101, 11_000, 11_600, 14_000,
    20_000, 24_000, 24_901, 25_000, 27_182, 29_032, 30_000, 35_000, 36_000, 42_195, 50_000, 54_321,
    60_000, 75_000, 80_000, 89_214, 99_999, 100_000, 102_015, 125_000, 148_157, 150_000, 175_000,
    186_282, 200_000, 209_117, 238_900, 250_000,
];
/// `df`: messages sent.
const SENDS: Ladder = Ladder {
    levels: &[
        25, 100, 150, 500, 1_500, 3_000, 3_500, 4_000, 4_500, 5_500, 6_000, 6_500, 7_000, 8_000,
        8_500, 9_500,
    ],
    step: 500,
};
/// The send counts with a popup of their own.
const SENDS_FUN: &[u64] = &[
    42, 64, 212, 255, 314, 365, 404, 455, 666, 777, 1_000, 1_024, 1_123, 1_234, 1_337, 1_576,
    1_800, 1_969, 1_999, 2_500, 2_718, 3_142, 3_405, 3_840, 4_500, 4_877, 5_000, 5_280, 5_387,
    7_500, 9_001, 10_000, 12_345, 12_500, 15_000, 19_500, 20_000, 25_000, 30_000, 32_292, 33_000,
    40_000, 45_000, 50_000, 55_000, 60_000, 65_000, 70_000, 75_000, 90_210, 100_000, 112_000,
    120_000, 125_000,
];
/// Days with Inbox Zero.
const DAYS: Ladder = Ladder {
    levels: &[1, 2, 3, 5, 10, 15, 20, 25, 30, 40, 50],
    step: 25,
};
/// The weekly streak.
const STREAK: Ladder = Ladder {
    levels: &[1, 4, 8, 12, 16, 26, 39, 52],
    step: 13,
};
/// Conversations Get Me To Zero cleared.
const GMTZ: Ladder = Ladder {
    levels: &[25, 50, 100, 250, 500, 750, 1_000, 2_500, 5_000],
    step: 5_000,
};

impl Ladder {
    /// How many more `value` needs for its next level.
    fn remaining(&self, value: u64) -> u64 {
        if let Some(level) = self.levels.iter().find(|&&level| value < level) {
            return level - value;
        }
        match value.div_ceil(self.step) * self.step - value {
            0 => self.step,
            left => left,
        }
    }

    fn last(&self) -> u64 {
        self.levels[self.levels.len() - 1]
    }

    /// `value` is a level, and the step to the next.
    fn level(&self, value: u64) -> Option<u64> {
        let index = self.levels.iter().position(|&level| level == value)?;
        Some(
            self.levels
                .get(index + 1)
                .map_or(self.step, |next| next - value),
        )
    }

    /// Past the list, a multiple of the step is a level too.
    fn reached(&self, value: u64) -> Option<u64> {
        if value > self.last() && value.is_multiple_of(self.step) {
            Some(self.step)
        } else {
            self.level(value)
        }
    }
}

/// A level passed: marks the category's card `New`, then shows a popup unless this device has shown
/// one for the count.
fn passed(seen: &mut Achievements, kind: Kind, completed: u64, next: u64) -> Option<Reached> {
    if let Some(card) = kind.badge() {
        seen.new_badges.insert(card.key().to_owned());
    }
    let shown = seen
        .seen_thresholds
        .entry(kind.key().to_owned())
        .or_default();
    if shown.contains(&completed) {
        return None;
    }
    shown.push(completed);
    Some(Reached {
        kind,
        completed,
        next,
    })
}

/// `count` conversations marked done.
pub fn mark_done(
    statistics: &mut Statistics,
    seen: &mut Achievements,
    count: u64,
) -> Option<Reached> {
    let before = statistics.archived_all_time;
    let after = before + count;
    statistics.archived_all_time = after;
    // The highest milestone of its own crossed.
    let fun = MARK_DONE_FUN
        .iter()
        .rev()
        .find(|&&milestone| before < milestone && milestone <= after)
        .copied()
        .unwrap_or(0);
    if after >= MARK_DONE.last() {
        let level = after / MARK_DONE.step * MARK_DONE.step;
        if before < level {
            return passed(seen, Kind::MarkDone, level, MARK_DONE.step);
        }
        return (level < fun)
            .then(|| passed(seen, Kind::MarkDone, fun, MARK_DONE.step))
            .flatten();
    }
    let crossed = MARK_DONE
        .levels
        .iter()
        .rposition(|&level| before < level && level <= after);
    let (level, next) = crossed.map_or((0, 0), |index| {
        (
            MARK_DONE.levels[index],
            MARK_DONE.levels[index + 1] - MARK_DONE.levels[index],
        )
    });
    let level = level.max(fun);
    (level != 0)
        .then(|| passed(seen, Kind::MarkDone, level, next))
        .flatten()
}

/// A message sent on `today`, counted all time and for its week.
pub fn sent(statistics: &mut Statistics, seen: &mut Achievements, today: Date) -> Option<Reached> {
    let week = monday(today);
    let counted = statistics
        .sends_week
        .as_deref()
        .and_then(|day| day.parse::<Date>().ok());
    if counted != Some(week) {
        statistics.sends_prev_week = if counted == Some(previous_monday(week)) {
            statistics.sends_this_week
        } else {
            0
        };
        statistics.sends_week = Some(week.to_string());
        statistics.sends_this_week = 0;
    }
    statistics.sends_this_week += 1;
    statistics.sends_week_max = statistics.sends_week_max.max(statistics.sends_this_week);
    statistics.sends_all_time += 1;
    let sends = statistics.sends_all_time;
    if SENDS_FUN.contains(&sends) {
        return passed(seen, Kind::Send, sends, 0);
    }
    SENDS
        .reached(sends)
        .and_then(|next| passed(seen, Kind::Send, sends, next))
}

/// `count` conversations Get Me To Zero cleared.
pub fn cleared(
    statistics: &mut Statistics,
    seen: &mut Achievements,
    count: u64,
) -> Option<Reached> {
    let before = statistics.gmtz_archived_all_time;
    let after = before + count;
    statistics.gmtz_archived_all_time = after;
    let last = GMTZ.last();
    if after >= last {
        let level = if before < last {
            last
        } else {
            before.div_ceil(GMTZ.step) * GMTZ.step
        };
        return (before < level && level <= after)
            .then(|| passed(seen, Kind::Gmtz, level, GMTZ.step))
            .flatten();
    }
    let index = GMTZ
        .levels
        .iter()
        .position(|&level| before < level && level <= after)?;
    passed(
        seen,
        Kind::Gmtz,
        GMTZ.levels[index],
        GMTZ.levels[index + 1] - GMTZ.levels[index],
    )
}

/// What reaching Inbox Zero counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hit {
    /// The first Inbox Zero of the week: the streak moved on, and its animation plays.
    pub week: bool,
    /// A popup for a level reached.
    pub reached: Option<Reached>,
}

impl From<Hit> for Option<Reached> {
    fn from(hit: Hit) -> Self {
        hit.reached
    }
}

/// Inbox Zero in Inbox or Important on `today`: the week's first moves the streak on, and each
/// day's first counts the day.
pub fn hit_inbox_zero(statistics: &mut Statistics, seen: &mut Achievements, today: Date) -> Hit {
    let week = monday(today);
    let parse = |day: &Option<String>| day.as_deref().and_then(|day| day.parse::<Date>().ok());
    let new_week = parse(&statistics.last_inbox_zero_week).is_none_or(|last| last < week);
    let new_day = parse(&statistics.last_inbox_zero_day).is_none_or(|last| last < today);
    if new_day {
        statistics.inbox_zero_days_all_time += 1;
        statistics.last_inbox_zero_day = Some(today.to_string());
    }
    if new_week {
        let continued = parse(&statistics.last_inbox_zero_week) == Some(previous_monday(week));
        statistics.inbox_zero_streak_week_current = if continued {
            statistics.inbox_zero_streak_week_current + 1
        } else {
            1
        };
        // The record is the longest streak yet, a first one included.
        statistics.inbox_zero_streak_week_max = statistics
            .inbox_zero_streak_week_max
            .max(statistics.inbox_zero_streak_week_current);
        statistics.last_inbox_zero_week = Some(week.to_string());
    }
    // The streak's level, else the days'.
    let streak = u64::from(statistics.inbox_zero_streak_week_current);
    let days = u64::from(statistics.inbox_zero_days_all_time);
    let reached = match (
        new_week.then(|| STREAK.reached(streak)).flatten(),
        new_day.then(|| DAYS.reached(days)).flatten(),
    ) {
        (Some(next), _) => passed(seen, Kind::InboxZero, streak, next),
        (None, Some(next)) => passed(seen, Kind::InboxZeroDays, days, next),
        (None, None) => None,
    };
    Hit {
        week: new_week,
        reached,
    }
}

/// Whether reaching Inbox Zero on `today` would count anything: a new week or a new day.
pub fn would_count(statistics: &Statistics, today: Date) -> bool {
    let parse = |day: &Option<String>| day.as_deref().and_then(|day| day.parse::<Date>().ok());
    parse(&statistics.last_inbox_zero_week).is_none_or(|last| last < monday(today))
        || parse(&statistics.last_inbox_zero_day).is_none_or(|last| last < today)
}

/// Messages sent the week before `today`'s.
pub fn sends_last_week(statistics: &Statistics, today: Date) -> u64 {
    let week = monday(today);
    match statistics
        .sends_week
        .as_deref()
        .and_then(|day| day.parse::<Date>().ok())
    {
        Some(counted) if counted == week => statistics.sends_prev_week,
        Some(counted) if counted == previous_monday(week) => statistics.sends_this_week,
        _ => 0,
    }
}

/// The Monday of `day`'s week.
fn monday(day: Date) -> Date {
    day.checked_sub(jiff::Span::new().days(i64::from(day.weekday().to_monday_zero_offset())))
        .unwrap_or(day)
}

fn previous_monday(monday: Date) -> Date {
    monday
        .checked_sub(jiff::Span::new().weeks(1))
        .unwrap_or(monday)
}

/// How many more `card`'s count needs for its next level, for its subheader.
fn remaining(card: Card, value: u64) -> u64 {
    match card {
        Card::InboxZero => DAYS.remaining(value),
        Card::CurrentStreak => STREAK.remaining(value),
        Card::MarkedDone => MARK_DONE.remaining(value),
        Card::GetMeToZero => GMTZ.remaining(value),
        Card::SentMessages => SENDS.remaining(value),
        Card::LongestStreak | Card::SendsLastWeek | Card::WeeklySendRecord => 0,
    }
}

/// `count` with its thousands grouped in English: `1,000`.
pub fn grouped(count: u64) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// A popup's title.
pub fn title(reached: Reached) -> String {
    let n = grouped(reached.completed);
    match reached.kind {
        Kind::Send if reached.completed == 1 => "You sent your first message!".to_owned(),
        Kind::Send => format!("You've sent {n} messages!"),
        Kind::MarkDone => format!("You've cleared {n} conversations!"),
        Kind::InboxZero if reached.completed == 1 => {
            "You've started an Inbox Zero streak!".to_owned()
        }
        Kind::InboxZero => format!("You've hit Inbox Zero {n} weeks in a row!"),
        Kind::InboxZeroDays => format!("You've hit Inbox Zero on {n} days!"),
        Kind::Gmtz => format!("You've cleared over {n} conversations!"),
    }
}

/// A popup's body, a line each; `pick` chooses from a pool, given its size.
pub fn body(reached: Reached, pick: impl FnOnce(usize) -> usize) -> Vec<String> {
    let next = grouped(reached.next);
    let special = |table: &[(u64, &'static [&'static str])]| {
        table
            .iter()
            .find(|(count, _)| *count == reached.completed)
            .map(|(_, lines)| *lines)
    };
    let lines = |lines: &[&str]| lines.iter().map(|line| (*line).to_owned()).collect();
    match reached.kind {
        Kind::Send => match special(copy::SEND_SPECIAL) {
            Some(message) => lines(message),
            None if reached.completed <= 200 => vec![format!(
                "Nice job! Send {next} more to reach the next level."
            )],
            None => lines(copy::SEND_POOL[pick(copy::SEND_POOL.len())]),
        },
        Kind::MarkDone => match special(copy::CLEAR_SPECIAL) {
            Some(message) => lines(message),
            None if reached.completed <= 500 => vec![format!(
                "Nice job! Clear {next} more to reach the next level."
            )],
            None => lines(copy::CLEAR_POOL[pick(copy::CLEAR_POOL.len())]),
        },
        Kind::InboxZero => match reached.completed {
            1 => lines(&["Hit Inbox Zero next week to build your streak!"]),
            4 => lines(&["Nice! You've conquered your inbox for a whole month."]),
            8 => lines(&["Two months and counting! You're on top of your email."]),
            12 => lines(&["Nearly three months! You're saving countless hours."]),
            16 => lines(&[
                "Amazing! Keep it up — you're close to",
                "half a year of Inbox Zero!",
            ]),
            26 => lines(&["Congratulations! You've hit Inbox Zero for half a year!"]),
            40 => lines(&[
                "Impressive! Just 12 more weeks to",
                "reach a year of Inbox Zero.",
            ]),
            52 => lines(&["A whole year of Inbox Zero — what an accomplishment!"]),
            _ => vec![
                format!("Nice job! Hit Inbox Zero {next} more weeks in a row to"),
                "reach the next level.".to_owned(),
            ],
        },
        Kind::InboxZeroDays => match reached.completed {
            1 => lines(&["Well done! Come back tomorrow and hit Inbox Zero again."]),
            2 => lines(&["Two days of clarity and focus — remember this feeling!"]),
            3 => lines(&["You've hit your stride."]),
            5 => lines(&["You've made time to focus on what matters."]),
            10 => lines(&["Enjoy the zen — and Hit Inbox Zero again tomorrow!"]),
            _ => vec![
                format!("Nice job! Hit Inbox Zero {next} more times to"),
                "reach the next level.".to_owned(),
            ],
        },
        Kind::Gmtz => vec![
            format!("Nice job! Use Get Me To Zero and clear {next} more"),
            "to reach the next level.".to_owned(),
        ],
    }
}

/// Under 1,000 as is, then `1k`, `1.5k`, `150k`, `1M`…, truncated.
pub fn abbreviated(value: u64) -> String {
    if value < 1_000 {
        return value.to_string();
    }
    let mut unit = 1_000;
    for suffix in ["k", "M", "B", "T"] {
        if value < 1_000 * unit {
            let tenth = unit / 10;
            let truncated = value / tenth * tenth;
            if truncated.is_multiple_of(unit) {
                return format!("{}{suffix}", truncated / unit);
            }
            let scaled = truncated as f64 / unit as f64;
            return if scaled > 100.0 {
                format!("{}{suffix}", scaled.floor())
            } else {
                format!("{scaled:.1}{suffix}")
            };
        }
        unit *= 1_000;
    }
    value.to_string()
}

#[cfg(test)]
mod tests;
