//! Send later: ⌘⇧L, the palette's `Send Later` and the action pane's button open the picker; the
//! time picked is the draft's, which Send then schedules: the store holds the draft, a Gmail draft
//! meanwhile, until then. A reply scheduled `if no reply` is called off by a reply (`Unscheduled:
//! {name} replied`); a time that passes while the draft is still being written is cleared.

use exo_commands::{Command, Dropdown, SubPalette, Toast, command_score};
use exo_store::{MessageDetail, PreviousSchedule, SendLater, ThreadDetail};
use jiff::{Timestamp, ToSpan as _, Zoned, civil::Weekday};

use super::{Action, Open, save_changes};
use crate::{
    Mail, commands,
    format::{
        friendly,
        names::{self, NameFormat, Person},
    },
    time_suggest::{self, Options},
};

/// The dropdown a reply's picker has, on `if no reply` to start with.
const DROPDOWN: [&str; 2] = ["regardless", "if no reply"];
const IF_NO_REPLY: usize = 1;
/// The time of day a typed date without one gets.
const DEFAULT_TIME: (i8, i8) = (8, 0);
/// The toast when a draft's time passes while it's being written.
const PASSED: &str = "Selected time has passed. Please pick a new time to send.";

/// What the picker needs of its draft and the account.
pub(super) struct Picker {
    /// The draft.
    pub draft: String,
    /// Now, in the account's zone.
    pub now: Zoned,
    /// A reply, which gets the dropdown.
    pub reply: bool,
    /// The draft's time, if it has one.
    pub current: Option<Timestamp>,
    /// A reply called its schedule off: no `Unschedule`.
    pub unscheduled: bool,
    /// The `last used` row, as last picked.
    pub previous: Option<PreviousSchedule>,
}

/// A row: its title and the time it picks (`None`: at once, no time).
#[derive(Clone, Debug)]
struct Choice {
    title: String,
    time: Option<Timestamp>,
    /// A time typed in another zone.
    converted: bool,
}

impl Choice {
    fn at(title: impl Into<String>, time: Option<Timestamp>) -> Self {
        Self {
            title: title.into(),
            time,
            converted: false,
        }
    }
}

/// `now`'s day `days` ahead at `hour`:00.
fn at_hour(now: &Zoned, days: i64, hour: i8) -> Option<Timestamp> {
    let day = now.date().checked_add(days.days()).ok()?;
    Some(
        day.at(hour, 0, 0, 0)
            .to_zoned(now.time_zone().clone())
            .ok()?
            .timestamp(),
    )
}

impl Picker {
    /// The rows with no query: this morning and afternoon before 4 am, else tomorrow's, then next
    /// Monday's morning (not on a Sunday), with `last used` first while it's ahead.
    fn defaults(&self) -> Vec<Choice> {
        let now = &self.now;
        let mut rows = if now.hour() < 4 {
            vec![
                Choice::at("this morning", at_hour(now, 0, 8)),
                Choice::at("this afternoon", at_hour(now, 0, 13)),
            ]
        } else {
            vec![
                Choice::at("tomorrow morning", at_hour(now, 1, 8)),
                Choice::at("tomorrow afternoon", at_hour(now, 1, 13)),
            ]
        };
        if now.weekday() != Weekday::Sunday {
            // `Date.future("monday")`: the next Monday after today.
            let days = i64::from(7 - now.weekday().to_monday_zero_offset());
            rows.push(Choice::at("Monday morning", at_hour(now, days, 8)));
        }
        if let Some(previous) = &self.previous {
            let named = rows.iter().any(|row| row.title == previous.title);
            if !named && previous.time > now.timestamp() && self.current != Some(previous.time) {
                rows.insert(0, Choice::at("last used", Some(previous.time)));
            }
        }
        rows
    }

    /// The rows only a query shows: four hours on from this hour (`a few hours`, `later today` or
    /// `later` past midnight), and with a time set, `now` and `immediately`, which clear it.
    fn hidden(&self) -> Vec<Choice> {
        let now = &self.now;
        let hour = now
            .date()
            .at(now.hour(), 0, 0, 0)
            .to_zoned(now.time_zone().clone())
            .ok()
            .and_then(|hour| hour.checked_add(4.hours()).ok());
        let later = if hour.as_ref().is_some_and(|hour| hour.date() == now.date()) {
            "later today"
        } else {
            "later"
        };
        let time = hour.map(|hour| hour.timestamp());
        let mut rows = vec![Choice::at("a few hours", time), Choice::at(later, time)];
        if self.current.is_some() {
            rows.extend([Choice::at("now", None), Choice::at("immediately", None)]);
        }
        rows
    }

