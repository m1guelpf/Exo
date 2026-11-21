//! Making and changing events: the form opening and closing, Save with its checks and dialogs,
//! Delete, the saves a drag in the grid makes, and the toasts with their Undo.
//!
//! A change goes to the calendar task, which queues it in the store, held for Undo, until
//! Calendar has it ([`exo_store::CalendarChange`]).

use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Rect, Ui, pos2, vec2};
use exo_commands::{Toast, ToastKind};
use exo_store::{CalendarChange, Cut, StoredEvent};
use gmail::calendar::{Event as ApiEvent, EventAttendee, EventDateTime, EventPerson};
use jiff::{SignedDuration, ToSpan, Zoned};

use super::{
    Action,
    event::Event,
    form::{
        self, CalendarOption, Edited, EventForm, Field, FormAction, Guest, Meeting, Mode, Values,
    },
    grid::DRAFT,
};

/// The 1:1 titles a new event without guests is asked about.
const ONE_ON_ONE: [&str; 6] = [
    r"(?i)\b1\s*[:x-]\s*1\b",
    r"(?i)\b1\s+on\s+1\b",
    r"(?i)\b[a-z0-9][a-z0-9.'&-]*\s*<>\s*[a-z0-9][a-z0-9.'&-]*\b",
    r"(?i)\b[a-z0-9][a-z0-9.'&-]*\s+\+\s+[a-z0-9][a-z0-9.'&-]*\b",
    r"(?i)^[a-z0-9][a-z0-9.'&-]*\s+and\s+[a-z0-9][a-z0-9.'&-]*$",
    r"(?i)^[a-z0-9][a-z0-9.'&-]*\s*:\s*[a-z0-9][a-z0-9.'&-]*(?:\s+[a-z0-9][a-z0-9.'&-]*)?$",
];

/// What Undo takes back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Undone {
    /// A new event (`Scheduling undone.`): its form opens again.
    Create,
    /// An edit (`Event edit undone.`).
    Edit,
    /// A deletion (`Event deletion undone.`): the calendar goes to the event's day.
    Delete(jiff::civil::Date),
}

/// Which events of a series an edit or deletion covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Only,
    Following,
    All,
}

impl Scope {
    fn label(self) -> &'static str {
        match self {
            Scope::Only => "Only this event",
            Scope::Following => "This and all following events",
            Scope::All => "All events",
        }
    }
}

/// An alert or dialog over the page, and what its actions lead to.
pub(super) enum Prompt {
    /// A confirmation or a message: the first action does `then`, any other closes it.
    Confirm {
        title: &'static str,
        text: &'static str,
        actions: Vec<&'static str>,
        selected: usize,
        then: Then,
    },
    /// Saving an edit: which events, then whether to tell the guests.
    Edit {
        notify_step: bool,
        scopes: Vec<Scope>,
        choice: usize,
        ask_notify: bool,
        form: Box<EventForm>,
    },
    /// Deleting: which events, then whether to tell the guests.
    Delete {
        notify_step: bool,
        scopes: Vec<Scope>,
        choice: usize,
        ask_notify: bool,
        target: Box<Target>,
    },
    /// The invite card's `RSVP` alert: signed-out calendar account `email` signs in, with the
    /// provider chosen, to give `response` to `invite`.
    SignIn {
        email: String,
        invite: Box<crate::thread::invite::Invite>,
        response: crate::thread::invite::Response,
        provider: super::provider::ProviderPicker,
    },
}

impl Prompt {
    /// Its title.
    pub(super) fn title(&self) -> &'static str {
        match self {
            Prompt::Confirm { title, .. } => title,
            Prompt::Edit {
                notify_step: false, ..
            } => "Edit recurring event",
            Prompt::Edit {
                notify_step: true, ..
            } => "Send an update?",
            Prompt::Delete {
                notify_step: false, ..
            } => "Delete recurring event",
            Prompt::Delete {
                notify_step: true, ..
            } => "Send a cancellation email?",
            Prompt::SignIn { .. } => "RSVP",
        }
    }
}

/// The invite card's `RSVP` alert over `page`: the ask, and under it (8px apart,
/// `display: flex; gap: 8px`) the provider dropdown, nudged down 3px as a dialog's dropdown button
/// is (`top: 3px`), its text black at `.75` and its caret black at `.5` in both themes. The ask
/// wraps plainly, not as the alert's own text does (the flex item's anonymous block). Returns the
/// action clicked.
fn show_sign_in(
    ui: &mut Ui,
    page: Rect,
    title: &str,
    email: &str,
    provider: &mut super::provider::ProviderPicker,
) -> Option<usize> {
    const DROPDOWN: f32 = 20.0;
    const NUDGE: f32 = 3.0;
    let text = exo_ui::ALERT_TEXT;
    let lines = text.wrap(
        ui.ctx(),
        &super::provider::ask(email),
        exo_ui::ALERT_TEXT_WIDTH,
    );
    let asked = lines.len() as f32 * text.line_height;
    let height = asked + exo_ui::ALERT_PARAGRAPH_GAP + DROPDOWN;
    exo_ui::Alert {
        title,
        text: "",
        actions: &["Sign in", "Cancel"],
        selected: 0,
    }
    .show_body(ui, page, height, |ui, body| {
        for (index, line) in lines.iter().enumerate() {
            text.paint_line(
                ui.painter(),
                egui::pos2(body.left(), body.top() + index as f32 * text.line_height),
                line,
                exo_ui::ALERT_TEXT_INK,
                None,
            );
        }
        let at = egui::pos2(
            body.left(),
            body.top() + asked + exo_ui::ALERT_PARAGRAPH_GAP + NUDGE,
        );
        provider.show(
            ui,
            exo_ui::id(ui.ctx(), "calendar-rsvp-provider"),
            at,
            exo_ui::ALERT_TEXT_INK,
            egui::Color32::from_black_alpha(128),
        );
    })
}

/// What a confirmation's first action does.
pub(super) enum Then {
    /// Close the alert (a validation message's `OK`).
    Nothing,
    /// Throw the form away.
    Discard,
    /// Throw the form away, then open a new one (a new event's form over an open one).
    Replace(Box<EventForm>),
    /// Go on saving, past the time check (`Make History`).
    SaveFromPast,
    /// Go on saving without guests (`Save without attendee`).
    SaveAlone,
}

/// The event a deletion is about.
pub(super) struct Target {
    calendar: String,
    event: ApiEvent,
    start: Zoned,
}

