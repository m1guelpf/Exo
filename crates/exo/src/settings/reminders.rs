//! The Reminders dialog: Remind me's defaults. Everything applies as it's changed.
//!
//! `in {n} days` and `Weekdays Only` drive Remind me's `in {n} days` row and typed days,
//! `Automatic Reminders` puts the default reminder on drafts with an external recipient
//! (`compose::reminder`), and the time and `if no reply` / `regardless` are what the picker
//! uses. Opening it writes `Weekdays Only`'s default while it's unset.
//!
//! Metrics at a 1280×800 window. In Carbon the card keeps Snow's colours.

use egui::{Color32, Context, Id, Rect, Sense, Stroke, Ui, pos2, text::CCursorRange, vec2};
use exo_store::{ReminderSettings, TriggerCondition};
use exo_ui::{Dialog, DialogDropdown, DialogResponse, Family, Font, Menu, TextStyle};
use jiff::Zoned;

use crate::time_suggest::{self, Options};

/// The rows' text: 12px (13px only on a 1× screen), the headers bold in 20px lines, the
/// subheaders in 16px ones; the labels and inputs 20px, the inputs bold.
const HEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 20.0);
const SUBHEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
const LABEL: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
const INPUT: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 20.0);
/// `rgba(0,0,0,.9)` for the headers, labels and the dropdown's value, `.55` the subheaders, `.8`
/// the inputs; the caret `.3`, black while the dropdown has the keyboard or the pointer.
const STRONG: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 230);
const SUBDUED: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 140);
const INPUT_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 204);
const CARET: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 77);
/// The inputs' `border-bottom`: 1px `rgba(0,0,0,.1)`, 2px `#aeb1dd` focused, `#ef8e8e` invalid
/// (its text too); their selection `#d5d7ed`.
const UNDERLINE: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 26);
const FOCUSED: Color32 = Color32::from_rgb(0xae, 0xb1, 0xdd);
const INVALID: Color32 = Color32::from_rgb(0xef, 0x8e, 0x8e);
const SELECTION: Color32 = Color32::from_rgb(0xd5, 0xd7, 0xed);
/// `hr`: a 1px `rgba(0,0,0,.1)` line.
const RULE: Color32 = UNDERLINE;

/// Where things sit below the body's top, measured: the Default Reminder block's header and
/// subheader, the row's line, the rule, then each checkbox row's header, its subheader and its
/// checkbox's image, and the clickable rows.
const DEFAULT_HEADER_TOP: f32 = 23.0;
const DEFAULT_SUBHEADER_TOP: f32 = 49.0;
const ROW_TOP: f32 = 79.0;
const VALUE_TOP: f32 = 81.0;
const RULE_TOP: f32 = 123.0;
const AUTOMATIC_TOP: f32 = 147.0;
const WEEKDAYS_TOP: f32 = 223.0;
/// A header's subheader 26px below it (`margin-bottom: 6px`), and the checkbox's image 3px below
/// the header's top (the box's `margin-top: -5px` and `padding: 8px`).
const SUBHEADER_BELOW: f32 = 26.0;
const CHECKBOX_DOWN: f32 = 3.0;
/// The checkbox image's left, 14px in from the content's right (`margin-right: -9px`).
const CHECKBOX_LEFT: f32 = 472.0;
/// The clickable rows: their tops above their headers, and heights.
const ROW_ABOVE: f32 = 4.0;
const AUTOMATIC_HEIGHT: f32 = 62.0;
const WEEKDAYS_HEIGHT: f32 = 78.0;
/// The body, to the content's bottom.
const BODY_HEIGHT: f32 = 296.0;
/// The space after a row's label.
const LABEL_GAP: f32 = 16.0;
/// The days input is `size` characters wide: 7px each and 4px more (Chromium's average and
/// widest Exo Sans Bold digits); the time input 52px.
const DAYS_CHAR: f32 = 7.0;
const DAYS_EXTRA: f32 = 4.0;
const TIME_WIDTH: f32 = 52.0;
/// The dropdown's value 1px into its box, the caret 4px after it; its list 18px left of the value
/// and 26px under the button's top.
const VALUE_LEFT: f32 = 1.0;
const MENU_OFFSET: egui::Vec2 = vec2(-18.0, 26.0);
/// The dropdown's rows: their names and the triggers they pick.
const TRIGGERS: [(&str, TriggerCondition); 2] = [
    ("if no reply", TriggerCondition::IfNoReply),
    ("regardless", TriggerCondition::Regardless),
];

