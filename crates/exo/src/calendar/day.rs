//! The right pane's day: the date with chevrons to the days around it, over that day's grid.

use egui::{Rect, Ui, Vec2, pos2, vec2};
use exo_ui::{Family, Font, IconButton, KeyTooltip, TextStyle};
use jiff::civil::Date;

use super::grid::{Grid, GridAction, GridView, Variant};

/// The heading: 16px/24px, `padding: 32px 2rem 24px`.
const HEADING: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 24.0);
const HEADING_TOP: f32 = 32.0;
const HEADING_HEIGHT: f32 = 32.0 + 24.0 + 24.0;
/// The date: `min-width: 90px; margin-right: 12px`.
const TITLE_MIN_WIDTH: f32 = 90.0;
const TITLE_MARGIN: f32 = 12.0;
/// The chevrons: `margin-top: -1px`; each is `padding: 0 7px` around the icon nudged `top: 2px`,
/// which lands it 4px below the line's top.
const CHEVRON: f32 = 14.0;
const CHEVRON_PADDING: f32 = 7.0;
const CHEVRON_ICON_TOP: f32 = 4.0;
/// Today and `+`: 19px tall.
const BUTTON_HEIGHT: f32 = 19.0;
/// The form's day: 24px under the form and 20px in, a 12px/20px grey title nudged `top: 3px` (at
/// least 80px wide this year, 100px otherwise), 20px chevrons (`margin-top: -3px`) with a 12px
/// legacy chevron 7px in and 4px down, the grid 8px under the title.
pub(super) const COMPACT_TOP: f32 = 24.0;
const COMPACT_LEFT: f32 = 20.0;
const COMPACT_TITLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
const COMPACT_GAP: f32 = 8.0;
/// The footer below the pane's content: `padding: 22px 2rem 12px` around 28px buttons.
pub const FOOTER_HEIGHT: f32 = 22.0 + 28.0 + 12.0;

/// What a click in the day view asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum DayAction {
    /// A chevron: the day before (-1) or after (1).
    Step(i64),
    /// Today.
    Today,
    /// `+`: a new event's form.
    Create,
    Grid(GridAction),
}

/// `{Dow}, {Mon} {d}`, with `, {yyyy}` outside the current year.
pub fn title(date: Date, today: Date) -> String {
    if date.year() == today.year() {
        date.strftime("%a, %b %-d").to_string()
    } else {
        date.strftime("%a, %b %-d, %Y").to_string()
    }
}

/// Paints the day view of `date` in `pane` (the right pane inside its border). `compose` is
/// whether a draft has the keyboard, which adds ⌘⇧ to the chevrons' keys.
pub fn show(
    ui: &Ui,
    pane: Rect,
    rem: f32,
    date: Date,
    grid: &Grid<'_>,
    view: &mut GridView,
    compose: bool,
) -> Vec<DayAction> {
    let mut actions = heading(ui, pane, rem, pane.top() + HEADING_TOP, date, grid, compose);
    let calendar = Rect::from_min_max(
        pos2(pane.left(), pane.top() + HEADING_HEIGHT),
        pos2(pane.right(), pane.bottom() - FOOTER_HEIGHT),
    );
    actions.extend(
        view.show(ui, calendar, rem, grid, Variant::Pane)
            .into_iter()
            .map(DayAction::Grid),
    );
    actions
}

/// The day's spinner while its events haven't been read (`Loading events`): 16px, 200px down
/// the pane, centred on the grid's column (`left: calc((100% - 8px + 36px + 4px)/2 - 16px/2)`),
/// black at `.3`, Carbon's white at `.7`.
pub fn paint_loading(ui: &Ui, pane: Rect) {
    let spinner = Rect::from_min_size(
        pos2(
            pane.left() + (pane.width() - 8.0 + 36.0 + 4.0) / 2.0 - 8.0,
            pane.top() + 200.0,
        ),
        Vec2::splat(16.0),
    );
    exo_ui::paint_spinner(
        ui.painter(),
        spinner,
        ui.input(|input| input.time),
        exo_ui::palette(ui.ctx()).ink(0.3, 0.7),
    );
}

