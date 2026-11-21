//! Detected dates on screen: the hairline under each, and the tooltip under the one the caret is
//! on.

use egui::{Area, Color32, CornerRadius, Order, Pos2, Rect, Sense, Ui, pos2, text::CCursor};
use exo_ui::{KeyTooltip, ShortcutTooltip};

use super::{BlockLayout, BodyText};
use crate::{dates::ActiveDate, editor::Editor, model::Pos};

/// `.sh-date::after`'s `inset 0 .5px #aaa`, in both themes (Carbon's other shadow has no size).
const LINE: Color32 = Color32::from_rgb(0xaa, 0xaa, 0xaa);
/// `::after { bottom: 0; height: 3px }`: the hairline tops a 3px box at the bottom of the span's
/// content area.
const LINE_ABOVE_BOTTOM: f32 = 3.0;
const LINE_HEIGHT: f32 = 0.5;
/// The date's tooltip: 3px under the date, 22.5px from the window's sides, fading in over 200ms. A
/// popped-out draft's keeps 7.5px from the left, and while another draft is popped out an inline
/// one's keeps 7.5px from the right.
const BELOW: f32 = 3.0;
const SIDE: f32 = 22.5;
const POPPED_OUT_SIDE: f32 = 7.5;
const FADE: f64 = 0.2;

/// `.sh-date::after` under characters `chars` of `view` (a galley range), on each row it's on:
/// `#aaa`, half a pixel, its ends on whole device pixels, over the text (`z-index: unset`).
pub(super) fn paint_line(
    painter: &egui::Painter,
    view: &BlockLayout,
    chars: std::ops::Range<usize>,
    galley_origin: Pos2,
    body: &BodyText,
) {
    let ppp = painter.pixels_per_point();
    let snap = |x: f32| (x * ppp).round() / ppp;
    let mut start = 0;
    for (row_index, row) in view.galley.rows.iter().enumerate() {
        let count = row.char_count_excluding_newline().0;
        let (from, to) = (chars.start.max(start), chars.end.min(start + count));
        if from < to {
            let x0 = snap(galley_origin.x + view.galley.pos_from_cursor(CCursor::new(from)).min.x);
            let x1 = snap(galley_origin.x + view.galley.pos_from_cursor(CCursor::new(to)).min.x);
            let line = view.lines[row_index.min(view.lines.len() - 1)];
            let top =
                galley_origin.y + line.top + line.shift + body.content_top + body.content_height
                    - LINE_ABOVE_BOTTOM;
            painter.rect_filled(
                Rect::from_x_y_ranges(x0..=x1, top..=top + LINE_HEIGHT),
                CornerRadius::ZERO,
                LINE,
            );
        }
        start += row.char_count_including_newline().0;
    }
}

impl Editor {
    /// The tooltip under `date` (`Previous Day` ⌘ shift – / `Next Day` ⌘ shift =), for an editor
    /// at `origin`: centred under the date's box, kept clear of the window's sides, hidden while
    /// the date is scrolled out of view, fading in when it first shows.
    pub(super) fn show_date_tooltip(&mut self, ui: &Ui, origin: Pos2, date: &ActiveDate, now: f64) {
        let since = *self.view.date_since.get_or_insert(now);
        let Some(target) = self
            .text_boxes(
                origin,
                Pos::new(date.block, date.range.start),
                Pos::new(date.block, date.range.end),
            )
            .into_iter()
            .reduce(Rect::union)
        else {
            return;
        };
        if !ui.clip_rect().intersects(target) {
            return;
        }
        let ctx = ui.ctx().clone();
        let tooltip = ShortcutTooltip {
            rows: &[
                KeyTooltip {
                    text: "Previous Day",
                    keys: &["⌘", "shift", "–"],
                },
                KeyTooltip {
                    text: "Next Day",
                    keys: &["⌘", "shift", "="],
                },
            ],
            note: None,
        };
        let size = tooltip.size(&ctx);
        let window = ctx.content_rect();
        // Centred to a whole pixel of the editor's box (`Math.ceil` of the shadow host's
        // coordinates).
        let mut left =
            origin.x + ((target.left() + target.right()) / 2.0 - origin.x - size.x / 2.0).ceil();
        let (min, max) = match (self.settings.popped_out, self.settings.another_popped_out) {
            (true, _) => (
                window.left() + POPPED_OUT_SIDE,
                window.right() - SIDE - size.x,
            ),
            (false, true) => (
                window.left() + SIDE,
                window.right() - POPPED_OUT_SIDE - size.x,
            ),
            (false, false) => (window.left() + SIDE, window.right() - SIDE - size.x),
        };
        if left < min {
            left = min;
        } else if left > max {
            left = max.max(min);
        }
        let card = Rect::from_min_size(pos2(left, target.bottom() + BELOW), size);
        let fade = ((now - since) / FADE).min(1.0) as f32;
        if fade < 1.0 {
            ctx.request_repaint();
        }
        Area::new(exo_ui::id(&ctx, ("exo_editor_date", origin.x.to_bits())))
            .order(Order::Foreground)
            .fixed_pos(card.min)
            .show(&ctx, |area| {
                // It takes clicks off the body under it.
                area.allocate_rect(card, Sense::CLICK);
                let mut painter = area.painter().clone();
                painter.multiply_opacity(exo_ui::motion::CubicBezier::EASE.at(fade));
                // In its own layer, in the app's theme even over a dialog drawn in Snow's (Out of
                // Office Reply's `Theme-light`).
                exo_ui::with_theme(&ctx, exo_ui::app_theme(&ctx), || {
                    tooltip.paint(&painter, card.min)
                });
            });
    }
}
