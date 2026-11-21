//! The calendar: the day view in the right pane, the week view, their keys and palette rows, and
//! the calendar's data.

mod accounts;
mod availability;
mod block;
mod day;
pub(crate) mod description;
mod details;
mod editing;
mod event;
mod form;
mod grid;
mod guests;
mod jump;
pub(crate) mod layout;
mod meet;
mod month;
mod options;
pub mod places;
mod popup;
mod provider;
mod repeat;
mod share;
mod spelling;
mod sync;
mod timezones;
mod week;
mod when;
mod when_edit;

pub(crate) use event::{Rsvp, my_attendance, unique_key as event_key};
pub(crate) use share::Target;
pub use sync::CalendarSync;
pub(crate) use week::ListedCalendar;

use std::collections::BTreeMap;

use egui::{Rect, Ui};
use exo_commands::{Command, Commands, Hint, Region, Toast};
use exo_editor::FormatCommand;
use exo_store::{
    AccountCalendar, CalendarAccount, CalendarNotifications, CalendarSetting, Settings,
};
use exo_ui::CalendarColor;
use gmail::calendar::CalendarListEntry;
use jiff::{SignedDuration, ToSpan, Zoned, civil::Date};

use self::{
    details::{Details, DetailsAction, MenuItem, Shown},
    editing::{Prompt, Undone},
    event::Event,
    form::{EventForm, Field, Meeting, Mode},
    grid::{Grid, GridAction, GridView},
    popup::Placement,
    sync::Window,
    week::{Page, Week, WeekAction},
};
use crate::{
    commands::Action as AppAction,
    thread::invite::{Invite, Response},
};

/// Where the calendar's keys work from the mail views.
const MAIL: &[Region] = &[
    Region::ThreadList,
    Region::ThreadMessagesHeader,
    Region::ThreadMessages,
    Region::ThreadMessageCalendarInvite,
    Region::ActionPane,
    Region::ThreadListHeader,
];
/// Where the week view's keys work.
const WEEK: &[Region] = &[Region::SeeYourWeek, Region::SeeYourWeekSidebar];
/// Where the event form's keys work.
const FORM: &[Region] = &[Region::CalendarCreate];
/// Where an alert's keys work.
const ALERT: &[Region] = &[Region::Alert];

/// Something the calendar does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// A key in a calendar's colour picker.
    PickerKey(exo_ui::PickerKey),
    /// ⌘D with the right pane's form open: the keyboard to the next section.
    CycleFocus,
    /// `-` / `=`: the day view on the day before (-1) or after (1) the one it shows.
    Step(i64),
    /// `t`, the day view's Today: the day view on today, unless an event is being made.
    Today,
    /// `0`: the day view, or the contact pane when it shows.
    Cycle,
    /// The footer's calendar button.
    Footer,
    /// `Open Day` in the palette.
    OpenDay,
    /// `Close Calendar`.
    Close,
    /// `Show Declined Events` / `Hide Declined Events`.
    ToggleDeclined,
    /// `Show Weekends` / `Hide Weekends`.
    ToggleWeekends,
    /// `See Message` on a refused answer's toast: conversation `thread`.
    SeeMessage(String),
    /// `2`, `Open Week`, the rail's Calendar: the week view.
    OpenWeek,
    /// The week view one week back (-1) or on (1).
    WeekStep(i64),
    /// The week view on this week (`space`, `t`).
    WeekToday,
    /// Close the week view (`0`, `1`, Escape, the rail's Mail).
    CloseWeek,
    /// Escape with the event details up: the recurring alert, else the menu, else the details.
    DetailsEscape,
    /// Enter with the recurring alert up: its OK.
    DetailsConfirm,
    /// `b`, `shift+b`, `Create Empty Event`: a new event's form.
    Create,
    /// ⌘Enter, `Save Event`.
    Save,
    /// ⌘⇧, (or ⌘⇧.), `Discard Event`: the form closed, asking first.
    Discard,
    /// Escape in the form: an open dropdown first, else Discard.
    FormEscape,
    /// ⌘⇧= / ⌘⇧- with the form open: the draft a day (a week, in the week view) on or back.
    FormStep(i64),
    /// ⌘⇧0 with the form open: the draft on today.
    FormToday,
    /// The palette's `Edit …` rows: the keyboard to a field.
    FormFocus(Field),
    /// `Make all day` / `Remove all day`.
    FormAllDay,
    /// A formatting key in About.
    FormFormat(FormatCommand),
    /// `Add Google Meet Link` / `Remove Google Meet Link`.
    FormMeet,
    /// `Edit event in Google Calendar` and the rest that open Google Calendar's editor.
    FormExternal,
    /// An alert's or dialog's action, by index.
    Prompt(usize),
    /// ↑ (-1) / ↓ (1) in a dialog's options.
    PromptChoose(isize),
    /// Undo on a toast: the held change `token` taken back.
    Undo(u64, Undone),
    /// Escape while dragging in the grid.
    CancelDrag,
    /// `tab` in the messages: the keyboard to the focused message's invite card.
    InviteFocus,
    /// The invite card's next (1) or previous (-1) button.
    InviteMove(isize),
    /// Enter or space in the invite card: its focused button's answer.
    InviteSubmit,
    /// `m`, `Meet With`: the keyboard to Meet's input.
    MeetFocus,
    /// Escape in Meet's input.
    MeetBlur,
    /// `Calendar Settings`, the week view's gear.
    OpenSettings,
    /// Escape in Calendar Settings, its ×, the backdrop.
    CloseSettings,
    /// `Find Time`.
    FindTime,
    /// `Cancel Find Time`, Escape while finding.
    CancelFindTime,
    /// `Show next availability`: the week with the first free slot.
    ShowAvailability(Date),
    /// Escape in the invite card: back to the messages.
    InviteBlur,
    /// `Jump to Date`: its palette.
    OpenJumpToDate,
    /// A Jump to Date row: the week view, else the day view, on this day.
    JumpTo(Date),
    /// `Share Availability` from the list, a conversation or the week: the week picking free
    /// times for the conversation's draft or a Reply All, or a new message.
    ShareAvailability,
    /// Enter, ⌘Enter, Insert: the times into the draft.
    ShareInsert,
    /// Cancel, `0`, `1`, ⌘⇧0, or Escape outside a field: the page closes.
    ShareCancel,
    /// Escape in a field: its text goes, or the page closes.
    ShareEscape,
}

/// What opened the day view: a command's stays as the cursor moves, any other goes when the right
/// pane follows something else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Command,
    Other,
}

/// The day the right pane shows.
#[derive(Clone, Copy, Debug)]
struct DayPane {
    date: Date,
    source: Source,
}

/// What the right pane follows: the open thread and its focused message, or the thread the
/// contact pane follows in the list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Context {
    thread: Option<String>,
    message: Option<String>,
}

/// The calendar's UI state.
#[derive(Default)]
pub(crate) struct Calendar {
    day: Option<DayPane>,
    grid: GridView,
    /// Day changes with the chevrons or keys since the last hint.
    date_changes: u8,
    /// The window last asked of the data.
    window: Option<Window>,
    /// What the right pane followed last frame.
    context: Context,
    /// The invite of what it follows, once read, and whether the day view has opened for it.
    invite: Option<Invite>,
    invite_shown: bool,
    /// The week view, when it's up.
    week: Option<Week>,
    /// `0` from the day view has said, this session, that `2` opens the week.
    week_hint_shown: bool,
    /// The event details, when they're up.
    details: Option<Details>,
    /// The event form, while it's open.
    form: Option<EventForm>,
    /// Where the form's modal went (sticky, like the details').
    form_placement: Placement,
    /// The right pane's day under a new event's form, and its grid.
    form_day: Option<Date>,
    form_grid: GridView,
    /// An alert or dialog over the page.
    prompt: Option<Prompt>,
    /// New events held for Undo, by token: their form opens again on Undo or a failure.
    created: BTreeMap<u64, form::Values>,
    /// The address book, indexed for Who.
    book: Option<(
        std::sync::Arc<Vec<exo_store::ScoredContact>>,
        std::sync::Arc<crate::compose::contacts::Index>,
    )>,
    book_asked: bool,
    /// The open thread, and my answers to its invites given since it opened, by UID.
    answers: (String, BTreeMap<String, Response>),
    /// Meet's last free/busy ask, the week it last saw, Find Time's first-slot search waiting on
    /// data, and the week's free slots shown.
    meet_asked: Option<(Vec<String>, jiff::Timestamp, jiff::Timestamp)>,
    meet_week: Option<Date>,
    find_pending: bool,
    slots: Vec<(Zoned, Zoned)>,
    /// Calendar Settings is up.
    options_open: bool,
    /// Calendar Settings' secondary time zone input.
    options_zone: timezones::ZoneInput,
    /// Where's suggestions.
    places: places::Places,
    /// A calendar account's sign-in in flight, and what it's for.
    signing_in: Option<accounts::SignInFor>,
    /// The day view's `Add Calendar` provider dropdown.
    takeover: provider::ProviderPicker,
}

impl crate::Mail {
    /// The day the right pane's calendar shows, if it does.
    pub fn calendar_day(&self) -> Option<Date> {
        self.calendar.day.map(|day| day.date)
    }

    /// The day the week view is around, if it's up.
    pub fn calendar_week(&self) -> Option<Date> {
        self.calendar.week.as_ref().map(|week| week.active)
    }

    /// Whether the event details are up, and if so whether their recurring alert is.
    pub fn calendar_details(&self) -> Option<bool> {
        self.calendar
            .details
            .as_ref()
            .map(|details| details.alert.is_some())
    }

    /// The calendar's prompt, form or week view has the keyboard while it's up, and an invite
    /// card when it's taken it.
    pub(crate) fn calendar_region(&self) -> Option<Region> {
        if self.calendar.prompt.is_some() {
            Some(Region::Alert)
        } else if self
            .calendar
            .week
            .as_ref()
            .is_some_and(|week| week.picker.is_some())
        {
            Some(Region::CalendarColorPicker)
        } else if self.calendar.options_open {
            Some(Region::Dialog)
        } else if self.calendar.form.as_ref().is_some_and(|form| !form.away) {
            Some(Region::CalendarCreate)
        } else if let Some(share) = self
            .calendar
            .week
            .as_ref()
            .and_then(|week| week.share.as_ref())
        {
            Some(match share.typing {
                Some(share::Input::Who) => Region::ShareAvailabilitySidebarAttendees,
                Some(share::Input::Zone) => Region::ShareAvailabilitySidebarTimezoneInput,
                None => Region::ShareAvailability,
            })
        } else if let Some(week) = &self.calendar.week {
            Some(if week.meet.typing {
                Region::SeeYourWeekSidebarMeetWithInput
            } else {
                Region::SeeYourWeek
            })
        } else {
            self.thread
                .as_ref()
                .filter(|thread| thread.invite_focused())
                .map(|_| Region::ThreadMessageCalendarInvite)
        }
    }

    /// `CopyTooltip`'s copy: `text` on the clipboard, and its confirmation with the text cut at
    /// 80 characters.
    pub(super) fn calendar_copy(&mut self, ctx: &egui::Context, text: String) {
        ctx.copy_text(text.clone());
        let shown: String = if text.chars().count() > 80 {
            text.chars().take(80).chain(['…']).collect()
        } else {
            text
        };
        self.commands
            .hint(Hint::new().text(format!("\"{shown}\" copied to clipboard.")));
    }