/// What has the keyboard: the inputs and the dropdown (`tabIndex: 0`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Days,
    Time,
    Trigger,
}

const TAB_ORDER: [Control; 3] = [Control::Days, Control::Time, Control::Trigger];

/// Something done in the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Tab (1) or ⇧Tab (-1).
    Tab(isize),
    /// Enter on what has the keyboard.
    Enter,
    /// ↓ (1) or ↑ (-1) in the open list.
    MenuMove(isize),
    /// Escape with the list open: it closes.
    MenuClose,
    /// A click on an input or the dropdown.
    Focus(Control),
    /// A click on the dropdown's button.
    ToggleMenu,
    /// The pointer on a row of the list.
    Highlight(usize),
    /// A row of the list.
    Trigger(TriggerCondition),
    /// The `Automatic Reminders` row.
    ToggleAutomatic,
    /// The `Weekdays Only` row.
    ToggleWeekdays,
    /// The days input now reads this many.
    Days(u32),
    /// The time input now reads this time.
    Time(i8, i8),
}

/// The dialog's inputs as typed, and what has the keyboard.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// The days input's text.
    days: String,
    /// The time input's text.
    time: String,
    /// The time typed doesn't read as one.
    time_invalid: bool,
    focus: Option<Control>,
    /// The list is open, with this row highlighted.
    menu: Option<usize>,
    /// Select the focused input's text on the next frame (mounting, Tab, Enter).
    select: bool,
}

/// The reminder time of `settings`, as the time input shows it.
fn formatted(settings: &ReminderSettings, now: &Zoned) -> String {
    let at = now
        .date()
        .at(settings.hour, settings.minute, 0, 0)
        .to_zoned(now.time_zone().clone())
        .unwrap_or_else(|_| now.clone());
    crate::format::dates::time(&at)
}

/// The time `text` names (`tomorrow at {text}`), 8:00 for nothing.
fn parse_time(text: &str, now: &Zoned) -> Option<(i8, i8)> {
    if text.trim().is_empty() {
        return Some((8, 0));
    }
    let suggestion =
        time_suggest::suggest(&format!("tomorrow at {text}"), &Options::default(), now)
            .into_iter()
            .next()?;
    let at = suggestion.start.to_zoned(now.time_zone().clone());
    Some((at.hour(), at.minute()))
}

impl State {
    /// The dialog as it opens: the days input has the keyboard, its text
    /// selected.
    pub fn open(settings: &ReminderSettings, now: &Zoned) -> Self {
        Self {
            days: settings.days.to_string(),
            time: formatted(settings, now),
            time_invalid: false,
            focus: Some(Control::Days),
            menu: None,
            select: true,
        }
    }

