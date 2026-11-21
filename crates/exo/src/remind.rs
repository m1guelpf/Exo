//! Remind me (`H`): the picker's suggestions and typed times, its `if no reply` / `regardless`
//! dropdown and its footer, what choosing does, and the toasts and hint around it. The store keeps
//! the reminders and brings their threads back (`exo_store`'s `reminders`).
//!
//! The picker has two modes: Snooze (`H` on threads, which marks them done) and Draft (compose's
//! ⌘⇧H and Remind button, and the follow-up hint's `Set reminder` on a message just sent), whose
//! rows are a reminder's (`in 2 days`, `on Monday`). `H` on a sent thread is Snooze too.

use exo_commands::{Command, Dropdown, Hint, SubPalette, command_score};
use exo_store::{
    DraftReminder, PreviousDropdown, PreviousReminder, PreviousSuggestion, ReminderSettings,
    ReminderStatus, TriggerCondition,
};
use jiff::{Timestamp, ToSpan as _, Zoned, civil::Weekday};

use crate::{
    Mail, actionable, commands,
    format::{dates, friendly},
    mailbox::ThreadSummary,
    time_suggest::{self, Options},
};

/// The dropdown's options, as `PreviousReminder` and the settings name them.
const IF_NO_REPLY: &str = "if no reply";
const REGARDLESS: &str = "regardless";
/// The default reminder time: a reminder at it is "at the default time", whatever the setting
/// says.
const DEFAULT_HOUR: i8 = 8;

/// What the picker reminds (`Reminder`'s mode).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Threads, marked done until then (`SNOOZE`).
    Snooze(Vec<String>),
    /// A draft being written, whose reminder goes with it when it's sent (`DRAFT`).
    Draft(String),
    /// A draft just sent (the follow-up hint's `Set reminder`): `DRAFT`'s rows, then the
    /// reminder on its thread, which stays where it is (not marked done).
    Sent(String),
}

impl Target {
    /// Snooze's rows or a reminder's.
    fn snoozes(&self) -> bool {
        matches!(self, Self::Snooze(_))
    }
}

/// What Remind me does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open the picker for the actionable threads (`H`, the palette's `Remind Me`); `button`
    /// when a clock button did, which hints at `H`.
    Open {
        /// A clock button was clicked.
        button: bool,
    },
    /// A row of the picker: remind `target` at `at`, from the row titled `title`.
    Set {
        /// What the picker was opened for.
        target: Target,
        /// When.
        at: Timestamp,
        /// The row's title, for `last used`.
        title: String,
        /// The dropdown says `regardless`.
        regardless: bool,
    },
    /// `Remove Reminder`: of `threads`, or the actionable ones.
    Remove {
        /// The picker's threads; `None` for the palette's row.
        threads: Option<Vec<String>>,
        /// `Remove Reminder & Move to Inbox`.
        to_inbox: bool,
    },
    /// `Remove Reminder` on a draft: it's sent without one. From the picker's footer it's `manual`,
    /// which keeps Automatic Reminders off it while it's open; compose's command isn't.
    RemoveFromDraft {
        /// The draft.
        draft: String,
        /// The picker's footer.
        manual: bool,
    },
}

/// A row before it's a command: its title and time, and the zone it was typed in.
struct Suggestion {
    title: String,
    at: Timestamp,
    converted: bool,
}

/// `now`'s day `days` on at `hour`:`minute`.
fn days_on(now: &Zoned, days: i64, hour: i8, minute: i8) -> Timestamp {
    let date = now.date().checked_add(days.days()).unwrap_or(now.date());
    date.at(hour, minute, 0, 0)
        .to_zoned(now.time_zone().clone())
        .map_or(now.timestamp(), |zoned| zoned.timestamp())
}