    /// The rows for `query`: with none, the defaults; else what the parser makes of it, then the
    /// defaults and hidden rows whose titles match it, minus those at a time the parser gave.
    fn choices(&self, query: &str) -> Vec<Choice> {
        let rows = self.defaults();
        if query.is_empty() {
            return rows;
        }
        let matches =
            |row: &Choice| command_score(&row.title.to_lowercase(), &query.to_lowercase()) > 0.0;
        let mut rows: Vec<Choice> = rows.into_iter().filter(matches).collect();
        rows.extend(unique_times(
            self.hidden().into_iter().filter(matches).collect(),
        ));
        let options = Options {
            default_time: DEFAULT_TIME,
            ..Options::default()
        };
        let parsed = unique_times(
            time_suggest::suggest(query, &options, &self.now)
                .into_iter()
                .map(|suggestion| Choice {
                    title: suggestion.label,
                    time: Some(suggestion.start),
                    converted: suggestion.zone.is_some(),
                })
                .collect(),
        );
        rows.retain(|row| {
            !parsed
                .iter()
                .any(|parsed| parsed.time.is_some() && parsed.time == row.time)
        });
        parsed.into_iter().chain(rows).collect()
    }

    /// The picker's rows for `query` with the dropdown on `choice`: each title with its time on
    /// the right, upper-cased.
    fn rows(&self, query: &str, choice: usize) -> Vec<Command<commands::Action>> {
        let abort_on_reply = self.reply && choice == IF_NO_REPLY;
        self.choices(query)
            .into_iter()
            .enumerate()
            .map(|(at, row)| {
                let time = row.time.unwrap_or_else(|| self.now.timestamp());
                let side = if row.converted {
                    friendly::time_suggestor(time, &self.now)
                } else {
                    friendly::exact_friendly_compact(time, &self.now)
                };
                let remember = row.time.map(|time| PreviousSchedule {
                    title: row.title.clone(),
                    time,
                });
                let action = Action::Schedule {
                    draft: self.draft.clone(),
                    at: row.time,
                    abort_on_reply,
                    remember,
                };
                Command::new(
                    format!("SCHEDULE_{at}"),
                    row.title,
                    commands::Action::Compose(action),
                )
                .side_title(side.to_uppercase())
            })
            .collect()
    }

    /// The picker (`Send later`, the reminder's clock, `Try: 8 am, 3 days, aug 7`), with the
    /// dropdown for a reply and `Unschedule` for a draft with a time.
    pub fn palette(self) -> SubPalette<commands::Action> {
        let unschedule = (self.current.is_some() && !self.unscheduled).then(|| Action::Schedule {
            draft: self.draft.clone(),
            at: None,
            abort_on_reply: false,
            remember: None,
        });
        let palette = if self.reply {
            let dropdown = Dropdown::new(DROPDOWN.to_vec(), IF_NO_REPLY, "mod+shift+l");
            SubPalette::ordered_with(
                "SCHEDULE",
                "Send later",
                "data/snooze",
                dropdown,
                move |query, choice| self.rows(query, choice),
            )
        } else {
            SubPalette::ordered("SCHEDULE", "Send later", "data/snooze", move |query| {
                self.rows(query, 0)
            })
        };
        let palette = palette.placeholder("Try: 8 am, 3 days, aug 7");
        match unschedule {
            Some(action) => palette.footer(move |_| {
                Some(Command::new(
                    "UNSCHEDULE",
                    "Unschedule",
                    commands::Action::Compose(action.clone()),
                ))
            }),
            None => palette,
        }
    }
}

/// The first row at each time.
fn unique_times(rows: Vec<Choice>) -> Vec<Choice> {
    let mut seen = Vec::new();
    rows.into_iter()
        .filter(|row| {
            let new = !seen.contains(&row.time);
            seen.push(row.time);
            new
        })
        .collect()
}