    /// The list is open: Escape and the arrows are its.
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// Applies `event` to `settings`; returns the settings to save, if it changed them.
    pub fn handle(
        &mut self,
        event: Event,
        settings: &ReminderSettings,
        now: &Zoned,
    ) -> Option<ReminderSettings> {
        let mut saved = settings.clone();
        match event {
            Event::Tab(step) => {
                let at = self
                    .focus
                    .and_then(|focus| TAB_ORDER.iter().position(|control| *control == focus))
                    .map_or(0, |at| {
                        (at as isize + step).rem_euclid(TAB_ORDER.len() as isize) as usize
                    });
                self.menu = None;
                return self.move_focus(Some(TAB_ORDER[at]), settings, now);
            }
            // Enter opens the list, and in it takes the row.
            Event::Enter if self.menu.is_some() => {
                return self.handle(
                    Event::Trigger(TRIGGERS[self.menu.unwrap_or_default()].1),
                    settings,
                    now,
                );
            }
            Event::Enter => match self.focus {
                Some(Control::Trigger) => self.menu = Some(trigger_index(settings)),
                // In an input: the time as saved again, the input's text selected.
                Some(_) => {
                    self.time = formatted(settings, now);
                    self.time_invalid = false;
                    self.select = true;
                }
                None => {}
            },
            Event::MenuMove(step) => {
                self.menu = self
                    .menu
                    .map(|at| (at as isize + step).rem_euclid(TRIGGERS.len() as isize) as usize)
            }
            Event::MenuClose => self.menu = None,
            Event::Focus(control) => return self.move_focus(Some(control), settings, now),
            Event::ToggleMenu => {
                self.menu = if self.menu.is_some() {
                    None
                } else {
                    Some(trigger_index(settings))
                };
                return self.move_focus(Some(Control::Trigger), settings, now);
            }
            Event::Highlight(row) => self.menu = Some(row),
            Event::Trigger(trigger) => {
                self.menu = None;
                saved.trigger_condition = trigger;
            }
            // Automatic Reminders: `EXTERNAL` on, `NONE` off.
            Event::ToggleAutomatic => {
                saved.default_reminders_trigger = if saved.default_reminders_trigger == "NONE" {
                    "EXTERNAL".into()
                } else {
                    "NONE".into()
                }
            }
            Event::ToggleWeekdays => saved.business_days_only = Some(!saved.weekdays_only()),
            Event::Days(days) => saved.days = days,
            Event::Time(hour, minute) => (saved.hour, saved.minute) = (hour, minute),
        }
        (saved != *settings).then_some(saved)
    }

    /// Moves the keyboard to `to`, blurring what had it: the time input shows the saved time; an
    /// empty or zero days input becomes 1.
    fn move_focus(
        &mut self,
        to: Option<Control>,
        settings: &ReminderSettings,
        now: &Zoned,
    ) -> Option<ReminderSettings> {
        let from = self.focus;
        self.focus = to;
        if from == to {
            return None;
        }
        self.select = to.is_some();
        self.time = formatted(settings, now);
        self.time_invalid = false;
        if from == Some(Control::Days) && (self.days.is_empty() || self.days == "0") {
            self.days = "1".into();
            return (settings.days != 1).then(|| ReminderSettings {
                days: 1,
                ..settings.clone()
            });
        }
        None
    }
}

/// The dropdown's row for the saved trigger.
fn trigger_index(settings: &ReminderSettings) -> usize {
    TRIGGERS
        .iter()
        .position(|(_, trigger)| *trigger == settings.trigger_condition)
        .unwrap_or(0)
}

/// A checkbox row: where its header is, how tall its
/// clickable block is, its text, whether it's checked and what a click does.
struct CheckboxRow {
    top: f32,
    height: f32,
    header: &'static str,
    lines: &'static [&'static str],
    checked: bool,
    toggle: Event,
}

/// Draws the dialog over `area` while `open` (and as it fades), editing `state`'s inputs.
pub fn dialog(
    ctx: &Context,
    area: Rect,
    open: bool,
    state: &mut State,
    settings: &ReminderSettings,
    now: &Zoned,
) -> DialogResponse<Option<Event>> {
    Dialog::new("reminders", "Reminders", BODY_HEIGHT).show(ctx, area, open, |ui, body| {
        draw(ui, body, state, settings, now)
    })
}

