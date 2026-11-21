//! Keycaps: a shortcut drawn as one rounded cap per token, `⌘` `shift` `K`.
//!
//! A cap is 18px tall, at least 19px wide, with 5px of side padding and 3px corners; its label is
//! Exo Sans 12px bold whose baseline sits 13px below the cap's top in every placement measured
//! (palette rows, the shortcuts sheet, tooltips). Each cap has a 4px left margin.

use std::borrow::Cow;

use egui::{
    Color32, Context, CornerRadius, Painter, Pos2, Rect, Vec2, os::OperatingSystem, text::LayoutJob,
};

use crate::{Family, Font, Palette, TextStyle, Theme, paint_job, pixel_snap};

const LABEL: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 12.0);
const HEIGHT: f32 = 18.0;
const MIN_WIDTH: f32 = 19.0;
const PADDING_X: f32 = 5.0;
const RADIUS: u8 = 3;
/// Distance from a cap's top to its label's baseline.
const BASELINE: f32 = 13.0;
/// The margin before each cap.
pub const KEYCAP_GAP: f32 = 4.0;
/// A token that's text between caps rather than a cap: `⌘ K → Open Week`'s arrow, with no margin
/// before it.
pub const KEYCAP_ARROW: &str = " → ";
/// How far a cap's label's baseline is below its top: an inline cap's top is this far above the
/// text's baseline.
pub const KEYCAP_BASELINE: f32 = BASELINE;

/// The colors keycaps take in one placement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeycapColors {
    /// The cap.
    pub background: Color32,
    /// The label.
    pub text: Color32,
    /// The label of a `then` between the steps of a sequence, which has no cap.
    pub divider: Color32,
}

impl KeycapColors {
    /// The caps in palette rows and tooltips.
    pub fn base(palette: &Palette) -> Self {
        Self {
            background: palette.background_key_default,
            text: palette.text_light_default,
            divider: palette.text_light_default,
        }
    }

    /// The caps in the shortcuts sheet and the right-pane shortcut panels: `#abb7c5` with white
    /// labels in Snow, `#555b64` with 80% white ones in Carbon.
    pub fn shortcuts_pane(palette: &Palette) -> Self {
        let (background, text, divider) = match palette.theme {
            Theme::Snow => (
                Color32::from_rgb(0xab, 0xb7, 0xc5),
                Color32::WHITE,
                Color32::from_black_alpha(128),
            ),
            Theme::Carbon => (
                Color32::from_rgb(0x55, 0x5b, 0x64),
                Color32::from_white_alpha(204),
                Color32::from_white_alpha(179),
            ),
        };
        Self {
            background,
            text,
            divider,
        }
    }

    /// A search match in the shortcuts sheet.
    pub fn shortcuts_pane_highlighted(palette: &Palette) -> Self {
        Self {
            background: palette.background_key_active,
            text: Color32::WHITE,
            ..Self::shortcuts_pane(palette)
        }
    }
}

/// What a keycap shows for `token`: a single character is upper-cased (except `I`, always `i` so it
/// can't pass for `l`), modifier words are lower-cased, and platform words are spelled the
/// platform's way (`mod` is `⌘` on a Mac, `ctrl` elsewhere).
pub fn keycap_label(token: &str, os: OperatingSystem) -> Cow<'_, str> {
    let lower = token.to_lowercase();
    let mac = os == OperatingSystem::Mac;
    match lower.as_str() {
        "mod" | "⌘" | "cmd" | "command" => Cow::Borrowed(if mac { "⌘" } else { "ctrl" }),
        "alt" | "option" => Cow::Borrowed(if mac { "option" } else { "alt" }),
        "ctrl" | "control" => Cow::Borrowed(if mac { "control" } else { "ctrl" }),
        "ctralt" => Cow::Borrowed(if mac { "control" } else { "alt" }),
        "backspace" => Cow::Borrowed(if mac { "delete" } else { "backspace" }),
        "delete" => Cow::Borrowed(if mac { "forward delete" } else { "delete" }),
        "i" => Cow::Borrowed("i"),
        _ if token.encode_utf16().count() == 1 => Cow::Owned(token.to_uppercase()),
        // Words starting with `shift`, or containing `tab`, `enter` or `esc`, are lower-cased.
        _ if lower.starts_with("shift")
            || ["tab", "enter", "esc"]
                .iter()
                .any(|word| lower.contains(word)) =>
        {
            Cow::Owned(lower)
        }
        _ => Cow::Borrowed(token),
    }
}

/// A row of keycaps, laid out and ready to paint.
pub struct Keycaps {
    caps: Vec<Cap>,
    colors: KeycapColors,
    height: f32,
    /// Before each cap.
    gap: f32,
    baseline: f32,
}

