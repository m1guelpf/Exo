//! The Quick Tips bar: the status bar's gradient, centred variant, whose line teaches the keys of
//! what has the keyboard, and whose cross turns it off.
//!
//! The line is 12px/24px Exo Sans at `.8` black (`.8` white in Carbon), 1px down, centred in the bar
//! less `2rem` each side; keycaps `#abb7c5` (`#555b64`) 5px either side of words, a group's caps
//! 4px apart; `·` 7px after a word and 2px before the next, at `.5` black (`.4` white); a
//! `"keyword"` 5px after what's before it. The cross is a 12px icon at `.35` (`.5` hovered), `2rem`
//! from the right.
//!
//! The background is an SVG image (`background-size: cover`): a 1180×65 field of `#f2f4fa`
//! (`#2f3237` in Carbon) under three blurred ellipses, drawn here from the same shapes.

use std::sync::Arc;

use egui::{
    Color32, ColorImage, Context, CursorIcon, Id, Rect, Sense, TextureHandle, TextureOptions, Ui,
    Vec2, pos2, vec2,
};

use crate::{
    Family, Font, KeyTooltip, KeycapColors, Keycaps, MESSAGE_BAR_HEIGHT, Placement, TextStyle,
    Theme, paint_icon, palette, rem,
};

const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 24.0);
/// The line's top: `padding-top: 4px`, then `top: 1px`.
const LINE_TOP: f32 = 5.0;
/// A cap's top: its `margin-top: 1px` and the centred bar's `top: 1px`, in the line (measured as
/// the status bar's).
const KEYCAP_TOP: f32 = 6.0;
/// A cap's `max-height`, its label on the line's baseline.
const KEYCAP_HEIGHT: f32 = 19.0;
const KEYCAP_BASELINE: f32 = 14.0;
/// `margin: 0 5px` around a cap or a group of caps; a group's caps 4px apart.
const KEY_MARGIN: f32 = 5.0;
const GROUP_GAP: f32 = 4.0;
/// The `·`'s `margin: 0 2px 0 7px`.
const DOT_BEFORE: f32 = 7.0;
const DOT_AFTER: f32 = 2.0;
/// A keyword's `margin-left: 5px`.
const KEYWORD_BEFORE: f32 = 5.0;
/// The cross: `top: 4px; right: 2rem; margin-right: -5px; padding: 6px` around a 12px icon.
const CLOSE_TOP: f32 = 10.0;
const CLOSE_SIZE: f32 = 12.0;
const CLOSE_PADDING: f32 = 6.0;
const CLOSE_MARGIN: f32 = 5.0;
/// The image's own size, which `cover` scales to the bar's width.
const IMAGE: Vec2 = vec2(1180.0, 65.0);

/// A piece of the bar's line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TipPart<'a> {
    /// Words (`Hit`, `to Mark Done`).
    Text(&'a str),
    /// Caps side by side, one per token: a group when there are several (`["⌘", "K"]`).
    Keys(&'a [&'a str]),
    /// `·` between two tips.
    Dot,
    /// A word to type, in quotes (`"search"`).
    Keyword(&'a str),
}

/// The Quick Tips bar, to paint.
#[derive(Clone, Copy, Debug)]
pub struct QuickTipsBar<'a> {
    /// The line; with none the bar is empty and has no cross (the `default` context).
    pub parts: &'a [TipPart<'a>],
}

