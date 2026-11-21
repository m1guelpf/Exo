//! Out of Office Reply: the dialog that edits Gmail's vacation responder, the green status bar
//! while replies go out, and the nudge to set one for out-of-office days on the calendar.
//!
//! Exo reads and writes Gmail's `settings/vacation` itself, and keeps a copy of it in the account's
//! settings, which the bar and the nudge go by. The copy is brought up to date when the mail opens,
//! every hour, when the dialog opens and when it saves. Out-of-office events on the primary
//! calendar in the next 35 days are looked for when the mail opens and every 15 minutes, and the
//! nudge looks at them 2 s after the mail shows and whenever new ones turn up.

mod alerts;
mod date_input;
mod dialog;
mod spelling;

use std::sync::{Arc, Mutex};

use egui::{Context, Rect};
use exo_commands::{Command, Commands, Region, Toast};
use exo_editor::FormatCommand;
use exo_store::{OUT_OF_OFFICE_LOOKAHEAD, OutOfOfficeDays, OutOfOfficeResponder};
use gmail::VacationSettings;
use gmail::calendar::Event as CalendarEvent;

pub use alerts::Alert;
pub use dialog::{Control, Event as DialogEvent, Form};

use crate::{Mail, commands};

/// How often Gmail's responder is read: an hour.
const READ_EVERY: f64 = 3600.0;
/// How often the calendar is looked through: 15 minutes.
const EVENTS_EVERY: f64 = 900.0;
/// The nudge looks this long after the mail shows.
const NUDGE_AFTER: f64 = 2.0;
/// The palette's command.
const COMMAND: &str = "Out of Office Reply Settings";
/// The save's toast.
const SAVED: &str = "Out of Office Reply settings updated.";

/// What the commands, the menu row and the dialog ask for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// The dialog, or offline the alert.
    Open,
    /// Something in the dialog.
    Dialog(DialogEvent),
    /// A formatting key in the message.
    Format(FormatCommand),
    /// ⌘⇧– or ⌘⇧= on a date in the message: a day back or on.
    ShiftDate(i64),
    /// Something in an alert.
    Alert(AlertEvent),
}

/// What an alert's keys and buttons do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertEvent {
    /// Enter: the selected one.
    Confirm,
    /// Escape.
    Dismiss,
    /// The arrows: the selection moves by one.
    Cycle(isize),
}

/// What Gmail or Calendar answered, handed back to the UI thread.
enum Outcome {
    /// Gmail's responder, read for the dialog or on the hour; `Err` when it couldn't be.
    Read {
        settings: Result<VacationSettings, ()>,
        dialog: bool,
    },
    /// What Gmail kept of a save.
    Saved(Result<VacationSettings, ()>),
    /// The primary calendar's events coming up.
    Events(Vec<CalendarEvent>),
}

/// Out of Office Reply's dialog, alerts and polls.
#[derive(Default)]
pub(crate) struct OutOfOffice {
    dialog: Option<dialog::Dialog>,
    alert: Option<(Alert, usize)>,
    /// Answers from the account's runtime.
    outcomes: Arc<Mutex<Vec<Outcome>>>,
    /// When (in egui's time) Gmail and the calendar were last asked.
    read_at: Option<f64>,
    events_at: Option<f64>,
    /// When the mail first showed.
    shown_at: Option<f64>,
    /// The nudge should look at the events again.
    nudge_pending: bool,
}

impl OutOfOffice {
    /// The region with the keyboard: an alert's, or the dialog's.
    pub(crate) fn region(&self) -> Option<Region> {
        if self.alert.is_some() {
            Some(Region::Alert)
        } else {
            self.dialog
                .as_ref()
                .filter(|dialog| dialog.open)
                .map(|_| Region::Dialog)
        }
    }
}

