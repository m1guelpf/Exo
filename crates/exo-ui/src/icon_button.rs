//! Icon buttons: a bare icon that darkens while the pointer is over its (usually larger) hit
//! area, as the header's, the footer's and rows' actions do (Classic's legacy icons go from 30% to
//! 50% over `opacity 150ms ease`), under the pointing hand.

use egui::{Color32, CursorIcon, Id, Image, Rect, Response, Sense, Ui, Vec2};

use crate::motion;

/// An icon that acts as a button.
#[must_use]
pub struct IconButton<'a> {
    icon: &'a str,
    colors: Option<(Color32, Color32)>,
    upside_down: bool,
    lit: bool,
}

impl<'a> IconButton<'a> {
    /// Icon `icon` (a [`paint_icon`] name) in the icon colors.
    pub fn new(icon: &'a str) -> Self {
        Self {
            icon,
            colors: None,
            upside_down: false,
            lit: false,
        }
    }

    /// Shows the icon as hovered while `lit`: the keyboard is on it (`:focus-visible`).
    pub fn lit(self, lit: bool) -> Self {
        Self { lit, ..self }
    }

    /// Turns the icon over when `upside_down` (`transform: rotate(180deg)`), as a caret is for
    /// the other state.
    pub fn upside_down(self, upside_down: bool) -> Self {
        Self {
            upside_down,
            ..self
        }
    }

    /// Uses `rest` and `hover` instead of the palette's `icon_base_default` / `icon_base_hover`.
    pub fn colors(self, rest: Color32, hover: Color32) -> Self {
        Self {
            colors: Some((rest, hover)),
            ..self
        }
    }

    /// Paints the icon in `icon_rect` (snapped to whole px, as Chromium paints SVGs), clickable
    /// anywhere in `hit`.
    pub fn show(self, ui: &Ui, id: Id, hit: Rect, icon_rect: Rect) -> Response {
        let palette = crate::palette(ui.ctx());
        let (rest, hover) = self
            .colors
            .unwrap_or((palette.icon_base_default, palette.icon_base_hover));
        let response = ui
            .interact(hit, id, Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let hovered = motion::transition(
            ui.ctx(),
            id.with("hover"),
            response.hovered() || self.lit,
            0.15,
            motion::CubicBezier::EASE,
        );
        let image = Image::new(crate::icons::icon_uri(ui.ctx(), self.icon))
            .tint(motion::mix(rest, hover, hovered));
        let image = if self.upside_down {
            image.rotate(std::f32::consts::PI, Vec2::splat(0.5))
        } else {
            image
        };
        image.paint_at(
            ui,
            crate::icons::fit(ui.ctx(), self.icon, crate::pixel_snap(icon_rect)),
        );
        response
    }
}