    /// The title of the calendar's alert or dialog, if one is up.
    pub fn calendar_prompt(&self) -> Option<&'static str> {
        self.calendar.prompt.as_ref().map(Prompt::title)
    }

    /// The events the calendar shows on `date` (held changes included): their titles and
    /// start times (`9:00 AM`).
    pub fn calendar_events_on(&self, date: Date) -> Vec<(String, String)> {
        let now = self.now.clone();
        self.calendar_events(&now, None)
            .into_iter()
            .filter(|event| event.on(date))
            .map(|event| (event.title, event.start.strftime("%-I:%M %p").to_string()))
            .collect()
    }

    /// My answer on the open thread's invite card (`accepted`, `needsAction`, …), as it shows.
    pub fn invite_answer(&self) -> Option<&'static str> {
        let invite = self.thread.as_ref()?.invite()?;
        invite
            .answer_of(&self.calendar_invite_account(invite))
            .map(Response::status)
    }

    /// Calendar Settings is up (for tests).
    pub fn calendar_settings_open(&self) -> bool {
        self.calendar.options_open
    }

    /// The signed-out calendar account whose `Add Calendar` takes the day view's place, if it
    /// does (for tests).
    pub fn calendar_add_calendar(&self) -> Option<String> {
        self.calendar.day.and(self.calendar_takeover())
    }

    /// How many free slots Find Time shows.
    pub fn calendar_free_slots(&self) -> usize {
        if self
            .calendar
            .week
            .as_ref()
            .is_some_and(|week| week.meet.finding)
        {
            self.calendar.slots.len()
        } else {
            0
        }
    }

    /// The form's About, as the HTML it saves, and its Where.
    pub fn calendar_form_about(&self) -> Option<(String, String)> {
        let values = &self.calendar.form.as_ref()?.values;
        Some((values.about.clone(), values.location.clone()))
    }

    /// The open form's guests' chips: each address, the colour of their calendar's swatch (or
    /// `loading` / `empty`), and their busy status.
    pub fn calendar_form_chips(&self) -> Vec<(String, String, Option<&'static str>)> {
        self.calendar_guest_chips()
            .into_iter()
            .map(|(email, (marker, busy))| {
                let marker = match marker {
                    guests::Marker::Swatch(color) => color.name().to_owned(),
                    guests::Marker::Spinner => "loading".to_owned(),
                    guests::Marker::Empty => "empty".to_owned(),
                };
                (email, marker, busy)
            })
            .collect()
    }

    /// The guests' calendars overlaid on `day` in the week view or the right pane: each
    /// guest's address and their events' titles that day.
    pub fn calendar_guest_columns(&mut self, day: Date) -> Vec<(String, Vec<String>)> {
        let now = self.now.clone();
        let pane = self.calendar.week.is_none();
        let (guests, _) = self.calendar_overlay(&now, pane, &[day]);
        guests
            .into_iter()
            .map(|(email, events)| {
                (
                    email,
                    events
                        .into_iter()
                        .filter(|event| event.on(day))
                        .map(|event| event.title)
                        .collect(),
                )
            })
            .collect()
    }

    /// Whether the open form has given the keyboard to the mail (the right pane's, on a click
    /// there or ⌘D).
    pub fn calendar_form_away(&self) -> Option<bool> {
        self.calendar.form.as_ref().map(|form| form.away)
    }

    /// The open form's title and guests' addresses.
    pub fn calendar_form_values(&self) -> Option<(String, Vec<String>)> {
        let form = self.calendar.form.as_ref()?;
        Some((
            form.values.title.clone(),
            form.values
                .guests
                .iter()
                .map(|guest| guest.email.clone())
                .collect(),
        ))
    }

    /// The open form's calendar.
    pub fn calendar_form_calendar(&self) -> Option<&str> {
        Some(&self.calendar.form.as_ref()?.values.calendar)
    }

    /// The open details' event: its title and its calendar (a guest's address for a guest's
    /// event).
    pub fn calendar_details_event(&self) -> Option<(String, String)> {
        let key = self.calendar.details.as_ref()?.key.clone();
        let event = self
            .calendar_detailed(&key, &self.now)
            .map(|(event, ..)| event)
            .or_else(|| {
                self.calendar_guest_detailed(&key, &self.now)
                    .map(|(event, _)| event)
            })?;
        Some((event.title, event.calendar_id))
    }

    /// The open details' line under When for a series (`Weekly on Monday`).
    pub fn calendar_details_rule(&self) -> Option<String> {
        let key = self.calendar.details.as_ref()?.key.clone();
        let (event, source, _) = self.calendar_detailed(&key, &self.now)?;
        self.calendar_rule_line(&event, &source)
    }

    /// The open form's Repeat as its trigger reads, and whether its Custom dialog is up.
    pub fn calendar_form_repeat(&self) -> Option<(String, bool)> {
        let form = self.calendar.form.as_ref()?;
        let anchor = form
            .values
            .when
            .as_ref()
            .map_or_else(|| self.now.date(), |(start, _)| start.date());
        Some((
            repeat::label(form.values.repeat.as_ref(), anchor),
            form.custom.is_some(),
        ))
    }

    /// Whether the event form is open, and if so whether it's a new event's.
    pub fn calendar_form(&self) -> Option<bool> {
        self.calendar
            .form
            .as_ref()
            .map(|form| form.mode == Mode::Create)
    }

    /// What the calendar registers this frame besides its standing keys.
    pub(crate) fn calendar_state(&self) -> State {
        State {
            form: self.calendar.form.as_ref().map(|form| {
                let about = form.about_focused().then(|| {
                    let editor = form.about();
                    let formats = FormatCommand::ALL
                        .iter()
                        .enumerate()
                        .filter(|(_, command)| command.applies(editor))
                        .fold(0u16, |mask, (at, _)| mask | 1 << at);
                    (formats, editor.wants_escape(), editor.wants_tab())
                });
                FormState {
                    create: form.mode == Mode::Create,
                    all_day: form.values.all_day,
                    meet: form.values.meeting == Meeting::NewMeet,
                    about,
                }
            }),
            prompt: self.calendar_prompt_actions(),
            options: self.calendar.options_open,
            invite: self.thread.as_ref().and_then(|thread| {
                if thread.invite_focused() {
                    Some(true)
                } else {
                    thread.can_focus_invite().then_some(false)
                }
            }),
            meet: self
                .calendar
                .week
                .as_ref()
                .filter(|week| week.share.is_none())
                .map(|week| (!week.meet.guests.is_empty(), week.meet.finding)),
            share: self.calendar.week.as_ref().map(|week| week.share.is_some()),
            dragging: self.calendar.grid.dragging()
                || self.calendar.form_grid.dragging()
                || self.calendar.week.as_ref().is_some_and(Week::dragging),
            picking: self
                .calendar
                .week
                .as_ref()
                .is_some_and(|week| week.picker.is_some()),
            weekends: self.account.settings().calendar.show_weekends,
        }
    }

    /// Opens the week view around `date`, the settings menu and the right pane's day gone.
    pub(crate) fn calendar_open_week(&mut self, date: Date) {
        self.settings.hide();
        self.calendar.day = None;
        match &mut self.calendar.week {
            Some(week) => week.go_to(date),
            None => self.calendar.week = Some(Week::new(date)),
        }
    }

    /// The day the account switched to opens the week view on: the middle of the week on screen,
    /// if it's up, so either account's first weekday finds it.
    pub(crate) fn calendar_week_to_keep(&self) -> Option<Date> {
        self.calendar_week()
            .map(|active| self.calendar_week_start(active) + 3.days())
    }

    /// Opens the week view on `day`'s week, kept from the account switched from, as the
    /// account's view starts.
    pub(crate) fn calendar_keep_week(&mut self, day: Date) {
        self.calendar_open_week(day);
    }

    /// A calendar notification's click: the week view on `day` with event `key`'s details up once
    /// it shows, scrolled to its end at `end_hour`.
    pub(crate) fn calendar_open_event(&mut self, key: String, day: Date, end_hour: i8) {
        self.calendar.details = None;
        self.calendar_open_week(day);
        if let Some(week) = &mut self.calendar.week {
            week.select(key, end_hour);
        }
    }

    /// Opens the day view on `date`, out of the settings menu's way. Returns whether it wasn't up
    /// (a new day view, whose grid scrolls to the morning again).
    fn calendar_open_day(&mut self, date: Date, source: Source) -> bool {
        self.settings.hide();
        let mounted = self.calendar.day.is_none();
        if mounted {
            self.calendar.grid.reset();
        }
        self.calendar.day = Some(DayPane { date, source });
        mounted
    }

    /// The day view on `date`, from the open thread (an invite's When, or a date in a message).
    pub(crate) fn calendar_show_day(&mut self, date: Date) {
        self.calendar_open_day(date, Source::Other);
    }

    /// Moves the day view by `days`, or opens it on today moved by `days`; every third change
    /// hints at the week view.
    fn calendar_step(&mut self, days: i64) {
        let today = self.now.clone().date();
        let from = self.calendar.day.map_or(today, |day| day.date);
        let date = from.checked_add(days.days()).unwrap_or(from);
        if self.compose.focused().is_some() {
        } else if self.calendar.date_changes >= 2 {
            self.commands.hint(
                Hint::new()
                    .text("Next time, hit")
                    .keys(["2"])
                    .text("to see your week!"),
            );
            self.calendar.date_changes = 0;
        } else {
            self.calendar.date_changes += 1;
        }
        self.calendar_open_day(date, Source::Other);
    }

    /// Carries out a calendar command.
    pub(crate) fn calendar_run(&mut self, action: Action) {
        let today = self.now.clone().date();
        // Changing the date, or the view, closes the details.
        let keeps_details = matches!(
            action,
            Action::ToggleDeclined
                | Action::DetailsEscape
                | Action::DetailsConfirm
                | Action::Prompt(_)
                | Action::PromptChoose(_)
                | Action::Undo(..)
                | Action::CancelDrag
                | Action::InviteFocus
                | Action::InviteMove(_)
                | Action::InviteBlur
                | Action::InviteSubmit
                | Action::MeetFocus
                | Action::MeetBlur
                | Action::OpenJumpToDate
                | Action::ShareInsert
                | Action::ShareEscape
        );
        if !keeps_details {
            self.calendar.details = None;
        }
        match action {
            Action::Step(days) => self.calendar_step(days),
            Action::Today => {
                if self.calendar.form.is_none() {
                    self.settings.hide();
                    self.calendar_open_day(today, Source::Other);
                }
            }
            // With the day view up, `0` and the footer's button open the week on its day; the
            // first time `0` does, a hint says `2` does it now.
            Action::Cycle | Action::Footer if let Some(day) = self.calendar.day => {
                if action == Action::Cycle && !self.calendar.week_hint_shown {
                    self.calendar.week_hint_shown = true;
                    self.commands.hint(Hint::new().text("Please note: to make things faster, we're changing the calendar shortcut to").keys(["2"]).text("!"));
                }
                self.calendar_open_week(day.date);
            }
            Action::Cycle | Action::Footer => {
                self.calendar_open_day(today, Source::Other);
            }
            Action::OpenDay => {
                self.calendar_open_day(today, Source::Command);
            }
            // The week view closes, and the day view with it.
            Action::Close => {
                self.calendar.week = None;
                self.calendar.day = None;
            }
            Action::ToggleDeclined => self.account.update_settings(|settings| {
                settings.calendar.show_declined_events = !settings.calendar.show_declined_events
            }),
            Action::ToggleWeekends => self.account.update_settings(|settings| {
                settings.calendar.show_weekends = !settings.calendar.show_weekends
            }),
            Action::SeeMessage(thread) => self.open_thread(thread),
            Action::OpenWeek => {
                let date = self.calendar.day.map_or(today, |day| day.date);
                self.calendar_open_week(date);
            }
            Action::WeekStep(weeks) => {
                if let Some(week) = &mut self.calendar.week {
                    week.go_to(week.active + (7 * weeks).days());
                }
            }
            Action::WeekToday => {
                if let Some(week) = &mut self.calendar.week {
                    week.go_to(today);
                }
            }
            Action::CloseWeek => self.calendar.week = None,
            Action::OpenJumpToDate => {
                let now = self.now.clone();
                self.commands.open_sub_palette(jump::palette(now));
            }
            Action::JumpTo(date) => match &mut self.calendar.week {
                Some(week) => week.go_to(date),
                None => {
                    self.calendar_open_day(date, Source::Other);
                }
            },
            Action::DetailsEscape => match &mut self.calendar.details {
                Some(details) if details.alert.is_some() => details.alert = None,
                Some(details) if details.menu_open() => details.close_menu(),
                _ => self.calendar.details = None,
            },
            Action::DetailsConfirm => {
                if let Some(alert) = self
                    .calendar
                    .details
                    .as_mut()
                    .and_then(|details| details.alert.take())
                {
                    self.calendar_respond(alert.answer, alert.choice == 1);
                }
            }
            Action::Create => {
                let now = self.now.clone();
                self.calendar_new_event(&now);
            }
            Action::Save => match &mut self.calendar.form {
                Some(form) if form.custom.is_some() => form.finish_custom(),
                _ => {
                    let now = self.now.clone();
                    self.calendar_save(&now, false, false);
                }
            },
            Action::Discard => self.calendar_discard(),
            Action::FormEscape => match &mut self.calendar.form {
                Some(form) if form.custom.is_some() => form.escape_custom(),
                Some(form) if form.has_dropdown() => form.close_dropdown(),
                _ => self.calendar_discard(),
            },
            Action::FormStep(steps) => {
                let days = if self.calendar.week.is_some() {
                    7 * steps
                } else {
                    steps
                };
                self.calendar_form_shift(|start| start.checked_add(days.days()).ok());
            }
            Action::FormToday => {
                self.calendar_form_shift(|start| start.with().date(today).build().ok())
            }
            Action::FormFocus(field) => {
                if let Some(form) = &mut self.calendar.form {
                    form.focus(field);
                }
            }
            Action::FormAllDay => {
                if let Some(form) = &mut self.calendar.form {
                    form.toggle_all_day();
                }
            }
            Action::FormFormat(command) => {
                if let Some(form) = &mut self.calendar.form {
                    command.run(form.about_mut());
                }
            }
            Action::FormMeet => {
                if let Some(form) = &mut self.calendar.form {
                    form.values.meeting = if form.values.meeting == Meeting::None {
                        Meeting::NewMeet
                    } else {
                        Meeting::None
                    };
                }
            }
            Action::FormExternal => self.calendar_open_external(),
            Action::Prompt(index) => {
                let now = self.now.clone();
                self.calendar_prompt_action(index, &now);
            }
            Action::PromptChoose(delta) => self.calendar_prompt_choose(delta),
            Action::Undo(token, undone) => self.calendar_undo(token, undone),
            Action::InviteFocus => {
                if let Some(thread) = &mut self.thread {
                    thread.focus_invite();
                }
            }
            Action::InviteMove(delta) => {
                if let Some(thread) = &mut self.thread {
                    thread.move_invite_focus(delta);
                }
            }
            Action::InviteBlur => {
                if let Some(thread) = &mut self.thread {
                    thread.blur_invite();
                }
            }
            Action::InviteSubmit => {
                if let Some((invite, response)) = self
                    .thread
                    .as_mut()
                    .and_then(crate::thread::ThreadView::submit_invite)
                {
                    self.calendar_respond_to_invite(invite, response, false);
                }
            }
            Action::MeetFocus => {
                if let Some(week) = &mut self.calendar.week {
                    week.meet.focus();
                }
            }
            // Calendar Settings opens the day view on today too.
            Action::OpenSettings => {
                if self.calendar.week.is_none() && self.calendar.day.is_none() {
                    self.calendar_open_day(today, Source::Other);
                }
                self.calendar.options_open = true;
            }
            Action::CloseSettings => self.calendar.options_open = false,
            Action::MeetBlur => {
                if let Some(week) = &mut self.calendar.week {
                    week.meet.blur();
                }
            }
            Action::FindTime => {
                let now = self.now.clone();
                self.calendar_meet(meet::MeetAction::Find, &now);
            }
            Action::CancelFindTime => {
                let now = self.now.clone();
                self.calendar_meet(meet::MeetAction::Cancel, &now);
            }
            Action::ShowAvailability(date) => {
                if let Some(week) = &mut self.calendar.week {
                    week.go_to(date);
                }
                self.commands.toast(Toast::new(format!(
                    "Showing availability from {} {}",
                    date.strftime("%b"),
                    date.day()
                )));
            }
            Action::ShareAvailability => {
                let now = self.now.clone();
                let target = self.calendar_share_target();
                self.calendar_share(target, &now);
            }
            Action::ShareInsert => {
                let now = self.now.clone();
                self.calendar_share_insert(&now);
            }
            Action::ShareCancel => self.calendar.week = None,
            Action::ShareEscape => {
                if self
                    .calendar
                    .week
                    .as_mut()
                    .and_then(|week| week.share.as_mut())
                    .is_some_and(share::Share::escape)
                {
                    self.calendar.week = None;
                }
            }
            Action::PickerKey(key) => self.calendar_picker_key(key),
            Action::CycleFocus => self.calendar_cycle_focus(),
            Action::CancelDrag => {
                self.calendar.grid.cancel_drag();
                self.calendar.form_grid.cancel_drag();
                if let Some(week) = &mut self.calendar.week {
                    week.cancel_drag();
                }
            }
        }
    }

    /// Moves the form's draft by `shift` of its start, keeping its length, and the calendar
    /// with it.
    fn calendar_form_shift(&mut self, shift: impl Fn(&Zoned) -> Option<Zoned>) {
        let Some(form) = &mut self.calendar.form else {
            return;
        };
        let Some((start, end)) = form.values.when.clone() else {
            return;
        };
        let Some(moved) = shift(&start) else { return };
        let length = end.timestamp().duration_since(start.timestamp());
        let Ok(end) = moved.checked_add(length) else {
            return;
        };
        let date = moved.date();
        form.values.when = Some((moved, end));
        if let Some(week) = &mut self.calendar.week {
            week.go_to(date);
        } else if self.calendar.form_day.is_some() {
            self.calendar.form_day = Some(date);
        }
    }

    /// Keeps the calendar's data and the right pane in step with the app, once a frame: the right
    /// pane following the open thread's invite, the account's calendars switched on the first time
    /// they're listed, the days around the day view kept fresh, and the open thread's invite cards
    /// showing the calendar's answers.
    pub(crate) fn calendar_frame(&mut self, ctx: &egui::Context) {
        // A click anywhere takes the keyboard back from an invite card.
        if ctx.input(|input| input.pointer.any_pressed())
            && let Some(thread) = &mut self.thread
        {
            thread.blur_invite();
        }
        let now = self.now.clone();
        self.calendar_accounts_frame();
        self.calendar_follow_context(&now);
        self.calendar_read_answers();
        let data = self.account.calendar().data();
        if let Some(calendars) = &data.calendars {
            self.calendar_enable_by_default(calendars);
        }
        let center = self
            .calendar_week()
            .or(self.calendar_day())
            .unwrap_or(now.date());
        let window = self.calendar_window(center, &now);
        if self.calendar.window.as_ref() != Some(&window) {
            self.calendar.window = Some(window.clone());
            self.account.calendar().show(window);
        }
    }

    /// The days the week view around `active` shows: its week, without the weekend unless
    /// Calendar Settings shows it or the week is picking times to share.
    pub fn calendar_week_days(&self, active: Date) -> Vec<Date> {
        let start = self.calendar_week_start(active);
        let weekends = self.account.settings().calendar.show_weekends
            || self
                .calendar
                .week
                .as_ref()
                .is_some_and(|week| week.share.is_some());
        (0..7)
            .map(|offset| start + offset.days())
            .filter(|day| weekends || day.weekday().to_monday_one_offset() <= 5)
            .collect()
    }

    /// The week's secondary time zones, while Calendar Settings shows them.
    fn calendar_secondary_zones(&self) -> Vec<jiff::tz::TimeZone> {
        let calendar = &self.account.settings().calendar;
        if !calendar.show_additional_timezones {
            return Vec::new();
        }
        calendar
            .additional_timezones
            .iter()
            .filter_map(|zone| jiff::tz::TimeZone::get(&zone.iana_id).ok())
            .collect()
    }

    /// The first day of `day`'s week, as Calendar Settings' `Start week on Monday` says.
    pub(super) fn calendar_week_start(&self, day: Date) -> Date {
        week_start(day, self.account.settings().calendar.week_starts_on == 1)
    }

    /// The weekday weeks start on, for the date pickers' months.
    pub(super) fn calendar_week_start_day(&self) -> jiff::civil::Weekday {
        if self.account.settings().calendar.week_starts_on == 1 {
            jiff::civil::Weekday::Monday
        } else {
            jiff::civil::Weekday::Sunday
        }
    }

    /// The days around `center` the data keeps: the week before its week to two weeks after.
    fn calendar_window(&self, center: Date, now: &Zoned) -> Window {
        let settings = self.account.settings();
        let week_start = self.calendar_week_start(center);
        let (shown, synced) = (
            settings.calendars_where(|calendar| calendar.show),
            settings.calendars_where(|calendar| calendar.sync),
        );
        Window {
            days: week_start - 7.days()..week_start + 14.days(),
            shown,
            synced,
            tz: now.time_zone().clone(),
        }
    }

    /// When the right pane follows something new: a day view opened by anything but a command
    /// closes, and once the new thread's invite is read, the day view opens on its date; a
    /// command's day view stays put. In the list the thread's invite is read from the store; an
    /// open thread reads its own.
    fn calendar_follow_context(&mut self, now: &Zoned) {
        // The settings menu holds the right pane until it goes.
        if self.settings.visible() {
            return;
        }
        let thread = self.thread.as_ref();
        let context = Context {
            thread: thread
                .map(|thread| thread.id().to_owned())
                .or_else(|| self.pane_thread.clone()),
            message: thread.and_then(|thread| thread.focused().map(str::to_owned)),
        };
        if context != self.calendar.context {
            self.calendar.context = context.clone();
            self.calendar.invite = None;
            // A day a command opened stays, and invites don't take it over.
            let commanded = self
                .calendar
                .day
                .is_some_and(|day| day.source == Source::Command);
            self.calendar.invite_shown = commanded;
            if commanded {
                return;
            }
            self.calendar.day = None;
            let listed = self
                .inbox
                .threads()
                .iter()
                .find(|row| Some(&row.id) == context.thread.as_ref());
            if thread.is_none()
                && let Some(row) =
                    listed.filter(|row| row.attachment != crate::mailbox::Attachments::None)
            {
                self.account.calendar().load_invite(&row.id);
            }
        }
        if self.calendar.invite_shown {
            return;
        }
        let invite = match &self.thread {
            Some(thread) => thread.invite().cloned(),
            None => {
                let data = self.account.calendar().data();
                data.invite
                    .as_ref()
                    .filter(|(id, _)| Some(id) == self.calendar.context.thread.as_ref())
                    .and_then(|(_, ics)| {
                        crate::thread::invite::parse(&String::from_utf8_lossy(ics))
                    })
            }
        };
        let Some(invite) = invite else { return };
        self.calendar.invite_shown = true;
        let (start, end) = (
            invite.start.to_zoned(now.time_zone().clone()),
            invite.end.to_zoned(now.time_zone().clone()),
        );
        self.calendar.invite = Some(invite);
        if self.calendar_open_day(start.date(), Source::Other) {
            self.calendar.grid.reveal(end.hour());
        }
    }

    /// The events the data holds, as the calendar shows them: declined ones only when the
    /// settings show them, or when they're `invite` or the one whose details are open.
    fn calendar_events(&self, now: &Zoned, invite: Option<&str>) -> Vec<Event> {
        let open = self
            .calendar
            .details
            .as_ref()
            .map(|details| details.key.as_str());
        let data = self.account.calendar().data();
        let settings = self.account.settings();
        let email = self.account.email();
        let colors = event::colors(settings);
        let lists: BTreeMap<&str, &AccountCalendar> = data
            .calendars
            .iter()
            .flatten()
            .filter_map(|calendar| Some((calendar.entry.id.as_deref()?, calendar)))
            .collect();
        data.events
            .iter()
            .filter_map(|stored| {
                // Me on another account's calendar is that account.
                let list = lists.get(stored.calendar_id.as_str()).copied();
                Event::new(
                    stored,
                    list.map(|calendar| &calendar.entry),
                    &colors,
                    list.map_or(email, |calendar| calendar.account.as_str()),
                    now.time_zone(),
                )
            })
            .filter(|event| {
                event.shown(settings.calendar.show_declined_events)
                    || (invite.is_some() && event.ical_uid.as_deref() == invite)
                    || open == Some(event.key.as_str())
            })
            .collect()
    }

    /// The signed-out calendar account whose `Add Calendar` takes the day view's place: the open
    /// conversation's invite is to it, I could answer it, and it hasn't been answered.
    fn calendar_takeover(&self) -> Option<String> {
        let invite = self.calendar.invite.as_ref()?;
        let email = self.calendar_invite_needs_sign_in(invite)?;
        (invite.can_respond(&email)
            && !self
                .account
                .settings()
                .calendar_invite_hint
                .get(&email)
                .copied()
                .unwrap_or(false))
        .then_some(email)
    }

    /// The right pane's day view, when it's up. Returns whether it drew.
    pub(crate) fn calendar_show_pane(&mut self, ui: &mut Ui, pane: Rect, rem: f32) -> bool {
        if self.calendar_show_pane_form(ui, pane, rem) {
            return true;
        }
        let Some(day) = self.calendar.day else {
            return false;
        };
        if let Some(email) = self.calendar_takeover() {
            match provider::show_takeover(ui, pane, rem, &email, &mut self.calendar.takeover) {
                // Calendar Settings opens over the sign-in.
                Some(provider::TakeoverAction::SignIn) => {
                    self.calendar.options_open = true;
                    self.calendar_sign_in(Some(&email), accounts::SignInFor::List);
                    self.account.update_settings(|settings| {
                        _ = settings.calendar_invite_hint.insert(email.clone(), true)
                    });
                }
                Some(provider::TakeoverAction::Dismiss) => {
                    self.account.update_settings(|settings| {
                        _ = settings.calendar_invite_hint.insert(email.clone(), true)
                    })
                }
                None => {}
            }
            return true;
        }
        let now = self.now.clone();
        let (guests, expanded) = self.calendar_overlay(&now, true, &[day.date]);
        let email = self.account.email();
        let invite_uid = self
            .calendar
            .invite
            .as_ref()
            .map(|invite| invite.uid.as_str());
        let mut events = self.calendar_events(&now, invite_uid);
        // The invite being read, if the calendar doesn't have it.
        if let Some(invite) = &self.calendar.invite
            && !events
                .iter()
                .any(|event| event.ical_uid.as_deref() == Some(invite.uid.as_str()))
        {
            let colors = event::colors(self.account.settings());
            let color = colors.get(email).copied().unwrap_or(CalendarColor::BLUE);
            let event = Event::from_invite(invite, color, email, now.time_zone());
            if event.on(day.date) && event.rsvp != Some(event::Rsvp::Declined) {
                events.push(event);
            }
        }
        let days = [day.date];
        let selected = self
            .calendar
            .details
            .as_ref()
            .map(|details| details.key.clone());
        let draft = self.calendar_draft(&now);
        let hidden = self
            .calendar
            .form
            .as_ref()
            .and_then(|form| form.edited.as_ref())
            .map(|edited| edited.key.clone());
        let grid = Grid {
            days: &days,
            events: &events,
            now: &now,
            tz: now.time_zone(),
            invite: invite_uid,
            selected: selected.as_deref(),
            draft: draft.as_ref(),
            hidden: hidden.as_deref(),
            form: self.calendar.form.is_some(),
            details: self.calendar.details.is_some(),
            finding: false,
            guests: &guests,
            expanded: expanded.as_deref(),
            mine: true,
            sharing: None,
            zones: &[],
        };
        let compose = self.compose.focused().is_some();
        let actions = day::show(
            ui,
            pane,
            rem,
            day.date,
            &grid,
            &mut self.calendar.grid,
            compose,
        );
        // `this.props.events` unset: the day isn't in what Calendar has been read for.
        if !self
            .account
            .calendar()
            .data()
            .window
            .as_ref()
            .is_some_and(|window| window.days.contains(&day.date))
        {
            day::paint_loading(ui, pane);
        }
        for action in actions {
            match action {
                day::DayAction::Step(days) => self.calendar_step(days),
                day::DayAction::Today => self.calendar_run(Action::Today),
                day::DayAction::Create => self.calendar_create(None, false, false),
                day::DayAction::Grid(action) => self.calendar_grid(action, true),
            }
        }
        true
    }

    /// What the pointer did in a grid: an event's details, a new event, an event moved or
    /// opened.
    fn calendar_grid(&mut self, action: GridAction, pane: bool) {
        let now = self.now.clone();
        match action {
            GridAction::Select { key, .. } if key.starts_with(grid::FREE_SLOT) => {
                self.calendar_pick_slot(&key, &now)
            }
            GridAction::Select { key, anchor } => {
                self.calendar.details = Some(Details::new(key, anchor, pane))
            }
            GridAction::Background => self.calendar.details = None,
            GridAction::Restricted => {
                // A drag closes the details first.
                self.calendar.details = None;
                self.commands
                    .toast(Toast::new("You do not have permission to edit this event."));
            }
            GridAction::Slot {
                start,
                end,
                all_day,
            } => {
                self.calendar.details = None;
                let dragged = end.timestamp().duration_since(start.timestamp())
                    != SignedDuration::from_mins(30);
                self.calendar_slot(start, end, all_day, dragged, !pane);
            }
            GridAction::Change { key, start, end } => {
                self.calendar.details = None;
                self.calendar_moved(&key, start, end, &now);
            }
            // Only sharing availability picks times.
            GridAction::Past | GridAction::Remove(_) => {}
            // Calendar Settings, at the secondary time zone.
            GridAction::ZoneSettings => self.calendar.options_open = true,
            GridAction::Expand(guest) => self.calendar_expand(guest, pane),
            GridAction::Open(key) => {
                // Not while editing, or with Meet's guests loaded.
                if self.calendar.form.is_none() {
                    self.calendar.details = None;
                    self.calendar_edit(&key, Field::Title, &now);
                }
            }
        }
    }
}

