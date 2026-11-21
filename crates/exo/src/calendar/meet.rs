//! Meet with and Find Time: the week view's guests, their calendars over the week, and the free
//! times everyone shares in working hours.

use std::collections::BTreeMap;

use egui::{Color32, CornerRadius, Id, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use exo_store::Address;
use exo_ui::{CalendarColor, Family, Font, KeyTooltip, Menu, TextStyle};
use jiff::{SignedDuration, Timestamp, ToSpan, Zoned, civil::Date};

use crate::compose::{contacts::Index, recipients};

/// Working hours, 9 to 18, on weekdays.
pub const WORKING: (i8, i8) = (9, 18);
/// The meeting lengths offered, in minutes; 30 by default.
pub const DURATIONS: [i64; 4] = [15, 30, 45, 60];
/// How many weeks Find Time looks ahead: 20 days, in whole weeks.
pub const WEEKS: i64 = 3;
/// The most guests' calendars shown.
pub const MAX_GUESTS: usize = 7;

/// The section's type: the `Meet` header (as `Calendars`'), the input 28px under it, guests
/// 24px apart, the Find Time row.
const HEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 12.0, 16.0);
const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
const BUTTON: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 12.0, 16.0);
const INPUT_TOP: f32 = 28.0;
/// The input: 32px tall, 2px out on each side, a 2px border in its own fill,
/// `padding: 6px 8px 4px`, the text nudged `top: 1px`.
const INPUT_HEIGHT: f32 = 32.0;
/// The guests' list 8px under the input, a row 24px: an 8px swatch 6px in and 8px down, the
/// name 22px in, 5px down.
const GUEST: f32 = 24.0;
const GUESTS_GAP: f32 = 8.0;
/// The buttons, 14px under the guests: Find Time 75px and the length 56px, 27px tall, `#aeb1dd`
/// (Carbon `#a0a4d9`) with 2px outer corners, nudged `top: 1px`.
const BUTTONS_GAP: f32 = 14.0;
const BUTTONS: f32 = 27.0;
const FIND_WIDTH: f32 = 75.0;
const LENGTH_WIDTH: f32 = 56.0;
const SUGGESTION: f32 = 6.0 + 16.0 + 16.0 + 6.0;

/// A guest whose calendar shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guest {
    pub email: String,
    pub name: Option<String>,
}

/// The section's state.
pub struct Meet {
    pub guests: Vec<Guest>,
    /// The guests' calendars' colours, and the column a hover expanded.
    pub colors: super::guests::Colors,
    /// Find Time is on.
    pub finding: bool,
    /// The meeting's length, in minutes.
    pub duration: i64,
    input: String,
    suggestion: usize,
    /// The durations' list, open, and its highlighted row.
    durations: Option<usize>,
    /// The input takes the keyboard next frame (`m`), or gives it up (Escape).
    focus: bool,
    blur: bool,
    /// The input had the keyboard last frame.
    pub typing: bool,
}

impl Default for Meet {
    fn default() -> Self {
        Self {
            guests: Vec::new(),
            colors: super::guests::Colors::default(),
            finding: false,
            duration: 30,
            input: String::new(),
            suggestion: 0,
            durations: None,
            focus: false,
            blur: false,
            typing: false,
        }
    }
}

/// What the section asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeetAction {
    /// The guests changed: their calendars are wanted.
    Guests,
    /// `Find Time`, or Enter with guests and nothing typed.
    Find,
    /// `Cancel`.
    Cancel,
    /// `Hit enter to instantly find times!`'s ✕: it doesn't show again.
    DismissHint,
}

impl Meet {
    /// `m`: the keyboard to the input.
    pub fn focus(&mut self) {
        self.focus = true;
    }

    /// Escape in the input: it gives up the keyboard.
    pub fn blur(&mut self) {
        self.blur = true;
        self.input.clear();
    }

    /// The section's height.
    pub fn height(&self) -> f32 {
        INPUT_TOP
            + INPUT_HEIGHT
            + if self.guests.is_empty() {
                0.0
            } else {
                GUESTS_GAP + self.guests.len() as f32 * GUEST + BUTTONS_GAP + BUTTONS
            }
    }