/// Registers the palette's `Out of Office Reply Settings` and, while the dialog or an alert has
/// the keyboard, their keys.
pub(crate) fn register(commands: &mut Commands<commands::Action>, state: &OutOfOffice) {
    let act = commands::Action::OutOfOffice;
    commands.add(
        Command::new("OUT_OF_OFFICE_REPLY_SETTINGS", COMMAND, act(Action::Open))
            .aliases(["OOO Settings", "Out of Office Settings", "Vacation Mode"])
            .icon("data/settings"),
    );
    if state.alert.is_some() {
        const ALERT: &[Region] = &[Region::Alert];
        let key = |id: &'static str, event: AlertEvent, keys: &[&'static str]| {
            Command::shortcut(id, act(Action::Alert(event)))
                .keys(keys.iter().copied())
                .regions(ALERT)
        };
        commands.add(key(
            "OOO_ALERT_CONFIRM",
            AlertEvent::Confirm,
            &["enter", "mod+enter", "space"],
        ));
        commands.add(key("OOO_ALERT_ESCAPE", AlertEvent::Dismiss, &["esc"]));
        commands.add(key(
            "OOO_ALERT_NEXT",
            AlertEvent::Cycle(-1),
            &["right", "tab"],
        ));
        commands.add(key(
            "OOO_ALERT_PREVIOUS",
            AlertEvent::Cycle(1),
            &["left", "shift+tab"],
        ));
        return;
    }
    let Some(dialog) = state.dialog.as_ref().filter(|dialog| dialog.open) else {
        return;
    };
    const DIALOG: &[Region] = &[Region::Dialog];
    let key = |id: &'static str, event: DialogEvent, keys: &[&'static str]| {
        Command::shortcut(id, act(Action::Dialog(event)))
            .keys(keys.iter().copied())
            .regions(DIALOG)
    };
    let focus = dialog.focus();
    if !dialog.editor_wants_escape() {
        commands.add(key("OOO_ESCAPE", DialogEvent::Close, &["esc"]));
    }
    let in_list = focus == Some(Control::Message) && dialog.editor().in_list();
    if !in_list {
        // Ghost text or a chosen suggestion takes Tab in the message.
        if !(focus == Some(Control::Message) && dialog.editor().wants_tab()) {
            commands.add(key("OOO_TAB", DialogEvent::Tab(1), &["tab"]));
        }
        commands.add(key("OOO_SHIFT_TAB", DialogEvent::Tab(-1), &["shift+tab"]));
    }
    match focus {
        Some(Control::Message) => {
            for command in FormatCommand::ALL
                .into_iter()
                .filter(|command| command.applies(dialog.editor()))
            {
                commands.add(
                    Command::shortcut(command.id(), act(Action::Format(command)))
                        .keys(command.keys().iter().copied())
                        .regions(DIALOG),
                );
            }
            // The date tooltip's keys, so what it promises works here too.
            if dialog.on_date() {
                commands.add(
                    Command::shortcut("OOO_PREVIOUS_DAY", act(Action::ShiftDate(-1)))
                        .key("mod+shift+-")
                        .regions(DIALOG),
                );
                commands.add(
                    Command::shortcut("OOO_NEXT_DAY", act(Action::ShiftDate(1)))
                        .key("mod+shift+=")
                        .regions(DIALOG),
                );
            }
        }
        Some(Control::Subject) | None => {}
        Some(Control::Start | Control::End) => {
            commands.add(key("OOO_DATE_ENTER", DialogEvent::Enter, &["enter"]));
            commands.add(key("OOO_DATE_DOWN", DialogEvent::Down, &["down"]));
        }
        Some(_) => {
            commands.add(key("OOO_ENTER", DialogEvent::Enter, &["enter"]));
            commands.add(key("OOO_SPACE", DialogEvent::Space, &["space"]));
        }
    }
}

