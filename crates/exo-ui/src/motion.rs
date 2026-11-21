//! CSS transitions and their timing curves.
//!
//! A CSS transition advances linearly in time and maps that progress through its timing
//! function. egui's bool animations are linear in time, so [`transition`] gets the progress from
//! egui and applies the curve itself.

use egui::{Color32, Context, Id};

/// A CSS `cubic-bezier()` timing function.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubicBezier {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
}

impl CubicBezier {
    /// `linear`.
    pub const LINEAR: Self = Self::new(0.0, 0.0, 1.0, 1.0);
    /// `ease`, the CSS default.
    pub const EASE: Self = Self::new(0.25, 0.1, 0.25, 1.0);
    /// `ease-in`.
    pub const EASE_IN: Self = Self::new(0.42, 0.0, 1.0, 1.0);
    /// `ease-out`.
    pub const EASE_OUT: Self = Self::new(0.0, 0.0, 0.58, 1.0);
    /// `ease-in-out`.
    pub const EASE_IN_OUT: Self = Self::new(0.42, 0.0, 0.58, 1.0);

    /// `cubic-bezier(x1, y1, x2, y2)`.
    pub const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    /// The eased progress at time progress `t` (0–1). The result can leave 0–1 for curves that
    /// overshoot, as in CSS.
    pub fn at(&self, t: f32) -> f32 {
        if t <= 0.0 || t >= 1.0 {
            return t.clamp(0.0, 1.0);
        }
        bezier(self.y1, self.y2, self.parameter_for(t))
    }

    /// Solves `x(s) = t` for the curve parameter `s`, like browsers do: Newton's method, with
    /// bisection as the fallback when the slope is too flat.
    fn parameter_for(&self, t: f32) -> f32 {
        let mut s = t;
        for _ in 0..8 {
            let error = bezier(self.x1, self.x2, s) - t;
            if error.abs() < 1e-6 {
                return s;
            }
            let slope = bezier_slope(self.x1, self.x2, s);
            if slope.abs() < 1e-6 {
                break;
            }
            s -= error / slope;
        }
        let (mut low, mut high) = (0.0, 1.0);
        s = t;
        for _ in 0..32 {
            if bezier(self.x1, self.x2, s) < t {
                low = s;
            } else {
                high = s;
            }
            s = (low + high) / 2.0;
        }
        s
    }
}

/// One coordinate of a cubic Bézier from (0, 0) to (1, 1) with control points `p1`, `p2`.
fn bezier(p1: f32, p2: f32, s: f32) -> f32 {
    let inverse = 1.0 - s;
    3.0 * inverse * inverse * s * p1 + 3.0 * inverse * s * s * p2 + s * s * s
}

fn bezier_slope(p1: f32, p2: f32, s: f32) -> f32 {
    let inverse = 1.0 - s;
    3.0 * inverse * inverse * p1 + 6.0 * inverse * s * (p2 - p1) + 3.0 * s * s * (1.0 - p2)
}

/// Eased progress (0 = off, 1 = on) of a transition between two states, taking `duration`
/// seconds in both directions.
pub fn transition(ctx: &Context, id: Id, on: bool, duration: f32, curve: CubicBezier) -> f32 {
    curve.at(ctx.animate_bool_with_time(id, on, duration))
}

/// Like [`transition`], but only animates towards "on" and snaps back. That's what CSS does when
/// the `transition` property is declared only in the "on" state, e.g. inside `:hover`.
pub fn transition_in(ctx: &Context, id: Id, on: bool, duration: f32, curve: CubicBezier) -> f32 {
    transition(ctx, id, on, if on { duration } else { 0.0 }, curve)
}

/// A value easing to `target` over `duration` seconds once `delay` seconds have passed since it
/// changed, from wherever it was then (a CSS `transition` with a delay, such as a split tab's
/// count widening: `width 100ms 250ms ease-in-out`). It starts at its first target.
pub fn transition_value(
    ctx: &Context,
    id: Id,
    target: f32,
    duration: f32,
    delay: f32,
    curve: CubicBezier,
) -> f32 {
    #[derive(Clone, Copy)]
    struct Moving {
        from: f32,
        to: f32,
        since: f64,
    }
    let now = ctx.input(|input| input.time);
    let at = |moving: &Moving, time: f64| {
        let progress =
            ((time - moving.since - f64::from(delay)) / f64::from(duration)).clamp(0.0, 1.0) as f32;
        moving.from + (moving.to - moving.from) * curve.at(progress)
    };
    let mut moving = ctx.data_mut(|data| {
        *data.get_temp_mut_or_insert_with(id, || Moving {
            from: target,
            to: target,
            since: now,
        })
    });
    if moving.to != target {
        moving = Moving {
            from: at(&moving, now),
            to: target,
            since: now,
        };
        ctx.data_mut(|data| data.insert_temp(id, moving));
    }
    let end = moving.since + f64::from(delay + duration);
    // A value that hasn't changed since it started has nothing to animate.
    if now < end && moving.from != moving.to {
        ctx.request_repaint_after_secs((moving.since + f64::from(delay) - now).max(0.0) as f32);
    }
    at(&moving, now)
}

/// The color `progress` of the way from `from` to `to`, interpolated like a CSS transition:
/// per channel on premultiplied sRGB, which is how `Color32` already stores colors.
pub fn mix(from: Color32, to: Color32, progress: f32) -> Color32 {
    let (from, to) = (from.to_array(), to.to_array());
    let channel = |i: usize| {
        let (a, b) = (f32::from(from[i]), f32::from(to[i]));
        (a + (b - a) * progress).round().clamp(0.0, 255.0) as u8
    };
    Color32::from_rgba_premultiplied(channel(0), channel(1), channel(2), channel(3))
}

#[cfg(test)]
mod tests {
    use super::CubicBezier;

    #[test]
    fn keywords_match_browser_values() {
        // Reference outputs computed with Chromium's UnitBezier for the same inputs.
        let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
        assert!(close(CubicBezier::EASE.at(0.5), 0.8024));
        assert!(close(CubicBezier::EASE_IN.at(0.5), 0.3153));
        assert!(close(CubicBezier::EASE_OUT.at(0.5), 0.6847));
        assert!(close(CubicBezier::EASE_IN_OUT.at(0.25), 0.1291));
        assert!(close(CubicBezier::LINEAR.at(0.3), 0.3));
    }

    #[test]
    fn overshooting_curves_overshoot() {
        let spring = CubicBezier::new(0.38, 0.87, 0.64, 1.65);
        assert!(spring.at(0.6) > 1.0);
        assert_eq!(spring.at(1.0), 1.0);
    }
}
