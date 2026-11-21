//! A body wider than its column scrolls sideways (`overflow-x: auto`): by a sideways swipe or
//! Shift-wheel over it, or by dragging the thumb of its scrollbar, an 11px strip below the body
//! whose thumb shows only while the pointer moves over the body.

use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Ui, pos2, vec2};
use exo_mail::SCROLLBAR_HEIGHT;
use exo_ui::motion::{self, CubicBezier};

use crate::body::Bodies;

/// The thumb hides this long after the pointer last moved over the body.
const SHOWN_FOR: f64 = 1.0;
/// …unless it stopped this close above the body's bottom edge.
const NEAR_SCROLLBAR: f32 = 200.0;
/// `::-webkit-scrollbar-thumb { border: 2px solid transparent; background-clip: padding-box }`.
const THUMB_BORDER: f32 = 2.0;
/// The shortest thumb (its `width: 7px`).
const MIN_THUMB: f32 = 7.0;
/// The thumb: black at .5 while it shows, eased in and out over `.2s`.
const THUMB_OPACITY: f32 = 0.5;
const FADE: f32 = 0.2;

/// Whether the thumb shows, and when it hides.
#[derive(Clone, Copy, Debug, Default)]
struct Shown {
    shown: bool,
    hide_at: Option<f64>,
}

/// Scrolls message `id`'s body (on screen at `body`, scrollbar included) by the wheel and by
/// dragging its thumb. Call it before the body paints. Returns the part of `body` that's content,
/// which takes the pointer's hover, clicks and selections (the scrollbar doesn't).
pub(super) fn scroll(ui: &Ui, bodies: &mut Bodies, id: &str, body: Rect) -> Rect {
    let overflow = bodies.overflow(id);
    if overflow <= 0.0 {
        return body;
    }
    let scale = ui.ctx().pixels_per_point();
    // A sideways swipe (or the wheel with Shift, which egui turns sideways) scrolls the body; the
    // thread takes the vertical part of the gesture.
    if ui.rect_contains_pointer(body) {
        let delta = ui.input(|input| input.smooth_scroll_delta);
        if delta.x != 0.0 && delta.x.abs() >= delta.y.abs() {
            bodies.set_scroll_x(id, bodies.scroll_x(id) - delta.x, scale);
        }
    }
    let (track, thumb) = scrollbar_geometry(body, overflow, bodies.scroll_x(id));
    let response = ui.interact(thumb, drag_id(ui, id), Sense::drag());
    let from = ui.id().with(("body-scrollbar-drag", id));
    if response.drag_started()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        ui.data_mut(|data| data.insert_temp(from, (pointer.x, bodies.scroll_x(id))));
    }
    if response.dragged()
        && let (Some(pointer), Some((start, scrolled))) = (
            response.interact_pointer_pos(),
            ui.data(|data| data.get_temp::<(f32, f32)>(from)),
        )
    {
        let per_px = overflow / (track.width() - thumb.width()).max(1.0);
        bodies.set_scroll_x(id, scrolled + (pointer.x - start) * per_px, scale);
    }
    Rect::from_min_max(
        body.min,
        pos2(body.right(), body.bottom() - SCROLLBAR_HEIGHT),
    )
}

/// Paints message `id`'s scrollbar thumb over its body (on screen at `body`), when the body
/// scrolls sideways and the thumb shows.
pub(super) fn scrollbar(ui: &Ui, bodies: &Bodies, id: &str, body: Rect) {
    let overflow = bodies.overflow(id);
    if overflow <= 0.0 {
        return;
    }
    let now = ui.input(|input| input.time);
    let key = ui.id().with(("body-scrollbar", id));
    let mut shown: Shown = ui.data(|data| data.get_temp(key)).unwrap_or_default();
    let (pointer, moved) = ui.input(|input| {
        (
            input.pointer.hover_pos(),
            input.pointer.delta() != egui::Vec2::ZERO,
        )
    });
    if moved {
        shown = match pointer.filter(|at| body.contains(*at)) {
            Some(at) => Shown {
                shown: true,
                hide_at: (!near_scrollbar(at, body)).then_some(now + SHOWN_FOR),
            },
            // Moving anywhere else hides it at once.
            None => Shown::default(),
        };
    }
    if let Some(hide_at) = shown.hide_at {
        if now >= hide_at {
            shown = Shown::default();
        } else {
            ui.ctx().request_repaint_after_secs((hide_at - now) as f32);
        }
    }
    ui.data_mut(|data| data.insert_temp(key, shown));
    let (_, thumb) = scrollbar_geometry(body, overflow, bodies.scroll_x(id));
    // `::-webkit-scrollbar-thumb:hover` shows it too, as does holding it.
    let held = ui.ctx().is_being_dragged(drag_id(ui, id));
    let on_thumb = pointer.is_some_and(|at| thumb.contains(at));
    let opacity = motion::transition(
        ui.ctx(),
        key.with("opacity"),
        shown.shown || on_thumb || held,
        FADE,
        CubicBezier::EASE_IN_OUT,
    );
    if opacity > 0.0 {
        // `border-radius: 100px`: a capsule.
        let alpha = (THUMB_OPACITY * opacity * 255.0).round() as u8;
        ui.painter().with_clip_rect(ui.clip_rect()).rect_filled(
            thumb.shrink(THUMB_BORDER),
            CornerRadius::same(u8::MAX),
            Color32::from_black_alpha(alpha),
        );
    }
}

fn drag_id(ui: &Ui, id: &str) -> egui::Id {
    ui.id().with(("body-scrollbar-thumb", id))
}

/// The scrollbar's track (the 11px under the content) and its thumb, for a body `overflow` px
/// wider than its column and scrolled `scroll_x` px.
fn scrollbar_geometry(body: Rect, overflow: f32, scroll_x: f32) -> (Rect, Rect) {
    let track = Rect::from_min_max(
        pos2(body.left(), body.bottom() - SCROLLBAR_HEIGHT),
        body.max,
    );
    let column = body.width();
    let length = (track.width() * column / (column + overflow))
        .round()
        .max(MIN_THUMB);
    let left = track.left() + scroll_x / overflow * (track.width() - length);
    (
        track,
        Rect::from_min_size(pos2(left, track.top()), vec2(length, SCROLLBAR_HEIGHT)),
    )
}

/// Within 200px above the body's bottom edge.
fn near_scrollbar(at: Pos2, body: Rect) -> bool {
    at.y <= body.bottom()
        && at.y > body.bottom() - NEAR_SCROLLBAR
        && (body.left()..=body.right()).contains(&at.x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thumb_spans_the_part_in_view() {
        // A 1200px body in a 600px column, scrolled halfway: half the track, in its middle.
        let body = Rect::from_min_size(pos2(100.0, 0.0), vec2(600.0, 311.0));
        let (track, thumb) = scrollbar_geometry(body, 600.0, 300.0);
        assert_eq!(
            track,
            Rect::from_min_max(pos2(100.0, 300.0), pos2(700.0, 311.0))
        );
        assert_eq!(
            thumb,
            Rect::from_min_size(pos2(250.0, 300.0), vec2(300.0, 11.0))
        );
        let (_, end) = scrollbar_geometry(body, 600.0, 600.0);
        assert_eq!(end.right(), track.right());
        assert!(
            near_scrollbar(pos2(300.0, 200.0), body) && !near_scrollbar(pos2(300.0, 100.0), body)
        );
    }
}
