//! The scrolling panes' overlay scrollbar: a thumb that shows while the pane scrolls or the pointer
//! is over its 11px track, and fades out .2s after a .5s pause.

use egui::{Color32, CornerRadius, Id, Rect, Ui, pos2, vec2};

use crate::motion::{self, CubicBezier};

/// The track: 11px along the pane's edge.
const TRACK: f32 = 11.0;

/// A pane's scrollbar, remembering when it last scrolled.
#[derive(Clone, Copy, Debug)]
pub struct Scrollbar {
    scrolled_at: f64,
    scrolled_to: f32,
}

impl Default for Scrollbar {
    fn default() -> Self {
        Self {
            scrolled_at: f64::NEG_INFINITY,
            scrolled_to: 0.0,
        }
    }
}

impl Scrollbar {
    /// Paints the thumb over the right edge of `viewport` for a pane scrolled to `scroll` of
    /// `content` px.
    pub fn show(&mut self, ui: &Ui, id: Id, viewport: Rect, scroll: f32, content: f32) {
        self.paint(ui, id, viewport, scroll, content, false);
    }

    /// Paints the thumb over the bottom edge of `viewport` for a pane scrolled sideways to
    /// `scroll` of `content` px.
    pub fn show_horizontal(&mut self, ui: &Ui, id: Id, viewport: Rect, scroll: f32, content: f32) {
        self.paint(ui, id, viewport, scroll, content, true);
    }

    fn paint(
        &mut self,
        ui: &Ui,
        id: Id,
        viewport: Rect,
        scroll: f32,
        content: f32,
        horizontal: bool,
    ) {
        let now = ui.input(|input| input.time);
        if scroll != self.scrolled_to {
            self.scrolled_to = scroll;
            self.scrolled_at = now;
        }
        let length = if horizontal {
            viewport.width()
        } else {
            viewport.height()
        };
        if content <= length {
            return;
        }
        let track = if horizontal {
            Rect::from_min_max(
                pos2(viewport.left(), viewport.bottom() - TRACK),
                viewport.max,
            )
        } else {
            Rect::from_min_max(pos2(viewport.right() - TRACK, viewport.top()), viewport.max)
        };
        let shown = ui.rect_contains_pointer(track) || now - self.scrolled_at < 1.0;
        let opacity = motion::transition(
            ui.ctx(),
            id,
            shown,
            if shown { 0.0 } else { 0.2 },
            CubicBezier::LINEAR,
        );
        if opacity <= 0.0 {
            return;
        }
        if shown {
            ui.ctx().request_repaint_after_secs(0.25);
        }
        // At least 25px long, inset 2px, black at .5 with `border-radius: 7px`.
        let thumb_length = (length * length / content).max(25.0);
        let start = scroll / (content - length) * (length - thumb_length);
        let thumb = if horizontal {
            Rect::from_min_size(
                pos2(track.left() + start, track.top()),
                vec2(thumb_length, TRACK),
            )
        } else {
            Rect::from_min_size(
                pos2(track.left(), track.top() + start),
                vec2(TRACK, thumb_length),
            )
        };
        ui.painter().rect_filled(
            thumb.shrink(2.0),
            CornerRadius::same(4),
            Color32::from_black_alpha((0.5 * opacity * 255.0).round() as u8),
        );
    }
}