struct Cap {
    label: LayoutJob,
    label_width: f32,
    width: f32,
    divider: bool,
    /// Text between caps ([`KEYCAP_ARROW`]): no cap, no gap before it.
    text: bool,
}

impl Keycaps {
    /// Lays out one cap per token (`["mod", "shift", "k"]`, or `["g", "then", "i"]` for a
    /// sequence).
    pub fn new<S: AsRef<str>>(ctx: &Context, tokens: &[S], colors: KeycapColors) -> Self {
        let os = ctx.os();
        let caps = tokens
            .iter()
            .map(|token| {
                let text = token.as_ref() == KEYCAP_ARROW;
                let label = keycap_label(token.as_ref(), os);
                let divider = label == "then" || text;
                let color = if divider { colors.divider } else { colors.text };
                let mut label = LayoutJob::single_section(label.into_owned(), LABEL.format(color));
                // Chromium's inline box is the text's advance ceiled to its 1/64px layout unit,
                // where egui would round it to 1/32px: a cap's edges round as Chromium's do.
                label.round_output_to_gui = false;
                let label_width = (ctx
                    .fonts_mut(|fonts| fonts.layout_job(label.clone()))
                    .size()
                    .x
                    * 64.0)
                    .ceil()
                    / 64.0;
                // A divider keeps `min-width` but loses its padding; text is as wide as it is.
                let padding = if divider { 0.0 } else { 2.0 * PADDING_X };
                let width = if text {
                    label_width
                } else {
                    (label_width + padding).max(MIN_WIDTH)
                };
                Cap {
                    label,
                    label_width,
                    width,
                    divider,
                    text,
                }
            })
            .collect();
        Self {
            caps,
            colors,
            height: HEIGHT,
            gap: KEYCAP_GAP,
            baseline: BASELINE,
        }
    }

    /// Caps `gap` apart, and `gap` before the first (`margin-left`), instead of 4px.
    #[must_use]
    pub fn gap(self, gap: f32) -> Self {
        Self { gap, ..self }
    }

    /// Caps `height` tall: a cap's padding around a taller line, up to the placement's `max-height`
    /// (19px in a dialog footer's 20px line). The labels stay put.
    #[must_use]
    pub fn height(self, height: f32) -> Self {
        Self { height, ..self }
    }

    /// Labels whose baseline sits `baseline` below the cap's top: a placement whose line is taller
    /// than the cap (the status bar's 24px: 14px).
    #[must_use]
    pub fn baseline(self, baseline: f32) -> Self {
        Self { baseline, ..self }
    }

    fn gap_before(&self, cap: &Cap) -> f32 {
        if cap.text { 0.0 } else { self.gap }
    }

    /// The row's width, including the gap before every cap.
    pub fn width(&self) -> f32 {
        self.caps
            .iter()
            .map(|cap| self.gap_before(cap) + cap.width)
            .sum()
    }

    /// Paints the row with the first cap's gap starting at `left`, caps' tops at `top`.
    pub fn paint(&self, painter: &Painter, left: f32, top: f32) {
        let line_top = top + self.baseline - LABEL.baseline(painter.ctx());
        let mut x = left;
        for cap in &self.caps {
            x += self.gap_before(cap);
            let rect = Rect::from_min_size(Pos2::new(x, top), Vec2::new(cap.width, self.height));
            if !cap.divider {
                // The cap snaps to whole pixels; its label stays where layout put it.
                painter.rect_filled(
                    pixel_snap(rect),
                    CornerRadius::same(RADIUS),
                    self.colors.background,
                );
            }
            let text_left = rect.center().x - cap.label_width / 2.0;
            paint_job(
                painter,
                Pos2::new(text_left, line_top),
                cap.label.clone(),
                &LABEL,
            );
            x += cap.width;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_case_tokens_and_spell_them_per_platform() {
        let mac = OperatingSystem::Mac;
        assert_eq!(keycap_label("e", mac), "E");
        assert_eq!(keycap_label("i", mac), "i");
        assert_eq!(keycap_label("I", mac), "i");
        assert_eq!(keycap_label("Shift", mac), "shift");
        assert_eq!(keycap_label("Enter", mac), "enter");
        assert_eq!(keycap_label("mod", mac), "⌘");
        assert_eq!(keycap_label("mod", OperatingSystem::Windows), "ctrl");
        assert_eq!(keycap_label("ctralt", mac), "control");
        assert_eq!(keycap_label("then", mac), "then");
        assert_eq!(keycap_label("1-9", mac), "1-9");
    }
}
