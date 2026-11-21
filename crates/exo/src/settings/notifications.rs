//! The Notifications dialog: email notifications on or off, which calendars notify, and how long
//! before an event. Everything applies at once.

use egui::{Color32, Context, CornerRadius, Id, Rect, Sense, Ui, pos2, vec2};
use exo_store::{DeviceSettings, Settings};
use exo_ui::{CalendarColor, Dialog, DialogResponse, Family, Font, Menu, TextStyle};

use crate::calendar::ListedCalendar;

/// The `h1`s: 12px/20px semibold; the `h2`s, the calendars' names and `Notify me:` regular. All
/// sit 3px down their line (`top: 3px`).
const HEADING: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 12.0, 20.0);
const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
const NUDGE: f32 = 3.0;
/// `h1` `rgba(0,0,0,.85)`, `h2` `.55`, a calendar's name `.8`, `Notify me:` `.7`, the link
/// `#54acdc` (Classic, Carbon's card the same).
const HEADING_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 217);
const SUBHEADING_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 140);
const NAME_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 204);
const LABEL_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 179);
const LINK_INK: Color32 = Color32::from_rgb(0x54, 0xac, 0xdc);
/// Where things sit below the body's top: the header's `margin-bottom: 24px`, the two-line email
/// section, the events section's 24px `margin-top`, and the list 8px under its heading.
const EMAIL_TOP: f32 = 24.0;
const EVENTS_TOP: f32 = 88.0;
const LIST_TOP: f32 = 116.0;
/// A calendar's row: `padding: 4px 0 8px` around the name's line; its swatch and
/// checkbox `margin-top: 3px`.
const ROW: f32 = 32.0;
/// With more than one calendar account, each account's calendars come under its address
/// (12px/20px bold, black at `.9`, `top: 3px`): 24px for the first, 40px for the others, whose
/// address sits 16px further down.
const ACCOUNT_HEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 20.0);
const ACCOUNT_HEADER_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 230);
const FIRST_HEADER: f32 = 24.0;
const HEADER: f32 = 40.0;
const HEADER_TEXT_BELOW: f32 = 16.0;
const ROW_INSET: f32 = 7.0;
/// The email checkbox: `padding: 6px; margin-right: -6px`, so its 15px box ends at the content's
/// edge, 6px down.
const EMAIL_CHECKBOX_TOP: f32 = 6.0;
const CHECKBOX: f32 = 15.0;
/// The swatch: 14px with 2px corners, `margin-right: 4px` and `gap: 18px` before the checkbox.
const SWATCH: f32 = 14.0;
const SWATCH_RIGHT: f32 = CHECKBOX + 18.0 + 4.0;
/// The delimiter's margin either side.
const DELIMITER_MARGIN: f32 = 6.0;
/// The `Notify me:` row's `margin: 16px 0 20px`; the label's `margin-right: 4px`, and the
/// dropdown's value 1px further, 2px down.
const NOTIFY_GAP: f32 = 16.0;
const NOTIFY_BELOW: f32 = 20.0;
const VALUE_LEFT: f32 = 5.0;
const VALUE_TOP: f32 = 5.0;
/// The open list: 18px left of the value and 26px under the button.
const MENU_OFFSET: egui::Vec2 = vec2(-18.0, 26.0);

/// `Notify me:`'s choices, in seconds.
pub const LEAD_TIMES: [(i64, &str); 3] = [
    (60, "1 min before events"),
    (120, "2 mins before events"),
    (300, "5 mins before events"),
];

/// Something done in the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The email checkbox.
    ToggleEmail,
    /// A calendar's checkbox.
    ToggleCalendar(String),
    /// `Enable a calendar`: the dialog closes and the week view opens.
    EnableCalendar,
    /// Open or close `Notify me:`'s list.
    ToggleMenu,
    /// The pointer on a row of the list.
    Highlight(usize),
    /// A lead time, in seconds.
    Lead(i64),
    /// A calendar's swatch, and its colour: its colour picker opens or closes.
    Swatch(String, CalendarColor),
    /// A colour picked for a calendar. The picker stays open.
    Recolor(String, CalendarColor),
    /// A key in the colour picker.
    PickerKey(exo_ui::PickerKey),
    /// Escape: the colour picker closes, its colour put back, else the list.
    Escape,
}

/// The open list's highlighted row, between frames.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    menu: Option<usize>,
    /// The colour picker, while it's open.
    picker: Option<Picker>,
}

/// The open colour picker: whose it is, and the colours it opened on and is on (indices into
/// [`CalendarColor::picker`]).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Picker {
    id: String,
    start: usize,
    current: usize,
}

impl State {
    /// Closes the list.
    pub fn reset(&mut self) {
        self.menu = None;
        self.picker = None;
    }