impl Mail {
    /// What Gmail and Calendar answered since last frame, the polls that are due, and the
    /// nudge. Call each frame before drawing.
    pub(crate) fn out_of_office_frame(&mut self, ctx: &Context) {
        let time = ctx.input(|input| input.time);
        let outcomes = std::mem::take(
            &mut *self
                .out_of_office
                .outcomes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for outcome in outcomes {
            self.out_of_office_outcome(outcome);
        }
        if self
            .out_of_office
            .read_at
            .is_none_or(|at| time - at >= READ_EVERY)
        {
            self.out_of_office.read_at = Some(time);
            self.read_vacation(false);
        }
        if self
            .out_of_office
            .events_at
            .is_none_or(|at| time - at >= EVENTS_EVERY)
        {
            self.out_of_office.events_at = Some(time);
            self.read_events();
        }
        let shown = *self.out_of_office.shown_at.get_or_insert_with(|| {
            self.out_of_office.nudge_pending = true;
            time
        });
        let wait = shown + NUDGE_AFTER - time;
        if self.out_of_office.nudge_pending && wait <= 0.0 {
            self.maybe_nudge();
        } else if self.out_of_office.nudge_pending {
            ctx.request_repaint_after_secs(wait as f32);
        }
        let next = [
            self.out_of_office.read_at.map(|at| at + READ_EVERY),
            self.out_of_office.events_at.map(|at| at + EVENTS_EVERY),
        ]
        .into_iter()
        .flatten()
        .fold(f64::INFINITY, f64::min);
        ctx.request_repaint_after_secs((next - time).max(0.0) as f32);
    }

    /// Asks Gmail for its responder.
    fn read_vacation(&self, for_dialog: bool) {
        let outcomes = self.out_of_office.outcomes.clone();
        let ctx = self.ctx.clone();
        self.account.loader().run(
            |store| async move { store.vacation().await },
            move |result| {
                if let Err(error) = &result {
                    log::info!("reading the vacation responder: {error}");
                }
                push(
                    &outcomes,
                    &ctx,
                    Outcome::Read {
                        settings: result.map_err(drop),
                        dialog: for_dialog,
                    },
                );
            },
        );
    }

    /// Asks Calendar for the primary calendar's events coming up.
    fn read_events(&self) {
        let outcomes = self.out_of_office.outcomes.clone();
        let ctx = self.ctx.clone();
        let from = self.now.timestamp();
        self.account.loader().run(
            move |store| async move {
                store
                    .upcoming_events("primary", from, from + OUT_OF_OFFICE_LOOKAHEAD)
                    .await
            },
            move |result| match result {
                Ok(events) => push(&outcomes, &ctx, Outcome::Events(events)),
                Err(error) => log::info!("looking for out-of-office events: {error}"),
            },
        );
    }

    fn out_of_office_outcome(&mut self, outcome: Outcome) {
        let now = self.now.clone();
        match outcome {
            Outcome::Read { settings, dialog } => {
                if let Ok(settings) = &settings {
                    let copy = OutOfOfficeResponder::synced(
                        settings,
                        self.account.settings().out_of_office_responder.as_ref(),
                        now.timestamp(),
                    );
                    if let Some(copy) = copy {
                        self.account.update_settings(|account| {
                            account.out_of_office_responder = Some(copy)
                        });
                    }
                }
                let Some(open) = self
                    .out_of_office
                    .dialog
                    .as_mut()
                    .filter(|open| dialog && open.open && open.loading)
                else {
                    return;
                };
                match settings {
                    Ok(_) => open.loaded(
                        self.account.settings().out_of_office_responder.as_ref(),
                        &now,
                    ),
                    Err(()) => self.out_of_office.alert = Some((Alert::SyncFailed, 0)),
                }
            }
            Outcome::Saved(result) => {
                let Some(open) = self.out_of_office.dialog.as_mut() else {
                    return;
                };
                open.submitting = false;
                match result {
                    Ok(kept) => {
                        open.open = false;
                        let copy = OutOfOfficeResponder::of(&kept, now.timestamp());
                        self.account.update_settings(|account| {
                            account.out_of_office_responder = Some(copy)
                        });
                        self.commands.toast(Toast::new(SAVED).action(
                            COMMAND,
                            None,
                            commands::Action::OutOfOffice(Action::Open),
                        ));
                    }
                    Err(()) => self.out_of_office.alert = Some((Alert::SaveFailed, 0)),
                }
            }
            Outcome::Events(events) => {
                let detection = &self.account.settings().out_of_office_event_detection;
                if let Some(found) =
                    detection.detected(&events, self.account.email(), now.timestamp())
                {
                    self.account.update_settings(|account| {
                        account.out_of_office_event_detection.events = found
                    });
                    self.out_of_office.nudge_pending = true;
                }
            }
        }
    }

    /// Shows the nudge if there are out-of-office days to offer a reply for and nothing else
    /// is in the way. A dialog or alert in the way holds it until it's gone.
    fn maybe_nudge(&mut self) {
        let blocked = self.out_of_office.dialog.is_some()
            || self.out_of_office.alert.is_some()
            || self.settings.dialog_open()
            || self.bulk_dialogs.is_open()
            || self.pickers.region().is_some();
        if blocked {
            return;
        }
        self.out_of_office.nudge_pending = false;
        let settings = self.account.settings();
        if settings
            .out_of_office_responder
            .as_ref()
            .is_some_and(OutOfOfficeResponder::is_enabled)
        {
            return;
        }
        let mut detection = settings.out_of_office_event_detection.clone();
        let Some(days) = detection.upcoming(&self.now) else {
            return;
        };
        if detection.nudge(&days, self.now.timestamp()) {
            self.account
                .update_settings(|account| account.out_of_office_event_detection = detection);
            self.commands.close_palette();
            self.out_of_office.alert = Some((
                Alert::Nudge {
                    days,
                    opt_out: false,
                    created: false,
                },
                0,
            ));
        }
    }

    /// The user just made `event` on `calendar`: an out-of-office one of theirs on the primary
    /// calendar, starting within a week, is offered a reply at once.
    pub(crate) fn out_of_office_created(&mut self, calendar: &str, event: &CalendarEvent) {
        let data = self.account.calendar().data();
        let primary = data.calendars.as_ref().is_some_and(|calendars| {
            calendars.iter().any(|listed| {
                listed.account == self.account.email()
                    && listed.entry.id.as_deref() == Some(calendar)
                    && listed.entry.primary == Some(true)
            })
        });
        let Some(days) =
            OutOfOfficeDays::created(event, self.account.email(), &self.now).filter(|_| primary)
        else {
            return;
        };
        let settings = self.account.settings();
        if settings
            .out_of_office_responder
            .as_ref()
            .is_some_and(OutOfOfficeResponder::is_enabled)
            || self.out_of_office.dialog.is_some()
            || self.out_of_office.alert.is_some()
        {
            return;
        }
        let mut detection = settings.out_of_office_event_detection.clone();
        if detection.nudge_created(&days, self.now.timestamp()) {
            self.account
                .update_settings(|account| account.out_of_office_event_detection = detection);
            self.out_of_office.alert = Some((
                Alert::Nudge {
                    days,
                    opt_out: false,
                    created: true,
                },
                0,
            ));
        }
    }

    /// Carries out `action`.
    pub(crate) fn out_of_office_run(&mut self, action: Action) {
        match action {
            Action::Open => self.open_out_of_office(None),
            Action::Format(command) => {
                if let Some(open) = self.out_of_office.dialog.as_mut() {
                    command.run(open.editor_mut());
                }
            }
            Action::ShiftDate(days) => {
                let today = self.now.clone();
                if let Some(open) = self.out_of_office.dialog.as_mut() {
                    open.shift_date(days, &today);
                }
            }
            Action::Dialog(event) => self.out_of_office_dialog(event),
            Action::Alert(event) => {
                let Some((alert, selected)) = self.out_of_office.alert.as_mut() else {
                    return;
                };
                let count = alert.actions().len();
                match event {
                    AlertEvent::Cycle(step) => {
                        *selected = (*selected as isize + step).rem_euclid(count as isize) as usize
                    }
                    AlertEvent::Confirm => {
                        let selected = *selected;
                        self.out_of_office_alert(Some(selected));
                    }
                    AlertEvent::Dismiss => self.out_of_office_alert(None),
                }
            }
        }
    }

    /// Opens the dialog: offline, an alert; else the dialog, which reads Gmail. `days` are the
    /// nudge's.
    fn open_out_of_office(&mut self, days: Option<OutOfOfficeDays>) {
        self.commands.close_palette();
        if !self.snapshot.connection.is_online() {
            self.out_of_office.alert = Some((Alert::Offline, 0));
            return;
        }
        self.out_of_office.dialog =
            Some(dialog::Dialog::new(self.account.email(), days, &self.now));
        self.read_vacation(true);
    }

    fn out_of_office_dialog(&mut self, event: DialogEvent) {
        let now = self.now.clone();
        let Some(open) = self.out_of_office.dialog.as_mut() else {
            return;
        };
        let event = match event {
            DialogEvent::Tab(step) => {
                open.tab(step, &now);
                None
            }
            DialogEvent::Enter => open.enter(&now),
            DialogEvent::Space => open.space(&now),
            DialogEvent::Down => {
                open.down(&now);
                None
            }
            other => Some(other),
        };
        match event {
            Some(DialogEvent::Close) => {
                if open.changed() {
                    self.out_of_office.alert = Some((Alert::Discard, 0));
                } else {
                    open.open = false;
                }
            }
            Some(DialogEvent::Save) if !open.save_disabled() => {
                open.submitting = true;
                let settings = open.vacation();
                let outcomes = self.out_of_office.outcomes.clone();
                let ctx = self.ctx.clone();
                self.account.loader().run(
                    |store| async move { store.update_vacation(&settings).await },
                    move |result| {
                        if let Err(error) = &result {
                            log::warn!("saving the vacation responder: {error}");
                        }
                        push(&outcomes, &ctx, Outcome::Saved(result.map_err(drop)));
                    },
                );
            }
            Some(DialogEvent::PastedFile) => {
                self.out_of_office.alert = Some((
                    Alert::PastedFile {
                        email: self.account.email().to_owned(),
                    },
                    0,
                ))
            }
            _ => {}
        }
    }

    /// The alert's action `index` ran, or (`None`) Escape dismissed it.
    fn out_of_office_alert(&mut self, index: Option<usize>) {
        let Some((alert, _)) = self.out_of_office.alert.take() else {
            return;
        };
        match alert {
            // `ok`, Escape and the close all close the dialog too.
            Alert::SyncFailed => {
                if let Some(open) = self.out_of_office.dialog.as_mut() {
                    open.open = false;
                }
            }
            Alert::Discard if index == Some(0) => {
                if let Some(open) = self.out_of_office.dialog.as_mut() {
                    open.open = false;
                }
            }
            Alert::Nudge { days, opt_out, .. } => {
                // A checked box opts out, whichever button ran; Escape leaves it.
                if index.is_some() && opt_out {
                    let at = self.now.timestamp().to_string();
                    self.account.update_settings(|account| {
                        account.out_of_office_event_detection.opted_out_at = Some(at)
                    });
                }
                if index == Some(0) {
                    self.open_out_of_office(Some(days));
                }
            }
            Alert::Offline | Alert::SaveFailed | Alert::Discard | Alert::PastedFile { .. } => {}
        }
    }

    /// Draws the dialog and the alert over `page`.
    pub(crate) fn show_out_of_office(&mut self, ui: &mut egui::Ui, page: Rect) {
        let ctx = ui.ctx().clone();
        let now = self.now.clone();
        if let Some(open) = self.out_of_office.dialog.as_mut() {
            let shown = open.show(&ctx, page, &now);
            // A date in the message shows its day in the right pane, behind the dialog.
            if let Some(day) = open
                .take_day()
                .and_then(|text| crate::thread::dates::resolve(&text, &now))
            {
                self.calendar_show_day(day);
            }
            match shown {
                None => self.out_of_office.dialog = None,
                Some(Some(event)) => self.out_of_office_dialog(event),
                Some(None) => {}
            }
        }
        let tz = now.time_zone().clone();
        let Some((alert, selected)) = self.out_of_office.alert.as_mut() else {
            return;
        };
        let selected = *selected;
        let shown = egui::Area::new(exo_ui::id(&ctx, "out-of-office-alert"))
            .order(egui::Order::Tooltip)
            .fixed_pos(page.min)
            .show(&ctx, |ui| {
                // The overlay: the page dimmed to half, nothing behind it clickable.
                ui.painter()
                    .rect_filled(page, 0.0, egui::Color32::from_black_alpha(128));
                let _ = ui.allocate_rect(page, egui::Sense::click());
                alert.show(ui, page, selected, &tz)
            });
        let (clicked, link) = shown.inner;
        if link && let Some(url) = alert.link() {
            self.open_url(&url);
        }
        if let Some(index) = clicked {
            self.out_of_office_alert(Some(index));
        }
    }

    /// The Out of Office Reply status bar's showing: Gmail's replies are going out now.
    pub(crate) fn out_of_office(&self) -> bool {
        self.account
            .settings()
            .out_of_office_responder
            .as_ref()
            .is_some_and(|copy| copy.is_out_of_office(self.now.timestamp()))
    }

    /// The Out of Office Reply dialog's form, once it's loaded, for tests.
    pub fn out_of_office_form(&self) -> Option<&Form> {
        self.out_of_office
            .dialog
            .as_ref()
            .filter(|open| open.open && !open.loading)
            .map(|open| &open.form)
    }

    /// The control with the keyboard in the Out of Office Reply dialog, for tests.
    pub fn out_of_office_focus(&self) -> Option<Control> {
        self.out_of_office
            .dialog
            .as_ref()
            .and_then(dialog::Dialog::focus)
    }

    /// The Out of Office Reply alert showing, its title and text, for tests.
    pub fn out_of_office_alert_text(&self) -> Option<(&'static str, String)> {
        self.out_of_office
            .alert
            .as_ref()
            .map(|(alert, _)| (alert.title(), alert.text(self.now.time_zone())))
    }
}

/// Hands `outcome` to the UI thread and wakes it.
fn push(outcomes: &Mutex<Vec<Outcome>>, ctx: &Context, outcome: Outcome) {
    outcomes
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(outcome);
    ctx.request_repaint();
}