impl crate::Mail {
    /// The week view over `area` (the window right of the rail, under the tab strip), when it's
    /// up.
    pub(crate) fn calendar_show_week(&mut self, ui: &Ui, area: Rect) {
        let Some(active) = self.calendar_week() else {
            return;
        };
        let now = self.now.clone();
        let events = self.calendar_events(&now, None);
        let start = self.calendar_week_start(active);
        let days = self.calendar_week_days(active);
        let listed = self.calendar_list();
        let selected = self
            .calendar
            .details
            .as_ref()
            .map(|details| details.key.clone());
        let draft = self.calendar_draft(&now);
        let hidden = self
            .calendar
            .form
            .as_ref()
            .and_then(|form| form.edited.as_ref())
            .map(|edited| edited.key.clone());
        let slots = self.calendar_meet_view(&now);
        let (guests, expanded) = self.calendar_overlay(&now, false, &days);
        let markers = self.calendar_meet_markers();
        // Sharing availability, my events show while I'm listed, under the slots picked.
        let share = self
            .calendar
            .week
            .as_ref()
            .and_then(|week| week.share.as_ref());
        let sharing = share.map(|share| share.minutes);
        let shows_mine = share.is_none_or(share::Share::shows_mine);
        let events: Vec<Event> = match share {
            Some(share) => {
                let color = listed
                    .first()
                    .map_or(CalendarColor::BLUE, |calendar| calendar.color);
                let mine = if share.shows_mine() {
                    events
                } else {
                    Vec::new()
                };
                mine.into_iter().chain(share.events(color, &now)).collect()
            }
            None => events.into_iter().chain(slots).collect(),
        };
        let book = self.calendar_book();
        let zones = self.calendar_secondary_zones();
        let grid = Grid {
            days: &days,
            events: &events,
            now: &now,
            tz: now.time_zone(),
            invite: None,
            selected: selected.as_deref(),
            draft: draft.as_ref(),
            hidden: hidden.as_deref(),
            form: self.calendar.form.is_some(),
            details: self.calendar.details.is_some(),
            finding: self
                .calendar
                .week
                .as_ref()
                .is_some_and(|week| week.meet.finding)
                && self.calendar.form.is_none(),
            guests: &guests,
            expanded: expanded.as_deref(),
            mine: shows_mine,
            sharing,
            zones: &zones,
        };
        let loading = self.account.calendar().data().loading
            && events
                .iter()
                .all(|event| !days.iter().any(|day| event.on(*day)));
        let monday = self.account.settings().calendar.week_starts_on == 1;
        // Not dismissed, nothing else using the week, and no dialog.
        let find_time_hint = !self
            .account
            .settings()
            .calendar
            .has_dismissed_find_time_enter_shortcut_tooltip
            && self.calendar.form.is_none()
            && sharing.is_none()
            && !self.settings.dialog_open();
        let accounts = self.calendar_accounts_listed();
        let folded = self
            .account
            .device_settings()
            .folded_calendar_accounts
            .clone();
        let page = Page {
            grid,
            week_start: start,
            monday,
            account: self.account.email(),
            accounts: &accounts,
            loading,
            contacts: book.as_deref(),
            markers: &markers,
            find_time_hint,
            folded: &folded,
        };
        let compose = self.compose.focused().is_some() || self.calendar.form.is_some();
        let Some(week) = self.calendar.week.as_mut() else {
            return;
        };
        for action in week.show(ui, area, &page, compose) {
            match action {
                WeekAction::Today => self.calendar_run(Action::WeekToday),
                WeekAction::Create => {
                    let now = self.now.clone();
                    self.calendar_new_event(&now);
                }
                WeekAction::Step(weeks) => self.calendar_run(Action::WeekStep(weeks)),
                // While the form is open, a day moves the draft there.
                WeekAction::Select(date) if self.calendar.form.is_some() => {
                    self.calendar_form_shift(|start| start.with().date(date).build().ok())
                }
                WeekAction::Select(date) => {
                    self.calendar.details = None;
                    if let Some(week) = &mut self.calendar.week {
                        week.active = date;
                    }
                }
                WeekAction::ToggleCalendar(id) => self.calendar_toggle(&id),
                WeekAction::Recolor(id, color) => self.calendar_recolor(&id, color),
                WeekAction::AddAccount => self.calendar_sign_in(None, accounts::SignInFor::List),
                WeekAction::SignInAgain(email) => {
                    self.calendar_sign_in(Some(&email), accounts::SignInFor::List)
                }
                WeekAction::RemoveAccount(email) => self.account.remove_calendar_account(&email),
                WeekAction::Fold(email) => self.account.update_device_settings(|device| {
                    if !device.folded_calendar_accounts.remove(&email) {
                        device.folded_calendar_accounts.insert(email);
                    }
                }),
                WeekAction::Grid(action) => {
                    if !self.calendar_share_grid(action.clone(), &now) {
                        self.calendar_grid(action, false);
                    }
                }
                WeekAction::Share(action) => self.calendar_share_action(action, &now),
                WeekAction::Meet(action) => self.calendar_meet(action, &now),
                // Its gear opens Calendar Settings directly.
                WeekAction::Settings => self.calendar.options_open = true,
            }
        }
    }

