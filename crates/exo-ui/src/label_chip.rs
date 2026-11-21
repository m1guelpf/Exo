//! Label chips: a label's name, lower-cased, on a colored pill. The color comes from hashing the
//! name, not from Gmail's label color; Classic's eight colors take white text, `#27292d` in Carbon.

use egui::{
    Color32, CornerRadius, Painter, Pos2, Rect, pos2,
    text::{LayoutJob, TextWrapping},
    vec2,
};

use crate::{
    BoxShadow, Family, Font, TextStyle, Theme,
    motion::{self, CubicBezier},
};

/// The name: 10px/12px bold.
const STYLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 10.0, 12.0);
/// `padding: 3px 4px 1px`.
const PADDING_TOP: f32 = 3.0;
const PADDING_X: f32 = 4.0;
/// `max-width: 100px`, padding included.
const MAX_WIDTH: f32 = 100.0;
/// The chip's height: line height plus vertical padding.
pub const LABEL_CHIP_HEIGHT: f32 = 16.0;

/// The eight colors a label can get, by name, the same in both themes.
const HUES: [(&str, &str); 8] = [
    ("default", "#b0bdd1"),
    ("seoul", "#ccc6c3"),
    ("pink", "#f3b9da"),
    ("green", "#bdd1b0"),
    ("purple", "#c5bbda"),
    ("lavender", "#bbc5ef"),
    ("orange", "#efc5ad"),
    ("graypink", "#d3b9c5"),
];

fn hex(css: &str) -> Color32 {
    Color32::from_hex(css).expect("the hue table holds valid colors")
}

/// Spam's chip.
const WATERMELON: &str = "#ef8e8e";

/// The fill and text colors of a chip for label `name` in `theme`.
pub fn label_colors(name: &str, theme: Theme) -> (Color32, Color32) {
    let name = name.to_lowercase();
    // Spam and Trash, names Gmail keeps for its own labels, aren't hashed: watermelon and
    // `default`.
    let fill = match name.as_str() {
        "spam" => WATERMELON,
        "trash" => HUES[0].1,
        _ => {
            // Java's String.hashCode over UTF-16, on the lower-cased name.
            let hash = name.encode_utf16().fold(0i32, |hash, unit| {
                hash.wrapping_mul(31).wrapping_add(i32::from(unit))
            });
            HUES[hash.unsigned_abs() as usize % HUES.len()].1
        }
    };
    let text = match theme {
        Theme::Snow => Color32::WHITE,
        Theme::Carbon => Color32::from_rgb(0x27, 0x29, 0x2d),
    };
    (hex(fill), text)
}

/// A label chip, laid out and ready to paint.
pub struct LabelChip {
    job: LayoutJob,
    fill: Color32,
    width: f32,
}

impl LabelChip {
    /// The chip for label `name`. On a selected row (`selected`) its text turns black at .5; in
    /// Snow the chip keeps its color, in Carbon it turns white.
    pub fn new(painter: &Painter, name: &str, selected: bool) -> Self {
        let theme = crate::theme(painter.ctx());
        let (fill, text) = label_colors(name, theme);
        let (fill, text) = match (selected, theme) {
            (false, _) => (fill, text),
            (true, Theme::Snow) => (fill, Color32::from_black_alpha(128)),
            (true, Theme::Carbon) => (Color32::WHITE, Color32::from_black_alpha(128)),
        };
        let mut job = LayoutJob::single_section(name.to_lowercase(), STYLE.format(text));
        // A name too long for `max-width` overflows the content box, so the box stays at the
        // maximum and the text ends in an ellipsis inside it.
        let width = (painter.layout_job(job.clone()).size().x + 2.0 * PADDING_X).min(MAX_WIDTH);
        job.wrap = TextWrapping::truncate_at_width(MAX_WIDTH - 2.0 * PADDING_X);
        Self { job, fill, width }
    }

    /// The chip's width, padding included.
    pub fn width(&self) -> f32 {
        self.width
    }

    /// Paints the chip with its top left at `top_left`. `hover` (0–1) is the hover shadow's
    /// transition progress (see [`chip_hover`]).
    pub fn paint(self, painter: &Painter, top_left: Pos2, hover: f32) {
        // The box snaps to whole px; the text keeps its fractional position.
        let layout = Rect::from_min_size(top_left, vec2(self.width, LABEL_CHIP_HEIGHT));
        let rect = crate::pixel_snap(layout);
        if hover > 0.0 {
            // A clickable chip's hover: `box-shadow: 0 1px 3px 0 rgba(0,0,0,.3)`.
            BoxShadow::new(
                [0.0, 1.0],
                3.0,
                0.0,
                Color32::from_black_alpha((0.3 * hover * 255.0).round() as u8),
            )
            .paint(painter, rect, CornerRadius::same(2));
        }
        painter.rect_filled(rect, CornerRadius::same(2), self.fill);
        crate::paint_job(
            painter,
            pos2(layout.left() + PADDING_X, layout.top() + PADDING_TOP),
            self.job,
            &STYLE,
        );
    }
}

/// The hover shadow's progress for a chip with `id` (`transition: box-shadow 250ms`, which
/// `:active` cancels).
pub fn chip_hover(ctx: &egui::Context, id: egui::Id, hovered: bool, pressed: bool) -> f32 {
    motion::transition(ctx, id, hovered && !pressed, 0.25, CubicBezier::EASE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_hash_from_the_name() {
        // The mock mailbox's labels and the colors they hash to.
        let hue = |name: &str| label_colors(name, Theme::Snow).0;
        assert_eq!(hue("Hiring"), hex("#d3b9c5"), "graypink");
        assert_eq!(hue("Clients/Calloway"), hex("#bdd1b0"), "green");
        assert_eq!(hue("Newsletters"), hex("#efc5ad"), "orange");
        assert_eq!(hue("Family"), hex("#c5bbda"), "purple");
    }

    #[test]
    fn carbon_keeps_the_fill_and_darkens_the_text() {
        assert_eq!(
            label_colors("Family", Theme::Carbon),
            (hex("#c5bbda"), hex("#27292d"))
        );
        assert_eq!(label_colors("Family", Theme::Snow).1, Color32::WHITE);
    }
}
