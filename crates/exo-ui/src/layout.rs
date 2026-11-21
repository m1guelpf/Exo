//! The responsive root size, and where Chromium actually paints a box.
//!
//! `1rem` steps down as the window narrows. Only horizontal layout uses `rem` (gutters, widths,
//! offsets); font sizes and vertical metrics are fixed px, so nothing else scales.

use egui::Rect;

/// Where Chromium paints a box laid out at `rect`: backgrounds, borders and replaced elements
/// (SVG icons) snap their edges to whole CSS px, rounding halves up, before the device scale
/// applies; text keeps its fractional position. Measured at 2×: an icon laid out at x = 1009.64
/// paints at 1010, a pill at 4.5 paints at 5.
pub fn pixel_snap(rect: Rect) -> Rect {
    let round = |v: f32| (v + 0.5).floor();
    Rect::from_min_max(
        egui::pos2(round(rect.min.x), round(rect.min.y)),
        egui::pos2(round(rect.max.x), round(rect.max.y)),
    )
}

/// The right pane's width (its flex basis) in a window `width` px wide, which the shortcuts sheet
/// takes too: `clamp(260px, 423px, 24.47vw + 33 * (1440px - 100vw) / 480)`.
pub fn right_pane_width(width: f32) -> f32 {
    (0.2447 * width + 33.0 * (1440.0 - width) / 480.0).clamp(260.0, 423.0)
}

/// `1rem` in px for a window `width` px wide, `has_popped_out_draft` while a draft is popped out
/// beside the thread.
pub fn rem(width: f32, has_popped_out_draft: bool) -> f32 {
    // `@media (max-width: N)` matches widths up to and including N.
    const STEPS: [(f32, f32, f32); 5] = [
        (1024.0, 10.0, 10.0),
        (1152.0, 12.0, 10.0),
        (1280.0, 13.5, 10.0),
        (1440.0, 15.0, 11.0),
        (1680.0, 17.0, 13.0),
    ];
    let (_, normal, popped_out) = STEPS
        .into_iter()
        .find(|(max_width, _, _)| width <= *max_width)
        .unwrap_or((f32::INFINITY, 18.0, 18.0));
    if has_popped_out_draft {
        popped_out
    } else {
        normal
    }
}

#[cfg(test)]
mod tests {
    use super::rem;

    #[test]
    fn steps_at_each_breakpoint() {
        assert_eq!(rem(1920.0, false), 18.0);
        assert_eq!(rem(1681.0, false), 18.0);
        assert_eq!(rem(1680.0, false), 17.0);
        assert_eq!(rem(1280.0, false), 13.5, "the default 1280×800 window");
        assert_eq!(rem(1100.0, false), 12.0);
        assert_eq!(rem(780.0, false), 10.0, "the window's minimum width");
    }

    #[test]
    fn popped_out_drafts_shrink_further() {
        assert_eq!(rem(1600.0, true), 13.0);
        assert_eq!(rem(1400.0, true), 11.0);
        assert_eq!(rem(1920.0, true), 18.0);
    }
}