    /// The event details over `window`, when they're up, and the recurring-RSVP alert over
    /// `page`; also reports the calendar's failed changes.
    pub(crate) fn calendar_show_details(&mut self, ui: &Ui, window: Rect, page: Rect) {
        for failure in self.account.calendar().failures() {
            self.calendar_failed(failure);
        }
        let Some(key) = self
            .calendar
            .details
            .as_ref()
            .map(|details| details.key.clone())
        else {
            return;
        };
        let now = self.now.clone();
        let found = (self.calendar.week.is_some() || self.calendar.day.is_some())
            .then(|| {
                self.calendar_detailed(&key, &now)
                    .map(|(event, source, can_respond)| (event, source, can_respond, false))
                    .or_else(|| {
                        self.calendar_guest_detailed(&key, &now)
                            .map(|(event, source)| (event, source, false, true))
                    })
            })
            .flatten();
        let Some((event, source, can_respond, guest)) = found else {
            self.calendar.details = None;
            return;
        };
        let email = self.account.email().to_owned();
        let calendar = self.calendar_name(&event, &source);
        let writable = self
            .account
            .calendar()
            .data()
            .calendars
            .iter()
            .flatten()
            .find(|calendar| calendar.entry.id.as_deref() == Some(event.calendar_id.as_str()))
            .is_some_and(|calendar| {
                matches!(
                    calendar.entry.access_role.as_deref(),
                    Some("owner" | "writer")
                )
            });
        let mut menu = Vec::new();
        if event.editable {
            menu.push(MenuItem::Edit);
        }
        // Deleting needs a calendar I can write to.
        if writable && event.id.is_some() {
            menu.push(MenuItem::Delete);
        }
        if source.attendees.iter().any(|attendee| {
            !attendee
                .email
                .as_deref()
                .is_some_and(|address| address.eq_ignore_ascii_case(&email))
                && attendee.is_self != Some(true)
        }) {
            menu.push(MenuItem::EmailGuests);
        }
        if source.html_link.is_some() && event.title != "Busy" && !guest {
            menu.push(MenuItem::OpenInCalendar);
        }
        // A guest's series has no rule Exo knows: `Repeats`.
        let rule = if guest {
            source
                .recurring_event_id
                .as_ref()
                .map(|_| repeat::details_line(None, event.start.date()))
        } else {
            self.calendar_rule_line(&event, &source)
        };
        let shown = Shown {
            event: &event,
            source: &source,
            me: &email,
            calendar: calendar.as_deref(),
            now: &now,
            can_respond,
            menu: &menu,
            editable: event.editable,
            rule,
            guest,
        };
        let Some(details) = self.calendar.details.as_mut() else {
            return;
        };
        let (actions, popup) = details.show(ui, window, &shown);
        if details.dismissed(ui, popup) {
            self.calendar.details = None;
        }
        for action in actions {
            match action {
                DetailsAction::Respond { answer, all } => self.calendar_respond(answer, all),
                DetailsAction::Menu(MenuItem::EmailGuests) => {
                    self.calendar.details = None;
                    let to = source
                        .attendees
                        .iter()
                        .filter(|attendee| {
                            attendee.is_self != Some(true) && attendee.resource != Some(true)
                        })
                        .filter_map(|attendee| {
                            let address = attendee
                                .email
                                .clone()
                                .filter(|address| !address.eq_ignore_ascii_case(&email))?;
                            Some(exo_store::Address {
                                name: attendee.display_name.clone(),
                                email: address,
                            })
                        });
                    let fields = crate::compose::Fields {
                        to: to.collect(),
                        subject: source.summary.clone().unwrap_or_default(),
                        ..crate::compose::Fields::default()
                    };
                    self.compose_new(fields, Some(crate::compose::Field::Body));
                }
                DetailsAction::Menu(MenuItem::OpenInCalendar) => {
                    // The event's page, as this account.
                    if let Some(mut link) = source
                        .html_link
                        .as_deref()
                        .and_then(crate::format::urls::web)
                        .and_then(|link| url::Url::parse(&link).ok())
                    {
                        link.query_pairs_mut()
                            .append_pair("authuser", &email)
                            .append_pair("shxr", "0");
                        self.open_url(link.as_str());
                    }
                }
                DetailsAction::Open(url) => self.open_url(&url),
                DetailsAction::Menu(MenuItem::Edit) => self.calendar_edit(&key, Field::Title, &now),
                DetailsAction::Edit(field) => self.calendar_edit(&key, field, &now),
                DetailsAction::EditDate => {
                    self.calendar_edit(&key, Field::When, &now);
                    if let Some(form) = &mut self.calendar.form {
                        form.focus(Field::When);
                    }
                }
                DetailsAction::Copy(text) => self.calendar_copy(ui.ctx(), text),
                DetailsAction::Menu(MenuItem::Delete) => self.calendar_delete(&key, &now),
            }
        }
        if let Some(alert) = self
            .calendar
            .details
            .as_mut()
            .and_then(|details| details.alert.as_mut())
        {
            let title = format!("RSVP {} to recurring event", answer_word(alert.answer));
            let mut choice = alert.choice;
            let mut clicked = None;
            egui::Area::new(exo_ui::id(ui.ctx(), "calendar-rsvp-alert"))
                .order(egui::Order::Tooltip)
                .fixed_pos(page.min)
                .show(ui.ctx(), |area| {
                    // The alert's overlay: the page dimmed to half, nothing behind it clickable.
                    area.painter()
                        .rect_filled(page, 0.0, egui::Color32::from_black_alpha(128));
                    let _ = area.interact(
                        page,
                        exo_ui::id(area.ctx(), "calendar-rsvp-alert-overlay"),
                        egui::Sense::click(),
                    );
                    clicked = exo_ui::Alert {
                        title: &title,
                        text: "",
                        actions: &["OK", "Cancel"],
                        selected: 0,
                    }
                    .show_options(
                        area,
                        page,
                        &["This event", "All events"],
                        &mut choice,
                    );
                });
            alert.choice = choice;
            match clicked {
                Some(0) => self.calendar_run(Action::DetailsConfirm),
                Some(_) => self.calendar_run(Action::DetailsEscape),
                None => {}
            }
        }
    }