    /// Whether the list or the colour picker is open: Escape closes it, not the dialog.
    pub fn menu_open(&self) -> bool {
        self.menu.is_some() || self.picker.is_some()
    }

    /// Whether the colour picker is open: it has the keyboard.
    pub fn picker_open(&self) -> bool {
        self.picker.is_some()
    }

    /// Applies `event` to the list; returns what's left for the account.
    pub fn handle(&mut self, event: Event, lead: i64) -> Option<Event> {
        match event {
            Event::ToggleMenu => {
                self.menu = if self.menu.is_some() {
                    None
                } else {
                    Some(lead_index(lead))
                }
            }
            Event::Highlight(row) => self.menu = Some(row),
            Event::Swatch(id, color) => {
                let index = picker_index(color);
                self.picker = if self.picker.as_ref().is_some_and(|picker| picker.id == id) {
                    None
                } else {
                    Some(Picker {
                        id,
                        start: index,
                        current: index,
                    })
                };
            }
            Event::Escape if self.picker.is_some() => {
                return self.handle(Event::PickerKey(exo_ui::PickerKey::Escape), lead);
            }
            Event::Escape => self.menu = None,
            Event::Recolor(_, color) => {
                if let Some(picker) = &mut self.picker {
                    picker.current = picker_index(color);
                }
                return Some(event);
            }
            Event::PickerKey(key) => {
                let picker = self.picker.as_mut()?;
                let colors: Vec<CalendarColor> = CalendarColor::picker().collect();
                let (pick, close) = match exo_ui::picker_key(
                    key,
                    picker.current,
                    picker.start,
                    colors.len(),
                    crate::calendar::PICKER_ROW,
                ) {
                    exo_ui::PickerMove::Preview(pick) => (pick, false),
                    exo_ui::PickerMove::Close(pick) => (pick, true),
                };
                picker.current = pick;
                let recolor = Event::Recolor(picker.id.clone(), colors[pick]);
                if close {
                    self.picker = None;
                }
                return Some(recolor);
            }
            Event::Lead(_) => {
                self.menu = None;
                return Some(event);
            }
            other => return Some(other),
        }
        None
    }
}

/// The colour picker under `swatch` (its top centre on the swatch's bottom centre, 10px up less
/// the tooltip's 8px padding): five rows of the 25 colours, `calendar`'s chosen.
fn picker(ctx: &Context, swatch: Rect, calendar: &ListedCalendar) -> Option<Event> {
    let colors: Vec<CalendarColor> = CalendarColor::picker().collect();
    let bases: Vec<egui::Color32> = colors.iter().map(|color| color.base()).collect();
    let picker = exo_ui::ColorPicker {
        colors: &bases,
        per_row: crate::calendar::PICKER_ROW,
        chosen: colors.iter().position(|color| *color == calendar.color),
        padding_bottom: 6.0,
    };
    let size = picker.size();
    match picker.show(
        ctx,
        exo_ui::id(ctx, "notifications-color-picker"),
        pos2(swatch.center().x - size.x / 2.0, swatch.bottom() - 2.0),
        swatch,
    ) {
        exo_ui::Picked::Color(index) => Some(Event::Recolor(calendar.id.clone(), colors[index])),
        exo_ui::Picked::Outside => Some(Event::Swatch(calendar.id.clone(), calendar.color)),
        exo_ui::Picked::Nothing => None,
    }
}

/// `color`'s place in [`CalendarColor::picker`] (the first's when it isn't there).
fn picker_index(color: CalendarColor) -> usize {
    CalendarColor::picker()
        .position(|candidate| candidate == color)
        .unwrap_or(0)
}

/// The row of [`LEAD_TIMES`] `lead` is, or the default's (2 minutes) when it's none of them.
fn lead_index(lead: i64) -> usize {
    LEAD_TIMES
        .iter()
        .position(|(seconds, _)| *seconds == lead)
        .unwrap_or(1)
}

/// What the dialog shows.
pub struct Shown<'a> {
    pub settings: &'a Settings,
    pub device: &'a DeviceSettings,
    /// The calendar accounts' calendars, as the week view lists them.
    pub calendars: &'a [ListedCalendar],
}

/// A line of the calendar list: an account's address, or a calendar.
enum Line<'a> {
    Account(&'a str),
    Calendar(&'a ListedCalendar),
}

/// The shown calendars by account, headed by their account's address when there's more than one
/// account (counting accounts whose calendars all hide), with each line's height.
fn lines<'a>(calendars: &'a [ListedCalendar]) -> Vec<(Line<'a>, f32)> {
    let headed = calendars
        .iter()
        .any(|calendar| calendar.account != calendars[0].account);
    let mut lines = Vec::new();
    let mut account: Option<&str> = None;
    for calendar in calendars.iter().filter(|calendar| calendar.shown) {
        if headed && account != Some(calendar.account.as_str()) {
            lines.push((
                Line::Account(&calendar.account),
                if account.is_none() {
                    FIRST_HEADER
                } else {
                    HEADER
                },
            ));
        }
        account = Some(&calendar.account);
        lines.push((Line::Calendar(calendar), ROW));
    }
    lines
}

