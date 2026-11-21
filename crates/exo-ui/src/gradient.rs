//! CSS gradients by hand: a gradient's color along its stops, layers composited, a mesh that
//! paints a color field over a rectangle, and text filled with a gradient. egui interpolates
//! vertex colors linearly, so a grid fine enough for the gradient's curvature passes for
//! Chromium's per-pixel ones.

use std::sync::Arc;

use egui::{Color32, Galley, Mesh, Painter, Pos2, Rect, pos2};

/// A color stop: where it is along the gradient (0 to 1) and its straight RGBA, 0 to 1.
pub type Stop = (f32, [f32; 4]);

/// A CSS gradient's color at `t` along its `stops` (straight RGBA), its ends extended.
/// Interpolated premultiplied, as CSS does, so a stop fading to `transparent` keeps its hue.
pub fn ramp(stops: &[Stop], t: f32) -> [f32; 4] {
    let (first, last) = (stops[0], stops[stops.len() - 1]);
    if t <= first.0 {
        return first.1;
    }
    if t >= last.0 {
        return last.1;
    }
    let after = stops
        .iter()
        .position(|stop| stop.0 >= t)
        .unwrap_or(stops.len() - 1);
    let (from, to) = (stops[after - 1], stops[after]);
    let k = (t - from.0) / (to.0 - from.0);
    let premultiplied = |c: [f32; 4]| [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]];
    let (a, b) = (premultiplied(from.1), premultiplied(to.1));
    let mixed: Vec<f32> = (0..4).map(|i| a[i] + (b[i] - a[i]) * k).collect();
    let alpha = mixed[3];
    if alpha <= 0.0 {
        [0.0; 4]
    } else {
        [mixed[0] / alpha, mixed[1] / alpha, mixed[2] / alpha, alpha]
    }
}

/// `top` composited over the opaque `bottom`.
pub fn over(top: [f32; 4], bottom: [f32; 4]) -> [f32; 4] {
    let a = top[3];
    [
        top[0] * a + bottom[0] * (1.0 - a),
        top[1] * a + bottom[1] * (1.0 - a),
        top[2] * a + bottom[2] * (1.0 - a),
        1.0,
    ]
}

/// An opaque color from straight RGBA (its alpha ignored).
pub fn opaque(color: [f32; 4]) -> Color32 {
    let channel = |value: f32| (value * 255.0).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(channel(color[0]), channel(color[1]), channel(color[2]))
}

/// A `radial-gradient(ellipse {radii} at {center}, …)`'s position along its ray at `point`: 0
/// at the center, 1 on the ellipse.
pub fn radial(point: Pos2, center: Pos2, radii: egui::Vec2) -> f32 {
    let offset = point - center;
    egui::vec2(offset.x / radii.x, offset.y / radii.y).length()
}

/// Paints `color_at` over `rect` as a `columns` × `rows` grid of vertices, clipped by the
/// painter.
pub fn paint_field(
    painter: &Painter,
    rect: Rect,
    columns: u32,
    rows: u32,
    color_at: impl Fn(Pos2) -> Color32,
) {
    let mut mesh = Mesh::default();
    for row in 0..=rows {
        for column in 0..=columns {
            let point = pos2(
                rect.left() + rect.width() * column as f32 / columns as f32,
                rect.top() + rect.height() * row as f32 / rows as f32,
            );
            mesh.colored_vertex(point, color_at(point));
        }
    }
    let index = |row: u32, column: u32| row * (columns + 1) + column;
    for row in 0..rows {
        for column in 0..columns {
            mesh.add_triangle(
                index(row, column),
                index(row, column + 1),
                index(row + 1, column),
            );
            mesh.add_triangle(
                index(row, column + 1),
                index(row + 1, column + 1),
                index(row + 1, column),
            );
        }
    }
    painter.add(mesh);
}

/// `galley`'s glyphs filled with the horizontal gradient `stops` spans over a box `width` wide
/// whose left edge is at the galley's origin (`background: linear-gradient(left, …)` with
/// `background-clip: text`): each glyph's vertices take the color where they sit in the box.
pub fn tint_galley(galley: &Galley, width: f32, stops: &[Stop]) -> Arc<Galley> {
    let mut tinted = galley.clone();
    for placed in &mut tinted.rows {
        let left = placed.pos.x;
        for vertex in &mut Arc::make_mut(&mut placed.row).visuals.mesh.vertices {
            vertex.color = opaque(ramp(stops, (left + vertex.pos.x) / width));
        }
    }
    Arc::new(tinted)
}