    /// Event `key` as the calendar shows it, Calendar's event behind it, and whether I can
    /// answer it; an invite the calendar doesn't have stands in as its own event.
    fn calendar_detailed(
        &self,
        key: &str,
        now: &Zoned,
    ) -> Option<(Event, gmail::calendar::Event, bool)> {
        let email = self.account.email();
        let invite_uid = self
            .calendar
            .invite
            .as_ref()
            .map(|invite| invite.uid.as_str());
        if let Some(event) = self
            .calendar_events(now, invite_uid)
            .into_iter()
            .find(|event| event.key == key)
        {
            let data = self.account.calendar().data();
            let source = data
                .events
                .iter()
                .find(|stored| {
                    stored.calendar_id == event.calendar_id && stored.event.id == event.id
                })?
                .event
                .clone();
            let writable = data
                .calendars
                .iter()
                .flatten()
                .find(|calendar| calendar.entry.id.as_deref() == Some(event.calendar_id.as_str()))
                .is_some_and(|calendar| {
                    matches!(
                        calendar.entry.access_role.as_deref(),
                        Some("owner" | "writer")
                    )
                });
            let can_respond = writable && event.rsvp.is_some();
            return Some((event, source, can_respond));
        }
        let invite = self.calendar.invite.as_ref()?;
        let colors = event::colors(self.account.settings());
        let event = Event::from_invite(
            invite,
            colors.get(email).copied().unwrap_or(CalendarColor::BLUE),
            email,
            now.time_zone(),
        );
        let can_respond = invite.request && event.rsvp.is_some();
        (event.key == key).then(|| (event, invite_event(invite), can_respond))
    }

    /// A series instance's rule line: a new series' own, else what Calendar says of the series,
    /// asked for the first time the details open on it.
    fn calendar_rule_line(&self, event: &Event, source: &gmail::calendar::Event) -> Option<String> {
        let series = source.recurring_event_id.as_deref()?;
        let rules = if source.recurrence.is_empty() {
            let data = self.account.calendar().data();
            let known = data
                .series
                .get(&format!("{}/{series}", event.calendar_id))
                .cloned();
            if known.is_none() {
                self.account
                    .calendar()
                    .load_series(&event.calendar_id, series);
            }
            known.unwrap_or_default()
        } else {
            source.recurrence.clone()
        };
        // The instance's own start before any move.
        let tz = event.start.time_zone();
        let anchor = source
            .original_start_time
            .as_ref()
            .and_then(|time| match &time.date {
                Some(date) => date.parse::<Date>().ok(),
                None => time.timestamp(tz).map(|at| at.to_zoned(tz.clone()).date()),
            });
        let rule = exo_store::Recurrence::from_rules(&rules);
        Some(repeat::details_line(
            rule.as_ref(),
            anchor.unwrap_or_else(|| event.start.date()),
        ))
    }

    /// The `Label` row of an event with its own colour (`via {calendar}`), or the `Calendar` row:
    /// the event's calendar, when more than one shows.
    fn calendar_name(&self, event: &Event, source: &gmail::calendar::Event) -> Option<String> {
        let data = self.account.calendar().data();
        let calendar = &data
            .calendars
            .iter()
            .flatten()
            .find(|calendar| calendar.entry.id.as_deref() == Some(event.calendar_id.as_str()))?
            .entry;
        let name = calendar
            .summary_override
            .clone()
            .or_else(|| calendar.summary.clone())?;
        if source.color_id.is_some() {
            return Some(format!("via {name}"));
        }
        (self
            .account
            .settings()
            .calendars_where(|setting| setting.show)
            .len()
            > 1)
        .then_some(name)
    }

    /// Queues my answer to the open event (to the series when `all`), and says so.
    fn calendar_respond(&mut self, answer: Rsvp, all: bool) {
        let Some(key) = self
            .calendar
            .details
            .as_ref()
            .map(|details| details.key.clone())
        else {
            return;
        };
        let now = self.now.clone();
        let Some((event, source, _)) = self.calendar_detailed(&key, &now) else {
            return;
        };
        let status = match answer {
            Rsvp::Accepted => "accepted",
            Rsvp::Tentative => "tentative",
            Rsvp::Declined => "declined",
            Rsvp::NeedsAction => return,
        };
        if event.id.is_none() {
            let calendar = self.calendar.invite.as_ref().map_or_else(
                || self.account.email().to_owned(),
                |invite| self.calendar_invite_account(invite),
            );
            let thread = self.thread.as_ref().map(|thread| thread.id().to_owned());
            self.account
                .calendar()
                .queue(exo_store::CalendarChange::RespondToInvite {
                    calendar: calendar.clone(),
                    invite: source,
                    status: status.to_owned(),
                    thread,
                });
            if let Some(invite) = &mut self.calendar.invite {
                invite.answer(&calendar, answer.into());
            }
            self.commands
                .toast(Toast::new(self.rsvp_toast(answer.into())));
            return;
        }
        self.account
            .calendar()
            .queue(exo_store::CalendarChange::Respond {
                calendar: event.calendar_id.clone(),
                event: source,
                status: status.to_owned(),
                all,
            });
        let text = if self.snapshot.connection.is_online() {
            let scope = if all { "all events" } else { "this event" };
            format!("Responded {} to {scope}.", answer_word(answer))
        } else {
            "RSVP will be sent when online.".to_owned()
        };
        self.commands.toast(Toast::new(text));
    }

    /// An invite card's answer: to the calendar's event with its UID, added from the invite if need
    /// be. `invite` has my answer recorded; `mouse` says it came from a click, which earns the hint
    /// that `tab` gets there. With `RSVP + Mark Done` on, the conversation is marked done too, with
    /// that toast.
    pub(crate) fn calendar_respond_to_invite(
        &mut self,
        invite: Invite,
        response: Response,
        mouse: bool,
    ) {
        // To a signed-out calendar account, which signs in first: `RSVP`, or offline
        // `You're Offline`.
        if let Some(email) = self.calendar_invite_needs_sign_in(&invite) {
            if mouse {
                self.commands.hint(
                    Hint::new()
                        .text("Next time, hit")
                        .keys(["tab"])
                        .text("to RSVP instantly!"),
                );
            }
            if self.snapshot.connection.is_online() {
                self.calendar.prompt = Some(Prompt::SignIn {
                    email,
                    invite: Box::new(invite),
                    response,
                    provider: provider::ProviderPicker::default(),
                });
            } else {
                self.pickers.inform(
                    "You're Offline",
                    "Please go online to respond to this invitation.".to_owned(),
                );
            }
            return;
        }
        let mark_done = self.rsvp_marks_done();
        // The card shows this answer now, not the calendar's from before it.
        self.calendar.answers.1.insert(invite.uid.clone(), response);
        // To the calendar of the account it invites.
        let calendar = self.calendar_invite_account(&invite);
        let thread = self.thread.as_ref().map(|thread| thread.id().to_owned());
        self.account
            .calendar()
            .queue(exo_store::CalendarChange::RespondToInvite {
                calendar,
                invite: invite_event(&invite),
                status: response.status().to_owned(),
                thread,
            });
        if self
            .calendar
            .invite
            .as_ref()
            .is_some_and(|shown| shown.uid == invite.uid)
        {
            self.calendar.invite = Some(invite);
        }
        if mark_done {
            self.rsvp_mark_done(response);
        } else {
            self.commands.toast(Toast::new(self.rsvp_toast(response)));
        }
        // After a mouse RSVP, a hint that `tab` gets there.
        if mouse {
            let then = if response == Response::NeedsAction {
                "to add to your calendar instantly!"
            } else {
                "to RSVP instantly!"
            };
            self.commands
                .hint(Hint::new().text("Next time, hit").keys(["tab"]).text(then));
        }
    }

    /// The open thread's invite cards show my answer as given here since it opened, else as the
    /// calendar has it (the `.ics` has the answer from when it was sent).
    fn calendar_read_answers(&mut self) {
        let Some(thread) = &self.thread else {
            self.calendar.answers = (String::new(), BTreeMap::new());
            return;
        };
        if self.calendar.answers.0 != thread.id() {
            self.calendar.answers = (thread.id().to_owned(), BTreeMap::new());
        }
        let uids = thread.invite_uids();
        if uids.is_empty() {
            return;
        }
        let data = self.account.calendar().data();
        let me = self.account.email().to_owned();
        for uid in uids {
            let calendar = || {
                let stored = data
                    .events
                    .iter()
                    .find(|stored| stored.event.ical_uid.as_deref() == Some(uid.as_str()))?;
                let mine = stored.event.attendees.iter().find(|attendee| {
                    attendee.is_self == Some(true)
                        || attendee
                            .email
                            .as_deref()
                            .is_some_and(|email| email.eq_ignore_ascii_case(&me))
                })?;
                Response::parse(mine.response_status.as_deref()?)
            };
            let Some(answer) = self.calendar.answers.1.get(&uid).copied().or_else(calendar) else {
                continue;
            };
            if let Some(thread) = &mut self.thread {
                thread.show_calendar_answer(&uid, answer);
            }
        }
    }

