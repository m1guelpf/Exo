//! The switch: a 30×16 pill whose 12px white knob sits 2px in from the left and bottom,
//! `translateX(14px)` on. Classic greys the track out by a filter while off,
//! `grayscale(93%) brightness(125%)`; the track's colour and filter change over `.4s`, the knob
//! slides over `.1s`, both `ease`.

use egui::{Color32, CornerRadius, Id, Painter, Rect, pos2, vec2};

use crate::motion;

/// The switch's size.
pub const SLIDER_SIZE: egui::Vec2 = vec2(30.0, 16.0);
/// The knob: 12px, 2px in from the left and bottom, `translateX(14px)` on.
const KNOB: f32 = 12.0;
const KNOB_INSET: f32 = 2.0;
const KNOB_TRAVEL: f32 = 14.0;

/// A switch's track colours, off and on, before Classic's filter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderColors {
    /// Off.
    pub off: Color32,
    /// On.
    pub on: Color32,
}

impl SliderColors {
    /// Classic's: `#57adda` both ways.
    pub const CLASSIC: Self = Self {
        off: Color32::from_rgb(0x57, 0xad, 0xda),
        on: Color32::from_rgb(0x57, 0xad, 0xda),
    };
}

/// Paints a switch in `rect`, on or off, in `colors`; at half opacity when `disabled`
/// (`.slider.disabled`).
pub fn paint_slider(
    painter: &Painter,
    rect: Rect,
    on: bool,
    disabled: bool,
    id: Id,
    colors: SliderColors,
) {
    let ctx = painter.ctx();
    let track = motion::transition(ctx, id.with("track"), on, 0.4, motion::CubicBezier::EASE);
    let knob = motion::transition(ctx, id.with("knob"), on, 0.1, motion::CubicBezier::EASE);
    let opacity = if disabled { 0.5 } else { 1.0 };
    painter.rect_filled(
        rect,
        CornerRadius::same(8),
        track_color(track, colors).gamma_multiply(opacity),
    );
    // `.slider::before { background: #fff }`, which the filter leaves white.
    let center = pos2(
        rect.left() + KNOB_INSET + KNOB / 2.0 + KNOB_TRAVEL * knob,
        rect.bottom() - KNOB_INSET - KNOB / 2.0,
    );
    painter.circle_filled(center, KNOB / 2.0, Color32::WHITE.gamma_multiply(opacity));
}

/// The track `on` of the way from off to on: the background between `colors`, under
/// `grayscale(93%) brightness(125%)` off and none on, each filter's amount interpolated as CSS
/// transitions it, applied in sRGB as Chromium applies CSS filters.
fn track_color(on: f32, colors: SliderColors) -> Color32 {
    let background = motion::mix(colors.off, colors.on, on);
    let (grayscale, brightness) = (0.93 * (1.0 - on), 1.25 - 0.25 * on);
    let [red, green, blue] = [background.r(), background.g(), background.b()].map(f32::from);
    // `grayscale(g)` moves each channel `g` of the way to the luminance.
    let luminance = 0.2126 * red + 0.7152 * green + 0.0722 * blue;
    let channel = |value: f32| {
        ((value + (luminance - value) * grayscale) * brightness)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(channel(red), channel(green), channel(blue))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_track_greys_out_when_off() {
        // `#57adda` through `grayscale(93%) brightness(125%)`.
        assert_eq!(
            track_color(0.0, SliderColors::CLASSIC),
            Color32::from_rgb(191, 199, 203)
        );
        assert_eq!(
            track_color(1.0, SliderColors::CLASSIC),
            Color32::from_rgb(0x57, 0xad, 0xda)
        );
    }
}
