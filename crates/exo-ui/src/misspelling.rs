//! Chromium's misspelling marker on a Mac (`DocumentMarkerPainter`), which compose's body and
//! the spellchecked text fields draw under a misspelled word.

use std::ops::Range;

use egui::{Color32, CornerRadius, Painter, Rect, pos2};

use crate::TextStyle;

/// The marker's height (`kMarkerHeight`), and its dots' width.
const DOT: f32 = 2.0;
/// From one dot to the next.
const PERIOD: f32 = 4.0;
/// macOS's red, in both themes.
const SPELLING: Color32 = Color32::from_rgba_unmultiplied_const(255, 59, 48, 191);

/// Where the marker's top goes under a line of `style`, from the top of its line box: 1px under
/// the baseline, as Chromium puts it under 12px and 14px text.
pub fn misspelling_top(style: &TextStyle, ctx: &egui::Context) -> f32 {
    style.baseline(ctx) + 1.0
}

/// Paints the marker under the text between `x` on screen, its top at `top`: square dots 2px
/// across, 4px apart from `pattern` (where Chromium's tiled dots start: the word's first device
/// pixel in compose's body, the text's start in an `<input>`), over as much of the word as holds
/// whole dots and a space (`width -= fmodf(width + spacing, kMarkerWidth) - spacing`), cut to it;
/// `rgba(255, 59, 48, .75)`.
pub fn paint_misspelling(painter: &Painter, x: Range<f32>, top: f32, pattern: f32) {
    let ppp = painter.pixels_per_point();
    let snap = |at: f32| (at * ppp).floor() / ppp;
    let y = (top * ppp).round() / ppp;
    let width = x.end - x.start;
    let end = x.start + width - ((width + DOT).rem_euclid(PERIOD) - DOT);
    // The device pixels whose centres are in the word.
    let first = (x.start * ppp - 0.5).ceil() / ppp;
    let mut left = snap(pattern + ((x.start - pattern) / PERIOD).floor() * PERIOD);
    while left < end - 1e-3 {
        let rect = Rect::from_min_max(
            pos2(left.max(first), y),
            pos2((left + DOT).min(snap(end + 0.5 / ppp)), y + DOT),
        );
        if rect.width() > 0.0 {
            painter.rect_filled(rect, CornerRadius::ZERO, SPELLING);
        }
        left += PERIOD;
    }
}
