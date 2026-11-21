//! The solid checkbox: a 14px square in a 15px box. Checked, it's filled with the accent
//! (`#aeb1dd`) with 2px corners and a tick in the background color (`#fff`, `#27292d` in Carbon);
//! unchecked, a 13px outline of black (white in Carbon) at a quarter opacity with 1.5px corners.

use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};

use crate::Palette;

/// The box's side.
pub const CHECKBOX_SIZE: f32 = 15.0;

/// Paints a checkbox whose 14px mark has its top left at `at`, snapped to whole px as Chromium
/// paints SVGs.
pub fn paint_checkbox(painter: &Painter, at: Pos2, checked: bool, palette: &Palette) {
    paint_tinted_checkbox(
        painter,
        at,
        checked,
        palette.fill_activated_accent,
        palette.background_base_default,
        palette,
    );
}

/// [`paint_checkbox`] in a colour of its own, checked `fill` with a `tick` (a calendar's
/// checkbox: its swatch and accepted text colour).
pub fn paint_tinted_checkbox(
    painter: &Painter,
    at: Pos2,
    checked: bool,
    fill: Color32,
    tick: Color32,
    palette: &Palette,
) {
    let ink = match palette.theme {
        crate::Theme::Snow => Color32::BLACK,
        crate::Theme::Carbon => Color32::WHITE,
    };
    paint_checkbox_inks(painter, at, checked, [fill, tick, ink]);
}

/// [`paint_checkbox`] with every ink given: checked `fill` and `tick`, and unchecked the
/// `outline`'s colour, drawn at a quarter opacity (`stroke-opacity: .25`), as a component
/// forced into another theme sets them.
pub fn paint_checkbox_inks(
    painter: &Painter,
    at: Pos2,
    checked: bool,
    [fill, tick, outline]: [Color32; 3],
) {
    let at = crate::pixel_snap(Rect::from_min_size(at, vec2(14.0, 14.0))).min;
    if checked {
        painter.rect_filled(
            Rect::from_min_size(at, vec2(14.0, 14.0)),
            CornerRadius::same(2),
            fill,
        );
        // The tick: a 1.2px stroke with round ends, outlined by the path's own 1px stroke.
        let points = [pos2(3.0, 7.167), pos2(5.667, 9.833), pos2(11.0, 4.5)]
            .map(|point| at + point.to_vec2());
        painter.add(Shape::line(points.to_vec(), Stroke::new(2.2, tick)));
        for point in [points[0], points[2]] {
            painter.circle_filled(point, 1.1, tick);
        }
    } else {
        let rect = Rect::from_min_size(at + vec2(0.5, 0.5), vec2(13.0, 13.0));
        // `rx: 1.5`, as near as whole-px corners get.
        painter.rect_stroke(
            rect,
            CornerRadius::same(1),
            Stroke::new(1.0, outline.gamma_multiply(0.25)),
            StrokeKind::Middle,
        );
    }
}

/// The legacy checkbox (the Reminders dialog's): a 15×14 background SVG at `at`, a box with 2px
/// corners in a 1px `#999` line open at its top right (the line stops at 9px across the top and 8px
/// down the right), and checked, a `#6FBEEA` tick through the gap, 2px wide. Both themes draw it
/// the same.
pub fn paint_legacy_checkbox(painter: &Painter, at: Pos2, checked: bool) {
    const LINE: Color32 = Color32::from_rgb(0x99, 0x99, 0x99);
    const TICK: Color32 = Color32::from_rgb(0x6f, 0xbe, 0xea);
    let at = crate::pixel_snap(Rect::from_min_size(at, vec2(15.0, 14.0))).min;
    // The line's middle, 0.5px in from the box's edges: its corners are quarter circles of
    // 1.5px, drawn anticlockwise from `start`.
    let corner = |center: Pos2, start: f32| {
        (1..=4).map(move |step| {
            center + 1.5 * egui::Vec2::angled(start - step as f32 * std::f32::consts::FRAC_PI_8)
        })
    };
    let half_pi = std::f32::consts::FRAC_PI_2;
    let mut points = vec![pos2(if checked { 9.0 } else { 8.875 }, 0.5), pos2(2.0, 0.5)];
    points.extend(corner(pos2(2.0, 2.0), -half_pi));
    points.push(pos2(0.5, 12.0));
    points.extend(corner(pos2(2.0, 12.0), 2.0 * half_pi));
    points.push(pos2(12.0, 13.5));
    points.extend(corner(pos2(12.0, 12.0), half_pi));
    points.push(pos2(13.5, 8.0));
    let points: Vec<Pos2> = points
        .into_iter()
        .map(|point| at + point.to_vec2())
        .collect();
    painter.add(Shape::line(points, Stroke::new(1.0, LINE)));
    if checked {
        let tick = [pos2(4.0, 6.142), pos2(6.858, 9.0), pos2(13.445, 2.413)]
            .map(|point| at + point.to_vec2());
        painter.add(Shape::line(tick.to_vec(), Stroke::new(2.0, TICK)));
    }
}