impl QuickTipsBar<'_> {
    /// Paints the bar across `window`'s bottom. Returns whether its cross was clicked.
    pub fn show(&self, ui: &mut Ui, window: Rect) -> bool {
        let ctx = ui.ctx().clone();
        let bar = Rect::from_min_max(
            pos2(window.left(), window.bottom() - MESSAGE_BAR_HEIGHT),
            window.max,
        );
        let tokens = palette(&ctx);
        let painter = ui.painter_at(bar);
        // `background-size: cover` from the top left: the image as wide as the bar.
        let texture = background(&ctx, tokens.theme);
        let scale = bar.width() / IMAGE.x;
        let uv = Rect::from_min_max(
            pos2(0.0, 0.0),
            pos2(1.0, (bar.height() / scale / IMAGE.y).min(1.0)),
        );
        painter.image(texture.id(), bar, uv, Color32::WHITE);

        let rem = rem(window.width(), false);
        let (ink, dot) = match tokens.theme {
            Theme::Snow => (
                Color32::from_black_alpha(204),
                Color32::from_black_alpha(128),
            ),
            Theme::Carbon => (
                Color32::from_white_alpha(204),
                Color32::from_white_alpha(102),
            ),
        };
        let colors = KeycapColors::shortcuts_pane(tokens);
        let items: Vec<(f32, Item)> = self
            .parts
            .iter()
            .map(|part| item(&ctx, *part, colors, ink, dot))
            .collect();
        let width: f32 = items.iter().map(|(width, _)| width).sum();
        // The description: `padding: 0 2rem; justify-content: center`, the tip `margin: 0 auto` in
        // it.
        let inner = Rect::from_min_max(
            pos2(bar.left() + 2.0 * rem, bar.top()),
            pos2(bar.right() - 2.0 * rem, bar.bottom()),
        );
        let mut x = inner.center().x - width / 2.0;
        let line = painter.with_clip_rect(inner.intersect(bar));
        for (item_width, item) in items {
            match item {
                Item::Text { text, before, ink } => {
                    TEXT.paint_line(
                        &line,
                        pos2(x + before, bar.top() + LINE_TOP),
                        &text,
                        ink,
                        None,
                    );
                }
                Item::Keys(caps) => {
                    caps.paint(&line, x + KEY_MARGIN - GROUP_GAP, bar.top() + KEYCAP_TOP)
                }
            }
            x += item_width;
        }
        if self.parts.is_empty() {
            return false;
        }
        let right = bar.right() - 2.0 * rem + CLOSE_MARGIN - CLOSE_PADDING;
        let icon = Rect::from_min_size(
            pos2(right - CLOSE_SIZE, bar.top() + CLOSE_TOP),
            Vec2::splat(CLOSE_SIZE),
        );
        let response = ui
            .interact(
                icon.expand(CLOSE_PADDING),
                crate::id(ui.ctx(), "exo_ui::quick_tips_close"),
                Sense::click(),
            )
            .on_hover_cursor(CursorIcon::PointingHand);
        let opacity = crate::motion::transition(
            &ctx,
            crate::id(&ctx, "exo_ui::quick_tips_close_hover"),
            response.hovered(),
            0.15,
            crate::motion::CubicBezier::EASE,
        );
        let base = match tokens.theme {
            Theme::Snow => Color32::BLACK,
            Theme::Carbon => Color32::WHITE,
        };
        paint_icon(
            ui,
            "legacy/close",
            icon,
            base.gamma_multiply(0.35 + 0.15 * opacity),
        );
        KeyTooltip {
            text: "Close Quick Tips",
            keys: &[],
        }
        .show_at(ui, &response, Placement::Above(0.0));
        response.clicked()
    }
}

/// What a part draws: words in their colour (the space before them first), or caps.
enum Item {
    Text {
        text: String,
        before: f32,
        ink: Color32,
    },
    Keys(Keycaps),
}

/// A part laid out, with its margins in its width: words in `ink`, a `·` in `dot`.
fn item(
    ctx: &Context,
    part: TipPart<'_>,
    colors: KeycapColors,
    ink: Color32,
    dot: Color32,
) -> (f32, Item) {
    let text = |text: String, before: f32, after: f32, ink: Color32| {
        (
            before + TEXT.width(ctx, &text) + after,
            Item::Text { text, before, ink },
        )
    };
    match part {
        TipPart::Text(words) => text(words.to_owned(), 0.0, 0.0, ink),
        TipPart::Dot => text("·".to_owned(), DOT_BEFORE, DOT_AFTER, dot),
        TipPart::Keyword(word) => text(format!("\"{word}\""), KEYWORD_BEFORE, 0.0, ink),
        TipPart::Keys(keys) => {
            let caps = Keycaps::new(ctx, keys, colors)
                .gap(GROUP_GAP)
                .height(KEYCAP_HEIGHT)
                .baseline(KEYCAP_BASELINE);
            (
                caps.width() - GROUP_GAP + 2.0 * KEY_MARGIN,
                Item::Keys(caps),
            )
        }
    }
}