/// Draws the dialog over `area` while `open` (and as it fades); returns what was clicked.
pub fn dialog(
    ctx: &Context,
    area: Rect,
    open: bool,
    state: &State,
    shown: &Shown<'_>,
) -> DialogResponse<Option<Event>> {
    let lines = lines(shown.calendars);
    let list: f32 = lines.iter().map(|(_, height)| height).sum();
    let body_height = if lines.is_empty() {
        EVENTS_TOP + 2.0 * TEXT.line_height
    } else {
        LIST_TOP + list + NOTIFY_GAP + TEXT.line_height + NOTIFY_BELOW
    };
    let mut swatch = None;
    let mut response = Dialog::new("notifications", "Notifications", body_height).show(
        ctx,
        area,
        open,
        |ui, body| draw(ui, body, state, shown, &lines, &mut swatch),
    );
    // The picker is a tooltip of the page's, outside the dialog's Snow-drawn card.
    if let (Some((swatch, calendar)), Some(inner)) = (swatch, response.inner.as_mut())
        && let Some(picked) = picker(ctx, swatch, calendar)
    {
        *inner = Some(picked);
    }
    response
}

fn draw<'a>(
    ui: &mut Ui,
    body: Rect,
    state: &State,
    shown: &Shown<'_>,
    lines: &[(Line<'a>, f32)],
    picker_swatch: &mut Option<(Rect, &'a ListedCalendar)>,
) -> Option<Event> {
    let palette = exo_ui::palette(ui.ctx());
    let painter = ui.painter().clone();
    let (left, right, top) = (body.left(), body.right(), body.top());
    let mut event = None;
    let checkbox = |ui: &mut Ui, at: egui::Pos2, checked: bool, id: Id| {
        exo_ui::paint_checkbox(ui.painter(), at, checked, palette);
        ui.interact(
            Rect::from_min_size(at, vec2(CHECKBOX, CHECKBOX)),
            id,
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    };

    // Email notifications, and when they come.
    let email = top + EMAIL_TOP;
    HEADING.paint_line(
        &painter,
        pos2(left, email + NUDGE),
        "Email notifications",
        HEADING_INK,
        None,
    );
    let line = pos2(left, email + TEXT.line_height + NUDGE);
    let mut x = line.x
        + TEXT.paint_line(&painter, line, "If Important", SUBHEADING_INK, None)
        + DELIMITER_MARGIN;
    x += TEXT.paint_line(&painter, pos2(x, line.y), "•", SUBHEADING_INK, None) + DELIMITER_MARGIN;
    TEXT.paint_line(
        &painter,
        pos2(x, line.y),
        "Other is on, notify for Important only",
        SUBHEADING_INK,
        None,
    );
    if checkbox(
        ui,
        pos2(right - CHECKBOX, email + EMAIL_CHECKBOX_TOP),
        shown.settings.email_notifications(shown.device),
        exo_ui::id(ui.ctx(), "notifications-email"),
    ) {
        event = Some(Event::ToggleEmail);
    }

    // Calendar notifications: each shown calendar, or a way to show one.
    let events = top + EVENTS_TOP;
    HEADING.paint_line(
        &painter,
        pos2(left, events + NUDGE),
        "Calendar notifications",
        HEADING_INK,
        None,
    );
    if lines.is_empty() {
        let line = pos2(left, events + TEXT.line_height + NUDGE);
        let link = "Enable a calendar";
        let width = TEXT.paint_line(&painter, line, link, LINK_INK, None);
        TEXT.paint_line(
            &painter,
            pos2(line.x + width, line.y),
            " to receive notifications",
            SUBHEADING_INK,
            None,
        );
        let hit = Rect::from_min_size(line, vec2(width, TEXT.line_height));
        if ui
            .interact(
                hit,
                exo_ui::id(ui.ctx(), "notifications-enable-calendar"),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            event = Some(Event::EnableCalendar);
        }
        return event;
    }
    let mut row = top + LIST_TOP;
    for (line, height) in lines {
        let calendar = match line {
            Line::Account(email) => {
                let below = if *height == FIRST_HEADER {
                    0.0
                } else {
                    HEADER_TEXT_BELOW
                };
                ACCOUNT_HEADER.paint_line(
                    &painter,
                    pos2(left, row + below + NUDGE),
                    email,
                    ACCOUNT_HEADER_INK,
                    Some(right - left),
                );
                row += height;
                continue;
            }
            Line::Calendar(calendar) => *calendar,
        };
        let swatch_left = right - SWATCH_RIGHT - SWATCH;
        TEXT.paint_line(
            &painter,
            pos2(left, row + ROW_INSET),
            &calendar.name,
            NAME_INK,
            Some(swatch_left - left),
        );
        let swatch = Rect::from_min_size(pos2(swatch_left, row + ROW_INSET), vec2(SWATCH, SWATCH));
        painter.rect_filled(
            exo_ui::pixel_snap(swatch),
            CornerRadius::same(2),
            calendar.color.swatch(exo_ui::Theme::Snow),
        );
        if ui
            .interact(
                swatch,
                exo_ui::id(ui.ctx(), ("notifications-swatch", &calendar.id)),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            event = Some(Event::Swatch(calendar.id.clone(), calendar.color));
        }
        if state
            .picker
            .as_ref()
            .is_some_and(|picker| picker.id == calendar.id)
        {
            *picker_swatch = Some((swatch, calendar));
        }
        let notifies = shown
            .settings
            .calendar_accounts
            .get(&calendar.account)
            .and_then(|account| account.calendars.get(&calendar.id))
            .is_some_and(exo_store::CalendarSetting::notifies);
        if checkbox(
            ui,
            pos2(right - CHECKBOX, row + ROW_INSET),
            notifies,
            exo_ui::id(ui.ctx(), ("notifications-calendar", &calendar.id)),
        ) {
            event = Some(Event::ToggleCalendar(calendar.id.clone()));
        }
        row += height;
    }

    // `Notify me:` and its dropdown.
    let notify = row + NOTIFY_GAP;
    let label = TEXT.paint_line(
        &painter,
        pos2(left, notify + NUDGE),
        "Notify me:",
        LABEL_INK,
        None,
    );
    let lead = shown.settings.notifications.calendar_events.lead_time();
    let button = exo_ui::paint_dialog_dropdown(
        ui,
        pos2(left + label + VALUE_LEFT, notify + VALUE_TOP),
        LEAD_TIMES[lead_index(lead)].1,
    );
    let response = ui
        .interact(
            button,
            exo_ui::id(ui.ctx(), "notifications-lead"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        event = Some(Event::ToggleMenu);
    }
    if let Some(highlighted) = state.menu {
        let names = LEAD_TIMES.map(|(_, name)| name);
        let list = Menu {
            items: &names,
            highlighted,
        }
        .show_tall_in_dialog(
            ui,
            exo_ui::id(ui.ctx(), "notifications-lead-list"),
            button.left_top() + MENU_OFFSET,
        );
        if let Some(clicked) = list.clicked {
            event = Some(Event::Lead(LEAD_TIMES[clicked].0));
        } else if let Some(hovered) = list.hovered.filter(|hovered| *hovered != highlighted) {
            event = Some(Event::Highlight(hovered));
        } else if ui.input(|input| input.pointer.any_released())
            && !response.hovered()
            && !ui.rect_contains_pointer(list.rect)
        {
            event = Some(Event::ToggleMenu);
        }
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_opens_on_the_lead_time_and_a_choice_closes_it() {
        let mut state = State::default();
        assert_eq!(state.handle(Event::ToggleMenu, 300), None);
        assert_eq!(state.menu, Some(2));
        state.handle(Event::Highlight(0), 300);
        assert_eq!(state.handle(Event::Lead(60), 300), Some(Event::Lead(60)));
        assert!(!state.menu_open());
        assert_eq!(
            state.handle(Event::ToggleMenu, 45),
            None,
            "a lead time it doesn't offer shows the default"
        );
        assert_eq!(state.menu, Some(1));
        assert_eq!(
            state.handle(Event::ToggleEmail, 45),
            Some(Event::ToggleEmail)
        );
    }

    #[test]
    fn the_colour_picker_previews_its_keys_stays_open_on_a_click_and_escape_reverts() {
        let color = |name| CalendarColor::from_name(name).expect("a picker colour");
        let mut state = State::default();
        state.handle(Event::Swatch("work".into(), color("blue")), 120);
        assert!(state.picker_open());
        // A click keeps it open; → previews the next colour.
        assert_eq!(
            state.handle(Event::Recolor("work".into(), color("red")), 120),
            Some(Event::Recolor("work".into(), color("red")))
        );
        assert!(state.picker_open());
        assert_eq!(
            state.handle(Event::PickerKey(exo_ui::PickerKey::Next), 120),
            Some(Event::Recolor("work".into(), color("pink")))
        );
        // Escape puts back the colour it opened on and closes it.
        assert_eq!(
            state.handle(Event::Escape, 120),
            Some(Event::Recolor("work".into(), color("blue")))
        );
        assert!(!state.picker_open());
    }
}