    /// Paints the section from `at`'s top left, `at` wide, each guest after their calendar's
    /// marker in `markers`.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        at: Rect,
        contacts: Option<&Index>,
        me: &str,
        markers: &BTreeMap<String, super::guests::Marker>,
        hint: bool,
    ) -> Vec<MeetAction> {
        let (left, right, top) = (at.left(), at.right(), at.top());
        let ctx = ui.ctx().clone();
        let palette = exo_ui::palette(&ctx);
        let mut actions = Vec::new();
        // The header icon's disc: `#bfc8d2`, Carbon `#555b64`.
        let disc = match exo_ui::theme(&ctx) {
            exo_ui::Theme::Snow => Color32::from_rgb(0xbf, 0xc8, 0xd2),
            exo_ui::Theme::Carbon => Color32::from_rgb(0x55, 0x5b, 0x64),
        };
        exo_ui::paint_icon(
            ui,
            "legacy/DEPRECATED_meetWith",
            Rect::from_min_size(pos2(left, top), Vec2::splat(16.0)),
            disc,
        );
        // The sidebar's headings: `rgba(0,0,0,.8)`, as `Calendars`.
        HEADER.paint_line(
            ui.painter(),
            pos2(left + 24.0, top + 1.0),
            "Meet",
            palette.ink(0.8, 0.8),
            None,
        );

        // The input: `#f2f4fa` (Carbon `#373a3f`), its border `#aeb1dd` (Carbon `#a0a4d9`) with
        // the keyboard.
        let input = Rect::from_min_max(
            pos2(left - 2.0, top + INPUT_TOP),
            pos2(right + 2.0, top + INPUT_TOP + INPUT_HEIGHT),
        );
        let id = exo_ui::id(&ctx, "calendar-meet-input");
        let focused = ui.memory(|memory| memory.has_focus(id));
        let (fill, active) = match exo_ui::theme(&ctx) {
            exo_ui::Theme::Snow => (
                Color32::from_rgb(0xf2, 0xf4, 0xfa),
                Color32::from_rgb(0xae, 0xb1, 0xdd),
            ),
            exo_ui::Theme::Carbon => (
                Color32::from_rgb(0x37, 0x3a, 0x3f),
                Color32::from_rgb(0xa0, 0xa4, 0xd9),
            ),
        };
        ui.painter().rect(
            exo_ui::pixel_snap(input),
            CornerRadius::same(4),
            fill,
            Stroke::new(2.0, if focused { active } else { fill }),
            egui::StrokeKind::Inside,
        );
        let line = Rect::from_min_size(
            pos2(input.left() + 10.0, input.top() + 9.0),
            vec2(input.width() - 20.0, TEXT.line_height),
        );
        if self.input.is_empty() {
            // The placeholder: `rgba(0,0,0,.55)`.
            TEXT.paint_line(
                ui.painter(),
                line.min,
                "Enter name or email",
                palette.ink(0.55, 0.55),
                None,
            );
        }
        if std::mem::take(&mut self.focus) {
            ui.memory_mut(|memory| memory.request_focus(id));
        }
        let (up, down, enter) = ui.input(|input| {
            (
                input.key_pressed(egui::Key::ArrowUp),
                input.key_pressed(egui::Key::ArrowDown),
                input.key_pressed(egui::Key::Enter),
            )
        });
        // A single-line field lets go of the keyboard on Enter, before it's seen here.
        let had_focus = ui.memory(|memory| memory.has_focus(id));
        let response = exo_ui::text_field(
            ui,
            id,
            &mut self.input,
            line,
            &TEXT,
            palette.text_base_default,
            palette.text_selection_default,
        );
        if std::mem::take(&mut self.blur) {
            response.surrender_focus();
        }
        self.typing = response.has_focus();
        KeyTooltip {
            text: "Meet",
            keys: &["M"],
        }
        .show_below(ui, &response, 8.0);
        let suggestions: Vec<Address> = match contacts {
            Some(book) if !self.input.trim().is_empty() => book
                .top(self.input.trim(), 3 + self.guests.len())
                .into_iter()
                .filter(|contact| {
                    !contact.email.eq_ignore_ascii_case(me)
                        && !self
                            .guests
                            .iter()
                            .any(|guest| guest.email.eq_ignore_ascii_case(&contact.email))
                })
                .take(3)
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        if response.changed() {
            self.suggestion = 0;
        }
        if had_focus {
            if up {
                self.suggestion = self.suggestion.saturating_sub(1);
            } else if down {
                self.suggestion = (self.suggestion + 1).min(suggestions.len().saturating_sub(1));
            }
        }
        let mut add = None;
        if had_focus && enter {
            if let Some(pick) = suggestions.get(self.suggestion) {
                add = Some((pick.email.clone(), pick.name.clone()));
            } else if self.input.trim().is_empty() && !self.guests.is_empty() {
                actions.push(MeetAction::Find);
            } else {
                let typed = recipients::parse(self.input.trim());
                if recipients::is_valid(&typed.email) {
                    add = Some((typed.email, typed.name));
                }
            }
        }
        // The suggestions under the input.
        if response.has_focus() && !suggestions.is_empty() {
            let list = Rect::from_min_size(
                pos2(input.left(), input.bottom() + 4.0),
                vec2(input.width(), suggestions.len() as f32 * SUGGESTION + 2.0),
            );
            egui::Area::new(exo_ui::id(&ctx, "calendar-meet-suggestions"))
                .order(egui::Order::Tooltip)
                .fixed_pos(list.min)
                .constrain(false)
                .show(&ctx, |area| {
                    let painter = area.painter().clone();
                    painter.rect(
                        exo_ui::pixel_snap(list),
                        CornerRadius::same(4),
                        palette.background_base_default,
                        Stroke::new(1.0, palette.border_base_default),
                        egui::StrokeKind::Inside,
                    );
                    for (index, contact) in suggestions.iter().enumerate() {
                        let row = Rect::from_min_size(
                            pos2(
                                list.left() + 1.0,
                                list.top() + 1.0 + index as f32 * SUGGESTION,
                            ),
                            vec2(list.width() - 2.0, SUGGESTION),
                        );
                        let hit = area
                            .interact(
                                row,
                                exo_ui::id(area.ctx(), ("calendar-meet-suggestion", index)),
                                Sense::click(),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                        if hit.hovered() {
                            self.suggestion = index;
                        }
                        if index == self.suggestion {
                            painter.rect_filled(row, 0.0, palette.fill_hover_default);
                        }
                        let name = contact
                            .name
                            .clone()
                            .unwrap_or_else(|| contact.email.clone());
                        TEXT.paint_line(
                            &painter,
                            pos2(row.left() + 10.0, row.top() + 6.0),
                            &name,
                            palette.text_base_default,
                            Some(row.width() - 20.0),
                        );
                        TEXT.paint_line(
                            &painter,
                            pos2(row.left() + 10.0, row.top() + 22.0),
                            &contact.email,
                            palette.text_base_subdued,
                            Some(row.width() - 20.0),
                        );
                        if hit.clicked() {
                            add = Some((contact.email.clone(), contact.name.clone()));
                        }
                    }
                });
        }
        if let Some((email, name)) = add
            && self.guests.len() < MAX_GUESTS
        {
            self.guests.push(Guest { email, name });
            self.input.clear();
            self.focus = true;
            actions.push(MeetAction::Guests);
        }

        // The guests, each in their calendar's colour, × on hover.
        let mut y = input.bottom() + GUESTS_GAP;
        let mut removed = None;
        for (index, guest) in self.guests.iter().enumerate() {
            let row = Rect::from_min_max(pos2(left, y), pos2(right, y + GUEST));
            let hovered = ui.rect_contains_pointer(row);
            let marker = markers
                .get(&guest.email)
                .copied()
                .unwrap_or(super::guests::Marker::Empty);
            super::guests::paint_marker(
                ui,
                Rect::from_min_size(pos2(left + 6.0, row.top() + 8.0), Vec2::splat(8.0)),
                marker,
            );
            TEXT.paint_line(
                ui.painter(),
                pos2(left + 22.0, row.top() + 5.0),
                guest.name.as_deref().unwrap_or(&guest.email),
                palette.ink(0.9, 0.9),
                Some(right - left - 44.0),
            );
            if hovered {
                let cross =
                    Rect::from_min_size(pos2(right - 14.0, row.top() + 8.0), Vec2::splat(8.0));
                let close = "legacy/DEPRECATED_closeV3";
                if exo_ui::IconButton::new(close)
                    .show(
                        ui,
                        exo_ui::id(ui.ctx(), ("calendar-meet-remove", index)),
                        cross.expand(4.0),
                        exo_ui::icon_rect(&ctx, close, cross),
                    )
                    .clicked()
                {
                    removed = Some(index);
                }
            }
            y += GUEST;
        }
        if let Some(index) = removed {
            self.guests.remove(index);
            if self.guests.is_empty() {
                self.finding = false;
            }
            actions.push(MeetAction::Guests);
        }
        if self.guests.is_empty() {
            return actions;
        }

        // `Find Time` (`Cancel` while finding) and the length (`30m`), one periwinkle pill split
        // by a 1px rule, glowing on hover; white labels (Carbon `#27292d`).
        let row_top = y + BUTTONS_GAP + 1.0;
        let (fill, ink, rule) = match exo_ui::theme(&ctx) {
            exo_ui::Theme::Snow => (
                Color32::from_rgb(0xae, 0xb1, 0xdd),
                Color32::from_white_alpha(250),
                Color32::from_white_alpha(250),
            ),
            exo_ui::Theme::Carbon => (
                Color32::from_rgb(0xa0, 0xa4, 0xd9),
                Color32::from_rgb(0x27, 0x29, 0x2d),
                Color32::from_rgb(0x27, 0x29, 0x2d),
            ),
        };
        let glow = |id: Id, rect: Rect, hovered: bool, radius: CornerRadius| {
            let on = exo_ui::motion::transition(
                &ctx,
                id.with("glow"),
                hovered,
                0.15,
                exo_ui::motion::CubicBezier::EASE,
            );
            if on > 0.0 {
                exo_ui::BoxShadow::new(
                    [0.0, 0.0],
                    6.0,
                    0.0,
                    Color32::from_rgb(0xae, 0xb1, 0xdd).gamma_multiply(on),
                )
                .paint(ui.painter(), rect, radius);
                exo_ui::BoxShadow::new(
                    [0.0, 2.0],
                    12.0,
                    0.0,
                    Color32::from_rgba_unmultiplied(0xae, 0xb1, 0xdd, 128).gamma_multiply(on),
                )
                .paint(ui.painter(), rect, radius);
            }
        };
        let label = if self.finding { "CANCEL" } else { "FIND TIME" };
        let button = Rect::from_min_size(pos2(left, row_top), vec2(FIND_WIDTH, BUTTONS));
        let pressed = ui
            .interact(
                button,
                exo_ui::id(ui.ctx(), "calendar-meet-find"),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let corners = CornerRadius {
            nw: 2,
            sw: 2,
            ne: 0,
            se: 0,
        };
        glow(
            pressed.id,
            exo_ui::pixel_snap(button),
            pressed.hovered(),
            corners,
        );
        ui.painter()
            .rect_filled(exo_ui::pixel_snap(button), corners, fill);
        ui.painter().rect_filled(
            Rect::from_min_max(pos2(button.right() - 1.0, button.top()), button.max),
            0.0,
            rule,
        );
        BUTTON.paint_line(
            ui.painter(),
            pos2(
                button.center().x - 0.5 - BUTTON.width(&ctx, label) / 2.0,
                button.top() + 2.0 + 4.5,
            ),
            label,
            ink,
            None,
        );
        // While `Hit enter to instantly find times!` is up beside it, Find Time's own tooltip
        // isn't.
        if hint && !self.finding {
            if find_time_hint(ui, button) {
                actions.push(MeetAction::DismissHint);
            }
        } else {
            let tip = format!("Find time for a {} minute meeting.", self.duration);
            KeyTooltip {
                text: &tip,
                keys: &[],
            }
            .show_below(ui, &pressed, 8.0);
        }
        if pressed.clicked() {
            actions.push(if self.finding {
                MeetAction::Cancel
            } else {
                MeetAction::Find
            });
        }
        let length = format!("{}m", self.duration);
        let picker =
            Rect::from_min_size(pos2(button.right(), row_top), vec2(LENGTH_WIDTH, BUTTONS));
        let opened = ui
            .interact(
                picker,
                exo_ui::id(ui.ctx(), "calendar-meet-duration"),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let corners = CornerRadius {
            nw: 0,
            sw: 0,
            ne: 2,
            se: 2,
        };
        glow(
            opened.id,
            exo_ui::pixel_snap(picker),
            opened.hovered(),
            corners,
        );
        ui.painter()
            .rect_filled(exo_ui::pixel_snap(picker), corners, fill);
        // The length and a 12×7 caret 4px after it, centred.
        let text_width = BUTTON.width(&ctx, &length);
        let content_left = picker.center().x - (text_width + 4.0 + 12.0) / 2.0 + 1.0;
        BUTTON.paint_line(
            ui.painter(),
            pos2(content_left, picker.top() + 2.0 + 4.5),
            &length,
            ink,
            None,
        );
        exo_ui::paint_icon(
            ui,
            "legacy/DEPRECATED_downCaret",
            Rect::from_min_size(
                pos2(content_left + text_width + 4.0, picker.top() + 10.5),
                vec2(12.0, 7.0),
            ),
            ink,
        );
        KeyTooltip {
            text: "More durations",
            keys: &[],
        }
        .show_below(ui, &opened, 8.0);
        if opened.clicked() {
            self.durations = if self.durations.is_some() {
                None
            } else {
                DURATIONS
                    .iter()
                    .position(|minutes| *minutes == self.duration)
            };
        }
        if let Some(highlighted) = self.durations {
            let labels: Vec<String> = DURATIONS
                .iter()
                .map(|minutes| format!("{minutes}m"))
                .collect();
            let items: Vec<&str> = labels.iter().map(String::as_str).collect();
            let menu = Menu {
                items: &items,
                highlighted,
            }
            .show(
                ui,
                exo_ui::id(ui.ctx(), "calendar-meet-durations"),
                picker.left_bottom() + vec2(0.0, 4.0),
            );
            self.durations = Some(menu.hovered.unwrap_or(highlighted));
            if let Some(picked) = menu.clicked {
                self.duration = DURATIONS[picked];
                self.durations = None;
            } else if ui.input(|input| input.pointer.any_released())
                && !ui.rect_contains_pointer(menu.rect)
                && !opened.hovered()
            {
                self.durations = None;
            }
        }
        actions
    }
}

/// Find Time's Enter hint: a tooltip tethered by its top right to Find Time's top left (10px up
/// and 8px left, less the 8px padding of a tooltip placed below), its row 204px wide padded
/// `0 1px 3px`: `Hit` `enter` ` to instantly find times!` (12px semibold, `top: 1px`, the key
/// with no margin), and the ✕ (`DEPRECATED_closeV3`, 8px, white at 30%, Carbon's black) at the
/// right. Returns a click on the ✕.
fn find_time_hint(ui: &Ui, button: Rect) -> bool {
    const ROW: f32 = 204.0;
    const ROW_PADDING: f32 = 1.0;
    const PADDING: Vec2 = Vec2::new(7.0, 6.0);
    /// The row's 18px (its key) and 3px under it, and the box's 3px.
    const HEIGHT: f32 = 6.0 + 18.0 + 3.0 + 3.0;
    const OFFSET: Vec2 = Vec2::new(8.0, 10.0);
    const PLACEMENT_PADDING: f32 = 8.0;
    const CLOSE: f32 = 8.0;
    const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 12.0, 16.0);
    let ctx = ui.ctx();
    let size = vec2(ROW + 2.0 * PADDING.x, HEIGHT);
    let rect = Rect::from_min_size(
        pos2(
            button.left() - OFFSET.x - size.x,
            button.top() - OFFSET.y + PLACEMENT_PADDING,
        ),
        size,
    );
    let color = exo_ui::tooltip_text_color(ctx);
    let (keys, rest, hover) = match exo_ui::theme(ctx) {
        exo_ui::Theme::Snow => (
            color,
            Color32::from_white_alpha(77),
            Color32::from_white_alpha(153),
        ),
        exo_ui::Theme::Carbon => (
            Color32::from_white_alpha(230),
            Color32::from_black_alpha(77),
            Color32::from_black_alpha(153),
        ),
    };
    egui::Area::new(exo_ui::id(ctx, "calendar-meet-find-hint"))
        .order(egui::Order::Tooltip)
        .fixed_pos(rect.min)
        .constrain(false)
        .show(ctx, |area| {
            let painter = area.painter();
            exo_ui::paint_tooltip_box(painter, rect);
            // The text `top: 1px`, its line a pixel lower again for the key (an 18px inline block on
            // a 16px line); the key at the line's top, its label (`top: 0`) a pixel higher in it than
            // in other tooltips.
            let row = pos2(
                rect.left() + PADDING.x,
                exo_ui::pixel_snap(rect).top() + PADDING.y,
            );
            let line = row + vec2(ROW_PADDING, 2.0);
            let mut x = line.x + TEXT.paint_line(painter, line, "Hit ", color, None);
            // A tooltip key's colours.
            let caps = exo_ui::Keycaps::new(
                ctx,
                &["enter"],
                exo_ui::KeycapColors {
                    text: keys,
                    divider: color,
                    ..exo_ui::KeycapColors::base(exo_ui::palette(ctx))
                },
            )
            .gap(0.0)
            .baseline(exo_ui::KEYCAP_BASELINE - 1.0);
            caps.paint(painter, x, row.y + 1.0);
            x += caps.width();
            TEXT.paint_line(
                painter,
                pos2(x, line.y),
                " to instantly find times!",
                color,
                None,
            );
            let close = Rect::from_min_size(
                pos2(row.x + ROW - ROW_PADDING - CLOSE, row.y),
                vec2(CLOSE, 16.0),
            );
            let hit = area
                .interact(
                    close,
                    exo_ui::id(area.ctx(), "calendar-meet-find-hint-close"),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let icon =
                Rect::from_min_size(pos2(close.left(), close.top() + 4.5), Vec2::splat(CLOSE));
            exo_ui::paint_icon(
                area,
                "legacy/DEPRECATED_closeV3",
                icon,
                if hit.hovered() { hover } else { rest },
            );
            hit.clicked()
        })
        .inner
}

/// The sticky `No availability this week` toast's tag, taken back when the week changes.
const NEXT_AVAILABILITY: u64 = 0xca1e_0001;

impl crate::Mail {
    /// With Find Time on, the week's free slots as events, from the guests' calendars and my
    /// calendars' free/busy, asked for as needed.
    pub(super) fn calendar_meet_view(&mut self, now: &Zoned) -> Vec<super::event::Event> {
        let Some(week) = &self.calendar.week else {
            return Vec::new();
        };
        let (active, guests, finding, duration) = (
            week.active,
            week.meet.guests.clone(),
            week.meet.finding,
            week.meet.duration,
        );
        if self.calendar.meet_week != Some(active) {
            // A week change takes back the offer of another week.
            self.calendar.meet_week = Some(active);
            if self.commands.toast_tag() == Some(NEXT_AVAILABILITY) {
                self.commands.hide_toast();
            }
        }
        if guests.is_empty() || !finding {
            return Vec::new();
        }
        let shown = self
            .account
            .settings()
            .calendars_where(|calendar| calendar.show);
        let tz = now.time_zone().clone();
        let first = self
            .calendar_week_start(active)
            .min(self.calendar_week_start(now.date()));
        let (Ok(from), Ok(to)) = (
            first.to_zoned(tz.clone()),
            (first + (7 * (WEEKS + 1)).days()).to_zoned(tz.clone()),
        ) else {
            return Vec::new();
        };
        let (from, to) = (from.timestamp(), to.timestamp());
        let data = self.account.calendar().data();
        let covered = data
            .busy_range
            .is_some_and(|(start, end)| start <= from && end >= to)
            && shown.iter().all(|id| data.busy.contains_key(id));
        let asked = (shown.clone(), from, to);
        if !covered && self.calendar.meet_asked.as_ref() != Some(&asked) {
            self.calendar.meet_asked = Some(asked);
            self.account.calendar().busy(shown.clone(), from, to);
        }
        // Every guest's calendar read over the weeks, or given up on.
        let read = guests.iter().all(|guest| {
            data.guests.get(&guest.email).is_some_and(|calendar| {
                !calendar.loading && calendar.range.0 <= from && calendar.range.1 >= to
            })
        });
        if !covered || !read {
            return Vec::new();
        }
        let everyone: Vec<(Timestamp, Timestamp)> = shown
            .iter()
            .flat_map(|id| data.busy.get(id).cloned().flatten().unwrap_or_default())
            .chain(
                guests
                    .iter()
                    .flat_map(|guest| super::guests::busy(&data, &guest.email, &tz)),
            )
            .collect();
        if std::mem::take(&mut self.calendar.find_pending) {
            self.calendar_find_first(&everyone, active, duration, now);
        }
        // From now in this week, else from the week's start, for 7 days.
        let start = if self.calendar_week_start(now.date()) >= self.calendar_week_start(active) {
            Some(now.clone())
        } else {
            self.calendar_week_start(active).to_zoned(tz).ok()
        };
        let Some(start) = start else {
            return Vec::new();
        };
        let Ok(end) = start.checked_add(7.days()) else {
            return Vec::new();
        };
        self.calendar.slots = slots(&everyone, &start, &end, duration);
        let color = self
            .calendar_writable()
            .into_iter()
            .next()
            .map_or(CalendarColor::BLUE, |calendar| calendar.color);
        self.calendar
            .slots
            .iter()
            .enumerate()
            .map(|(index, (start, end))| {
                super::event::Event::draft(
                    format!("{}{index}", super::grid::FREE_SLOT),
                    format!("{} – {}", start.strftime("%-I:%M"), end.strftime("%-I:%M")),
                    start.clone(),
                    end.clone(),
                    false,
                    color,
                )
            })
            .collect()
    }

    /// The first week, of three from this one, with a free slot; when it's
    /// not the week on screen, offers it.
    fn calendar_find_first(
        &mut self,
        busy: &[(Timestamp, Timestamp)],
        active: Date,
        duration: i64,
        now: &Zoned,
    ) {
        let tz = now.time_zone().clone();
        let week = self.calendar_week_start(now.date().max(active));
        let first = (0..WEEKS).find_map(|offset| {
            let start = (week + (7 * offset).days()).to_zoned(tz.clone()).ok()?;
            let start = if start < *now { now.clone() } else { start };
            let end = (week + (7 * (offset + 1)).days())
                .to_zoned(tz.clone())
                .ok()?;
            slots(busy, &start, &end, duration)
                .first()
                .map(|(start, _)| start.date())
        });
        match first {
            Some(date) if self.calendar_week_start(date) == self.calendar_week_start(active) => {}
            Some(date) => {
                let toast = exo_commands::Toast::new("No availability this week")
                    .sticky()
                    .tag(NEXT_AVAILABILITY)
                    .action(
                        "Show next availability",
                        None,
                        crate::commands::Action::Calendar(super::Action::ShowAvailability(date)),
                    );
                self.commands.toast(toast);
            }
            None => self.commands.toast(exo_commands::Toast::new(format!(
                "No availability in the next {WEEKS} weeks. Try removing a guest."
            ))),
        }
    }

    /// What Meet asked for.
    pub(super) fn calendar_meet(&mut self, action: MeetAction, _now: &Zoned) {
        let Some(week) = &mut self.calendar.week else {
            return;
        };
        match action {
            MeetAction::Guests => {
                if week.meet.guests.is_empty()
                    && self.commands.toast_tag() == Some(NEXT_AVAILABILITY)
                {
                    self.commands.hide_toast();
                }
            }
            MeetAction::Find if !week.meet.guests.is_empty() => {
                week.meet.finding = true;
                self.calendar.details = None;
                self.calendar.find_pending = true;
            }
            MeetAction::Find => {}
            MeetAction::Cancel => {
                week.meet.finding = false;
                self.calendar.find_pending = false;
                if self.commands.toast_tag() == Some(NEXT_AVAILABILITY) {
                    self.commands.hide_toast();
                }
            }
            MeetAction::DismissHint => self.account.update_settings(|settings| {
                settings
                    .calendar
                    .has_dismissed_find_time_enter_shortcut_tooltip = true
            }),
        }
    }

    /// A free slot clicked: the new event's form for it, with Meet's guests and their names as
    /// its title.
    pub(super) fn calendar_pick_slot(&mut self, key: &str, _now: &Zoned) {
        let Some((start, end)) = key
            .strip_prefix(super::grid::FREE_SLOT)
            .and_then(|index| index.parse::<usize>().ok())
            .and_then(|index| self.calendar.slots.get(index).cloned())
        else {
            return;
        };
        let Some(week) = &self.calendar.week else {
            return;
        };
        let me = self
            .snapshot
            .aliases
            .iter()
            .find(|alias| alias.primary)
            .and_then(|alias| alias.name.clone());
        let title = auto_title(me.as_deref(), &week.meet.guests);
        let guests = week
            .meet
            .guests
            .iter()
            .map(|guest| super::form::Guest {
                email: guest.email.clone(),
                name: guest.name.clone(),
                optional: false,
                response: None,
                me: false,
                organizer: false,
            })
            .collect();
        let values = super::form::Values {
            title,
            when: Some((start, end)),
            all_day: false,
            guests,
            location: String::new(),
            about: String::new(),
            meeting: super::form::Meeting::None,
            calendar: self.account.email().to_owned(),
            repeat: None,
        };
        self.calendar.details = None;
        // The form takes Meet's guests' calendars as they are.
        let mut form = super::form::EventForm::create(values, true);
        form.guest_colors = week.meet.colors.clone();
        self.calendar_open_form(form);
    }
}

/// The free `minutes`-long slots everyone has between `from` and `to` in working hours, around
/// `busy`: busy spans widened to quarter hours and merged, taken out of each weekday's 9 to 18,
/// and what's left cut into meetings back to back.
pub fn slots(
    busy: &[(Timestamp, Timestamp)],
    from: &Zoned,
    to: &Zoned,
    minutes: i64,
) -> Vec<(Zoned, Zoned)> {
    let tz = from.time_zone().clone();
    let quarter = |at: Timestamp| -> Timestamp {
        let local = at.to_zoned(tz.clone());
        let over = i64::from(local.minute() % 15);
        let rounded = if over == 0 && local.second() == 0 {
            local
        } else {
            local
                .checked_add((15 - over).minutes())
                .and_then(|at| at.with().second(0).subsec_nanosecond(0).build())
                .unwrap_or(local)
        };
        rounded.timestamp()
    };
    let mut occupied: Vec<(Timestamp, Timestamp)> = busy
        .iter()
        .map(|(start, end)| (quarter(*start), quarter(*end)))
        .collect();
    occupied.sort();
    let mut merged: Vec<(Timestamp, Timestamp)> = Vec::new();
    for (start, end) in occupied {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    let start = quarter(from.timestamp());
    let mut free = Vec::new();
    let mut day: Date = from.date();
    while day <= to.date() {
        let weekday = day.weekday().to_monday_one_offset();
        if weekday <= 5
            && let (Ok(open), Ok(close)) = (
                day.at(WORKING.0, 0, 0, 0).to_zoned(tz.clone()),
                day.at(WORKING.1, 0, 0, 0).to_zoned(tz.clone()),
            )
            && close.timestamp() > start
            && open.timestamp() < to.timestamp()
        {
            let begin = open.timestamp().max(start);
            let end = close.timestamp().min(to.timestamp());
            let mut cursor = begin;
            for (busy_start, busy_end) in merged
                .iter()
                .filter(|(busy_start, busy_end)| *busy_start < end && *busy_end > begin)
            {
                if cursor < *busy_start {
                    free.push((cursor, (*busy_start).min(end)));
                }
                cursor = cursor.max((*busy_end).min(end));
            }
            if cursor < end {
                free.push((cursor, end));
            }
        }
        let Ok(next) = day.tomorrow() else { break };
        day = next;
    }
    let length = SignedDuration::from_mins(minutes);
    let mut slots = Vec::new();
    for (mut at, end) in free {
        while let Ok(next) = at.checked_add(length)
            && next <= end
        {
            slots.push((at.to_zoned(tz.clone()), next.to_zoned(tz.clone())));
            at = next;
        }
    }
    slots
}

/// A Meet event's title: `Maya / Lena` from first names, for two or three people.
pub fn auto_title(me: Option<&str>, guests: &[Guest]) -> String {
    if guests.len() > 2 {
        return String::new();
    }
    let first = |name: &str| name.split_whitespace().next().unwrap_or(name).to_owned();
    me.into_iter()
        .map(first)
        .chain(guests.iter().map(|guest| {
            guest
                .name
                .as_deref()
                .map_or_else(|| guest.email.clone(), first)
        }))
        .collect::<Vec<_>>()
        .join(" / ")
}

/// Disabled hours: all of Saturday and Sunday, and weekdays outside 9 to 18, as spans of
/// `day` in hours.
pub fn disabled_hours(day: Date) -> Vec<(i8, i8)> {
    if day.weekday().to_monday_one_offset() > 5 {
        vec![(0, 24)]
    } else {
        vec![(0, WORKING.0), (WORKING.1, 24)]
    }
}

/// The shade over disabled hours: `rgba(246,246,246,.7)` in Snow, `rgba(54,58,64,.8)` in Carbon.
pub fn disabled_fill(theme: exo_ui::Theme) -> Color32 {
    match theme {
        exo_ui::Theme::Snow => Color32::from_rgba_unmultiplied(246, 246, 246, 179),
        exo_ui::Theme::Carbon => Color32::from_rgba_unmultiplied(54, 58, 64, 204),
    }
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

    #[test]
    fn slots_fill_working_hours_around_busy_times() {
        // Tuesday from 9:10: the first slot waits for the quarter hour; a 10:05–11:00 meeting's
        // start rounds up too, so it takes 10:15–11:00.
        let busy = [(
            at("2026-11-17T10:05").timestamp(),
            at("2026-11-17T11:00").timestamp(),
        )];
        let found = slots(&busy, &at("2026-11-17T09:10"), &at("2026-11-18T00:00"), 30);
        let starts: Vec<String> = found
            .iter()
            .take(4)
            .map(|(start, _)| start.strftime("%H:%M").to_string())
            .collect();
        assert_eq!(starts, ["09:15", "09:45", "11:00", "11:30"]);
        assert_eq!(
            found
                .last()
                .map(|(_, end)| end.strftime("%H:%M").to_string())
                .as_deref(),
            Some("18:00")
        );
        // Weekends have none.
        assert!(slots(&[], &at("2026-11-21T00:00"), &at("2026-11-23T00:00"), 30).is_empty());
    }

    #[test]
    fn titles_name_two_or_three_people() {
        let guest = |name: &str| Guest {
            email: "x@y.z".into(),
            name: Some(name.into()),
        };
        assert_eq!(
            auto_title(Some("Maya Okafor"), &[guest("Lena Fischer")]),
            "Maya / Lena"
        );
        assert_eq!(
            auto_title(Some("Maya"), &[guest("A"), guest("B"), guest("C")]),
            ""
        );
    }
}