fn draw(
    ui: &mut Ui,
    body: Rect,
    state: &mut State,
    settings: &ReminderSettings,
    now: &Zoned,
) -> Option<Event> {
    let ctx = ui.ctx().clone();
    let painter = ui.painter().clone();
    let (left, top) = (body.left(), body.top());
    let mut event = None;

    HEADER.paint_line(
        &painter,
        pos2(left, top + DEFAULT_HEADER_TOP),
        "Default Reminder",
        STRONG,
        None,
    );
    SUBHEADER.paint_line(
        &painter,
        pos2(left, top + DEFAULT_SUBHEADER_TOP),
        "For Auto Reminders and when no date or time is specified, remind me:",
        SUBDUED,
        None,
    );

    // `in [days] days`, `at [time]`, and the dropdown, 16px apart.
    let space = LABEL.width(&ctx, "a a") - LABEL.width(&ctx, "aa");
    let row = top + ROW_TOP;
    LABEL.paint_line(&painter, pos2(left, row), "in", STRONG, None);
    let mut x = left + LABEL.width(&ctx, "in ");
    let days_width = DAYS_CHAR * state.days.chars().count().max(1) as f32 + DAYS_EXTRA;
    let days_box = Rect::from_min_size(pos2(x, row), vec2(days_width, LABEL.line_height));
    let typed = state.days.clone();
    if input(
        ui,
        exo_ui::id(ui.ctx(), "reminders-days"),
        &mut state.days,
        days_box,
        Control::Days,
        state.focus,
        &mut state.select,
        false,
    ) {
        event = Some(Event::Focus(Control::Days));
    }
    if state.days != typed {
        // Two digits at most, nothing else; a number is saved.
        if state.days.chars().count() > 2 || !state.days.chars().all(|c| c.is_ascii_digit()) {
            state.days = typed;
        } else if let Ok(days) = state.days.parse() {
            event = Some(Event::Days(days));
        }
    }
    x += days_width + space;
    x += LABEL.paint_line(&painter, pos2(x, row), "days", STRONG, None) + LABEL_GAP;
    LABEL.paint_line(&painter, pos2(x, row), "at", STRONG, None);
    x += LABEL.width(&ctx, "at ");
    let time_box = Rect::from_min_size(pos2(x, row), vec2(TIME_WIDTH, LABEL.line_height));
    let typed = state.time.clone();
    if input(
        ui,
        exo_ui::id(ui.ctx(), "reminders-time"),
        &mut state.time,
        time_box,
        Control::Time,
        state.focus,
        &mut state.select,
        state.time_invalid,
    ) {
        event = Some(Event::Focus(Control::Time));
    }
    if state.time != typed {
        match parse_time(&state.time, now) {
            Some((hour, minute)) => {
                state.time_invalid = false;
                event = Some(Event::Time(hour, minute));
            }
            None => state.time_invalid = true,
        }
    }
    x += TIME_WIDTH + LABEL_GAP;

    let value = pos2(x + VALUE_LEFT, top + VALUE_TOP);
    // Black while it has the keyboard or (as of the last frame) the pointer.
    let lit = state.focus == Some(Control::Trigger)
        || ctx
            .read_response(exo_ui::id(&ctx, "reminders-trigger"))
            .is_some_and(|response| response.hovered());
    let style = DialogDropdown {
        value: STRONG,
        caret: if lit { Color32::BLACK } else { CARET },
        gap: 4.0,
    };
    let button =
        exo_ui::paint_dialog_dropdown_with(ui, value, TRIGGERS[trigger_index(settings)].0, style);
    let response = ui
        .interact(
            button,
            exo_ui::id(ui.ctx(), "reminders-trigger"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        event = Some(Event::ToggleMenu);
    }
    painter.hline(body.x_range(), top + RULE_TOP + 0.5, Stroke::new(1.0, RULE));

    // The two checkbox rows, each clickable whole.
    let rows = [
        CheckboxRow {
            top: AUTOMATIC_TOP,
            height: AUTOMATIC_HEIGHT,
            header: "Automatic Reminders",
            lines: &["Add your reminder to all drafts with external recipients."],
            checked: settings.default_reminders_trigger != "NONE",
            toggle: Event::ToggleAutomatic,
        },
        CheckboxRow {
            top: WEEKDAYS_TOP,
            height: WEEKDAYS_HEIGHT,
            header: "Weekdays Only",
            lines: &[
                "When reminders are set as a number of days, count weekdays only.",
                "With this option enabled, a reminder set for 2 days on Friday will return on Tuesday.",
            ],
            checked: settings.weekdays_only(),
            toggle: Event::ToggleWeekdays,
        },
    ];
    for row in rows {
        let row_top = top + row.top;
        HEADER.paint_line(&painter, pos2(left, row_top), row.header, STRONG, None);
        for (index, line) in row.lines.iter().enumerate() {
            SUBHEADER.paint_line(
                &painter,
                pos2(
                    left,
                    row_top + SUBHEADER_BELOW + index as f32 * SUBHEADER.line_height,
                ),
                line,
                SUBDUED,
                None,
            );
        }
        exo_ui::paint_legacy_checkbox(
            &painter,
            pos2(left + CHECKBOX_LEFT, row_top + CHECKBOX_DOWN),
            row.checked,
        );
        let hit = Rect::from_min_size(
            pos2(left, row_top - ROW_ABOVE),
            vec2(body.width(), row.height),
        );
        if ui
            .interact(
                hit,
                exo_ui::id(ui.ctx(), ("reminders-row", row.header)),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            event = Some(row.toggle);
        }
    }

    if let Some(highlighted) = state.menu {
        let names = TRIGGERS.map(|(name, _)| name);
        let list = Menu {
            items: &names,
            highlighted,
        }
        .show_in_dialog(
            ui,
            exo_ui::id(ui.ctx(), "reminders-trigger-list"),
            button.left_top() + MENU_OFFSET,
        );
        if let Some(clicked) = list.clicked {
            event = Some(Event::Trigger(TRIGGERS[clicked].1));
        } else if let Some(hovered) = list.hovered.filter(|hovered| *hovered != highlighted) {
            event = Some(Event::Highlight(hovered));
        } else if ui.input(|input| input.pointer.any_released())
            && !response.hovered()
            && !ui.rect_contains_pointer(list.rect)
        {
            event = Some(Event::MenuClose);
        }
    }
    event
}

/// One of the row's inputs in `field` (its box: the 20px line, the underline below it): `text`
/// centered, bold, editable while `control` has the keyboard (`focus`), its text selected when
/// `select` asks. Returns whether it was clicked.
#[expect(
    clippy::too_many_arguments,
    reason = "one input of the dialog, drawn from its state"
)]
fn input(
    ui: &mut Ui,
    id: Id,
    text: &mut String,
    field: Rect,
    control: Control,
    focus: Option<Control>,
    select: &mut bool,
    invalid: bool,
) -> bool {
    let ctx = ui.ctx().clone();
    let focused = focus == Some(control);
    let (ink, line, thickness) = match (invalid, focused) {
        (true, true) => (INVALID, INVALID, 2.0),
        (true, false) => (INVALID, INVALID, 1.0),
        (false, true) => (INPUT_INK, FOCUSED, 2.0),
        (false, false) => (INPUT_INK, UNDERLINE, 1.0),
    };
    // Chromium paints the input's box from the whole pixel nearest its layout position (its
    // `getBoundingClientRect` left, rounded); its text is centered
    // in the unrounded box.
    ui.painter().rect_filled(
        Rect::from_min_size(
            pos2(field.left().round(), field.bottom()),
            vec2(field.width(), thickness),
        ),
        0.0,
        line,
    );
    let clicked = ui
        .interact(field, id.with("box"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::Text)
        .clicked();
    // `text-align: center`: the field spans the text, from where centering puts it.
    let width = INPUT.width(&ctx, text);
    let line = Rect::from_min_size(
        pos2(field.center().x - width / 2.0, field.top()),
        vec2(width.max(1.0), field.height()),
    );
    let response = exo_ui::text_field(ui, id, text, line, &INPUT, ink, SELECTION);
    let has = ui.memory(|memory| memory.has_focus(id));
    if focused && (!has || *select) {
        let end = egui::text::CCursor::new(text.chars().count());
        exo_ui::focus_field(
            &ctx,
            id,
            CCursorRange::two(egui::text::CCursor::new(0), end),
        );
        *select = false;
    } else if !focused && has {
        ui.memory_mut(|memory| memory.surrender_focus(id));
    }
    clicked || response.clicked() && !focused
}

#[cfg(test)]
mod tests {
    use super::*;

    /// gmail-mock's now: Tuesday, November 17th 2026, 9:00 AM in New York.
    fn now() -> Zoned {
        "2026-11-17T09:00[America/New_York]".parse().expect("now")
    }

    #[test]
    fn it_opens_on_the_days_and_tab_goes_round() {
        let settings = ReminderSettings::default();
        let mut state = State::open(&settings, &now());
        assert_eq!(
            (state.days.as_str(), state.time.as_str(), state.focus),
            ("2", "8:00 AM", Some(Control::Days))
        );
        state.handle(Event::Tab(1), &settings, &now());
        state.handle(Event::Tab(1), &settings, &now());
        assert_eq!(state.focus, Some(Control::Trigger));
        state.handle(Event::Enter, &settings, &now());
        assert_eq!(state.menu, Some(0), "Enter opens the list on `if no reply`");
        state.handle(Event::MenuMove(1), &settings, &now());
        assert_eq!(
            state
                .handle(
                    Event::Trigger(TriggerCondition::Regardless),
                    &settings,
                    &now()
                )
                .map(|saved| saved.trigger_condition),
            Some(TriggerCondition::Regardless)
        );
        state.handle(Event::Tab(1), &settings, &now());
        assert_eq!(state.focus, Some(Control::Days), "Tab wraps");
    }

    #[test]
    fn an_empty_days_input_becomes_one_and_the_time_shows_what_was_saved() {
        let settings = ReminderSettings::default();
        let mut state = State::open(&settings, &now());
        state.days.clear();
        assert_eq!(
            state
                .handle(Event::Tab(1), &settings, &now())
                .map(|saved| saved.days),
            Some(1),
            "blurred empty, it's 1 day"
        );
        assert_eq!(state.days, "1");
        state.time = "9:3".into();
        let saved = state
            .handle(Event::Time(9, 30), &settings, &now())
            .expect("a time");
        assert_eq!((saved.hour, saved.minute), (9, 30));
        state.handle(Event::Tab(1), &saved, &now());
        assert_eq!(
            state.time, "9:30 AM",
            "formatted once it loses the keyboard"
        );
    }

    #[test]
    fn parses_typed_times_and_reads_nothing_as_8am() {
        assert_eq!(parse_time("", &now()), Some((8, 0)));
        assert_eq!(parse_time("9:30 pm", &now()), Some((21, 30)));
        assert_eq!(parse_time("7am", &now()), Some((7, 0)));
        assert_eq!(parse_time("whenever", &now()), None);
    }

    #[test]
    fn the_rows_toggle_their_settings() {
        let settings = ReminderSettings::default();
        let mut state = State::open(&settings, &now());
        let automatic = state
            .handle(Event::ToggleAutomatic, &settings, &now())
            .expect("on");
        assert_eq!(automatic.default_reminders_trigger, "EXTERNAL");
        assert_eq!(
            state
                .handle(Event::ToggleAutomatic, &automatic, &now())
                .map(|saved| saved.default_reminders_trigger),
            Some("NONE".to_owned())
        );
        let mut opened = settings.clone();
        assert!(opened.fill_defaults(), "opening writes Weekdays Only, on");
        assert!(!opened.fill_defaults(), "once");
        let mut written = ReminderSettings {
            default_reminders_trigger: "EXTERNAL".into(),
            ..settings.clone()
        };
        assert!(
            !written.fill_defaults(),
            "a reminders object written without it stays without it"
        );
        assert_eq!(
            state
                .handle(Event::ToggleWeekdays, &opened, &now())
                .map(|saved| saved.business_days_only),
            Some(Some(false))
        );
        assert_eq!(
            state
                .handle(Event::ToggleWeekdays, &settings, &now())
                .map(|saved| saved.business_days_only),
            Some(Some(true)),
            "unset reads as off"
        );
    }
}