/// `days` on from `now` at `hour`:`minute`, counting only Monday to Friday when `weekdays`
/// (a day at a time from today, so from a weekend the first is Monday).
pub(crate) fn in_days(now: &Zoned, days: u32, weekdays: bool, hour: i8, minute: i8) -> Timestamp {
    if !weekdays {
        return days_on(now, i64::from(days), hour, minute);
    }
    let (mut date, mut left) = (now.date(), days);
    while left > 0 {
        date = date.tomorrow().unwrap_or(date);
        if !matches!(date.weekday(), Weekday::Saturday | Weekday::Sunday) {
            left -= 1;
        }
    }
    date.at(hour, minute, 0, 0)
        .to_zoned(now.time_zone().clone())
        .map_or(now.timestamp(), |zoned| zoned.timestamp())
}

/// A count of days as rows say it: `2 days`, `1 weekday`, `3 weekdays`.
fn days_copy(days: u32, weekdays: bool) -> String {
    format!(
        "{days} {}{}",
        if weekdays { "weekday" } else { "day" },
        if days > 1 { "s" } else { "" }
    )
}

/// The next `weekday` at `hour`:`minute`, a week on when it's today.
fn next(now: &Zoned, weekday: Weekday, hour: i8, minute: i8) -> Timestamp {
    let ahead = i64::from(weekday.to_sunday_zero_offset() - now.weekday().to_sunday_zero_offset());
    days_on(
        now,
        if ahead <= 0 { ahead + 7 } else { ahead },
        hour,
        minute,
    )
}

/// The default rows at the settings' time, after `last used` while that's still ahead.
/// Snooze mode's (`snoozes`): `tomorrow`, `next week` (Monday), `this weekend` (`next weekend` on
/// a Saturday or Sunday), `someday` and `never`. Draft mode's: `in {n} days` (or `weekdays`, with
/// Weekdays Only) as the settings say, `tomorrow`, `on Monday` and the weekend.
fn defaults(
    now: &Zoned,
    settings: &ReminderSettings,
    previous: Option<&PreviousReminder>,
    snoozes: bool,
) -> Vec<Suggestion> {
    let (hour, minute) = (settings.hour, settings.minute);
    let weekend = if matches!(now.weekday(), Weekday::Saturday | Weekday::Sunday) {
        "next weekend"
    } else {
        "this weekend"
    };
    let row = |title: &str, at: Timestamp| Suggestion {
        title: title.into(),
        at,
        converted: false,
    };
    let sentinel = |ms: i64| Timestamp::from_millisecond(ms).unwrap_or_default();
    let (monday, saturday) = (
        next(now, Weekday::Monday, hour, minute),
        next(now, Weekday::Saturday, hour, minute),
    );
    let mut rows = if snoozes {
        vec![
            row("tomorrow", days_on(now, 1, hour, minute)),
            row("next week", monday),
            row(weekend, saturday),
            row("someday", sentinel(dates::SOMEDAY)),
            row("never", sentinel(dates::NEVER)),
        ]
    } else {
        // `reminders/days || 2`.
        let days = if settings.days == 0 { 2 } else { settings.days };
        let weekdays = settings.weekdays_only();
        let ahead = row(
            &format!("in {}", days_copy(days, weekdays)),
            in_days(now, days, weekdays, hour, minute),
        );
        vec![
            ahead,
            row("tomorrow", days_on(now, 1, hour, minute)),
            row("on Monday", monday),
            row(weekend, saturday),
        ]
    };
    if let Some(previous) = previous.filter(|previous| previous.suggestion.time > now.timestamp()) {
        rows.insert(0, row("last used", previous.suggestion.time));
    }
    rows
}

/// The rows listed only when typed for: `forever`, `a few hours` (four hours on, on the hour) and
/// `later today` (`later` when that's tomorrow).
fn hidden(now: &Zoned) -> Vec<Suggestion> {
    let later = now
        .checked_add(4.hours())
        .ok()
        .and_then(|at| {
            at.with()
                .minute(0)
                .second(0)
                .subsec_nanosecond(0)
                .build()
                .ok()
        })
        .unwrap_or_else(|| now.clone());
    let row = |title: &str, at: Timestamp| Suggestion {
        title: title.into(),
        at,
        converted: false,
    };
    vec![
        row(
            "forever",
            Timestamp::from_millisecond(dates::SOMEDAY).unwrap_or_default(),
        ),
        row("a few hours", later.timestamp()),
        row(
            if later.date() == now.date() {
                "later today"
            } else {
                "later"
            },
            later.timestamp(),
        ),
    ]
}