/// The background image for `theme`, made once.
fn background(ctx: &Context, theme: Theme) -> TextureHandle {
    let id = Id::new(("exo_ui::quick_tips_background", theme));
    if let Some(texture) = ctx.data(|data| data.get_temp::<TextureHandle>(id)) {
        return texture;
    }
    let image = Arc::new(gradient(theme));
    let texture = ctx.load_texture("quick-tips-background", image, TextureOptions::LINEAR);
    ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
    texture
}

/// One of the image's ellipses: its centre and radii, colour and opacity, the blur's standard
/// deviation (`feGaussianBlur stdDeviation`), and its filter's region, which clips the blur.
struct Blob {
    center: Vec2,
    radii: Vec2,
    color: [f32; 3],
    opacity: f32,
    sigma: f32,
    region: Rect,
}

/// The image, a pixel per unit: the field, then each ellipse blurred over it (in sRGB, as
/// `color-interpolation-filters="sRGB"` asks).
fn gradient(theme: Theme) -> ColorImage {
    let base: [u8; 3] = match theme {
        Theme::Snow => [0xf2, 0xf4, 0xfa],
        Theme::Carbon => [0x2f, 0x32, 0x37],
    };
    let rgb = |hex: u32| {
        [
            ((hex >> 16) & 0xff) as f32,
            ((hex >> 8) & 0xff) as f32,
            (hex & 0xff) as f32,
        ]
    };
    let blobs = [
        Blob {
            center: vec2(-29.0, 39.5),
            radii: vec2(91.0, 86.5),
            color: rgb(0xfa9da9),
            opacity: 0.45,
            sigma: 75.0,
            region: Rect::from_min_size(pos2(-270.0, -197.0), vec2(482.0, 473.0)),
        },
        Blob {
            center: vec2(950.0, 62.0),
            radii: vec2(79.0, 75.0),
            color: rgb(0x72c0ea),
            opacity: 0.6,
            sigma: 125.0,
            region: Rect::from_min_size(pos2(621.0, -263.0), vec2(658.0, 650.0)),
        },
        Blob {
            center: vec2(839.0, -44.5),
            radii: vec2(115.0, 109.5),
            color: rgb(0x894aee),
            opacity: 0.14,
            sigma: 75.0,
            region: Rect::from_min_size(pos2(574.0, -304.0), vec2(530.0, 519.0)),
        },
    ];
    let (width, height) = (IMAGE.x as usize, IMAGE.y as usize);
    let mut pixels: Vec<[f32; 3]> = vec![base.map(f32::from); width * height];
    for blob in &blobs {
        let coverage = blurred(blob);
        for y in 0..height {
            for x in 0..width {
                let at = pos2(x as f32 + 0.5, y as f32 + 0.5);
                if !blob.region.contains(at) {
                    continue;
                }
                let alpha = blob.opacity * coverage.sample(at);
                let pixel = &mut pixels[y * width + x];
                for (channel, color) in pixel.iter_mut().zip(blob.color) {
                    *channel += (color - *channel) * alpha;
                }
            }
        }
    }
    let bytes: Vec<u8> = pixels
        .iter()
        .flat_map(|pixel| pixel.map(|channel| channel.round().clamp(0.0, 255.0) as u8))
        .collect();
    ColorImage::from_rgb([width, height], &bytes)
}

/// An ellipse's coverage blurred, on a coarse grid (the blur is wide enough that a cell of a
/// quarter of its deviation loses nothing).
struct Coverage {
    origin: Vec2,
    step: f32,
    columns: usize,
    rows: usize,
    values: Vec<f32>,
}

impl Coverage {
    /// The coverage at `at`, bilinearly between cells.
    fn sample(&self, at: egui::Pos2) -> f32 {
        let u = ((at.x - self.origin.x) / self.step).clamp(0.0, (self.columns - 1) as f32);
        let v = ((at.y - self.origin.y) / self.step).clamp(0.0, (self.rows - 1) as f32);
        let (x0, y0) = (u.floor() as usize, v.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.columns - 1), (y0 + 1).min(self.rows - 1));
        let (fx, fy) = (u - x0 as f32, v - y0 as f32);
        let value = |x: usize, y: usize| self.values[y * self.columns + x];
        let top = value(x0, y0) + (value(x1, y0) - value(x0, y0)) * fx;
        let bottom = value(x0, y1) + (value(x1, y1) - value(x0, y1)) * fx;
        top + (bottom - top) * fy
    }
}

