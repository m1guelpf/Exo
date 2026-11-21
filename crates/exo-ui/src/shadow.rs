//! CSS `box-shadow`s, and the scroll shadow under headers.
//!
//! A CSS shadow is the shape blurred with a Gaussian of σ = blur / 2. egui can only blur an edge
//! linearly, so [`BoxShadow::paint`] stacks three linear blurs whose widths were fitted to the
//! Gaussian's profile (within 1% of the shadow's alpha everywhere) while the color is faint.
//! egui can't blur wider than the box's smaller side, so a shadow blurred wider than that, or in
//! a mostly opaque color, is sampled on a grid instead.

use egui::{
    Color32, CornerRadius, Id, Mesh, Painter, Pos2, Rect, Shape, Vec2, epaint::RectShape, vec2,
};

use crate::motion::{self, CubicBezier};

/// Linear blur widths, in units of σ, whose average best fits a Gaussian edge.
const LAYERS: [f32; 3] = [1.7548, 3.2025, 4.7284];

/// One CSS `box-shadow` (outer, not `inset`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxShadow {
    offset: Vec2,
    blur: f32,
    spread: f32,
    color: Color32,
}

impl BoxShadow {
    /// `box-shadow: {x} {y} {blur} {spread} {color}`.
    pub const fn new(offset: [f32; 2], blur: f32, spread: f32, color: Color32) -> Self {
        Self {
            offset: Vec2::new(offset[0], offset[1]),
            blur,
            spread,
            color,
        }
    }

    /// The same shadow with its color at `opacity`, as a fading shadow is.
    #[must_use]
    pub fn with_opacity(self, opacity: f32) -> Self {
        Self {
            color: self.color.gamma_multiply(opacity),
            ..self
        }
    }

    /// Paints the shadow of a box at `rect` with corners `radius`.
    pub fn paint(&self, painter: &Painter, rect: Rect, radius: CornerRadius) {
        let rect = rect.translate(self.offset).expand(self.spread);
        let radius = radius + CornerRadius::from(self.spread);
        if self.blur <= 0.0 {
            painter.rect_filled(rect, radius, self.color);
            return;
        }
        // Layers composite with "over", so each gets the alpha that makes three of them add up
        // to the shadow's own where they all cover. Along the edges that only approximates their
        // average while the color is faint: an opaque one (Carbon's `#000`, a `#aeb1dd` glow)
        // would come out nearly solid where the blur is half done, so it's sampled instead.
        let sigma = self.blur / 2.0;
        let alpha = f32::from(self.color.a()) / 255.0;
        if LAYERS[2] * sigma > rect.size().min_elem() || alpha > 0.5 {
            painter.add(self.sampled(rect, sigma, None));
            return;
        }
        let layer_alpha = 1.0 - (1.0 - alpha).powf(1.0 / LAYERS.len() as f32);
        let [r, g, b, _] = self.color.to_srgba_unmultiplied();
        let color = Color32::from_rgba_unmultiplied(r, g, b, (layer_alpha * 255.0).round() as u8);
        for width in LAYERS {
            painter.add(RectShape::filled(rect, radius, color).with_blur_width(width * sigma));
        }
    }

    /// Paints the shadow of a box at `rect` only outside it, as CSS clips a `box-shadow` to
    /// outside its border box: for a box whose background doesn't cover it (a glowing button
    /// with no fill). Corners are square.
    pub fn paint_outside(&self, painter: &Painter, rect: Rect) {
        let shadow = rect.translate(self.offset).expand(self.spread);
        painter.add(self.sampled(shadow, (self.blur / 2.0).max(0.25), Some(rect)));
    }

    /// The shadow of `rect` (already offset and spread) as a mesh: a Gaussian-blurred box is the
    /// product of its blurred edges along each axis, sampled every σ/2 out to 3σ, less the box
    /// `hole` (whose edges join the grid). Corners are square, which the blur hides (the dense
    /// ones are glows round 2px corners).
    fn sampled(&self, rect: Rect, sigma: f32, hole: Option<Rect>) -> Shape {
        let edge = |t: f32, lo: f32, hi: f32| {
            0.5 * (erf((hi - t) / (std::f32::consts::SQRT_2 * sigma))
                - erf((lo - t) / (std::f32::consts::SQRT_2 * sigma)))
        };
        let steps = |lo: f32, hi: f32, cuts: [f32; 2]| {
            let count = ((hi - lo + 6.0 * sigma) / (sigma / 2.0)).ceil() as u32;
            let mut steps: Vec<f32> = (0..=count)
                .map(|i| lo - 3.0 * sigma + (hi - lo + 6.0 * sigma) * i as f32 / count as f32)
                .collect();
            if hole.is_some() {
                steps.extend(cuts);
                steps.sort_by(f32::total_cmp);
                steps.dedup();
            }
            steps
        };
        let hole_x = hole.map_or([0.0; 2], |hole| [hole.left(), hole.right()]);
        let hole_y = hole.map_or([0.0; 2], |hole| [hole.top(), hole.bottom()]);
        let xs = steps(rect.left(), rect.right(), hole_x);
        let ys = steps(rect.top(), rect.bottom(), hole_y);
        let mut mesh = Mesh::default();
        for &y in &ys {
            let along_y = edge(y, rect.top(), rect.bottom());
            for &x in &xs {
                mesh.colored_vertex(
                    Pos2::new(x, y),
                    self.color
                        .gamma_multiply(along_y * edge(x, rect.left(), rect.right())),
                );
            }
        }
        let row = xs.len() as u32;
        for j in 0..ys.len() as u32 - 1 {
            for i in 0..row - 1 {
                let center = Pos2::new(
                    (xs[i as usize] + xs[i as usize + 1]) / 2.0,
                    (ys[j as usize] + ys[j as usize + 1]) / 2.0,
                );
                if hole.is_some_and(|hole| hole.contains(center)) {
                    continue;
                }
                let corner = j * row + i;
                mesh.add_triangle(corner, corner + 1, corner + row + 1);
                mesh.add_triangle(corner, corner + row + 1, corner + row);
            }
        }
        Shape::mesh(mesh)
    }
}