/// The picker's rows: the defaults; with a query, the parser's times first, then the
/// defaults and hidden rows it fuzzy-matches, each time once.
fn suggestions(
    query: &str,
    now: &Zoned,
    settings: &ReminderSettings,
    previous: Option<&PreviousReminder>,
    snoozes: bool,
) -> Vec<Suggestion> {
    let mut rows = defaults(now, settings, previous, snoozes);
    let query = query.trim();
    if query.is_empty() {
        return rows;
    }
    let matches = |row: &Suggestion| command_score(&row.title, query) > 0.0;
    rows.retain(matches);
    let mut once: Vec<Timestamp> = Vec::new();
    rows.extend(hidden(now).into_iter().filter(matches).filter(|row| {
        !once.contains(&row.at) && {
            once.push(row.at);
            true
        }
    }));
    // Typed days count weekdays with Weekdays Only, in every mode.
    let options = Options {
        default_time: (settings.hour, settings.minute),
        business_days_only: settings.weekdays_only(),
        ..Options::default()
    };
    let mut parsed: Vec<Suggestion> = Vec::new();
    for suggestion in time_suggest::suggest(query, &options, now) {
        if !parsed.iter().any(|row| row.at == suggestion.start) {
            parsed.push(Suggestion {
                title: suggestion.label,
                at: suggestion.start,
                converted: suggestion.zone.is_some(),
            });
        }
    }
    rows.retain(|row| !parsed.iter().any(|typed| typed.at == row.at));
    parsed.extend(rows);
    parsed
}

/// A row's time on its right, upper-cased.
fn side_title(row: &Suggestion, now: &Zoned) -> String {
    let millis = row.at.as_millisecond();
    let text = if millis == dates::SOMEDAY || millis == dates::NEVER {
        "¯\\_(ツ)_/¯".to_owned()
    } else if row.converted {
        friendly::time_suggestor(row.at, now)
    } else {
        friendly::exact_friendly_compact(row.at, now)
    };
    text.to_uppercase()
}

/// Snooze mode's footer for `threads`, whose rows are `actionable`: removing the reminder, and
/// moving them back when some aren't in the inbox.
pub fn snooze_footer(
    threads: &[String],
    actionable: &[&ThreadSummary],
) -> Option<Command<commands::Action>> {
    let reminded = actionable.iter().any(|thread| thread.has_reminder());
    let out = actionable.iter().any(|thread| thread.done);
    reminded.then(|| {
        let (title, to_inbox) = if out {
            ("Remove Reminder & Move to Inbox", true)
        } else {
            ("Remove Reminder", false)
        };
        Command::new(
            "REMOVE_REMINDER",
            title,
            commands::Action::Remind(Action::Remove {
                threads: Some(threads.to_vec()),
                to_inbox,
            }),
        )
    })
}

/// Draft mode's footer: `Remove Reminder` for a draft that has one.
pub fn draft_footer(draft: &str, reminded: bool) -> Option<Command<commands::Action>> {
    reminded.then(|| {
        Command::new(
            "REMOVE_REMINDER",
            "Remove Reminder",
            commands::Action::Remind(Action::RemoveFromDraft {
                draft: draft.to_owned(),
                manual: true,
            }),
        )
    })
}

