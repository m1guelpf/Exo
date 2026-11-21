//! The markers between a thread's messages: a message in Trash folded into `This message was
//! moved to Trash.` `Show message.`, and `Reminder returned` with when.

use egui::{Color32, Rect, Sense, Ui, pos2, vec2};
use exo_ui::{Family, Font, KeyTooltip, Placement, TextStyle};
use jiff::{Timestamp, Zoned};

use super::Layout;
use crate::format::dates;

/// `padding: 4px 2rem` around a 16px line.
pub(super) const HEIGHT: f32 = 24.0;
/// `margin-top: -8px`, and 20px after another marker.
pub(super) const MARGIN_TOP: f32 = -8.0;
pub(super) const MARGIN_TOP_AFTER_MARKER: f32 = 20.0;
/// `margin-bottom: 20px`, 24px before a message.
pub(super) const MARGIN_BOTTOM: f32 = 20.0;
pub(super) const MARGIN_BOTTOM_BEFORE_MESSAGE: f32 = 24.0;
/// What a message before a marker keeps under it at least, without a footer hint.
pub(super) const MESSAGE_MARGIN_BEFORE: f32 = 20.0;
const PADDING_TOP: f32 = 4.0;

/// The marker's text: 14px/16px, weight 600, `top: 0`.
const BODY: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 14.0, 16.0);
/// The marker's date: 12px/16px, `top: 1px`, `letter-spacing: .017em`.
const DATE: TextStyle =
    TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0).letter_spacing(0.017 * 12.0);
/// `Show message.`: `#54acdc`, Snow and Carbon.
const LINK: Color32 = Color32::from_rgb(0x54, 0xac, 0xdc);

/// The words before `Show message.`, its trailing space included.
pub(super) const TRASHED: &str = "This message was moved to Trash. ";
pub(super) const SHOW: &str = "Show message.";
/// What a returned reminder's marker says.
pub(super) const REMINDER: &str = "Reminder returned";

/// A trashed message's marker in `rect` (the marker's box, full width). Returns whether
/// `Show message.` was clicked.
pub(super) fn trashed(ui: &Ui, rect: Rect, layout: &Layout, id: egui::Id) -> bool {
    let palette = exo_ui::palette(ui.ctx());
    let painter = ui.painter();
    let line = pos2(layout.inner.left(), rect.top() + PADDING_TOP);
    // `color: rgba(0,0,0,.4)`, white in Carbon.
    let lead = BODY.paint_line(painter, line, TRASHED, palette.ink(0.4, 0.4), None);
    let width = BODY.paint_line(painter, line + vec2(lead, 0.0), SHOW, LINK, None);
    let show = Rect::from_min_size(line + vec2(lead, 0.0), vec2(width, BODY.line_height));
    ui.interact(show, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// `Reminder returned`, and on the right when it returned (its full date on hover).
pub(super) fn reminder(
    ui: &Ui,
    rect: Rect,
    layout: &Layout,
    at: Timestamp,
    now: &Zoned,
    id: egui::Id,
) {
    let palette = exo_ui::palette(ui.ctx());
    let painter = ui.painter();
    BODY.paint_line(
        painter,
        pos2(layout.inner.left(), rect.top() + PADDING_TOP),
        REMINDER,
        palette.ink(0.4, 0.4),
        None,
    );
    // `color: rgba(0,0,0,.5)`, white in Carbon; right-aligned with its trailing letter spacing.
    let date = dates::row(at, now);
    let width = DATE
        .single_line(painter, &date, Color32::PLACEHOLDER, None)
        .size()
        .x;
    let left = layout.inner.right() - width - DATE.letter_spacing;
    DATE.paint_line(
        painter,
        pos2(left, rect.top() + PADDING_TOP + 1.0),
        &date,
        palette.ink(0.5, 0.5),
        None,
    );
    date_tooltip(
        ui,
        layout,
        Rect::from_min_size(
            pos2(left, rect.top() + PADDING_TOP + 1.0),
            vec2(width, DATE.line_height),
        ),
        at,
        now,
        id,
    );
}

/// A date's hover tooltip on `date` (`Tue, Nov 17 2026 at 8:31 AM EST`): beside it, centred on
/// it, past the tooltip's 8px, on its left when the message column has no room on its right.
pub(super) fn date_tooltip(
    ui: &Ui,
    layout: &Layout,
    date: Rect,
    at: Timestamp,
    now: &Zoned,
    id: egui::Id,
) {
    let response = ui.interact(date, id, Sense::hover());
    KeyTooltip {
        text: &dates::tooltip(at, now),
        keys: &[],
    }
    .show_at(
        ui,
        &response,
        Placement::RightOrLeft {
            gap: 8.0,
            until: layout.column.right(),
        },
    );
}
