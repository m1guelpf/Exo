//! The hovered link's URL at the window's bottom-left: elided to fit, the full URL after 1.5s,
//! fading out over 1.5s once the pointer leaves the link.

use egui::{Color32, CornerRadius, Painter, Rect, pos2, vec2};
use exo_ui::{BoxShadow, Family, Font, TextStyle, motion::CubicBezier};

use crate::format::urls;

/// The URL: 12px/24px in the system font.
const TEXT: TextStyle = TextStyle::new(Font::new(Family::System, 400), 12.0, 24.0);
/// `padding: 0 6px 0 7px`.
const PADDING_LEFT: f32 = 7.0;
const PADDING: f32 = 13.0;
/// The full URL replaces the elided one after this long…
const EXPAND_AFTER: f64 = 1.5;
/// …widening over `transition: width 150ms linear`.
const EXPAND: f32 = 0.15;
/// Hiding fades it out over `transition: opacity 1500ms ease`.
const FADE: f32 = 1.5;

/// The preview's state across frames.
#[derive(Default)]
pub struct LinkPreview {
    /// The URL shown (kept while it fades out).
    url: String,
    /// When the pointer arrived on it, or left it.
    since: f64,
    visible: bool,
}

impl LinkPreview {
    /// Shows `hovered` (or starts fading out without one) and paints the preview at the bottom
    /// left of `window`.
    pub fn show(&mut self, painter: &Painter, window: Rect, hovered: Option<&str>, now: f64) {
        match hovered {
            Some(url) if !self.visible || url != self.url => {
                self.url = url.to_owned();
                self.since = now;
                self.visible = true;
            }
            None if self.visible => {
                self.visible = false;
                self.since = now;
            }
            _ => {}
        }
        if self.url.is_empty() {
            return;
        }
        let elapsed = now - self.since;
        let opacity = if self.visible {
            1.0
        } else {
            1.0 - CubicBezier::EASE.at((elapsed as f32 / FADE).min(1.0))
        };
        if opacity <= 0.0 {
            self.url.clear();
            return;
        }
        let full = urls::shown(&self.url);
        // A third of the window at 56 characters per 420px.
        let max = (0.33 * window.width() * 56.0 / 420.0) as usize;
        let elided = urls::elide(&full, max);
        let measure = |text: &str| {
            TEXT.single_line(painter, text, Color32::PLACEHOLDER, None)
                .size()
                .x
        };
        let expanded = self.visible && elapsed >= EXPAND_AFTER;
        let text = if expanded {
            full.as_str()
        } else {
            elided.as_str()
        };
        let width = if expanded {
            let grow = ((elapsed - EXPAND_AFTER) as f32 / EXPAND).min(1.0);
            measure(&elided) + (measure(&full) - measure(&elided)) * grow + PADDING
        } else {
            measure(&elided) + PADDING
        };
        let width = width.min(window.width());
        let rect = Rect::from_min_size(
            pos2(window.left(), window.bottom() - TEXT.line_height),
            vec2(width, TEXT.line_height),
        );
        let palette = exo_ui::palette(painter.ctx());
        let radius = CornerRadius {
            ne: 6,
            ..CornerRadius::ZERO
        };
        let alpha = |a: f32| Color32::from_black_alpha((a * opacity * 255.0).round() as u8);
        // `0 0 6px 0 rgba(0,0,0,.1), 0 2px 6px 0 rgba(0,0,0,.15), 0 0 .5px 0 rgba(0,0,0,.1)`.
        for shadow in [
            BoxShadow::new([0.0, 0.0], 6.0, 0.0, alpha(0.1)),
            BoxShadow::new([0.0, 2.0], 6.0, 0.0, alpha(0.15)),
            BoxShadow::new([0.0, 0.0], 0.5, 0.0, alpha(0.1)),
        ] {
            shadow.paint(painter, rect, radius);
        }
        painter.rect_filled(
            rect,
            radius,
            palette.background_base_default.gamma_multiply(opacity),
        );
        TEXT.paint_line(
            &painter.with_clip_rect(rect),
            pos2(rect.left() + PADDING_LEFT, rect.top()),
            text,
            palette.text_base_default.gamma_multiply(opacity),
            Some(width - PADDING),
        );
        if self.visible && !expanded {
            painter
                .ctx()
                .request_repaint_after_secs((EXPAND_AFTER - elapsed) as f32);
        } else if !self.visible || elapsed < EXPAND_AFTER + f64::from(EXPAND) {
            painter.ctx().request_repaint();
        }
    }
}