/// The picker for `target`, with `footer` under its rows.
pub fn palette(
    target: Target,
    footer: Option<Command<commands::Action>>,
    now: Zoned,
    settings: ReminderSettings,
    previous: Option<PreviousReminder>,
) -> SubPalette<commands::Action> {
    // The saved trigger first.
    let options = match settings.trigger_condition {
        TriggerCondition::IfNoReply => vec![IF_NO_REPLY, REGARDLESS],
        TriggerCondition::Regardless => vec![REGARDLESS, IF_NO_REPLY],
    };
    let regardless_at = options
        .iter()
        .position(|option| *option == REGARDLESS)
        .unwrap_or_default();
    // ⌘⇧H cycles it; `regardless` picked from its menu hints at that.
    let dropdown = Dropdown::new(options, 0, "mod+shift+h").hint_on(
        regardless_at,
        Hint::new()
            .text("Hit")
            .keys(["⌘", "shift", "H"])
            .text("to toggle this reminder setting!"),
    );
    let snoozes = target.snoozes();
    SubPalette::ordered_with(
        "SNOOZE",
        "Remind me",
        "data/snooze",
        dropdown,
        move |query, choice| {
            suggestions(query, &now, &settings, previous.as_ref(), snoozes)
                .into_iter()
                .enumerate()
                .map(|(index, row)| {
                    let side = side_title(&row, &now);
                    let action = Action::Set {
                        target: target.clone(),
                        at: row.at,
                        title: row.title.clone(),
                        regardless: choice == regardless_at,
                    };
                    Command::new(
                        format!("SNOOZE_{index}"),
                        row.title,
                        commands::Action::Remind(action),
                    )
                    .side_title(side)
                })
                .collect()
        },
    )
    .placeholder("Try: 8 am, 3 days, aug 7")
    .footer(move |_| footer.clone())
}

/// The toast once a reminder is set: `Reminder set: {when}{ if no reply}.`, the date without the
/// time when it's the default time.
pub fn toast(at: Timestamp, regardless: bool, now: &Zoned) -> String {
    let local = at.to_zoned(now.time_zone().clone());
    let when = if is_default_time(&local) {
        friendly::exact_friendly_omit_time(at, now)
    } else {
        friendly::exact_friendly(at, now)
    };
    format!(
        "Reminder set: {when}{}.",
        if regardless { "" } else { " if no reply" }
    )
}

/// Whether `at` is at the default time: [`DEFAULT_HOUR`], not the setting's.
fn is_default_time(at: &Zoned) -> bool {
    at.hour() == DEFAULT_HOUR && at.minute() == 0
}

/// The clock button's status line: `Returning: {when}{ at h:mm AM}{ if no reply}`, `Returned:
/// {relative}` or `Removed as {name} replied`.
pub fn status(reminder: &ReminderStatus, now: &Zoned) -> String {
    match reminder {
        ReminderStatus::Set { at, keep_on_reply } => {
            let at = at.to_zoned(now.time_zone().clone());
            let month = |full: bool| {
                let name = at.strftime("%B").to_string();
                if full { name } else { name[..3].to_owned() }
            };
            let week = now
                .checked_add(7.days())
                .map_or(now.timestamp(), |week| week.timestamp());
            let (short, full) = if at.date() == now.date() {
                ("today".to_owned(), "today".to_owned())
            } else if now
                .date()
                .tomorrow()
                .is_ok_and(|tomorrow| tomorrow == at.date())
            {
                ("tomorrow".to_owned(), "tomorrow".to_owned())
            } else if at.timestamp() < week {
                let weekday = at.strftime("%A").to_string();
                (weekday.clone(), weekday)
            } else if at.year() == now.year() {
                (
                    format!("{} {}", month(false), dates::ordinal(at.day())),
                    format!("{} {}", month(true), dates::ordinal(at.day())),
                )
            } else {
                (
                    format!(
                        "{} {}, {}",
                        month(false),
                        dates::ordinal(at.day()),
                        at.year()
                    ),
                    format!(
                        "{} {}, {}",
                        month(true),
                        dates::ordinal(at.day()),
                        at.year()
                    ),
                )
            };
            let when = if is_default_time(&at) {
                full
            } else {
                format!("{short} at {}", dates::time(&at))
            };
            format!(
                "Returning: {when}{}",
                if *keep_on_reply { "" } else { " if no reply" }
            )
        }
        ReminderStatus::Returned { at, .. } => {
            format!("Returned: {}", dates::relative(*at, now.timestamp()))
        }
        ReminderStatus::Aborted { by } => {
            let name = by
                .name
                .as_deref()
                .filter(|name| !name.is_empty())
                .unwrap_or(&by.email);
            format!("Removed as {name} replied")
        }
    }
}