    /// The calendars for the week view's list: the mail account's, then each other calendar
    /// account's by address, each primary first and then by name; rooms, weather and deleted
    /// calendars left out.
    pub(crate) fn calendar_list(&self) -> Vec<ListedCalendar> {
        let data = self.account.calendar().data();
        let email = self.account.email();
        let settings = self.account.settings();
        let mut listed: Vec<(bool, ListedCalendar)> = data
            .calendars
            .iter()
            .flatten()
            .filter(|calendar| listable(&calendar.entry))
            .filter_map(
                |AccountCalendar {
                     account,
                     entry: calendar,
                 }| {
                    let id = calendar.id.clone()?;
                    let setting = settings
                        .calendar_accounts
                        .get(account)
                        .and_then(|account| account.calendars.get(&id));
                    let name = calendar
                        .summary_override
                        .clone()
                        .or_else(|| calendar.summary.clone())
                        .unwrap_or_else(|| id.clone());
                    let color = setting
                        .and_then(|setting| setting.color.as_deref())
                        .and_then(CalendarColor::from_name)
                        .unwrap_or(CalendarColor::BLUE);
                    Some((
                        calendar.primary == Some(true),
                        ListedCalendar {
                            account: account.clone(),
                            shown: setting.is_some_and(|setting| setting.show),
                            id,
                            name,
                            color,
                        },
                    ))
                },
            )
            .collect();
        let key = |account: &str| (account != email, account.to_lowercase());
        listed.sort_by(|(a_primary, a), (b_primary, b)| {
            key(&a.account)
                .cmp(&key(&b.account))
                .then(b_primary.cmp(a_primary))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        listed.into_iter().map(|(_, calendar)| calendar).collect()
    }

    /// Each calendar account's calendars switched on the first time its list comes: the mail
    /// account's primary and the ones Google shows; another account's primary alone, as signing it
    /// in does.
    fn calendar_enable_by_default(&mut self, calendars: &[AccountCalendar]) {
        let email = self.account.email().to_owned();
        let settings = self.account.settings();
        let mut accounts: Vec<String> = settings
            .calendar_accounts
            .iter()
            .filter(|(_, account)| account.authed && account.calendars.is_empty())
            .map(|(address, _)| address.clone())
            .collect();
        if !settings.calendar_accounts.contains_key(&email) {
            accounts.push(email.clone());
        }
        for account in accounts {
            let theirs: Vec<CalendarListEntry> = calendars
                .iter()
                .filter(|calendar| calendar.account == account)
                .map(|calendar| calendar.entry.clone())
                .collect();
            if theirs.is_empty() {
                continue;
            }
            self.account.update_settings(|settings| {
                let enabled = enable_by_default(settings, &account, &theirs, account != email);
                settings.calendar_accounts.insert(account.clone(), enabled);
            });
        }
    }

    /// ⌘D with the right pane's form open: the keyboard goes from the mail to the form (the field
    /// it left), from the form to a popped-out draft if there is one, else back to the mail, and
    /// from the draft to the mail.
    fn calendar_cycle_focus(&mut self) {
        let popout = self.popout_focus();
        let ctx = self.ctx.clone();
        let Some(form) = self.calendar.form.as_mut() else {
            return;
        };
        match popout {
            Some(true) => self.focus_popout(false),
            _ if form.away => form.take_keyboard(),
            Some(false) => {
                form.give_up_keyboard(&ctx);
                self.focus_popout(true);
            }
            None => form.give_up_keyboard(&ctx),
        }
    }

    /// A key in a calendar row's colour picker: the calendar takes the colour at once, and Enter or
    /// Escape closes it, Escape putting back the colour it opened with.
    fn calendar_picker_key(&mut self, key: exo_ui::PickerKey) {
        let Some((id, start)) = self
            .calendar
            .week
            .as_ref()
            .and_then(|week| week.picker.clone())
        else {
            return;
        };
        let colors: Vec<CalendarColor> = CalendarColor::picker().collect();
        let index = |color: CalendarColor| {
            colors
                .iter()
                .position(|candidate| *candidate == color)
                .unwrap_or(0)
        };
        let current = self
            .calendar_list()
            .into_iter()
            .find(|calendar| calendar.id == id)
            .map_or(index(start), |calendar| index(calendar.color));
        match exo_ui::picker_key(key, current, index(start), colors.len(), PICKER_ROW) {
            exo_ui::PickerMove::Preview(pick) => self.calendar_recolor(&id, colors[pick]),
            exo_ui::PickerMove::Close(pick) => {
                self.calendar_recolor(&id, colors[pick]);
                if let Some(week) = &mut self.calendar.week {
                    week.picker = None;
                }
            }
        }
    }

    /// The calendar account calendar `id` is read through (the mail account for one no list
    /// has), and Google's colour for it.
    fn calendar_holder(&self, id: &str) -> (String, Option<String>) {
        let data = self.account.calendar().data();
        let calendar = data
            .calendars
            .iter()
            .flatten()
            .find(|calendar| calendar.entry.id.as_deref() == Some(id));
        (
            calendar.map_or_else(
                || self.account.email().to_owned(),
                |calendar| calendar.account.clone(),
            ),
            calendar.and_then(|calendar| calendar.entry.background_color.clone()),
        )
    }

    fn calendar_recolor(&mut self, id: &str, color: CalendarColor) {
        let (holder, _) = self.calendar_holder(id);
        self.account.update_settings(|settings| {
            settings.update_calendar(&holder, id, |setting| {
                setting.color = Some(color.name().to_owned())
            })
        });
    }

    fn calendar_toggle(&mut self, id: &str) {
        let (holder, provider) = self.calendar_holder(id);
        self.account.update_settings(|settings| {
            let on = settings
                .calendar_accounts
                .get(&holder)
                .and_then(|account| account.calendars.get(id))
                .is_none_or(|setting| !setting.show);
            let color = settings
                .calendar_accounts
                .get(&holder)
                .and_then(|account| account.calendars.get(id))
                .and_then(|setting| setting.color.clone())
                .unwrap_or_else(|| {
                    provider
                        .as_deref()
                        .and_then(snap)
                        .unwrap_or_else(|| {
                            next_unused(
                                settings
                                    .calendar_accounts
                                    .values()
                                    .flat_map(|account| account.calendars.values()),
                            )
                        })
                        .name()
                        .to_owned()
                });
            settings.update_calendar(&holder, id, |setting| {
                setting.show = on;
                setting.sync = on;
                setting.color.get_or_insert_with(|| color.clone());
            });
        });
    }
}

/// `Yes`, `Maybe` or `No`.
fn answer_word(answer: Rsvp) -> &'static str {
    match answer {
        Rsvp::Accepted => "Yes",
        Rsvp::Tentative => "Maybe",
        Rsvp::Declined | Rsvp::NeedsAction => "No",
    }
}

/// An invite's event as Calendar would have it: what the event details show of an invite the
/// calendar doesn't have, and what's imported to answer it.
pub(crate) fn invite_event(invite: &Invite) -> gmail::calendar::Event {
    use gmail::calendar::{Event as ApiEvent, EventAttendee, EventDateTime, EventPerson};
    let at = |time: jiff::Timestamp| EventDateTime {
        date_time: Some(time.to_string()),
        ..EventDateTime::default()
    };
    let organizer = invite.organizer.as_ref().map(|(name, email)| EventPerson {
        email: Some(email.clone()),
        display_name: (!name.is_empty()).then(|| name.clone()),
        ..EventPerson::default()
    });
    ApiEvent {
        ical_uid: Some(invite.uid.clone()),
        summary: (!invite.summary.is_empty()).then(|| invite.summary.clone()),
        description: (!invite.description.is_empty()).then(|| invite.description.clone()),
        location: (!invite.location.is_empty()).then(|| invite.location.clone()),
        start: Some(at(invite.start)),
        end: Some(at(invite.end)),
        attendees: invite
            .attendees
            .iter()
            .map(|(name, email, response)| EventAttendee {
                email: Some(email.clone()),
                display_name: (!name.is_empty()).then(|| name.clone()),
                organizer: invite
                    .organizer
                    .as_ref()
                    .is_some_and(|(_, organizer)| organizer.eq_ignore_ascii_case(email))
                    .then_some(true),
                response_status: Some(response.status().to_owned()),
                ..EventAttendee::default()
            })
            .collect(),
        organizer,
        ..ApiEvent::default()
    }
}

/// Whether calendar settings list `calendar`: not deleted, not a weather or room calendar.
fn listable(calendar: &CalendarListEntry) -> bool {
    let id = calendar.id.as_deref().unwrap_or_default();
    calendar.deleted != Some(true)
        && !id.contains("weather@group.v.calendar.google.com")
        && !id.contains("@resource.calendar.google.com")
}

/// The event form's state, for its palette rows.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FormState {
    create: bool,
    all_day: bool,
    meet: bool,
    /// About has the keyboard: the formatting keys that apply to it, by their place in
    /// [`FormatCommand::ALL`], and whether the editor takes Escape and Tab.
    about: Option<(u16, bool, bool)>,
}

/// What the calendar registers besides its standing keys.
#[derive(Clone, Copy, Debug)]
pub(crate) struct State {
    form: Option<FormState>,
    /// An alert or dialog: how many actions, and the selected one.
    prompt: Option<(usize, usize)>,
    /// Calendar Settings is up.
    options: bool,
    /// The open thread's invite card: whether it has the keyboard, or can take it.
    invite: Option<bool>,
    /// The week view's Meet: whether it has guests, and whether Find Time is on.
    meet: Option<(bool, bool)>,
    /// The week view is up, and whether it's sharing availability.
    share: Option<bool>,
    dragging: bool,
    /// A calendar's colour picker is open.
    picking: bool,
    /// The week shows its weekends.
    weekends: bool,
}

/// ⌘D while the right pane's form is open: after compose's, so it wins over the pop-out's own.
pub(crate) fn register_focus_cycle(commands: &mut Commands<AppAction>) {
    const SECTIONS: &[Region] = &[
        Region::ThreadList,
        Region::ThreadListHeader,
        Region::ThreadMessagesHeader,
        Region::ThreadMessages,
        Region::ThreadMessagesNewMessages,
        Region::ThreadMessageCalendarInvite,
        Region::ActionPane,
        Region::SearchBar,
        Region::ComposeForm,
        Region::PopOutComposeForm,
        Region::RecipientList,
        Region::CalendarCreate,
    ];
    commands.add(
        Command::shortcut("CYCLE_APP_FOCUS", AppAction::Calendar(Action::CycleFocus))
            .key("mod+d")
            .regions(SECTIONS),
    );
}

/// A calendar colour picker's rows: five colours to a row, of 25.
pub(crate) const PICKER_ROW: usize = 5;

/// Where a calendar colour picker has the keyboard.
pub(crate) const PICKER: &[Region] = &[Region::CalendarColorPicker];

/// A colour picker's keys while it has the keyboard in `regions` (every picker's: the
/// calendars', Notifications' and the Accounts dialog's): the arrows and Tab repeat while held,
/// ⌘O / ⌘⇧O and ⌘ an arrow don't.
pub(crate) fn register_picker_keys<A: Clone + 'static>(
    commands: &mut Commands<A>,
    regions: &'static [Region],
    action: impl Fn(exo_ui::PickerKey) -> A,
) {
    use exo_ui::PickerKey;
    let held = [
        (
            "CALENDAR_COLOR_PICKER_PREVIOUS",
            PickerKey::Previous,
            &["left", "shift+tab"][..],
        ),
        (
            "CALENDAR_COLOR_PICKER_NEXT",
            PickerKey::Next,
            &["right", "tab"],
        ),
        ("CALENDAR_COLOR_PICKER_UP", PickerKey::Up, &["up"]),
        ("CALENDAR_COLOR_PICKER_DOWN", PickerKey::Down, &["down"]),
    ];
    for (id, key, keys) in held {
        commands.add(
            Command::shortcut(id, action(key))
                .keys(keys.iter().copied())
                .repeat(action(key))
                .regions(regions),
        );
    }
    let pressed = [
        (
            "CALENDAR_COLOR_PICKER_CYCLE_BACK",
            PickerKey::Previous,
            &["mod+shift+o"][..],
        ),
        ("CALENDAR_COLOR_PICKER_CYCLE", PickerKey::Next, &["mod+o"]),
        (
            "CALENDAR_COLOR_PICKER_FIRST",
            PickerKey::First,
            &["mod+left", "mod+up"],
        ),
        (
            "CALENDAR_COLOR_PICKER_LAST",
            PickerKey::Last,
            &["mod+right", "mod+down"],
        ),
        ("CALENDAR_COLOR_PICKER_ESCAPE", PickerKey::Escape, &["esc"]),
        (
            "CALENDAR_COLOR_PICKER_ENTER",
            PickerKey::Enter,
            &["enter", "space"],
        ),
    ];
    for (id, key, keys) in pressed {
        commands.add(
            Command::shortcut(id, action(key))
                .keys(keys.iter().copied())
                .regions(regions),
        );
    }
}