/// The reply that called off `open`'s scheduled send, if one did: the first message of `thread` the
/// send wasn't scheduled with that isn't a draft or the user's.
pub(super) fn aborting<'a>(open: &Open, thread: &'a ThreadDetail) -> Option<&'a MessageDetail> {
    let later = open
        .record
        .send_later
        .as_ref()
        .filter(|later| later.abort_on_reply && !later.current_message_ids.is_empty())?;
    thread.messages.iter().find(|message| {
        !later.current_message_ids.contains(&message.id)
            && !message.is_draft()
            && !message.has_label("SENT")
            && !message.from.as_ref().is_some_and(|from| from.is_me)
    })
}

/// `Unscheduled: {name} replied`'s name: the sender's display name.
pub(super) fn replier(message: &MessageDetail) -> String {
    let from = message.from.as_ref();
    let person = Person::new(
        from.and_then(|from| from.name.clone()).unwrap_or_default(),
        from.map(|from| from.email.clone()).unwrap_or_default(),
    );
    names::display_name(&person, false, NameFormat::First, false)
}

impl Mail {
    /// Opens the picker for the focused draft (or the new-message pane's).
    pub(super) fn open_send_later(&mut self) {
        let Some(id) = self
            .compose
            .focus
            .clone()
            .or_else(|| self.compose.pane().map(str::to_owned))
        else {
            return;
        };
        let Some(open) = self.compose.get_mut(&id).filter(|open| !open.is_snippet()) else {
            return;
        };
        let picker = Picker {
            draft: id,
            now: self.now.clone(),
            reply: open.record.in_reply_to.is_some(),
            current: open.record.send_later.as_ref().map(|later| later.at),
            unscheduled: open.unscheduled_by.is_some(),
            previous: self.account.settings().previous_schedule.clone(),
        };
        self.commands.open_sub_palette(picker.palette());
    }

    /// Draft `id` takes the time picked (`None` clears it), and the row picked is remembered as
    /// `last used`. A time that's already passed is cleared at once, with the toast.
    pub(super) fn schedule_picked(
        &mut self,
        id: &str,
        at: Option<Timestamp>,
        abort_on_reply: bool,
        remember: Option<PreviousSchedule>,
    ) {
        if let Some(remember) = remember {
            self.account
                .update_settings(|settings| settings.previous_schedule = Some(remember));
        }
        let aliases = self.snapshot.aliases.clone();
        let Some(open) = self.compose.get_mut(id) else {
            return;
        };
        // A time picked for a send a reply called off starts afresh.
        open.unscheduled_by = None;
        open.record.send_later = at.map(|at| SendLater {
            at,
            abort_on_reply,
            current_message_ids: Vec::new(),
        });
        save_changes(open, &self.account, &aliases, &self.now);
        self.unschedule_passed();
    }

    /// The open drafts whose time has passed while they're still being written lose it, and with a
    /// thread or the new-message pane on screen, the toast says so.
    pub(super) fn unschedule_passed(&mut self) {
        let now = self.now.timestamp();
        let aliases = self.snapshot.aliases.clone();
        let mut passed = false;
        for open in &mut self.compose.drafts {
            if open
                .record
                .send_later
                .as_ref()
                .is_some_and(|later| later.at <= now)
            {
                open.record.send_later = None;
                open.unscheduled_by = None;
                save_changes(open, &self.account, &aliases, &self.now);
                passed = true;
            }
        }
        if passed && (self.thread.is_some() || self.compose.pane().is_some()) {
            self.commands.toast(Toast::new(PASSED));
        }
    }