impl Mail {
    /// Carries out a Remind me `action`.
    pub(crate) fn remind(&mut self, action: Action, now: f64) {
        match action {
            Action::Open { button } => {
                // `H` on a select-all reaching rows not loaded.
                if self.refuse_unloaded("set a reminder") {
                    return;
                }
                let rows = actionable(self.thread.as_ref(), &self.inbox);
                // Not for drafts alone.
                if rows.is_empty() || rows.iter().all(|thread| thread.draft_only) {
                    return;
                }
                if button {
                    self.commands.hint(
                        Hint::new()
                            .text("Next time, hit")
                            .keys(["H"])
                            .text("to set a Reminder instantly!"),
                    );
                }
                let threads: Vec<String> = rows.iter().map(|thread| thread.id.clone()).collect();
                let footer = snooze_footer(&threads, &rows);
                self.open_remind_me(Target::Snooze(threads), footer);
            }
            Action::Set {
                target,
                at,
                title,
                regardless,
            } => {
                let reminder = DraftReminder {
                    at,
                    keep_on_reply: regardless,
                    automatic: false,
                };
                match target {
                    Target::Snooze(threads) => {
                        let text = toast(at, regardless, &self.now.clone());
                        let mutation = exo_store::Mutation::Remind {
                            at,
                            keep_on_reply: regardless,
                            snooze: true,
                        };
                        self.triage_threads(&threads, mutation, Some(text), 0, now);
                    }
                    Target::Draft(draft) => self.set_draft_reminder(&draft, Some(reminder)),
                    Target::Sent(draft) => self.remind_sent(&draft, reminder),
                }
                let value = if regardless {
                    "KEEP_ON_REPLY"
                } else {
                    "IF_NO_REPLY"
                };
                let previous = PreviousReminder {
                    suggestion: PreviousSuggestion { title, time: at },
                    dropdown: PreviousDropdown {
                        value: value.into(),
                    },
                };
                self.account
                    .update_settings(|settings| settings.previous_reminder = Some(previous));
            }
            Action::Remove { threads, to_inbox } => {
                let ids = threads.unwrap_or_else(|| self.actionable_ids());
                let count = self
                    .inbox
                    .threads()
                    .iter()
                    .filter(|thread| ids.contains(&thread.id) && thread.has_reminder())
                    .count();
                let text = if count > 1 {
                    "Reminders removed."
                } else {
                    "Reminder removed."
                };
                self.triage_threads(
                    &ids,
                    exo_store::Mutation::CancelReminder { to_inbox },
                    Some(text.into()),
                    0,
                    now,
                );
            }
            Action::RemoveFromDraft { draft, manual } => {
                self.set_draft_reminder(&draft, None);
                if manual {
                    self.reminder_removed(&draft);
                }
            }
        }
    }