/// `blob`'s ellipse, filled, then blurred with a separable Gaussian.
fn blurred(blob: &Blob) -> Coverage {
    let step = blob.sigma / 4.0;
    let reach = 3.0 * blob.sigma;
    let origin = blob.center - blob.radii - Vec2::splat(reach);
    let size = 2.0 * (blob.radii + Vec2::splat(reach));
    let (columns, rows) = (
        (size.x / step).ceil() as usize + 1,
        (size.y / step).ceil() as usize + 1,
    );
    let mut values = vec![0.0f32; columns * rows];
    // Each cell's share of the ellipse, from a 4×4 grid of samples in it.
    for y in 0..rows {
        for x in 0..columns {
            let mut inside = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let at = origin
                        + vec2(
                            (x as f32 + (sx as f32 + 0.5) / 4.0 - 0.5) * step,
                            (y as f32 + (sy as f32 + 0.5) / 4.0 - 0.5) * step,
                        );
                    let d = (at - blob.center) / blob.radii;
                    if d.length_sq() <= 1.0 {
                        inside += 1;
                    }
                }
            }
            values[y * columns + x] = inside as f32 / 16.0;
        }
    }
    let radius = (reach / step).ceil() as isize;
    let kernel: Vec<f32> = (-radius..=radius)
        .map(|i| (-((i as f32 * step) / blob.sigma).powi(2) / 2.0).exp())
        .collect();
    let total: f32 = kernel.iter().sum();
    let kernel: Vec<f32> = kernel.iter().map(|weight| weight / total).collect();
    let pass = |values: &[f32], horizontal: bool| -> Vec<f32> {
        let mut out = vec![0.0f32; values.len()];
        for y in 0..rows {
            for x in 0..columns {
                let mut sum = 0.0;
                for (k, weight) in kernel.iter().enumerate() {
                    let offset = k as isize - radius;
                    let (sx, sy) = if horizontal {
                        (x as isize + offset, y as isize)
                    } else {
                        (x as isize, y as isize + offset)
                    };
                    if (0..columns as isize).contains(&sx) && (0..rows as isize).contains(&sy) {
                        sum += weight * values[sy as usize * columns + sx as usize];
                    }
                }
                out[y * columns + x] = sum;
            }
        }
        out
    };
    let values = pass(&pass(&values, true), false);
    Coverage {
        origin,
        step,
        columns,
        rows,
        values,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_field_shows_where_no_ellipse_reaches_and_the_blue_one_tints_its_corner() {
        let image = gradient(Theme::Snow);
        let at = |x: usize, y: usize| image.pixels[y * image.size[0] + x];
        assert_eq!(
            at(450, 30),
            Color32::from_rgb(0xf2, 0xf4, 0xfa),
            "between the ellipses' reach"
        );
        let blue = at(950, 60);
        assert!(
            blue.b() > blue.r() && blue.r() < 0xf2,
            "under the blue ellipse: {blue:?}"
        );
        let pink = at(0, 40);
        assert!(
            pink.r() >= 0xf2 && pink.g() < 0xf4,
            "under the pink one: {pink:?}"
        );
    }

    #[test]
    fn a_blur_keeps_an_ellipses_area() {
        let blob = Blob {
            center: vec2(0.0, 0.0),
            radii: vec2(80.0, 80.0),
            color: [0.0; 3],
            opacity: 1.0,
            sigma: 40.0,
            region: Rect::EVERYTHING,
        };
        let coverage = blurred(&blob);
        let area: f32 = coverage.values.iter().sum::<f32>() * coverage.step * coverage.step;
        let expected = std::f32::consts::PI * 80.0 * 80.0;
        assert!(
            (area - expected).abs() / expected < 0.02,
            "{area} vs {expected}"
        );
        assert!(coverage.sample(pos2(0.0, 0.0)) < 1.0 && coverage.sample(pos2(0.0, 0.0)) > 0.5);
    }
}