/// Registers the calendar's keys and palette rows for this frame.
pub(crate) fn register(
    commands: &mut Commands<AppAction>,
    day_open: bool,
    week_open: bool,
    show_declined: bool,
    compose: bool,
    details: Option<bool>,
    state: State,
) {
    let action = AppAction::Calendar;
    if state.share == Some(true) {
        register_share(commands);
        if details.is_some() {
            commands.add(
                Command::shortcut("EVENT_DETAILS_CLOSE", action(Action::DetailsEscape)).key("esc"),
            );
        }
        return;
    }
    if week_open {
        register_week(commands, details.is_some());
    }
    // `Share Availability`: the list's and a conversation's row, and the week's, with its key,
    // while it's doing nothing else.
    if state.share == Some(false)
        && state.form.is_none()
        && !state.meet.is_some_and(|(_, finding)| finding)
    {
        commands.add(
            share_availability_command(action(Action::ShareAvailability))
                .key("mod+shift+a")
                .key_code(["⌘", "shift", "a"])
                .regions(WEEK),
        );
    } else if state.share.is_none() {
        commands.add(share_availability_command(action(Action::ShareAvailability)).regions(MAIL));
    }
    // `b` / `shift+b`: a new event's form (the instant event needs AI, which Exo leaves out).
    commands.add(
        Command::shortcut("CALENDAR_CREATE_EVENT", action(Action::Create))
            .keys(["b", "shift+b"])
            .regions(MAIL),
    );
    if state.form.is_none() {
        commands.add(
            Command::new("CREATE_EVENT", "Create Empty Event", action(Action::Create))
                .aliases([
                    "Empty Event",
                    "Create Empty Calendar Event",
                    "New Empty Event",
                    "Add Empty Event",
                    "Create Meeting",
                    "Send Invitation",
                ])
                .operative_word("event")
                .key_code(["B"])
                .default_score(1.3)
                .scale(0.99)
                .icon("data/calendar"),
        );
    }
    if let Some(form) = state.form {
        register_form(commands, form);
    }
    commands.add(
        Command::new(
            "CALENDAR_SETTINGS",
            "Calendar Settings",
            action(Action::OpenSettings),
        )
        .aliases([
            "Multiple Calendars",
            "Add Calendar",
            "Add Calendar Account",
            "Timezone Settings",
            "Meeting Link Settings",
            "Zoom Links",
            "Google Meet Links",
            "Conference Links",
            "Conference Calls",
            "Video Calls",
            "Event Settings",
        ])
        .icon("data/calendar"),
    );
    if state.options {
        commands.add(
            Command::shortcut("CALENDAR_SETTINGS_CLOSE", action(Action::CloseSettings))
                .key("esc")
                .regions(&[Region::Dialog]),
        );
    }
    if let Some((count, selected)) = state.prompt {
        commands.add(
            Command::shortcut("CALENDAR_ALERT_ENTER", action(Action::Prompt(selected)))
                .key("enter")
                .regions(ALERT),
        );
        commands.add(
            Command::shortcut("CALENDAR_ALERT_ESC", action(Action::Prompt(count - 1)))
                .key("esc")
                .regions(ALERT),
        );
        commands.add(
            Command::shortcut("CALENDAR_ALERT_UP", action(Action::PromptChoose(-1)))
                .key("up")
                .regions(ALERT),
        );
        commands.add(
            Command::shortcut("CALENDAR_ALERT_DOWN", action(Action::PromptChoose(1)))
                .key("down")
                .regions(ALERT),
        );
    }
    match state.invite {
        // With no draft to go to, an invite card takes the keyboard.
        Some(false) => commands.add(
            Command::shortcut("FOCUS_CALENDAR_INVITE", action(Action::InviteFocus))
                .key("tab")
                .regions(&[Region::ThreadMessages]),
        ),
        // The invite card's own keys.
        Some(true) => {
            const INVITE: &[Region] = &[Region::ThreadMessageCalendarInvite];
            commands.add(
                Command::shortcut("CALENDAR_INVITE_SUBMIT", action(Action::InviteSubmit))
                    .keys(["enter", "space"])
                    .regions(INVITE),
            );
            commands.add(
                Command::shortcut("CALENDAR_INVITE_ESCAPE", action(Action::InviteBlur))
                    .key("esc")
                    .regions(INVITE),
            );
            commands.add(
                Command::shortcut("CALENDAR_INVITE_NEXT", action(Action::InviteMove(1)))
                    .keys(["right", "down", "ctralt+j", "ctralt+n", "tab"])
                    .regions(INVITE),
            );
            commands.add(
                Command::shortcut("CALENDAR_INVITE_PREVIOUS", action(Action::InviteMove(-1)))
                    .keys(["left", "up", "ctralt+k", "ctralt+p"])
                    .regions(INVITE),
            );
        }
        None => {}
    }
    if let Some((guests, finding)) = state.meet.filter(|_| state.form.is_none()) {
        commands.add(
            Command::shortcut("MEET_WITH_FOCUS", action(Action::MeetFocus))
                .key("m")
                .regions(WEEK),
        );
        commands.add(
            Command::shortcut("MEET_WITH_INPUT_ESCAPE", action(Action::MeetBlur))
                .key("esc")
                .regions(&[Region::SeeYourWeekSidebarMeetWithInput]),
        );
        commands.add(
            Command::new("MEET_WITH", "Meet With", action(Action::MeetFocus))
                .key_code(["M"])
                .icon("data/calendar"),
        );
        if finding {
            // Over the week's own Escape.
            commands.add(
                Command::shortcut("FIND_TIME_ESCAPE", action(Action::CancelFindTime))
                    .key("esc")
                    .regions(WEEK),
            );
            commands.add(
                Command::new(
                    "FIND_TIME",
                    "Cancel Find Time",
                    action(Action::CancelFindTime),
                )
                .icon("data/calendar"),
            );
        } else if guests {
            commands.add(
                Command::new("FIND_TIME", "Find Time", action(Action::FindTime))
                    .icon("data/calendar"),
            );
        }
    }
    if state.picking {
        register_picker_keys(commands, PICKER, |key| action(Action::PickerKey(key)));
    }
    if state.dragging {
        // A drag's Escape, over everything else.
        commands
            .add(Command::shortcut("CALENDAR_DRAG_CANCEL", action(Action::CancelDrag)).key("esc"));
    }
    commands.add(
        Command::shortcut("OPEN_WEEK", action(Action::OpenWeek))
            .key("2")
            .regions(MAIL),
    );
    if !week_open {
        commands.add(
            Command::new("SEE_YOUR_WEEK", "Open Week", action(Action::OpenWeek))
                .aliases([
                    "Open Calendar",
                    "Open Weekly Calendar",
                    "Week View",
                    "See My Week",
                    "See Your Week",
                    "Open Calendar Week",
                ])
                .key_code(if compose { Vec::new() } else { vec!["2"] })
                .scale(1.0)
                .follow("CALENDAR_DAY")
                .icon("data/calendar"),
        );
    }
    commands.add(
        Command::shortcut("PREVIOUS_DAY", action(Action::Step(-1)))
            .key("-")
            .regions(MAIL),
    );
    commands.add(
        Command::shortcut("NEXT_DAY", action(Action::Step(1)))
            .keys(["=", "plus"])
            .regions(MAIL),
    );
    commands.add(
        Command::shortcut("CYCLE_RIGHT_PANE", action(Action::Cycle))
            .key("0")
            .regions(MAIL),
    );
    commands.add(
        Command::shortcut("CALENDAR_TODAY", action(Action::Today))
            .key("t")
            .regions(MAIL),
    );
    // Anywhere, composing included (`mod+shift+` on a Mac).
    commands.add(
        Command::shortcut("PREVIOUS_DAY_ANYWHERE", action(Action::Step(-1))).key("mod+shift+-"),
    );
    commands
        .add(Command::shortcut("NEXT_DAY_ANYWHERE", action(Action::Step(1))).key("mod+shift+="));
    commands.add(
        Command::shortcut("CYCLE_RIGHT_PANE_ANYWHERE", action(Action::Cycle)).key("mod+shift+0"),
    );
    let zero: &[&str] = if compose {
        &["⌘", "shift", "0"]
    } else {
        &["0"]
    };
    if day_open || week_open {
        commands.add(
            Command::new(
                "CHANGE_CALENDAR_DAY",
                "Jump to Date",
                action(Action::OpenJumpToDate),
            )
            .aliases(["Go to Date", "Change day"])
            .icon("data/calendar"),
        );
        commands.add(
            Command::new("CLOSE_CALENDAR", "Close Calendar", action(Action::Close))
                .key_code(if day_open { zero.to_vec() } else { vec!["1"] })
                .default_score(1.35)
                .icon("data/calendar"),
        );
    }
    if !day_open && !week_open {
        commands.add(
            Command::new("CALENDAR_DAY", "Open Day", action(Action::OpenDay))
                .aliases([
                    "Open Daily Calendar",
                    "Day View",
                    "See My Day",
                    "See Your Day",
                ])
                .key_code(zero.iter().copied())
                .default_score(1.35)
                .scale(0.5)
                .icon("data/calendar"),
        );
    }
    // `Show Weekends` / `Hide Weekends`, in the week view.
    if week_open {
        let title = if state.weekends {
            "Hide Weekends"
        } else {
            "Show Weekends"
        };
        commands.add(
            Command::new(
                "TOGGLE_CALENDAR_SHOW_WEEKENDS",
                title,
                action(Action::ToggleWeekends),
            )
            .aliases(["Show Weekends", "Hide Weekends"])
            .exclude_alias_from_title(["Show Weekends", "Hide Weekends"])
            .operative_word("weekends")
            .scale(1.01)
            .icon("data/calendar"),
        );
    }
    let title = if show_declined {
        "Hide Declined Events"
    } else {
        "Show Declined Events"
    };
    commands.add(
        Command::new(
            "TOGGLE_SHOW_DECLINED_EVENTS",
            title,
            action(Action::ToggleDeclined),
        )
        .aliases(["Show Declined Events", "Hide Declined Events"])
        .exclude_alias_from_title(["Show Declined Events", "Hide Declined Events"])
        .operative_word("declined")
        .scale(1.0)
        .follow("SEE_YOUR_WEEK")
        .icon("data/calendar"),
    );
    // The event details' own keys, registered last to win: Escape closes the alert, the menu or the
    // details; Enter is the alert's OK.
    if let Some(alert) = details {
        commands.add(
            Command::shortcut("EVENT_DETAILS_CLOSE", action(Action::DetailsEscape)).key("esc"),
        );
        if alert {
            commands.add(
                Command::shortcut("EVENT_DETAILS_CONFIRM", action(Action::DetailsConfirm))
                    .key("enter"),
            );
        }
    }
}

/// The form's keys and palette rows.
fn register_form(commands: &mut Commands<AppAction>, form: FormState) {
    let action = AppAction::Calendar;
    commands.add(
        Command::shortcut("CALENDAR_CREATE_SAVE_KEY", action(Action::Save))
            .key("mod+enter")
            .regions(FORM),
    );
    if !form.about.is_some_and(|(_, escape, _)| escape) {
        commands.add(
            Command::shortcut("CALENDAR_CREATE_ESCAPE", action(Action::FormEscape))
                .key("esc")
                .regions(FORM),
        );
    }
    // About's formatting keys, as compose's; outside a list Tab and ⇧Tab leave it (the editor
    // keeps them otherwise, and Tab while it has ghost text or a chosen suggestion).
    let applies = |command: FormatCommand| {
        form.about.is_some_and(|(formats, _, _)| {
            FormatCommand::ALL
                .iter()
                .position(|each| *each == command)
                .is_some_and(|at| formats & 1 << at != 0)
        })
    };
    for command in FormatCommand::ALL
        .into_iter()
        .filter(|command| applies(*command))
    {
        commands.add(
            Command::shortcut(command.id(), action(Action::FormFormat(command)))
                .keys(command.keys().iter().copied())
                .regions(FORM),
        );
    }
    if form.about.is_some() && !applies(FormatCommand::IndentList) {
        if form.create && !form.about.is_some_and(|(_, _, tab)| tab) {
            commands.add(
                Command::shortcut(
                    "CALENDAR_CREATE_ABOUT_TAB",
                    action(Action::FormFocus(Field::Calendar)),
                )
                .key("tab")
                .regions(FORM),
            );
        }
        commands.add(
            Command::shortcut(
                "CALENDAR_CREATE_ABOUT_SHIFT_TAB",
                action(Action::FormFocus(if form.create {
                    Field::Location
                } else {
                    Field::Guests
                })),
            )
            .key("shift+tab")
            .regions(FORM),
        );
    }
    commands.add(
        Command::shortcut("CALENDAR_CREATE_DISCARD", action(Action::Discard))
            .keys(["mod+shift+,", "mod+shift+."])
            .regions(FORM),
    );
    commands.add(
        Command::shortcut("CALENDAR_CREATE_NEXT", action(Action::FormStep(1)))
            .key("mod+shift+=")
            .regions(FORM),
    );
    commands.add(
        Command::shortcut("CALENDAR_CREATE_PREVIOUS", action(Action::FormStep(-1)))
            .key("mod+shift+-")
            .regions(FORM),
    );
    commands.add(
        Command::shortcut("CALENDAR_CREATE_TODAY", action(Action::FormToday))
            .key("mod+shift+0")
            .regions(FORM),
    );
    let rows: [(&str, &str, &[&str], Field, &str); 5] = [
        (
            "CALENDAR_CREATE_LOCATION",
            "Edit Location",
            &["Edit Place", "Edit Where"],
            Field::Location,
            "data/location",
        ),
        (
            "CALENDAR_CREATE_DESCRIPTION",
            "Edit Description",
            &["Edit About", "Edit Agenda"],
            Field::About,
            "data/description",
        ),
        (
            "CALENDAR_CREATE_ATTENDEES",
            "Edit Guests",
            &["Edit Who", "Add Guest", "Invite", "Edit Attendees"],
            Field::Guests,
            "data/edit",
        ),
        (
            "CALENDAR_CREATE_WHEN",
            "Edit Date & Time",
            &["Edit Date", "Edit Time", "Edit When"],
            Field::When,
            "data/edit",
        ),
        (
            "CALENDAR_CREATE_TITLE",
            "Edit Title",
            &["Edit Name"],
            Field::Title,
            "data/edit",
        ),
    ];
    for (id, title, aliases, field, icon) in rows {
        commands.add(
            Command::new(id, title, action(Action::FormFocus(field)))
                .aliases(aliases.iter().copied())
                .default_score(20.0)
                .icon(icon),
        );
    }
    if form.create {
        commands.add(
            Command::new(
                "CALENDAR_CREATE_CALENDAR",
                "Change Calendar",
                action(Action::FormFocus(Field::Calendar)),
            )
            .aliases(["Change Account", "Edit Calendar"])
            .default_score(20.0)
            .icon("data/edit"),
        );
    }
    commands.add(
        Command::new("CALENDAR_CREATE_SAVE", "Save Event", action(Action::Save))
            .aliases(["Create Event"])
            .key_code(["⌘", "Enter"])
            .default_score(20.0)
            .scale(5.0)
            .icon("data/done"),
    );
    let discard = if form.create {
        "Discard Event"
    } else {
        "Discard Changes"
    };
    commands.add(
        Command::new("CALENDAR_CREATE_CANCEL", discard, action(Action::Discard))
            .aliases(["Discard", "Close", "Cancel"])
            .key_code(["⌘", "shift", ","])
            .default_score(20.0)
            .scale(5.0)
            .icon("data/discard"),
    );
    let all_day = if form.all_day {
        "Remove all day"
    } else {
        "Make all day"
    };
    commands.add(
        Command::new("EDIT_CALENDAR_ALL_DAY", all_day, action(Action::FormAllDay))
            .aliases([
                "Make all day",
                "Remove all day",
                "Make full day",
                "Remove full day",
                "Toggle all day",
            ])
            .exclude_alias_from_title(["Make all day", "Remove all day"])
            .default_score(20.0)
            .icon("data/calendar"),
    );
    let verb = if form.meet { "Remove" } else { "Add" };
    commands.add(
        Command::new(
            "CALENDAR_CREATE_CONFERENCE_MEET",
            format!("{verb} Google Meet Link"),
            action(Action::FormMeet),
        )
        .aliases(
            [
                "Google Meet",
                "Google Video",
                "Google Hangouts",
                "G.co",
                "GMeet",
            ]
            .map(|name| format!("{verb} {name}")),
        )
        .icon("data/g-suite"),
    );
    // What only Google Calendar's own editor does: it opens with the form's event.
    if form.create {
        let external: [(&str, &str, &[&str]); 11] = [
            ("EDIT_CALENDAR_EVENT", "Edit event in Google Calendar", &[]),
            ("EDIT_CALENDAR_PERMISSIONS", "Change Guest Permissions", &[]),
            (
                "EDIT_CALENDAR_VISIBILITY",
                "Change Visibility",
                &["Change Event Privacy"],
            ),
            ("EDIT_CALENDAR_BUSY", "Change Busy/Free", &[]),
            ("EDIT_CALENDAR_COLOR", "Change Calendar Color", &[]),
            (
                "EDIT_CALENDAR_NOTIFICATIONS",
                "Add Notification",
                &["Add Reminder"],
            ),
            ("EDIT_CALENDAR_CONFERENCE", "Add Conference Room", &[]),
            ("EDIT_CALENDAR_RECURRING", "Make Recurring", &[]),
            ("EDIT_CALENDAR_MORE", "More Calendar Actions", &[]),
            ("EDIT_CALENDAR_TZ", "Change Time Zone", &[]),
            ("EDIT_CALENDAR_REPEAT", "Repeat Event", &[]),
        ];
        for (id, title, aliases) in external {
            commands.add(
                Command::new(id, title, action(Action::FormExternal))
                    .aliases(aliases.iter().copied())
                    .default_score(0.0)
                    .icon("data/calendar"),
            );
        }
    }
}