/// The day under a new event's form in the right pane: the same heading 24px into `area`, the
/// day's grid down to its bottom. The chevrons' keys take ⌘⇧ there.
pub fn show_compact(
    ui: &Ui,
    area: Rect,
    rem: f32,
    date: Date,
    grid: &Grid<'_>,
    view: &mut GridView,
    _compose: bool,
) -> Vec<DayAction> {
    let top = area.top() + COMPACT_TOP;
    let palette = exo_ui::palette(ui.ctx());
    let left = area.left() + COMPACT_LEFT;
    let text = title(date, grid.now.date());
    let min_width = if date.year() == grid.now.date().year() {
        80.0
    } else {
        100.0
    };
    COMPACT_TITLE.paint_line(
        &ui.painter_at(area),
        pos2(left, top + 3.0),
        &text,
        palette.ink(0.35, 0.35),
        None,
    );
    let mut actions = Vec::new();
    let mut x = left + COMPACT_TITLE.width(ui.ctx(), &text).max(min_width);
    for (step, icon, label, key) in [
        (-1, "legacy/DEPRECATED_leftChevron", "Previous Day", "–"),
        (1, "legacy/DEPRECATED_rightChevron", "Next Day", "="),
    ] {
        let hit = Rect::from_min_size(pos2(x, top), vec2(20.0, 23.0));
        let icon_rect = Rect::from_min_size(hit.min + vec2(7.0, 4.0), vec2(6.0, 16.0));
        // The chevron black at `.3` (Carbon white).
        let response = IconButton::new(icon)
            .colors(palette.ink(0.3, 0.3), palette.ink(0.5, 0.5))
            .show(
                ui,
                ui.id().with(("calendar-form-day-chevron", step)),
                hit,
                exo_ui::icon_rect(ui.ctx(), icon, icon_rect),
            );
        KeyTooltip {
            text: label,
            keys: &["⌘", "shift", key],
        }
        .show_below(ui, &response, 8.0);
        if response.clicked() {
            actions.push(DayAction::Step(step));
        }
        x = hit.right();
    }
    // The grid `width: calc(100% + 40px - 1rem); margin-left: -20px` in the form's 20px padding:
    // the pane's width less 1rem.
    let calendar = Rect::from_min_max(
        pos2(area.left(), top + COMPACT_TITLE.line_height + COMPACT_GAP),
        pos2(area.right() - rem, area.bottom()),
    );
    actions.extend(
        view.show(ui, calendar, rem, grid, Variant::Pane)
            .into_iter()
            .map(DayAction::Grid),
    );
    actions
}

/// The date and its chevrons at `top`.
fn heading(
    ui: &Ui,
    pane: Rect,
    rem: f32,
    top: f32,
    date: Date,
    grid: &Grid<'_>,
    compose: bool,
) -> Vec<DayAction> {
    let palette = exo_ui::palette(ui.ctx());
    let painter = ui.painter_at(pane);
    let left = pane.left() + 2.0 * rem;
    let text = title(date, grid.now.date());
    let width = HEADING.width(ui.ctx(), &text).max(TITLE_MIN_WIDTH);
    HEADING.paint_line(
        &painter,
        pos2(left, top),
        &text,
        palette.text_base_default,
        None,
    );

    let mut actions = Vec::new();
    let mut x = left + width + TITLE_MARGIN;
    for (step, icon, label, key) in [
        (-1, "legacy/DEPRECATED_leftChevron", "Previous Day", "–"),
        (1, "legacy/DEPRECATED_rightChevron", "Next Day", "="),
    ] {
        // The legacy chevron at its own size (6px wide), so each chevron is 20px.
        let size = exo_ui::icon_size(ui.ctx(), icon).unwrap_or(Vec2::splat(CHEVRON));
        let hit = Rect::from_min_size(
            pos2(x, top - 1.0),
            vec2(size.x + 2.0 * CHEVRON_PADDING, HEADING.line_height),
        );
        let icon_rect = Rect::from_min_size(
            hit.min + vec2(CHEVRON_PADDING, CHEVRON_ICON_TOP),
            vec2(size.x, CHEVRON),
        );
        let response = IconButton::new(icon).show(
            ui,
            ui.id().with(("calendar-day-chevron", step)),
            hit,
            exo_ui::icon_rect(ui.ctx(), icon, icon_rect),
        );
        let keys: &[&str] = if compose {
            &["⌘", "shift", key]
        } else {
            &[key]
        };
        // Tethered 8px under the chevron.
        KeyTooltip { text: label, keys }.show_below(ui, &response, 8.0);
        if response.clicked() {
            actions.push(DayAction::Step(step));
        }
        x = hit.right();
    }

    // Today (`margin-left: auto`) and `+` (no right padding) at the heading's right, the
    // chevrons' look and nudges.
    let mut right = pane.right() - 2.0 * rem;
    for (icon, label, key, action, padding_right) in [
        (
            "legacy/DEPRECATED_plus",
            "Create Event",
            "B",
            DayAction::Create,
            0.0,
        ),
        (
            "legacy/DEPRECATED_undoArrow",
            "Today",
            "T",
            DayAction::Today,
            CHEVRON_PADDING,
        ),
    ] {
        let size = exo_ui::icon_size(ui.ctx(), icon).unwrap_or(Vec2::splat(CHEVRON));
        let hit = Rect::from_min_max(
            pos2(right - padding_right - size.x - CHEVRON_PADDING, top - 1.0),
            pos2(right, top - 1.0 + BUTTON_HEIGHT),
        );
        let glyph = Rect::from_min_size(hit.min + vec2(CHEVRON_PADDING, CHEVRON_ICON_TOP), size);
        let response =
            IconButton::new(icon).show(ui, ui.id().with(("calendar-day-button", key)), hit, glyph);
        KeyTooltip {
            text: label,
            keys: &[key],
        }
        .show_below(ui, &response, 8.0);
        if response.clicked() {
            actions.push(action);
        }
        right = hit.left();
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_carry_the_year_outside_this_one() {
        let today: Date = "2026-11-17".parse().expect("date");
        assert_eq!(title(today, today), "Tue, Nov 17");
        assert_eq!(
            title("2027-01-04".parse().expect("date"), today),
            "Mon, Jan 4, 2027"
        );
    }
}