    /// Opens the picker for `target`, `footer` under its rows.
    pub(crate) fn open_remind_me(
        &mut self,
        target: Target,
        footer: Option<Command<commands::Action>>,
    ) {
        let settings = self.account.settings();
        let palette = palette(
            target,
            footer,
            self.now.clone(),
            settings.reminders.clone(),
            settings.previous_reminder.clone(),
        );
        self.commands.open_sub_palette(palette);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// gmail-mock's now: Tuesday, November 17th 2026, 9:00 AM in New York.
    fn now() -> Zoned {
        "2026-11-17T09:00[America/New_York]".parse().expect("now")
    }

    fn picker(previous: Option<PreviousReminder>) -> SubPalette<commands::Action> {
        palette(
            Target::Snooze(vec!["t1".into()]),
            None,
            now(),
            ReminderSettings::default(),
            previous,
        )
    }

    fn rows_for(query: &str, snoozes: bool) -> Vec<(String, String)> {
        suggestions(query, &now(), &ReminderSettings::default(), None, snoozes)
            .iter()
            .map(|row| (row.title.clone(), side_title(row, &now())))
            .collect()
    }

    fn rows(query: &str) -> Vec<(String, String)> {
        rows_for(query, true)
    }

    #[test]
    fn the_defaults_are_snoozes_mornings() {
        let owned = |rows: &[(&str, &str)]| {
            rows.iter()
                .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            rows(""),
            owned(&[
                ("tomorrow", "WED, 8:00 AM"),
                ("next week", "MON, 8:00 AM"),
                ("this weekend", "SAT, 8:00 AM"),
                ("someday", "¯\\_(ツ)_/¯"),
                ("never", "¯\\_(ツ)_/¯")
            ])
        );
        let at: Timestamp = "2026-11-20T15:00:00Z".parse().expect("a time");
        let previous = PreviousReminder {
            suggestion: PreviousSuggestion {
                title: "fri 10am".into(),
                time: at,
            },
            dropdown: PreviousDropdown {
                value: "IF_NO_REPLY".into(),
            },
        };
        assert_eq!(
            picker(Some(previous))
                .titles("")
                .first()
                .map(String::as_str),
            Some("last used")
        );
    }

    #[test]
    fn a_drafts_defaults_are_a_reminders() {
        // The rows ⌘⇧H lists in a reply.
        let owned = |rows: &[(&str, &str)]| {
            rows.iter()
                .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            rows_for("", false),
            owned(&[
                ("in 2 days", "THU, 8:00 AM"),
                ("tomorrow", "WED, 8:00 AM"),
                ("on Monday", "MON, 8:00 AM"),
                ("this weekend", "SAT, 8:00 AM")
            ])
        );
        let settings = ReminderSettings {
            hour: 9,
            minute: 30,
            days: 5,
            ..ReminderSettings::default()
        };
        let late = suggestions("", &now(), &settings, None, false);
        assert_eq!(
            (
                late[0].title.as_str(),
                side_title(&late[0], &now()).as_str()
            ),
            ("in 5 days", "SUN, 9:30 AM"),
            "the setting's days and time"
        );
        let weekdays = ReminderSettings {
            business_days_only: Some(true),
            ..settings
        };
        let late = suggestions("", &now(), &weekdays, None, false);
        assert_eq!(
            (
                late[0].title.as_str(),
                side_title(&late[0], &now()).as_str()
            ),
            ("in 5 weekdays", "TUE, NOV 24, 9:30 AM"),
            "Weekdays Only"
        );
        let one = suggestions(
            "",
            &now(),
            &ReminderSettings {
                days: 1,
                business_days_only: Some(true),
                ..ReminderSettings::default()
            },
            None,
            false,
        );
        assert_eq!(one[0].title, "in 1 weekday");
        let at: Timestamp = "2026-11-20T15:00:00Z".parse().expect("a time");
        let previous = PreviousReminder {
            suggestion: PreviousSuggestion {
                title: "fri 10am".into(),
                time: at,
            },
            dropdown: PreviousDropdown {
                value: "IF_NO_REPLY".into(),
            },
        };
        let titles: Vec<String> = palette(
            Target::Draft("d".into()),
            draft_footer("d", true),
            now(),
            ReminderSettings::default(),
            Some(previous),
        )
        .titles("");
        assert_eq!(
            titles,
            [
                "last used",
                "in 2 days",
                "tomorrow",
                "on Monday",
                "this weekend",
                "Remove Reminder"
            ]
        );
        let monday: Zoned = "2026-11-23T09:00[America/New_York]".parse().expect("now");
        let rows = suggestions("", &monday, &ReminderSettings::default(), None, false);
        assert_eq!(
            side_title(&rows[2], &monday),
            "MON, NOV 30, 8:00 AM",
            "on a Monday, the next one"
        );
    }

    #[test]
    fn weekdays_skip_the_weekend() {
        let at = |text: &str| text.parse::<Zoned>().expect("a time");
        let ahead = |from: &str, days| {
            in_days(&at(from), days, true, 8, 0)
                .to_zoned(at(from).time_zone().clone())
                .date()
                .to_string()
        };
        // The dialog's own example: 2 days on Friday is Tuesday.
        assert_eq!(ahead("2026-11-20T09:00[America/New_York]", 2), "2026-11-24");
        assert_eq!(
            ahead("2026-11-21T09:00[America/New_York]", 1),
            "2026-11-23",
            "Saturday's first is Monday"
        );
        assert_eq!(ahead("2026-11-22T09:00[America/New_York]", 2), "2026-11-24");
        // Typed days count them too, `weeks` not.
        let weekdays = ReminderSettings {
            business_days_only: Some(true),
            ..ReminderSettings::default()
        };
        let friday = at("2026-11-20T09:00[America/New_York]");
        let typed = |query: &str| {
            suggestions(query, &friday, &weekdays, None, true)
                .first()
                .map(|row| (row.title.clone(), side_title(row, &friday)))
        };
        assert_eq!(
            typed("3 days"),
            Some(("in 3 weekdays".to_owned(), "WED, 8:00 AM".to_owned()))
        );
        assert_eq!(
            typed("1 week"),
            Some(("in 1 week".to_owned(), "FRI, NOV 27, 8:00 AM".to_owned()))
        );
    }

    #[test]
    fn typing_lists_parsed_times_then_the_rows_it_matches() {
        // The rows for each query, at the same now.
        let cases: [(&str, &[(&str, &str)]); 6] = [
            ("3 days", &[("in 3 days", "FRI, 8:00 AM")]),
            ("later", &[("later today", "1:00 PM")]),
            (
                "dec 1",
                &[
                    ("on December 1st", "TUE, DEC 01, 8:00 AM"),
                    ("on December 10th", "THU, DEC 10, 8:00 AM"),
                    ("on December 11th", "FRI, DEC 11, 8:00 AM"),
                ],
            ),
            ("fri 10am", &[("on Friday at 10 am", "FRI, 10:00 AM")]),
            (
                "2pm pst",
                &[("today at 2 pm PST", "TUE, NOV 17, 5:00 PM EST")],
            ),
            (
                "for",
                &[
                    ("in a minute", "9:01 AM"),
                    ("in an hour", "10:00 AM"),
                    ("in a day", "WED, 8:00 AM"),
                    ("forever", "¯\\_(ツ)_/¯"),
                    ("a few hours", "1:00 PM"),
                ],
            ),
        ];
        for (query, expected) in cases {
            let expected: Vec<(String, String)> = expected
                .iter()
                .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
                .collect();
            assert_eq!(rows(query), expected, "{query}");
        }
        assert!(rows("blah").is_empty());
    }

    #[test]
    fn the_toast_says_when_and_whether_a_reply_cancels_it() {
        let at = |text: &str| -> Timestamp { text.parse::<Zoned>().expect("a time").timestamp() };
        assert_eq!(
            toast(at("2026-11-18T08:00[America/New_York]"), false, &now()),
            "Reminder set: Wednesday, November 18th if no reply."
        );
        assert_eq!(
            toast(at("2026-11-17T15:00[America/New_York]"), true, &now()),
            "Reminder set: 3:00 PM."
        );
        assert_eq!(
            toast(at("2026-11-25T08:00[America/New_York]"), true, &now()),
            "Reminder set: November 25th."
        );
        assert_eq!(
            toast(at("2027-12-25T08:30[America/New_York]"), true, &now()),
            "Reminder set: December 25th, 2027, 8:30 AM."
        );
        assert_eq!(
            toast(
                Timestamp::from_millisecond(dates::SOMEDAY).expect("someday"),
                false,
                &now()
            ),
            "Reminder set: Someday if no reply."
        );
    }

    #[test]
    fn the_status_line_says_when_it_returns() {
        let set = |text: &str, keep_on_reply: bool| ReminderStatus::Set {
            at: text.parse::<Zoned>().expect("a time").timestamp(),
            keep_on_reply,
        };
        assert_eq!(
            status(&set("2026-11-18T08:00[America/New_York]", false), &now()),
            "Returning: tomorrow if no reply"
        );
        assert_eq!(
            status(&set("2026-11-20T15:30[America/New_York]", true), &now()),
            "Returning: Friday at 3:30 PM"
        );
        assert_eq!(
            status(&set("2026-12-01T08:00[America/New_York]", true), &now()),
            "Returning: December 1st"
        );
        assert_eq!(
            status(&set("2026-12-01T09:00[America/New_York]", true), &now()),
            "Returning: Dec 1st at 9:00 AM"
        );
        let returned = ReminderStatus::Returned {
            at: now().timestamp().checked_sub(5.minutes()).expect("a time"),
            unread: true,
        };
        assert_eq!(status(&returned, &now()), "Returned: 5 minutes ago");
    }
}