    /// Notes, for each draft open in the thread on screen, the reply that called off its
    /// schedule, if one did.
    pub(super) fn note_unscheduled(&mut self) {
        let Some(detail) = self
            .thread
            .as_ref()
            .and_then(crate::thread::ThreadView::detail)
            .cloned()
        else {
            return;
        };
        let held: Vec<&str> = self
            .snapshot
            .drafts
            .iter()
            .filter(|draft| draft.send_at.is_some())
            .map(|draft| draft.id.as_str())
            .collect();
        for open in self
            .compose
            .drafts
            .iter_mut()
            .filter(|open| open.thread.as_deref() == Some(detail.id.as_str()))
        {
            let by = if held.contains(&open.id.as_str()) {
                None
            } else {
                aborting(open, &detail).map(replier)
            };
            open.unscheduled_by = by;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picker(now: &str, reply: bool, current: Option<Timestamp>) -> Picker {
        Picker {
            draft: "draft00000000000001".into(),
            now: now.parse().expect("now"),
            reply,
            current,
            unscheduled: false,
            previous: None,
        }
    }

    /// The rows' titles and times, with the dropdown on `if no reply`.
    fn rows(picker: Picker, query: &str) -> Vec<(String, String)> {
        picker
            .palette()
            .rows(query, IF_NO_REPLY)
            .into_iter()
            .map(|(title, side, _)| (title, side.unwrap_or_default()))
            .collect()
    }

    fn time(text: &str) -> Timestamp {
        text.parse().expect("a time")
    }

    #[test]
    fn the_defaults_are_tomorrow_and_monday() {
        // Tuesday 9 am in New York.
        let tuesday = picker("2026-11-17T09:00[America/New_York]", true, None);
        assert_eq!(
            rows(tuesday, ""),
            [
                ("tomorrow morning".into(), "WED, 8:00 AM".into()),
                ("tomorrow afternoon".into(), "WED, 1:00 PM".into()),
                ("Monday morning".into(), "MON, 8:00 AM".into())
            ]
        );
        let early = picker("2026-11-17T03:00[America/New_York]", false, None);
        assert_eq!(
            rows(early, "")
                .iter()
                .map(|(title, _)| title.as_str())
                .collect::<Vec<_>>(),
            ["this morning", "this afternoon", "Monday morning"]
        );
        let sunday = picker("2026-11-22T09:00[America/New_York]", false, None);
        assert_eq!(rows(sunday, "").len(), 2, "no Monday morning on a Sunday");
    }

    #[test]
    fn last_used_comes_first_while_ahead() {
        let with = |title: &str, at: &str| Picker {
            previous: Some(PreviousSchedule {
                title: title.into(),
                time: time(at),
            }),
            ..picker("2026-11-17T09:00[America/New_York]", false, None)
        };
        assert_eq!(
            rows(with("fri 8am", "2026-11-20T13:00:00Z"), "")[0],
            ("last used".into(), "FRI, 8:00 AM".into())
        );
        assert_ne!(
            rows(with("tomorrow morning", "2026-11-20T13:00:00Z"), "")[0].0,
            "last used",
            "not under a default's title"
        );
        assert_ne!(
            rows(with("fri 8am", "2026-11-16T13:00:00Z"), "")[0].0,
            "last used",
            "not once it's passed"
        );
    }

    #[test]
    fn typing_parses_then_filters_the_defaults() {
        let tuesday = || picker("2026-11-17T09:00[America/New_York]", false, None);
        let typed = rows(tuesday(), "tom");
        assert_eq!(typed[0].1, "WED, 8:00 AM", "the parse, at 8 am: {typed:?}");
        assert!(
            !typed.iter().any(|(title, _)| title == "tomorrow morning"),
            "the default at the parsed time goes: {typed:?}"
        );
        assert!(
            typed.iter().any(|(title, _)| title == "tomorrow afternoon"),
            "{typed:?}"
        );
        let later = rows(tuesday(), "later");
        assert_eq!(
            later.last(),
            Some(&("later today".into(), "1:00 PM".into())),
            "four hours on from 9 am: {later:?}"
        );
        assert!(
            rows(tuesday(), "now")
                .iter()
                .all(|(title, _)| title != "now"),
            "only with a time set"
        );
        let set = picker(
            "2026-11-17T09:00[America/New_York]",
            false,
            Some(time("2026-11-18T13:00:00Z")),
        );
        assert!(rows(set, "now").iter().any(|(title, _)| title == "now"));
    }

    #[test]
    fn the_dropdown_says_whether_a_reply_calls_it_off() {
        let abort = |choice| match picker("2026-11-17T09:00[America/New_York]", true, None)
            .palette()
            .rows("", choice)
            .remove(0)
            .2
        {
            commands::Action::Compose(Action::Schedule { abort_on_reply, .. }) => abort_on_reply,
            other => panic!("{other:?}"),
        };
        assert!(abort(IF_NO_REPLY), "if no reply, to start with");
        assert!(!abort(0), "regardless");
        let reply = picker("2026-11-17T09:00[America/New_York]", true, None);
        assert_eq!(
            reply.palette().titles(""),
            ["tomorrow morning", "tomorrow afternoon", "Monday morning"],
            "no Unschedule without a time"
        );
        let set = picker(
            "2026-11-17T09:00[America/New_York]",
            false,
            Some(time("2026-11-18T13:00:00Z")),
        );
        assert_eq!(
            set.palette().titles("").last().map(String::as_str),
            Some("Unschedule")
        );
    }
}