/// The error function, within 1.5e-7 (Abramowitz and Stegun 7.1.26).
fn erf(x: f32) -> f32 {
    let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
    let poly = t
        * (0.254_829_6
            + t * (-0.284_496_74 + t * (1.421_413_7 + t * (-1.453_152_1 + t * 1.061_405_4))));
    (1.0 - poly * (-x * x).exp()).copysign(x)
}

/// Carbon's raised-card highlight, `0.15px 0.5px 0 0 rgba(255, 255, 255, 0.1) inset`
/// (an expanded message's), on a card at `rect` with corners `radius` filled with `fill`: white
/// along the top, where the card shifted half a px down doesn't cover it. Its 0.15px on the left is
/// too thin to show.
pub fn paint_carbon_highlight(painter: &Painter, rect: Rect, radius: CornerRadius, fill: Color32) {
    let band = (2.0 * f32::from(radius.nw.max(radius.ne))).max(1.0);
    let top = painter.with_clip_rect(Rect::from_min_max(
        rect.min,
        Pos2::new(rect.right(), rect.top() + band),
    ));
    top.rect_filled(rect, radius, Color32::from_white_alpha(26));
    top.rect_filled(rect.translate(vec2(0.15, 0.5)), radius, fill);
}

/// `linear-gradient(to right, transparent 0, fill 12px)` over `rect`: `fill` fading in across
/// its first 12px, then solid (a chip's tail, a suggestion's remove button).
pub fn paint_fade_in(painter: &Painter, rect: Rect, fill: Color32) {
    if fill == Color32::TRANSPARENT || rect.width() <= 0.0 {
        return;
    }
    let solid = (rect.left() + 12.0).min(rect.right());
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), Color32::TRANSPARENT);
    mesh.colored_vertex(Pos2::new(solid, rect.top()), fill);
    mesh.colored_vertex(Pos2::new(solid, rect.bottom()), fill);
    mesh.colored_vertex(rect.left_bottom(), Color32::TRANSPARENT);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
    painter.rect_filled(
        Rect::from_min_max(Pos2::new(solid, rect.top()), rect.max),
        0.0,
        fill,
    );
}

/// The fade under a header once what's below it has scrolled (the list's header and others like
/// it): an 8px `linear-gradient(0deg, transparent, rgba(0,0,0,.05))` from the header's bottom edge
/// at `top_left`, under a 1px `box-shadow: 0 -1px 0 0 rgba(0,0,0,.05)` line. It fades in and out
/// with `scrolled` over 600ms (`cubic-bezier(0.165, 0.84, 0.44, 1)`).
pub fn scroll_shadow(painter: &Painter, id: Id, top_left: Pos2, width: f32, scrolled: bool) {
    let opacity = motion::transition(
        painter.ctx(),
        id,
        scrolled,
        0.6,
        CubicBezier::new(0.165, 0.84, 0.44, 1.0),
    );
    if opacity <= 0.0 {
        return;
    }
    let rect = Rect::from_min_size(top_left, vec2(width, 8.0));
    let top = Color32::from_black_alpha((0.05 * opacity * 255.0).round() as u8);
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), Color32::TRANSPARENT);
    mesh.colored_vertex(rect.left_bottom(), Color32::TRANSPARENT);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
    painter.rect_filled(
        Rect::from_min_size(rect.min - vec2(0.0, 1.0), vec2(width, 1.0)),
        0.0,
        top,
    );
}

#[cfg(test)]
mod tests {
    use egui::pos2;

    use super::*;

    #[test]
    fn erf_matches_known_values() {
        for (x, expected) in [
            (0.0, 0.0),
            (0.5, 0.520_499_9),
            (1.0, 0.842_700_8),
            (-2.0, -0.995_322_3),
        ] {
            assert!((erf(x) - expected).abs() < 1e-6, "erf({x})");
        }
    }

    #[test]
    fn a_wide_blur_is_sampled_to_half_alpha_at_the_edge() {
        let shadow = BoxShadow::new([0.0, 0.0], 40.0, 0.0, Color32::from_black_alpha(200));
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 30.0));
        let Shape::Mesh(mesh) = shadow.sampled(rect, 20.0, None) else {
            panic!("a mesh")
        };
        let alpha_at = |pos: Pos2| {
            mesh.vertices
                .iter()
                .min_by(|a, b| a.pos.distance(pos).total_cmp(&b.pos.distance(pos)))
                .map(|v| v.color.a())
        };
        // A 30px-tall box under a σ=20 blur peaks well below the color's alpha mid-row.
        let peak = alpha_at(pos2(200.0, 15.0)).expect("vertices");
        assert!((70..=110).contains(&peak), "peak {peak}");
        assert_eq!(alpha_at(pos2(-60.0, 15.0)), Some(0));
    }
}