/// `Share Availability`'s palette row.
pub(crate) fn share_availability_command(action: AppAction) -> Command<AppAction> {
    Command::new("SHARE_AVAILABILITY", "Share Availability", action)
        .aliases([
            "Share Time",
            "Share Time Slots",
            "When Am I Free",
            "Find Times",
            "Suggest Times",
            "Propose Times",
            "Schedule",
            "Insert Booking Link",
            "Insert Booking Page",
            "Insert Calendar Link",
            "Insert Scheduling Link",
            "Insert Meeting Link",
            "Insert Meeting Times",
            "Insert Free Times",
            "Insert Availability",
            "Insert One-Time Link",
            "Insert One-Off Link",
            "Insert Single-Use Link",
            "Insert Recurring Link",
            "Calendly",
        ])
        .exclude_alias_from_title(["Calendly", "Find Times", "Schedule"])
        .default_score(1.4)
        .icon("data/upgrade-calendar")
}

/// Share Availability's keys: the week's, Enter and ⌘Enter to insert, Escape to cancel, and in
/// its fields ⌘Enter, ⌘⇧= / ⌘⇧– and Escape.
fn register_share(commands: &mut Commands<AppAction>) {
    const PAGE: &[Region] = &[Region::ShareAvailability];
    const FIELDS: &[Region] = &[
        Region::ShareAvailabilitySidebarAttendees,
        Region::ShareAvailabilitySidebarTimezoneInput,
    ];
    const ALL: &[Region] = &[
        Region::ShareAvailability,
        Region::ShareAvailabilitySidebarAttendees,
        Region::ShareAvailabilitySidebarTimezoneInput,
    ];
    let action = AppAction::Calendar;
    let next = action(Action::WeekStep(1));
    let previous = action(Action::WeekStep(-1));
    commands.add(
        Command::shortcut("SHARE_NEXT", next.clone())
            .keys(["=", "shift+=", "plus", "j", "n", "right"])
            .repeat(next.clone())
            .regions(PAGE),
    );
    commands.add(
        Command::shortcut("SHARE_PREVIOUS", previous.clone())
            .keys(["-", "shift+-", "k", "p", "left"])
            .repeat(previous.clone())
            .regions(PAGE),
    );
    commands.add(
        Command::shortcut("SHARE_NEXT_ANYWHERE", next.clone())
            .key("mod+shift+=")
            .repeat(next)
            .regions(ALL),
    );
    commands.add(
        Command::shortcut("SHARE_PREVIOUS_ANYWHERE", previous.clone())
            .key("mod+shift+-")
            .repeat(previous)
            .regions(ALL),
    );
    commands.add(
        Command::shortcut("SHARE_TODAY", action(Action::WeekToday))
            .keys(["space", "t"])
            .regions(PAGE),
    );
    commands.add(
        Command::shortcut("SHARE_CLOSE", action(Action::ShareCancel))
            .keys(["0", "mod+shift+0", "1", "esc"])
            .regions(PAGE),
    );
    commands.add(
        Command::shortcut("SHARE_INSERT", action(Action::ShareInsert))
            .keys(["enter", "mod+enter"])
            .regions(PAGE),
    );
    commands.add(
        Command::shortcut("SHARE_INSERT_FIELD", action(Action::ShareInsert))
            .key("mod+enter")
            .regions(FIELDS),
    );
    commands.add(
        Command::shortcut("SHARE_ESCAPE", action(Action::ShareEscape))
            .key("esc")
            .regions(FIELDS),
    );
}

/// The week view's keys; `b` waits while the details are up.
fn register_week(commands: &mut Commands<AppAction>, details: bool) {
    let action = AppAction::Calendar;
    if !details {
        commands.add(
            Command::shortcut("WEEK_CREATE_EVENT", action(Action::Create))
                .keys(["b", "shift+b"])
                .regions(WEEK),
        );
    }
    let next = action(Action::WeekStep(1));
    let previous = action(Action::WeekStep(-1));
    commands.add(
        Command::shortcut("WEEK_NEXT", next.clone())
            .keys(["=", "shift+=", "mod+shift+=", "plus", "j", "n", "right"])
            .repeat(next)
            .regions(WEEK),
    );
    commands.add(
        Command::shortcut("WEEK_PREVIOUS", previous.clone())
            .keys(["-", "shift+-", "mod+shift+-", "k", "p", "left"])
            .repeat(previous)
            .regions(WEEK),
    );
    commands.add(
        Command::shortcut("WEEK_TODAY", action(Action::WeekToday))
            .keys(["space", "t"])
            .regions(WEEK),
    );
    commands.add(
        Command::shortcut("WEEK_CLOSE", action(Action::CloseWeek))
            .keys(["0", "mod+shift+0", "1", "esc"])
            .regions(WEEK),
    );
}

/// The first day of `day`'s week, a Sunday or with `monday` a Monday.
fn week_start(day: Date, monday: bool) -> Date {
    let offset = if monday {
        day.weekday().to_monday_zero_offset()
    } else {
        day.weekday().to_sunday_zero_offset()
    };
    day - i64::from(offset).days()
}

/// The week from `first` as the tab's title names it: `Nov 16–22`, or `Nov 30–Dec 6` across two
/// months. It names the week the view shows, from whichever day it starts on
/// ([`crate::Mail::calendar_week_start`]).
pub(crate) fn week_title(first: Date) -> String {
    let last = first + 6.days();
    if first.month() == last.month() {
        format!("{} {}–{}", first.strftime("%b"), first.day(), last.day())
    } else {
        format!(
            "{} {}–{} {}",
            first.strftime("%b"),
            first.day(),
            last.strftime("%b"),
            last.day()
        )
    }
}

/// Calendar account `email`'s calendars switched on the first time its list comes, from its list
/// `calendars`: the primary one, and unless `primary_only` those Google shows, each in the colour
/// nearest Google's (or the first unused one of every account's).
fn enable_by_default(
    settings: &Settings,
    email: &str,
    calendars: &[CalendarListEntry],
    primary_only: bool,
) -> CalendarAccount {
    let mut account = CalendarAccount {
        email: email.to_owned(),
        ..settings
            .calendar_accounts
            .get(email)
            .cloned()
            .unwrap_or_default()
    };
    for calendar in calendars {
        let Some(id) = calendar.id.clone() else {
            continue;
        };
        let primary = calendar.primary == Some(true);
        if !(primary || (!primary_only && listable(calendar) && calendar.selected == Some(true))) {
            continue;
        }
        let others = settings
            .calendar_accounts
            .iter()
            .filter(|(address, _)| *address != email)
            .flat_map(|(_, other)| other.calendars.values());
        let color = calendar
            .background_color
            .as_deref()
            .and_then(snap)
            .unwrap_or_else(|| next_unused(others.chain(account.calendars.values())));
        account.calendars.insert(
            id.clone(),
            CalendarSetting {
                id,
                sync: true,
                show: true,
                color: Some(color.name().to_owned()),
                check_for_scheduling_conflicts: primary,
                notifications: primary.then_some(CalendarNotifications {
                    enabled: true,
                    enabled_desktop: true,
                }),
            },
        );
    }
    account
}

/// The first colour in the picker's order no calendar of any account has, or the least used.
fn next_unused<'a>(calendars: impl Iterator<Item = &'a CalendarSetting> + Clone) -> CalendarColor {
    let used = |color: CalendarColor| {
        calendars
            .clone()
            .filter(|calendar| calendar.color.as_deref() == Some(color.name()))
            .count()
    };
    CalendarColor::picker()
        .min_by_key(|color| used(*color))
        .unwrap_or(CalendarColor::BLUE)
}

/// The calendar colour nearest `hex` by a CIE94-style distance in Lab.
fn snap(hex: &str) -> Option<CalendarColor> {
    let hex = hex.strip_prefix('#')?;
    let channel = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
    let rgb = [channel(0)?, channel(2)?, channel(4)?];
    let target = lab(rgb);
    let base = |color: CalendarColor| {
        let [r, g, b, _] = color.base().to_array();
        lab([r, g, b])
    };
    // The first of equals wins.
    CalendarColor::all()
        .map(|color| (color, distance(target, base(color))))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(color, _)| color)
}

/// `rgb` (sRGB) in CIE Lab, against D65 white.
fn lab([r, g, b]: [u8; 3]) -> [f64; 3] {
    let linear = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v > 0.04045 {
            ((v + 0.055) / 1.055).powf(2.4)
        } else {
            v / 12.92
        }
    };
    let (r, g, b) = (linear(r), linear(g), linear(b));
    let f = |v: f64| {
        if v > 0.008856 {
            v.cbrt()
        } else {
            7.787 * v + 16.0 / 116.0
        }
    };
    let x = f((0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047);
    let y = f(0.2126 * r + 0.7152 * g + 0.0722 * b);
    let z = f((0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883);
    [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
}

/// The distance between two Lab colours that [`snap`] minimizes.
fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    let (dl, da, db) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    let (c1, c2) = (a[1].hypot(a[2]), b[1].hypot(b[2]));
    let dc = c1 - c2;
    let dh = (da * da + db * db - dc * dc).max(0.0).sqrt();
    let squared = dl * dl + (dc / (1.0 + 0.045 * c1)).powi(2) + (dh / (1.0 + 0.015 * c1)).powi(2);
    squared.max(0.0).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_week_starts_on_sunday_or_monday() {
        let tuesday = Date::constant(2026, 11, 17);
        assert_eq!(week_start(tuesday, false), Date::constant(2026, 11, 15));
        assert_eq!(week_start(tuesday, true), Date::constant(2026, 11, 16));
        assert_eq!(
            week_start(Date::constant(2026, 11, 15), true),
            Date::constant(2026, 11, 9),
            "a Sunday is its week's last day"
        );
    }

    #[test]
    fn the_tab_names_the_week_shown() {
        assert_eq!(week_title(Date::constant(2026, 11, 15)), "Nov 15–21");
        assert_eq!(
            week_title(Date::constant(2026, 11, 16)),
            "Nov 16–22",
            "from a Monday"
        );
        assert_eq!(
            week_title(Date::constant(2026, 11, 29)),
            "Nov 29–Dec 5",
            "across two months"
        );
        assert_eq!(
            week_title(Date::constant(2026, 12, 27)),
            "Dec 27–Jan 2",
            "and two years"
        );
    }

    #[test]
    fn google_colours_snap_to_the_nearest_calendar_colour() {
        // The mock's calendars.
        assert_eq!(snap("#9fe1e7").map(CalendarColor::name), Some("blue"));
        assert_eq!(snap("#42d692").map(CalendarColor::name), Some("green"));
        assert_eq!(snap("#16a765").map(CalendarColor::name), Some("forest"));
    }

    #[test]
    fn the_primary_and_selected_calendars_switch_on() {
        let entry =
            |id: &str, primary: bool, selected: bool, color: Option<&str>| CalendarListEntry {
                id: Some(id.into()),
                primary: primary.then_some(true),
                selected: Some(selected),
                background_color: color.map(Into::into),
                ..CalendarListEntry::default()
            };
        let mut settings = Settings::default();
        let account = enable_by_default(
            &settings,
            "maya@brightmoor.co",
            &[
                entry("maya@brightmoor.co", true, true, Some("#9fe1e7")),
                entry("team", false, true, None),
                entry("other", false, false, None),
                entry("room@resource.calendar.google.com", false, true, None),
            ],
            false,
        );
        let colors: Vec<(&str, Option<&str>)> = account
            .calendars
            .values()
            .map(|calendar| (calendar.id.as_str(), calendar.color.as_deref()))
            .collect();
        assert_eq!(
            colors,
            [("maya@brightmoor.co", Some("blue")), ("team", Some("red"))],
            "unlisted and room calendars stay off; no colour takes the first unused"
        );
        assert!(account.calendars["maya@brightmoor.co"].check_for_scheduling_conflicts);

        // Another account signed in switches its primary on alone, in a colour no account has.
        settings
            .calendar_accounts
            .insert("maya@brightmoor.co".into(), account);
        let other = enable_by_default(
            &settings,
            "maya.lin@example.com",
            &[
                entry("maya.lin@example.com", true, true, None),
                entry("family", false, true, None),
            ],
            true,
        );
        let colors: Vec<(&str, Option<&str>)> = other
            .calendars
            .values()
            .map(|calendar| (calendar.id.as_str(), calendar.color.as_deref()))
            .collect();
        assert_eq!(colors, [("maya.lin@example.com", Some("pink"))]);
    }
}