/// Counts ids, in case the system's randomness is out.
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// An id for a new event, chosen here (Calendar takes a client's own): 32 hex digits, which
/// are base32hex.
fn new_id() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        let fallback = jiff::Timestamp::now().as_nanosecond() as u128
            ^ u128::from(NEXT_ID.fetch_add(1, Ordering::Relaxed));
        bytes = fallback.to_le_bytes();
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl crate::Mail {
    /// The writable calendars a new event can go on, the primary first.
    pub(super) fn calendar_writable(&self) -> Vec<CalendarOption> {
        let data = self.account.calendar().data();
        let colors = super::event::colors(self.account.settings());
        let email = self.account.email();
        let mut options: Vec<(bool, CalendarOption)> = data
            .calendars
            .iter()
            .flatten()
            .filter(|calendar| {
                matches!(
                    calendar.entry.access_role.as_deref(),
                    Some("owner" | "writer")
                ) && super::listable(&calendar.entry)
            })
            .filter_map(
                |exo_store::AccountCalendar {
                     account,
                     entry: calendar,
                 }| {
                    let id = calendar.id.clone()?;
                    let color = colors
                        .get(&id)
                        .copied()
                        .unwrap_or(exo_ui::CalendarColor::BLUE);
                    let name = calendar
                        .summary_override
                        .clone()
                        .or_else(|| calendar.summary.clone())
                        .unwrap_or_else(|| id.clone());
                    Some((
                        calendar.primary == Some(true),
                        CalendarOption {
                            account: account.clone(),
                            id,
                            name,
                            color,
                        },
                    ))
                },
            )
            .collect();
        // The mail account's first, then by account; the primary first.
        let key = |account: &str| (account != email, account.to_lowercase());
        options.sort_by(|(a_primary, a), (b_primary, b)| {
            key(&a.account)
                .cmp(&key(&b.account))
                .then(b_primary.cmp(a_primary))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        options.into_iter().map(|(_, option)| option).collect()
    }

    /// A new event's form for `when` (in the week view when `week`), on the primary calendar.
    /// Over an open form that would lose something, asks first.
    pub(super) fn calendar_create(
        &mut self,
        when: Option<(Zoned, Zoned)>,
        all_day: bool,
        week: bool,
    ) {
        let form = EventForm::create(self.calendar_new_values(when, all_day), week);
        self.calendar_open_form(form);
    }

    /// A new event's values for `when`.
    fn calendar_new_values(&self, when: Option<(Zoned, Zoned)>, all_day: bool) -> Values {
        Values {
            title: String::new(),
            when,
            all_day,
            guests: Vec::new(),
            location: String::new(),
            about: String::new(),
            // A Meet link when Google Meet is the default meeting link.
            meeting: if self
                .account
                .settings()
                .meeting_links
                .default_meeting_link
                .as_deref()
                == Some("google")
            {
                Meeting::NewMeet
            } else {
                Meeting::None
            },
            calendar: self.account.email().to_owned(),
            repeat: None,
        }
    }

    /// A click on a date in a message: the right pane's new event titled `title` (the
    /// conversation's subject) with `guests` (its people: name, address), over the day `date`
    /// (the one its hover showed). When stays empty.
    pub(crate) fn calendar_create_for_thread(
        &mut self,
        title: String,
        guests: Vec<(Option<String>, String)>,
        date: jiff::civil::Date,
    ) {
        let mut values = self.calendar_new_values(None, false);
        values.title = title;
        values.guests = guests
            .into_iter()
            .map(|(name, email)| Guest {
                email,
                name,
                optional: false,
                response: None,
                me: false,
                organizer: false,
            })
            .collect();
        self.calendar_open_form(EventForm::create(values, false));
        if self
            .calendar
            .form
            .as_ref()
            .is_some_and(|form| form.mode == Mode::Create && !form.week)
        {
            self.calendar.form_day = Some(date);
        }
    }

    /// Opens `form`, asking first if the open one would lose something.
    pub(super) fn calendar_open_form(&mut self, form: EventForm) {
        self.calendar.details = None;
        if self
            .calendar
            .form
            .as_ref()
            .is_some_and(|open| !open.is_empty())
        {
            let title = if self
                .calendar
                .form
                .as_ref()
                .is_some_and(|open| open.mode == Mode::Edit)
            {
                "Discard changes?"
            } else {
                "Discard this event?"
            };
            self.calendar.prompt = Some(Prompt::Confirm {
                title,
                text: "Your changes will not be saved.",
                actions: vec!["Discard", "Cancel"],
                selected: 0,
                then: Then::Replace(Box::new(form)),
            });
            return;
        }
        if form.mode == Mode::Create && !form.week {
            self.calendar.form_day = form.values.when.as_ref().map(|(start, _)| start.date());
            self.calendar.form_grid.reset();
        }
        self.calendar.form = Some(form);
    }

    /// `b` (and `shift+b`): in the week view, a half hour from the next half hour on its day;
    /// elsewhere the right pane's empty form.
    pub(super) fn calendar_new_event(&mut self, now: &Zoned) {
        let Some(active) = self.calendar_week() else {
            self.calendar_create(None, false, false);
            return;
        };
        let minutes = now.minute();
        let (mut hour, rounded) = (
            now.hour(),
            if minutes == 0 {
                0
            } else if minutes <= 30 {
                30
            } else {
                60
            },
        );
        let mut date = active;
        if rounded == 60 {
            hour += 1;
            if hour >= 24 {
                hour = 0;
                date = date.tomorrow().unwrap_or(date);
            }
        }
        let start = date
            .at(hour, (rounded % 60) as i8, 0, 0)
            .to_zoned(now.time_zone().clone())
            .ok();
        let when =
            start.and_then(|start| start.checked_add(30.minutes()).ok().map(|end| (start, end)));
        self.calendar_create(when, false, true);
    }

    /// Event `key`'s form, `field` focused.
    pub(super) fn calendar_edit(&mut self, key: &str, field: Field, now: &Zoned) {
        let Some((event, source, _)) = self.calendar_detailed(key, now) else {
            return;
        };
        if !event.editable {
            return;
        }
        let values = values_of(&event, &source, self.account.email());
        let edited = Edited {
            key: event.key.clone(),
            event: source,
            color: event.color,
        };
        let form = EventForm::edit(values, edited, self.calendar.week.is_some(), field);
        self.calendar_open_form(form);
    }

    /// A click or drag on empty grid: a new event there, or with the form open, the draft moved
    /// there, keeping its length for a click.
    pub(super) fn calendar_slot(
        &mut self,
        start: Zoned,
        end: Zoned,
        all_day: bool,
        dragged: bool,
        week: bool,
    ) {
        if let Some(form) = &mut self.calendar.form {
            let length = form
                .values
                .when
                .as_ref()
                .map(|(from, to)| to.timestamp().duration_since(from.timestamp()));
            let end = match length {
                Some(length) if !dragged && !all_day && !form.values.all_day => {
                    start.checked_add(length).unwrap_or(end)
                }
                _ => end,
            };
            form.values.when = Some((start, end));
            form.values.all_day = all_day;
            return;
        }
        self.calendar_create(Some((start, end)), all_day, week);
    }

    /// An event moved or resized in the grid: the draft's new time, or an edit saved at once
    /// (through the edit's dialogs, without the check for a time in the past).
    pub(super) fn calendar_moved(&mut self, key: &str, start: Zoned, end: Zoned, now: &Zoned) {
        if key == DRAFT {
            if let Some(form) = &mut self.calendar.form {
                form.values.when = Some((start, end));
            }
            return;
        }
        let Some((event, source, _)) = self.calendar_detailed(key, now) else {
            return;
        };
        if !event.editable {
            return;
        }
        let mut values = values_of(&event, &source, self.account.email());
        let edited = Edited {
            key: event.key.clone(),
            event: source,
            color: event.color,
        };
        let mut form = EventForm::edit(
            values.clone(),
            edited,
            self.calendar.week.is_some(),
            Field::When,
        );
        values.when = Some((start, end));
        form.values = values;
        self.calendar.details = None;
        self.calendar_edit_dialogs(form);
    }

    /// Save: what's typed in Who, the checks, then the time and 1:1 confirmations, then the
    /// change.
    pub(super) fn calendar_save(&mut self, now: &Zoned, past_checked: bool, alone_checked: bool) {
        let Some(form) = &mut self.calendar.form else {
            return;
        };
        if form.mode == Mode::Edit && form.dirty().is_empty() {
            return;
        }
        if !form.commit_guests() {
            form.mark_guests_invalid();
            return self.calendar_alert(
                "Almost there…",
                "Please enter a valid email address for each guest.",
            );
        }
        if form.values.title.trim().is_empty() {
            form.focus(Field::Title);
            return self.calendar_alert("Almost there…", "Please enter a title for your event.");
        }
        let Some((start, end)) = form.values.when.clone() else {
            form.focus(Field::When);
            return self.calendar_alert(
                "Almost there…",
                "Please pick a date and time for your calendar event.",
            );
        };
        // In the past: a timed event starting, or an all-day one ending, before now.
        let past = if form.values.all_day {
            end < *now
        } else {
            start < *now
        };
        if past && !past_checked {
            self.calendar.prompt = Some(Prompt::Confirm {
                title: "Temporal anomaly…",
                text: "You selected a time in the past!",
                actions: vec!["Make History", "Cancel"],
                selected: 0,
                then: Then::SaveFromPast,
            });
            return;
        }
        if form.mode == Mode::Create {
            let me = self.account.email();
            let others = form
                .values
                .guests
                .iter()
                .any(|guest| !guest.me && !guest.email.eq_ignore_ascii_case(me));
            let title = form.values.title.trim();
            let one_on_one = ONE_ON_ONE.iter().any(|pattern| {
                regex::Regex::new(pattern).is_ok_and(|pattern| pattern.is_match(title))
            });
            if one_on_one && !others && !alone_checked {
                self.calendar.prompt = Some(Prompt::Confirm {
                    title: "Did you forget to add an attendee?",
                    text: "This looks like a 1:1, but there are no other attendees on the event.",
                    actions: vec!["Save without attendee", "Add attendee"],
                    selected: 0,
                    then: Then::SaveAlone,
                });
                return;
            }
            let Some(form) = self.calendar.form.take() else {
                return;
            };
            return self.calendar_schedule(form.values);
        }
        let Some(form) = self.calendar.form.take() else {
            return;
        };
        self.calendar_edit_dialogs(form);
    }

    /// Holds the new event and says so, with Undo.
    fn calendar_schedule(&mut self, values: Values) {
        let Some((start, end)) = values.when.clone() else {
            return;
        };
        let event = new_event(&values, &start, &end, self.account.email());
        self.out_of_office_created(&values.calendar, &event);
        let token = self.account.calendar().hold(CalendarChange::Create {
            calendar: values.calendar.clone(),
            event,
        });
        self.calendar.created.insert(token, values);
        let text = if self.snapshot.connection.is_online() {
            "Event scheduled."
        } else {
            "Event will be scheduled when online."
        };
        self.commands
            .toast(
                Toast::new(text).undo(crate::commands::Action::Calendar(Action::Undo(
                    token,
                    Undone::Create,
                ))),
            );
    }

    /// Whether Calendar hasn't got `event` of `calendar` yet: it's a new event still queued,
    /// which a deletion takes back from the queue, with no Undo.
    fn calendar_unsynced(&self, calendar: &str, event: &ApiEvent) -> bool {
        self.account.calendar().data().events.iter().any(|stored| {
            stored.unsynced && stored.calendar_id == calendar && stored.event.id == event.id
        })
    }

    /// Asks, for a recurring event, which events (only this one, unless its start stays on its
    /// day: all of them too); for a Google event with guests, whether to tell them. Then the
    /// edit.
    fn calendar_edit_dialogs(&mut self, form: EventForm) {
        let Some(edited) = &form.edited else { return };
        let dirty = form.dirty();
        let recurring = edited.event.recurring_event_id.is_some();
        let original_day = edited
            .event
            .original_start_time
            .as_ref()
            .or(edited.event.start.as_ref())
            .and_then(|time| time.timestamp(&jiff::tz::TimeZone::UTC));
        let same_day = match (&form.values.when, original_day, &edited.event.start) {
            (Some((start, _)), Some(original), _) => {
                original.to_zoned(start.time_zone().clone()).date() == start.date()
            }
            _ => true,
        };
        let allow_all = !dirty.contains(&Field::When) || same_day;
        let me = self.account.email();
        let notifiable = |is_self: bool, email: &str| !is_self && !email.eq_ignore_ascii_case(me);
        let ask_notify = form
            .values
            .guests
            .iter()
            .any(|guest| notifiable(guest.me, &guest.email))
            || (dirty.contains(&Field::Guests)
                && edited.event.attendees.iter().any(|attendee| {
                    notifiable(
                        attendee.is_self == Some(true),
                        attendee.email.as_deref().unwrap_or_default(),
                    )
                }));
        // A new series Calendar hasn't got yet has no instance of its own to change: it changes
        // as a whole.
        let unsynced = recurring && self.calendar_unsynced(&form.values.calendar, &edited.event);
        let scope_step = recurring && allow_all;
        if !scope_step && !ask_notify {
            return self.calendar_apply_edit(
                form,
                if unsynced { Scope::All } else { Scope::Only },
                false,
            );
        }
        let scopes = match (allow_all, unsynced) {
            (true, false) => vec![Scope::Only, Scope::Following, Scope::All],
            (true, true) => vec![Scope::Following, Scope::All],
            (false, _) => vec![Scope::Only],
        };
        self.calendar.prompt = Some(Prompt::Edit {
            notify_step: !scope_step,
            scopes,
            choice: 0,
            ask_notify,
            form: Box::new(form),
        });
    }

    /// Holds the changed fields for this event or its series, and says so with Undo.
    fn calendar_apply_edit(&mut self, form: EventForm, scope: Scope, notify: bool) {
        let Some(edited) = &form.edited else { return };
        let dirty = form.dirty();
        let me = self.account.email().to_owned();
        let patch = patch_of(&form.values, &dirty, &me);
        let calendar = form.values.calendar.clone();
        let data = self.account.calendar().data();
        let instance = edited.event.clone();
        let (id, shift, shown) = match (scope, instance.recurring_event_id.clone()) {
            (Scope::All, Some(series)) => {
                let shift = dirty
                    .contains(&Field::When)
                    .then(|| shift_of(&instance, &form.values))
                    .flatten();
                let mut series_patch = patch.clone();
                series_patch.start = None;
                series_patch.end = None;
                let shown = data
                    .events
                    .iter()
                    .filter(|stored| {
                        stored.calendar_id == calendar
                            && stored.event.recurring_event_id.as_deref() == Some(series.as_str())
                    })
                    .map(|stored| {
                        let mut event = merged(&stored.event, &series_patch);
                        if let Some((start, end)) = shift {
                            event.start = moved(event.start.as_ref(), start).or(event.start);
                            event.end = moved(event.end.as_ref(), end).or(event.end);
                        }
                        event
                    })
                    .collect();
                (series, shift, shown)
            }
            (Scope::Following, Some(series)) => {
                let token = self.calendar_split(&form, &instance, series, patch, notify);
                return self.calendar_edited_toast(token, notify);
            }
            _ => (
                instance.id.clone().unwrap_or_default(),
                None,
                vec![merged(&instance, &patch)],
            ),
        };
        let patch = if shift.is_some() || scope == Scope::All {
            ApiEvent {
                start: None,
                end: None,
                ..patch
            }
        } else {
            patch
        };
        let token = self.account.calendar().hold(CalendarChange::Edit {
            calendar,
            id,
            patch,
            notify,
            shift,
            shown,
        });
        self.calendar_edited_toast(token, notify);
    }

    /// `This and all following events`: series `series` ends before its edited `instance`, and
    /// a new series with `patch` takes over from it. Meanwhile the instances from it on show the
    /// change, moved as the instance moved. Returns the change's token.
    fn calendar_split(
        &mut self,
        form: &EventForm,
        instance: &ApiEvent,
        series: String,
        patch: ApiEvent,
        notify: bool,
    ) -> u64 {
        let calendar = form.values.calendar.clone();
        let original = instance
            .original_start_time
            .clone()
            .or_else(|| instance.start.clone())
            .unwrap_or_default();
        let from = match (&original.date, original.timestamp(&jiff::tz::TimeZone::UTC)) {
            (Some(day), _) => Cut::Day(day.parse().unwrap_or_else(|_| self.now.date())),
            (None, at) => Cut::At(at.unwrap_or_else(jiff::Timestamp::now)),
        };
        let shift = form
            .dirty()
            .contains(&Field::When)
            .then(|| shift_of(instance, &form.values))
            .flatten();
        let following = |event: &ApiEvent| {
            event.recurring_event_id.as_deref() == Some(series.as_str())
                && event
                    .start
                    .as_ref()
                    .and_then(|start| start.timestamp(&jiff::tz::TimeZone::UTC))
                    .zip(original.timestamp(&jiff::tz::TimeZone::UTC))
                    .is_none_or(|(start, cut)| start >= cut)
        };
        let series_patch = ApiEvent {
            start: None,
            end: None,
            ..patch.clone()
        };
        let shown = self
            .account
            .calendar()
            .data()
            .events
            .iter()
            .filter(|stored| stored.calendar_id == calendar && following(&stored.event))
            .map(|stored| {
                let mut event = merged(&stored.event, &series_patch);
                if let Some((start, end)) = shift {
                    event.start = moved(event.start.as_ref(), start).or(event.start);
                    event.end = moved(event.end.as_ref(), end).or(event.end);
                }
                event
            })
            .collect();
        let (start, end) = (
            instance.start.clone().unwrap_or_default(),
            instance.end.clone().unwrap_or_default(),
        );
        let patch = ApiEvent {
            id: Some(new_id()),
            ..patch
        };
        self.account.calendar().hold(CalendarChange::Split {
            calendar,
            series,
            from,
            patch,
            start,
            end,
            notify,
            shown,
        })
    }

    /// An edit's toast, with Undo of the change held under `token`.
    fn calendar_edited_toast(&mut self, token: u64, notify: bool) {
        let text = match (self.snapshot.connection.is_online(), notify) {
            (false, _) => "Event will be updated when online.",
            (true, true) => "Event updated and notification sent.",
            (true, false) => "Event updated.",
        };
        self.commands
            .toast(
                Toast::new(text).undo(crate::commands::Action::Calendar(Action::Undo(
                    token,
                    Undone::Edit,
                ))),
            );
    }

    /// The details menu's `Delete`: for a recurring event, which events; for my event with
    /// guests, whether to tell them; then the deletion, with Undo.
    pub(super) fn calendar_delete(&mut self, key: &str, now: &Zoned) {
        let Some((event, source, _)) = self.calendar_detailed(key, now) else {
            return;
        };
        self.calendar.details = None;
        let me = self.account.email();
        let organizer = source.organizer.as_ref().is_some_and(|organizer| {
            organizer.is_self == Some(true)
                || organizer
                    .email
                    .as_deref()
                    .is_some_and(|email| email.eq_ignore_ascii_case(me))
        });
        // Ask whether to tell them when it's mine, with someone else on it, and Calendar has it.
        let ask_notify = organizer
            && !self.calendar_unsynced(&event.calendar_id, &source)
            && source.attendees.iter().any(|attendee| {
                attendee.is_self != Some(true)
                    && !attendee
                        .email
                        .as_deref()
                        .is_some_and(|email| email.eq_ignore_ascii_case(me))
            });
        let target = Target {
            calendar: event.calendar_id.clone(),
            event: source.clone(),
            start: event.start.clone(),
        };
        if source.recurring_event_id.is_some() {
            let mut scopes = vec![Scope::Only];
            if organizer {
                scopes.push(Scope::Following);
            }
            scopes.push(Scope::All);
            self.calendar.prompt = Some(Prompt::Delete {
                notify_step: false,
                scopes,
                choice: 0,
                ask_notify,
                target: Box::new(target),
            });
        } else if ask_notify {
            self.calendar.prompt = Some(Prompt::Delete {
                notify_step: true,
                scopes: vec![Scope::Only],
                choice: 0,
                ask_notify,
                target: Box::new(target),
            });
        } else {
            self.calendar_apply_delete(&target, Scope::Only, false);
        }
    }

    /// Holds the deletion and says so, with Undo; a new event Calendar hasn't got yet is simply
    /// taken back, with none.
    fn calendar_apply_delete(&mut self, target: &Target, scope: Scope, notify: bool) {
        let instance = target.event.id.clone().unwrap_or_default();
        let series = target.event.recurring_event_id.clone();
        let change = match (scope, series) {
            (Scope::All, Some(series)) => CalendarChange::Delete {
                calendar: target.calendar.clone(),
                id: series,
                notify,
            },
            (Scope::Following, Some(series)) => {
                let all_day = target
                    .event
                    .start
                    .as_ref()
                    .is_some_and(|start| start.date.is_some());
                let from = if all_day {
                    Cut::Day(target.start.date())
                } else {
                    Cut::At(target.start.timestamp())
                };
                CalendarChange::Truncate {
                    calendar: target.calendar.clone(),
                    series,
                    from,
                    notify,
                }
            }
            _ => CalendarChange::Delete {
                calendar: target.calendar.clone(),
                id: instance,
                notify,
            },
        };
        if self.calendar_unsynced(&target.calendar, &target.event) {
            self.account.calendar().queue(change);
            self.commands.toast(Toast::new("Event deleted."));
            return;
        }
        let token = self.account.calendar().hold(change);
        let text = if self.snapshot.connection.is_online() {
            "Event deleted."
        } else {
            "Event will be deleted when online."
        };
        self.commands
            .toast(
                Toast::new(text).undo(crate::commands::Action::Calendar(Action::Undo(
                    token,
                    Undone::Delete(target.start.date()),
                ))),
            );
    }

    /// Undo on a toast: takes the held change back while it's held. Past the hold (a hovered
    /// toast outlives it) the change went, so nothing is undone and the toast goes.
    pub(super) fn calendar_undo(&mut self, token: u64, undone: Undone) {
        if !self.account.calendar().undo(token) {
            self.commands.hide_toast();
            return;
        }
        match undone {
            Undone::Create => {
                self.commands.toast(Toast::new("Scheduling undone."));
                if let Some(values) = self.calendar.created.remove(&token) {
                    let week = self.calendar.week.is_some();
                    self.calendar_open_form(EventForm::create(values, week));
                }
            }
            Undone::Edit => self.commands.toast(Toast::new("Event edit undone.")),
            Undone::Delete(date) => {
                self.commands.toast(Toast::new("Event deletion undone."));
                // The calendar on the event's day.
                if let Some(week) = &mut self.calendar.week {
                    week.go_to(date);
                } else if self.calendar.day.is_some() {
                    self.calendar_open_day(date, super::Source::Other);
                }
            }
        }
    }

    /// A queued change Calendar refused: the toast or alert that says so.
    pub(super) fn calendar_failed(&mut self, failure: super::sync::Failure) {
        use super::sync::Failure;
        match failure {
            Failure::Respond => self
                .commands
                .toast(Toast::new("Sorry! Failed to save your RSVP.").kind(ToastKind::Error)),
            Failure::RespondToInvite { uid, thread, gone } => {
                // The card shows the calendar's answer again.
                self.calendar.answers.1.remove(&uid);
                let elsewhere = thread
                    .clone()
                    .filter(|thread| self.thread.as_ref().is_none_or(|open| open.id() != thread));
                if gone {
                    let mut toast = Toast::new("RSVP not saved because event has changed.");
                    if let Some(thread) = elsewhere {
                        toast = toast.action(
                            "See Message",
                            None,
                            crate::commands::Action::Calendar(Action::SeeMessage(thread)),
                        );
                    }
                    self.commands.toast(toast);
                } else if let Some(thread) = thread {
                    // The alert takes the keyboard from the card.
                    if let Some(open) = &mut self.thread {
                        open.blur_invite();
                    }
                    self.pickers.rsvp_failed(thread);
                } else {
                    self.commands.toast(
                        Toast::new("Failed to respond to a calendar invitation.")
                            .kind(ToastKind::Error),
                    );
                }
            }
            Failure::Create {
                token,
                calendar,
                event,
            } => {
                // The form opens again with what was saved, and Try Again goes back to it. One
                // queued before Exo last quit has its values read back from the event.
                let values = self
                    .calendar
                    .created
                    .remove(&token)
                    .or_else(|| self.calendar_values_of_new(&calendar, &event));
                if self.calendar.form.is_none()
                    && let Some(values) = values
                {
                    let mut form = EventForm::create(values, self.calendar.week.is_some());
                    form.failed = true;
                    self.calendar_open_form(form);
                }
                self.commands.toast(
                    Toast::new("Scheduling failed.")
                        .kind(ToastKind::Error)
                        .action(
                            "Try Again",
                            None,
                            crate::commands::Action::Calendar(Action::FormFocus(Field::Title)),
                        ),
                );
            }
            Failure::Edit => self
                .commands
                .toast(Toast::new("Event update failed.").kind(ToastKind::Error)),
            Failure::Delete => self
                .commands
                .toast(Toast::new("Event deletion failed.").kind(ToastKind::Error)),
        }
    }

    /// The form's values for a new event of `calendar` that Calendar refused.
    fn calendar_values_of_new(&self, calendar: &str, event: &ApiEvent) -> Option<Values> {
        let me = self.account.email();
        let stored = StoredEvent {
            calendar_id: calendar.to_owned(),
            event: event.clone(),
            unsynced: true,
        };
        let colors = super::event::colors(self.account.settings());
        let shown = Event::new(&stored, None, &colors, me, self.now.time_zone())?;
        let mut values = values_of(&shown, event, me);
        values.repeat = exo_store::Recurrence::from_rules(&event.recurrence);
        // A Meet link still to make (`createRequest`) comes again with the form's Save.
        if event
            .conference_data
            .as_ref()
            .is_some_and(|conference| conference.get("createRequest").is_some())
        {
            values.meeting = Meeting::NewMeet;
        }
        Some(values)
    }

    /// Cancel, Escape, ⌘⇧,: closes the form, asking first if that loses something.
    pub(super) fn calendar_discard(&mut self) {
        let Some(form) = &self.calendar.form else {
            return;
        };
        if form.is_empty() {
            self.calendar.form = None;
            return;
        }
        let title = if form.mode == Mode::Edit {
            "Discard changes?"
        } else {
            "Discard this event?"
        };
        self.calendar.prompt = Some(Prompt::Confirm {
            title,
            text: "Your changes will not be saved.",
            actions: vec!["Discard", "Cancel"],
            selected: 0,
            then: Then::Discard,
        });
    }

    /// A message under `title` with `OK`.
    fn calendar_alert(&mut self, title: &'static str, text: &'static str) {
        self.calendar.prompt = Some(Prompt::Confirm {
            title,
            text,
            actions: vec!["OK"],
            selected: 0,
            then: Then::Nothing,
        });
    }

    /// An action of the prompt, by index (Enter: the selected one; Escape: the last).
    pub(super) fn calendar_prompt_action(&mut self, index: usize, now: &Zoned) {
        let Some(prompt) = self.calendar.prompt.take() else {
            return;
        };
        match prompt {
            Prompt::Confirm { then, actions, .. } => {
                let confirmed = index == 0 && actions.len() > 1;
                match then {
                    Then::Nothing => {}
                    Then::Discard if confirmed => self.calendar.form = None,
                    Then::Replace(form) if confirmed => {
                        self.calendar.form = None;
                        self.calendar_open_form(*form);
                    }
                    Then::SaveFromPast if confirmed => self.calendar_save(now, true, false),
                    Then::SaveAlone if confirmed => self.calendar_save(now, true, true),
                    // `Add attendee` takes the keyboard to Who.
                    Then::SaveAlone => {
                        if let Some(form) = &mut self.calendar.form {
                            form.focus(Field::Guests);
                        }
                    }
                    Then::Discard | Then::Replace(_) | Then::SaveFromPast => {}
                }
            }
            // Scope step: `Ok` (0) / `Cancel` (1). Notify step: `Send` (0) / `Don't send` (1) /
            // `Go back` (2).
            Prompt::Edit {
                notify_step: false,
                scopes,
                choice,
                ask_notify,
                form,
            } => {
                if index != 0 {
                    return;
                }
                if ask_notify {
                    self.calendar.prompt = Some(Prompt::Edit {
                        notify_step: true,
                        scopes,
                        choice,
                        ask_notify,
                        form,
                    });
                } else {
                    self.calendar_apply_edit(*form, scopes[choice], false);
                }
            }
            Prompt::Edit {
                notify_step: true,
                scopes,
                choice,
                ask_notify,
                form,
            } => match index {
                0 | 1 => self.calendar_apply_edit(*form, scopes[choice], index == 0),
                // `Go back` to the scopes, or (without them) cancel.
                _ if scopes.len() > 1
                    || form
                        .edited
                        .as_ref()
                        .is_some_and(|edited| edited.event.recurring_event_id.is_some()) =>
                {
                    self.calendar.prompt = Some(Prompt::Edit {
                        notify_step: false,
                        scopes,
                        choice,
                        ask_notify,
                        form,
                    });
                }
                _ => {}
            },
            Prompt::Delete {
                notify_step: false,
                scopes,
                choice,
                ask_notify,
                target,
            } => {
                if index != 0 {
                    return;
                }
                if ask_notify {
                    self.calendar.prompt = Some(Prompt::Delete {
                        notify_step: true,
                        scopes,
                        choice,
                        ask_notify,
                        target,
                    });
                } else {
                    self.calendar_apply_delete(&target, scopes[choice], false);
                }
            }
            Prompt::Delete {
                notify_step: true,
                scopes,
                choice,
                ask_notify,
                target,
            } => match index {
                0 | 1 => self.calendar_apply_delete(&target, scopes[choice], index == 0),
                _ if target.event.recurring_event_id.is_some() => {
                    self.calendar.prompt = Some(Prompt::Delete {
                        notify_step: false,
                        scopes,
                        choice,
                        ask_notify,
                        target,
                    });
                }
                _ => {}
            },
            // `Sign in` does nothing until a provider is chosen; the alert stays up while the
            // browser signs in. `Cancel` (and Escape, once the list is closed) closes it.
            Prompt::SignIn {
                email,
                invite,
                response,
                mut provider,
            } => {
                if index == 0 && provider.chosen {
                    self.calendar_sign_in(
                        Some(&email),
                        super::accounts::SignInFor::Invite(invite.clone(), response),
                    );
                }
                if index == 0 || provider.close() {
                    self.calendar.prompt = Some(Prompt::SignIn {
                        email,
                        invite,
                        response,
                        provider,
                    });
                }
            }
        }
    }

    /// Closes the invite card's `RSVP` alert, if it's up: its sign-in came back.
    pub(super) fn calendar_close_sign_in_prompt(&mut self) {
        if matches!(self.calendar.prompt, Some(Prompt::SignIn { .. })) {
            self.calendar.prompt = None;
        }
    }

    /// ↑/↓ in a dialog's options.
    pub(super) fn calendar_prompt_choose(&mut self, delta: isize) {
        if let Some(
            Prompt::Edit {
                notify_step: false,
                scopes,
                choice,
                ..
            }
            | Prompt::Delete {
                notify_step: false,
                scopes,
                choice,
                ..
            },
        ) = &mut self.calendar.prompt
        {
            *choice = choice.saturating_add_signed(delta).min(scopes.len() - 1);
        }
    }

    /// The prompt's actions and how many there are, for its keys.
    pub(super) fn calendar_prompt_actions(&self) -> Option<(usize, usize)> {
        Some(match self.calendar.prompt.as_ref()? {
            Prompt::Confirm {
                actions, selected, ..
            } => (actions.len(), *selected),
            Prompt::Edit {
                notify_step: true, ..
            }
            | Prompt::Delete {
                notify_step: true, ..
            } => (3, 0),
            Prompt::Edit { .. } | Prompt::Delete { .. } | Prompt::SignIn { .. } => (2, 0),
        })
    }

    /// The prompt over the dimmed `page`, if one is up.
    pub(crate) fn calendar_show_prompt(&mut self, ui: &Ui, page: Rect) {
        self.calendar_show_options(ui.ctx(), page);
        let week_start = self.calendar_week_start_day();
        if let Some(form) = &mut self.calendar.form {
            form.show_custom(ui.ctx(), page, &self.now, week_start);
        }
        let Some(prompt) = &mut self.calendar.prompt else {
            return;
        };
        let mut clicked = None;
        egui::Area::new(exo_ui::id(ui.ctx(), "calendar-prompt")).order(egui::Order::Tooltip).fixed_pos(page.min).show(ui.ctx(), |area| {
            // The overlay: the page dimmed to half, nothing behind it clickable.
            area.painter().rect_filled(page, 0.0, egui::Color32::from_black_alpha(128));
            let _ = area.interact(page, exo_ui::id(area.ctx(), "calendar-prompt-overlay"), egui::Sense::click());
            let title = prompt.title();
            clicked = match prompt {
                Prompt::Confirm { text, actions, selected, .. } => exo_ui::Alert { title, text, actions, selected: *selected }.show(area, page),
                Prompt::Edit { notify_step: false, scopes, choice, .. } => {
                    let labels: Vec<&str> = scopes.iter().map(|scope| scope.label()).collect();
                    exo_ui::Alert { title, text: "", actions: &["Ok", "Cancel"], selected: 0 }.show_options(area, page, &labels, choice)
                }
                Prompt::Delete { notify_step: false, scopes, choice, .. } => {
                    let labels: Vec<&str> = scopes.iter().map(|scope| scope.label()).collect();
                    exo_ui::Alert { title, text: "", actions: &["Delete", "Cancel"], selected: 0 }.show_options(area, page, &labels, choice)
                }
                Prompt::Edit { notify_step: true, .. } => exo_ui::Alert { title, text: "Existing guests will receive an update email.", actions: &["Send", "Don't send", "Go back"], selected: 0 }.show(area, page),
                Prompt::Delete { notify_step: true, .. } => exo_ui::Alert { title, text: "Would you like to send a cancellation email to Google Calendar guests?", actions: &["Send", "Don't send", "Go back"], selected: 0 }.show(area, page),
                Prompt::SignIn { email, provider, .. } => show_sign_in(area, page, title, email, provider),
            };
        });
        if let Some(index) = clicked {
            let now = self.now.clone();
            self.calendar_prompt_action(index, &now);
        }
    }

    /// Calendar Settings over `page`, while it's up or fading.
    fn calendar_show_options(&mut self, ctx: &egui::Context, page: Rect) {
        use super::options::{OptionsAction, Setting};
        let settings = self.account.settings();
        let link = settings.meeting_links.default_meeting_link.clone();
        let calendar = &settings.calendar;
        let zone = calendar
            .additional_timezones
            .first()
            .map(|zone| zone.iana_id.clone());
        let options = [
            (Setting::Monday, calendar.week_starts_on == 1),
            (Setting::Weekends, calendar.show_weekends),
            (Setting::Declined, calendar.show_declined_events),
            (Setting::SecondaryZone, calendar.show_additional_timezones),
        ];
        // The mail account first, then the others by address.
        let email = self.account.email();
        let mut emails: Vec<&String> = settings
            .calendar_accounts
            .keys()
            .filter(|address| *address != email)
            .collect();
        emails.sort_by_key(|address| address.to_lowercase());
        let color = |address: &str| {
            self.accounts()
                .iter()
                .find(|account| account.email == address)
                .map_or_else(
                    || crate::accounts::default_color(address),
                    crate::accounts::color,
                )
        };
        let accounts: Vec<super::options::ShownAccount<'_>> = std::iter::once(email)
            .chain(emails.into_iter().map(String::as_str))
            .map(|address| super::options::ShownAccount {
                email: address,
                color: color(address),
                primary: address == email,
            })
            .collect();
        let shown = super::options::Shown {
            options,
            zone: zone.as_deref(),
            now: self.now.timestamp(),
            meeting_link: link.as_deref(),
            accounts: &accounts,
        };
        for action in super::options::show(
            ctx,
            page,
            self.calendar.options_open,
            &shown,
            &mut self.calendar.options_zone,
        ) {
            match action {
                OptionsAction::Toggle(setting) => self.account.update_settings(|settings| {
                    let calendar = &mut settings.calendar;
                    match setting {
                        Setting::Monday => {
                            calendar.week_starts_on = u8::from(calendar.week_starts_on != 1)
                        }
                        Setting::Weekends => calendar.show_weekends = !calendar.show_weekends,
                        Setting::Declined => {
                            calendar.show_declined_events = !calendar.show_declined_events
                        }
                        Setting::SecondaryZone => {
                            calendar.show_additional_timezones = !calendar.show_additional_timezones
                        }
                    }
                }),
                OptionsAction::SecondaryZone(iana) => self.account.update_settings(|settings| {
                    settings.calendar.additional_timezones = vec![exo_store::AdditionalTimezone {
                        iana_id: iana.clone(),
                    }]
                }),
                OptionsAction::MeetingLink(link) => self.account.update_settings(|settings| {
                    settings.meeting_links.default_meeting_link = link.clone()
                }),
                OptionsAction::Close => self.calendar.options_open = false,
                // Adding an account: the dialog goes, the week view opens, and Google's consent.
                OptionsAction::SignIn => {
                    self.calendar.options_open = false;
                    if self.calendar.week.is_none() {
                        self.calendar_run(super::Action::OpenWeek);
                    }
                    self.calendar_sign_in(None, super::accounts::SignInFor::List);
                }
                OptionsAction::Remove(email) => self.account.remove_calendar_account(&email),
                // Notification settings, in this dialog's place.
                OptionsAction::Notifications => {
                    self.calendar.options_open = false;
                    let now = ctx.input(|input| input.time);
                    self.run(
                        crate::commands::Action::Settings(crate::settings::Action::Open(
                            crate::settings::Dialog::Notifications,
                        )),
                        now,
                    );
                }
            }
        }
    }

    /// The form: the week view's modal beside the draft, the right pane's form over its day, or
    /// an edit's modal beside the event in the right pane's day.
    pub(crate) fn calendar_show_form(&mut self, ui: &mut Ui, window: Rect) {
        let Some(form) = &self.calendar.form else {
            return;
        };
        if !(form.week || form.mode == Mode::Edit)
            || (self.calendar.week.is_none() && self.calendar.day.is_none())
        {
            return;
        }
        let pane = self.calendar.week.is_none();
        let anchor = if pane {
            self.calendar.grid.draft_anchor
        } else {
            self.calendar
                .week
                .as_ref()
                .and_then(|week| week.draft_anchor())
        };
        let Some(anchor) = anchor else { return };
        let now = self.now.clone();
        let calendars = self.calendar_writable();
        let book = self.calendar_book();
        let me = self.account.email().to_owned();
        let conversation = self.calendar_conversation();
        let places = self.calendar_places(ui.ctx());
        let chips = self.calendar_guest_chips();
        let shown = form::Shown {
            calendars: &calendars,
            contacts: book.as_deref(),
            me: &me,
            now: &now,
            week_start: self.calendar_week_start_day(),
            conversation: &conversation,
            places: &places,
            chips: &chips,
        };
        let Some(form) = self.calendar.form.as_mut() else {
            return;
        };
        let height = form.content_height(false) + form::FOOTER_HEIGHT;
        let popup =
            self.calendar
                .form_placement
                .place(window, anchor, form::MODAL_WIDTH, height, pane);
        let mut clicked = None;
        egui::Area::new(exo_ui::id(ui.ctx(), "calendar-form"))
            .order(egui::Order::Foreground)
            .fixed_pos(popup.min)
            .constrain(false)
            .show(ui.ctx(), |area| {
                // The area covers the card, so the pointer over it is over the area (`layer_id_at`),
                // which its dropdowns ask to stay open on a click inside.
                area.expand_to_include_rect(popup);
                let mut card = area.new_child(egui::UiBuilder::new().max_rect(popup));
                let snapped = exo_ui::pixel_snap(popup);
                super::popup::paint_card(card.painter(), snapped);
                let _ = card.interact(
                    popup,
                    exo_ui::id(card.ctx(), "calendar-form-card"),
                    egui::Sense::click_and_drag(),
                );
                let content = Rect::from_min_max(
                    popup.min,
                    pos2(popup.right(), popup.bottom() - form::FOOTER_HEIGHT),
                );
                form.show(&mut card, content, &shown, false);
                clicked = form.footer(
                    &card,
                    Rect::from_min_size(
                        pos2(popup.left(), content.bottom()),
                        vec2(popup.width(), form::FOOTER_HEIGHT),
                    ),
                    false,
                );
            });
        self.calendar_form_clicked(clicked, &now);
        self.calendar_chip_clicked(ui.ctx());
    }

    /// The right pane's new-event form: the form over a day of the calendar showing the draft,
    /// Save and Cancel at the bottom. Returns whether it drew.
    pub(super) fn calendar_show_pane_form(&mut self, ui: &mut Ui, pane: Rect, rem: f32) -> bool {
        if !self.calendar_pane_form() {
            return false;
        }
        let now = self.now.clone();
        let calendars = self.calendar_writable();
        let book = self.calendar_book();
        let me = self.account.email().to_owned();
        let conversation = self.calendar_conversation();
        let places = self.calendar_places(ui.ctx());
        let chips = self.calendar_guest_chips();
        let shown = form::Shown {
            calendars: &calendars,
            contacts: book.as_deref(),
            me: &me,
            now: &now,
            week_start: self.calendar_week_start_day(),
            conversation: &conversation,
            places: &places,
            chips: &chips,
        };
        let draft = self.calendar_draft(&now);
        // The day follows When's highlighted suggestion.
        if let Some(preview) = self
            .calendar
            .form
            .as_ref()
            .and_then(EventForm::when_preview)
        {
            self.calendar.form_day = Some(preview.start.date());
        }
        let date = self
            .calendar
            .form_day
            .or(draft.as_ref().map(|draft| draft.start.date()))
            .unwrap_or(now.date());
        let events = self.calendar_events(&now, None);
        let days = [date];
        let (guests, expanded) = self.calendar_overlay(&now, true, &days);
        let Some(form) = self.calendar.form.as_mut() else {
            return false;
        };
        let content = form.content_height(true);
        let form_rect = Rect::from_min_max(pane.min, pos2(pane.right(), pane.top() + content));
        // The form covers the pane, the app's footer included, on `#fff`, Carbon `#18191a`.
        let fill = match exo_ui::theme(ui.ctx()) {
            exo_ui::Theme::Snow => egui::Color32::WHITE,
            exo_ui::Theme::Carbon => egui::Color32::from_rgb(0x18, 0x19, 0x1a),
        };
        ui.painter().rect_filled(pane, 0.0, fill);
        let footer = Rect::from_min_max(
            pos2(pane.left(), pane.bottom() - form::PANE_FOOTER_HEIGHT),
            pane.max,
        );
        // A press in the mail gives it the keyboard; one in the pane takes it back. What's over
        // the page (the palette, a toast) doesn't count.
        let pressed = ui.input(|input| {
            input
                .pointer
                .primary_pressed()
                .then(|| input.pointer.press_origin())
                .flatten()
        });
        if let Some(at) = pressed.filter(|at| {
            ui.ctx().layer_id_at(*at).is_none_or(|layer| {
                layer == ui.layer_id() || layer.order == egui::Order::Background
            })
        }) {
            form.away = !pane.contains(at);
        }
        form.show(ui, form_rect, &shown, true);
        let clicked = form.footer(ui, footer, true);
        let hint = form::paint_away_hint(ui, pos2(form_rect.left(), form_rect.bottom()), form.away);
        // The day under it, from the form's bottom (the hint's) to the footer's top, 40px lower
        // while When's list covers it (`margin-top: 64px`, over `transition: margin-top .1s`).
        let covered = form.when_covers(form_rect.bottom() + hint + super::day::COMPACT_TOP);
        let spacing = 40.0
            * exo_ui::motion::transition(
                ui.ctx(),
                exo_ui::id(ui.ctx(), "calendar-form-when-spacing"),
                covered,
                0.1,
                exo_ui::motion::CubicBezier::EASE,
            );
        let day = Rect::from_min_max(
            pos2(pane.left(), form_rect.bottom() + hint + spacing),
            pos2(pane.right(), footer.top()),
        );
        let grid = super::grid::Grid {
            sharing: None,
            days: &days,
            events: &events,
            now: &now,
            tz: now.time_zone(),
            invite: None,
            selected: None,
            draft: draft.as_ref(),
            hidden: None,
            form: true,
            details: false,
            finding: false,
            guests: &guests,
            expanded: expanded.as_deref(),
            mine: true,
            zones: &[],
        };
        let compose = self.compose.focused().is_some();
        let actions = super::day::show_compact(
            ui,
            day,
            rem,
            date,
            &grid,
            &mut self.calendar.form_grid,
            compose,
        );
        for action in actions {
            match action {
                super::day::DayAction::Step(days) => {
                    let date = date.checked_add(days.days()).unwrap_or(date);
                    self.calendar.form_day = Some(date);
                }
                super::day::DayAction::Grid(super::grid::GridAction::Slot {
                    start,
                    end,
                    all_day,
                }) => self.calendar_slot(start, end, all_day, false, false),
                super::day::DayAction::Grid(super::grid::GridAction::Change {
                    key,
                    start,
                    end,
                }) => self.calendar_moved(&key, start, end, &now),
                super::day::DayAction::Grid(super::grid::GridAction::Expand(guest)) => {
                    self.calendar_expand(guest, true)
                }
                super::day::DayAction::Grid(_)
                | super::day::DayAction::Today
                | super::day::DayAction::Create => {}
            }
        }
        self.calendar_form_clicked(clicked, &now);
        self.calendar_chip_clicked(ui.ctx());
        true
    }

    /// A click on a guest's tooltip: a new message to them, or their address copied.
    fn calendar_chip_clicked(&mut self, ctx: &egui::Context) {
        match self
            .calendar
            .form
            .as_mut()
            .and_then(|form| form.chip_clicked.take())
        {
            Some(form::ChipTip::Compose(address, pop_out)) => {
                let fields = crate::compose::Fields {
                    to: vec![address],
                    ..crate::compose::Fields::default()
                };
                if pop_out {
                    self.compose_new_popped(fields)
                } else {
                    self.compose_new(fields, None)
                }
            }
            Some(form::ChipTip::Copy(email)) => self.calendar_copy(ctx, email),
            None => {}
        }
    }

    /// Whether a new event's form fills the right pane.
    pub(crate) fn calendar_pane_form(&self) -> bool {
        self.calendar
            .form
            .as_ref()
            .is_some_and(|form| !form.week && form.mode == Mode::Create)
            && self.calendar.week.is_none()
    }

    fn calendar_form_clicked(&mut self, clicked: Option<FormAction>, now: &Zoned) {
        match clicked {
            Some(FormAction::Save) => self.calendar_save(now, false, false),
            Some(FormAction::Cancel) => self.calendar_discard(),
            None => {}
        }
    }

    /// The draft the form shows on the calendar.
    pub(super) fn calendar_draft(&self, now: &Zoned) -> Option<Event> {
        let form = self.calendar.form.as_ref()?;
        let (start, end, time_text) = form.draft_when(now)?;
        let color = match &form.edited {
            Some(edited) => edited.color,
            None => self
                .calendar_writable()
                .into_iter()
                .find(|calendar| calendar.id == form.values.calendar)
                .map_or(exo_ui::CalendarColor::BLUE, |calendar| calendar.color),
        };
        let title = if form.values.title.trim().is_empty() {
            "(No title)".to_owned()
        } else {
            form.values.title.clone()
        };
        Some(Event {
            time_text,
            ..Event::draft(
                DRAFT.to_owned(),
                title,
                start,
                end,
                form.values.all_day,
                color,
            )
        })
    }

    /// The people of the conversation open behind the form, once each and me left out, for
    /// Who's `Everybody in this conversation` (senders, then Cc, then To; none while the week
    /// view is up).
    fn calendar_conversation(&self) -> Vec<exo_store::Address> {
        let Some(detail) = self
            .thread
            .as_ref()
            .and_then(|thread| thread.detail())
            .filter(|_| self.calendar.week.is_none())
        else {
            return Vec::new();
        };
        let me = self.account.email();
        let mut people: Vec<exo_store::Address> = Vec::new();
        for contact in detail
            .messages
            .iter()
            .flat_map(|message| message.from.iter().chain(&message.cc).chain(&message.to))
        {
            if contact.is_me || contact.email.eq_ignore_ascii_case(me) {
                continue;
            }
            // A later mention replaces the earlier's name, in its place.
            match people
                .iter_mut()
                .find(|person| person.email == contact.email)
            {
                Some(person) => person.name = contact.name.clone(),
                None => people.push(exo_store::Address {
                    name: contact.name.clone(),
                    email: contact.email.clone(),
                }),
            }
        }
        people
    }

    /// Where's places: asked for what's typed while it has the keyboard, forgotten once it
    /// hasn't.
    fn calendar_places(&mut self, ctx: &egui::Context) -> Vec<super::places::Place> {
        match self
            .calendar
            .form
            .as_ref()
            .and_then(EventForm::location_query)
            .map(str::to_owned)
        {
            Some(text) => {
                self.calendar
                    .places
                    .ask(ctx, &text, self.snapshot.connection.is_online())
            }
            None => self.calendar.places.reset(),
        }
        self.calendar.places.found()
    }

    /// The address book, indexed for Who; read the first time it's wanted.
    pub(super) fn calendar_book(
        &mut self,
    ) -> Option<std::sync::Arc<crate::compose::contacts::Index>> {
        match &self.snapshot.address_book {
            None => {
                if !self.calendar.book_asked {
                    self.calendar.book_asked = true;
                    self.account.load_address_book();
                }
                None
            }
            Some(book) => {
                if self
                    .calendar
                    .book
                    .as_ref()
                    .is_none_or(|(indexed, _)| !std::sync::Arc::ptr_eq(indexed, book))
                {
                    self.calendar.book = Some((
                        book.clone(),
                        std::sync::Arc::new(crate::compose::contacts::Index::book(book)),
                    ));
                }
                self.calendar.book.as_ref().map(|(_, index)| index.clone())
            }
        }
    }

    /// The external editor: Google Calendar's own new-event page with the form's title, times,
    /// guests, place and notes, as this account.
    pub(super) fn calendar_open_external(&mut self) {
        let Some(form) = &self.calendar.form else {
            return;
        };
        let values = &form.values;
        let Ok(mut link) =
            url::Url::parse("https://calendar.google.com/calendar/render?action=TEMPLATE")
        else {
            return;
        };
        let format = |at: &Zoned| at.strftime("%Y%m%dT%H%M%S").to_string();
        let dates = values
            .when
            .as_ref()
            .map(|(start, end)| format!("{}/{}", format(start), format(end)))
            .unwrap_or_default();
        let guests: Vec<&str> = values
            .guests
            .iter()
            .filter(|guest| !guest.me)
            .map(|guest| guest.email.as_str())
            .collect();
        link.query_pairs_mut()
            .append_pair("authuser", self.account.email())
            .append_pair("text", &values.title)
            .append_pair("dates", &dates)
            .append_pair("add", &guests.join(","))
            .append_pair("location", &values.location)
            .append_pair("details", &values.about)
            .append_pair("src", &values.calendar);
        self.open_url(link.as_str());
    }
}

/// The form's values for `event`: its notes as HTML for About's editor, a plain-text
/// description's lines kept as lines.
fn values_of(event: &Event, source: &ApiEvent, me: &str) -> Values {
    let description = source.description.clone().unwrap_or_default();
    let about = if description.contains('<') {
        description
    } else {
        description
            .lines()
            .map(|line| format!("<div>{}</div>", html_escape(line)))
            .collect()
    };
    let meet = source.hangout_link.clone().or_else(|| {
        let entries = source
            .conference_data
            .as_ref()?
            .get("entryPoints")?
            .as_array()?;
        entries
            .iter()
            .find(|entry| {
                entry.get("entryPointType").and_then(|kind| kind.as_str()) == Some("video")
            })?
            .get("uri")?
            .as_str()
            .map(str::to_owned)
    });
    Values {
        title: source.summary.clone().unwrap_or_default(),
        when: Some((event.start.clone(), event.end.clone())),
        all_day: source
            .start
            .as_ref()
            .is_some_and(|start| start.date.is_some()),
        guests: source
            .attendees
            .iter()
            .filter(|attendee| attendee.resource != Some(true))
            .filter_map(|attendee| {
                let email = attendee.email.clone()?;
                Some(Guest {
                    me: attendee.is_self == Some(true) || email.eq_ignore_ascii_case(me),
                    name: attendee.display_name.clone(),
                    optional: attendee.optional == Some(true),
                    response: attendee.response_status.clone(),
                    organizer: attendee.organizer == Some(true),
                    email,
                })
            })
            .collect(),
        location: source.location.clone().unwrap_or_default(),
        about,
        meeting: meet.map_or(Meeting::None, Meeting::Existing),
        calendar: event.calendar_id.clone(),
        repeat: None,
    }
}

/// `text` safe in HTML.
fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A start or end as Calendar takes it: a date for an all-day event, else the time with its
/// offset and zone.
fn date_time(at: &Zoned, all_day: bool) -> EventDateTime {
    if all_day {
        return EventDateTime {
            date: Some(at.date().to_string()),
            ..EventDateTime::default()
        };
    }
    EventDateTime {
        date_time: Some(at.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string()),
        time_zone: at.time_zone().iana_name().map(str::to_owned),
        ..EventDateTime::default()
    }
}

/// The guests as Calendar's attendees: me among them (the organizer's calendar), accepted, when
/// there's anyone else.
fn attendees(values: &Values, me: &str) -> Vec<EventAttendee> {
    let others: Vec<&Guest> = values
        .guests
        .iter()
        .filter(|guest| !guest.me && !guest.email.eq_ignore_ascii_case(me))
        .collect();
    if others.is_empty() {
        return Vec::new();
    }
    let mine = values.guests.iter().find(|guest| guest.me);
    let mut list = vec![EventAttendee {
        email: Some(me.to_owned()),
        organizer: Some(mine.is_none_or(|guest| guest.organizer)),
        response_status: Some(
            mine.and_then(|guest| guest.response.clone())
                .unwrap_or_else(|| "accepted".to_owned()),
        ),
        ..EventAttendee::default()
    }];
    list.extend(others.into_iter().map(|guest| {
        EventAttendee {
            email: Some(guest.email.clone()),
            display_name: guest.name.clone(),
            optional: guest.optional.then_some(true),
            response_status: Some(
                guest
                    .response
                    .clone()
                    .unwrap_or_else(|| "needsAction".to_owned()),
            ),
            ..EventAttendee::default()
        }
    }));
    list
}

/// A Google Meet link asked for (`conferenceData.createRequest`).
fn meet_request() -> serde_json::Value {
    serde_json::json!({ "createRequest": { "requestId": new_id(), "conferenceSolutionKey": { "type": "hangoutsMeet" } } })
}

/// A new event from the form, its id chosen here so it shows before Calendar has it.
fn new_event(values: &Values, start: &Zoned, end: &Zoned, me: &str) -> ApiEvent {
    let me_person = EventPerson {
        email: Some(me.to_owned()),
        is_self: Some(true),
        ..EventPerson::default()
    };
    // A series starts on its rule's first day, its end moved as many days.
    let (start, end) = match values.repeat.as_ref().and_then(|rule| rule.first(start)) {
        Some(first) => {
            let days = start
                .date()
                .until(first.date())
                .map_or(0, |span| span.get_days());
            (
                first,
                end.checked_add(i64::from(days).days())
                    .unwrap_or_else(|_| end.clone()),
            )
        }
        None => (start.clone(), end.clone()),
    };
    let (start, end) = (&start, &end);
    ApiEvent {
        recurrence: values
            .repeat
            .iter()
            .map(|rule| rule.rule(start, values.all_day))
            .collect(),
        id: Some(new_id()),
        summary: Some(values.title.trim().to_owned()),
        description: (!values.about.trim().is_empty()).then(|| values.about.clone()),
        location: (!values.location.trim().is_empty()).then(|| values.location.trim().to_owned()),
        start: Some(date_time(start, values.all_day)),
        end: Some(date_time(end, values.all_day)),
        attendees: attendees(values, me),
        conference_data: (values.meeting == Meeting::NewMeet).then(meet_request),
        organizer: Some(me_person.clone()),
        creator: Some(me_person),
        ..ApiEvent::default()
    }
}

/// The changed fields as a patch.
fn patch_of(values: &Values, dirty: &[Field], me: &str) -> ApiEvent {
    let mut patch = ApiEvent::default();
    for field in dirty {
        match field {
            Field::Title => patch.summary = Some(values.title.trim().to_owned()),
            Field::When => {
                if let Some((start, end)) = &values.when {
                    patch.start = Some(date_time(start, values.all_day));
                    patch.end = Some(date_time(end, values.all_day));
                }
            }
            Field::Location => patch.location = Some(values.location.trim().to_owned()),
            Field::About => patch.description = Some(values.about.clone()),
            Field::Guests => patch.attendees = attendees(values, me),
            Field::Meeting => {
                patch.conference_data = Some(match &values.meeting {
                    Meeting::NewMeet => meet_request(),
                    Meeting::None | Meeting::Existing(_) => serde_json::Value::Null,
                });
            }
            Field::Calendar | Field::Repeat => {}
        }
    }
    patch
}

/// `event` with what `patch` sets, as it'll look once Calendar has it.
fn merged(event: &ApiEvent, patch: &ApiEvent) -> ApiEvent {
    let mut event = event.clone();
    if let Some(summary) = &patch.summary {
        event.summary = Some(summary.clone());
    }
    if let Some(start) = &patch.start {
        event.start = Some(start.clone());
    }
    if let Some(end) = &patch.end {
        event.end = Some(end.clone());
    }
    if let Some(location) = &patch.location {
        event.location = Some(location.clone());
    }
    if let Some(description) = &patch.description {
        event.description = Some(description.clone());
    }
    if !patch.attendees.is_empty() {
        event.attendees = patch.attendees.clone();
    }
    event
}

/// How far an instance's start and end moved, for its series (`All events`).
fn shift_of(instance: &ApiEvent, values: &Values) -> Option<(SignedDuration, SignedDuration)> {
    let (start, end) = values.when.as_ref()?;
    let tz = start.time_zone();
    let from = instance.start.as_ref()?.timestamp(tz)?;
    let to = instance.end.as_ref()?.timestamp(tz)?;
    Some((
        start.timestamp().duration_since(from),
        end.timestamp().duration_since(to),
    ))
}

/// A timed start or end moved by `by`.
fn moved(time: Option<&EventDateTime>, by: SignedDuration) -> Option<EventDateTime> {
    let time = time?;
    let at: jiff::Timestamp = time.date_time.as_deref()?.parse().ok()?;
    let zone = time
        .time_zone
        .as_deref()
        .and_then(|name| jiff::tz::TimeZone::get(name).ok())
        .unwrap_or(jiff::tz::TimeZone::UTC);
    let at = at.checked_add(by).ok()?.to_zoned(zone);
    Some(EventDateTime {
        date_time: Some(at.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string()),
        ..time.clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::tz::TimeZone;

    fn at(text: &str) -> Zoned {
        text.parse::<jiff::civil::DateTime>()
            .expect("time")
            .to_zoned(TimeZone::get("America/New_York").expect("zone"))
            .expect("exists")
    }

    fn values() -> Values {
        Values {
            title: "Lunch".into(),
            when: Some((at("2026-11-20T12:00"), at("2026-11-20T13:00"))),
            all_day: false,
            guests: Vec::new(),
            location: String::new(),
            about: String::new(),
            meeting: Meeting::None,
            calendar: "maya@brightmoor.co".into(),
            repeat: None,
        }
    }

    #[test]
    fn a_new_event_carries_its_times_and_guests() {
        let mut values = values();
        let (start, end) = values.when.clone().expect("a time");
        let event = new_event(&values, &start, &end, "maya@brightmoor.co");
        assert_eq!(
            event.start.and_then(|start| start.date_time).as_deref(),
            Some("2026-11-20T12:00:00-05:00")
        );
        assert!(event.attendees.is_empty(), "alone, no attendee list");
        assert!(event.id.is_some_and(|id| {
            id.bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'v'))
        }));
        values.guests.push(Guest {
            email: "lena@brightmoor.co".into(),
            name: None,
            optional: true,
            response: None,
            me: false,
            organizer: false,
        });
        values.all_day = true;
        let event = new_event(&values, &start, &end, "maya@brightmoor.co");
        let emails: Vec<_> = event
            .attendees
            .iter()
            .map(|attendee| {
                (
                    attendee.email.clone().unwrap_or_default(),
                    attendee.response_status.clone().unwrap_or_default(),
                )
            })
            .collect();
        assert_eq!(
            emails,
            [
                ("maya@brightmoor.co".to_owned(), "accepted".to_owned()),
                ("lena@brightmoor.co".to_owned(), "needsAction".to_owned())
            ]
        );
        assert_eq!(
            event.start.and_then(|start| start.date).as_deref(),
            Some("2026-11-20")
        );
    }

    #[test]
    fn a_patch_has_only_what_changed() {
        let values = values();
        let patch = patch_of(&values, &[Field::Title], "maya@brightmoor.co");
        assert_eq!(
            patch,
            ApiEvent {
                summary: Some("Lunch".into()),
                ..ApiEvent::default()
            }
        );
        let removed = patch_of(
            &Values {
                meeting: Meeting::None,
                ..values
            },
            &[Field::Meeting],
            "maya@brightmoor.co",
        );
        assert_eq!(
            serde_json::to_value(&removed).expect("serializes"),
            serde_json::json!({ "conferenceData": null }),
            "a removed link is cleared"
        );
    }

    #[test]
    fn one_on_ones_are_recognized() {
        let matches = |title: &str| {
            ONE_ON_ONE
                .iter()
                .any(|pattern| regex::Regex::new(pattern).expect("pattern").is_match(title))
        };
        assert!(
            matches("Priya 1:1")
                && matches("Maya <> Lena")
                && matches("Maya and Lena")
                && matches("Sync: Lena")
        );
        assert!(!matches("Design crit"));
    }
}
