//! Dragging the selection to move it, as Chromium drags and drops in a `contenteditable` on a Mac:
//! a press on the selection doesn't move the caret; moved more than 4px with the button down, it
//! drags the selected text, its picture following the pointer and a caret where it would land; let
//! go there, it moves (⌥: a copy); let go on the selection itself, or Escape, nothing happens. A
//! press let go without dragging puts the caret there.
//!
//! Out of the editor, the Mac's own drag carries it on ([`exo_ui::text_drag`]), with the same
//! picture: into a field or another editor of the window (a move), another app (a copy), or
//! back. Text dragged over the editor from there shows the caret where it would go, and let go,
//! goes in there.

use egui::{
    Color32, ColorImage, CornerRadius, CursorIcon, Id, Key, LayerId, Modifiers, Order, Painter,
    Pos2, Rect, TextureId, Ui, Vec2,
    epaint::{ClippedPrimitive, Primitive},
    pos2,
};

use crate::{
    editor::Editor,
    model::{Pos, Selection},
};

/// How far the pointer moves with the button down before a press on the selection drags it
/// (`kDragThresholdX`, `kDragThresholdY`).
const THRESHOLD: f32 = 4.0;
/// The drag image's opacity (`kDragImageAlpha`).
const IMAGE_ALPHA: f32 = 0.75;

/// A press on the selection, and the drag it became.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TextDrag {
    /// Where the button went down.
    press: Pos2,
    /// The pointer has gone past the threshold: the text is being dragged.
    dragging: bool,
    /// Where it would land now: off the editor and over the selection, nowhere.
    drop: Option<Pos>,
    /// Out of the editor: the Mac's drag has it, until the app says it's over.
    native: bool,
}

impl TextDrag {
    pub(super) fn new(press: Pos2) -> Self {
        Self {
            press,
            dragging: false,
            drop: None,
            native: false,
        }
    }

    /// Whether the selected text is being dragged inside the editor (Escape is the drag's).
    pub(crate) fn dragging(&self) -> bool {
        self.dragging && !self.native
    }
}

impl Editor {
    /// Follows a press on the selection while the button's down, for editor `id` at `rect`.
    pub(super) fn drag_text(&mut self, ui: &Ui, rect: Rect, id: Id) {
        let Some(mut drag) = self.view.text_drag.take() else {
            return;
        };
        let ctx = ui.ctx();
        if drag.native {
            match exo_ui::text_drag::ended(ctx, id) {
                // Moved to a field or another editor of the window: it goes from here.
                Some(true) => self.delete_dragged(),
                Some(false) => {}
                None => self.view.text_drag = Some(drag),
            }
            return;
        }
        let (down, point, copy) = ui.input(|input| {
            (
                input.pointer.primary_down(),
                input.pointer.latest_pos(),
                input.modifiers.alt,
            )
        });
        if !drag.dragging
            && point.is_some_and(|at| {
                (at.x - drag.press.x).abs() > THRESHOLD || (at.y - drag.press.y).abs() > THRESHOLD
            })
        {
            drag.dragging = true;
        }
        if drag.dragging {
            if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
                ui.ctx().request_repaint();
                return;
            }
            // Out of the editor, the Mac's drag carries it on.
            let visible = rect.intersect(ui.clip_rect());
            if exo_ui::text_drag::native(ctx)
                && let Some(at) = point.filter(|at| !visible.contains(*at))
            {
                let image = self.drag_image(ui, rect.min, at - drag.press);
                exo_ui::text_drag::drag_out(
                    ctx,
                    exo_ui::text_drag::Outgoing {
                        source: id,
                        html: self.selection_markup(),
                        text: self.selected_text(),
                        image,
                    },
                );
                drag.native = true;
                drag.drop = None;
                self.view.text_drag = Some(drag);
                return;
            }
            // Not over the selection itself (`result.IsSelected()`).
            let (start, end) = (self.selection.start(), self.selection.end());
            drag.drop = point
                .filter(|at| rect.contains(*at))
                .map(|at| self.hit(rect.min, at))
                .filter(|pos| !(start <= *pos && *pos <= end));
            if let Some(at) = point {
                ui.scroll_to_rect(
                    Rect::from_center_size(at, Vec2::splat(self.view.body.line())),
                    None,
                );
            }
            ui.ctx().set_cursor_icon(if copy {
                CursorIcon::Copy
            } else {
                CursorIcon::Default
            });
        }
        if down {
            self.view.text_drag = Some(drag);
            return;
        }
        if !drag.dragging {
            self.view.toolbar.reset();
            self.set_selection(Selection::caret(self.hit(rect.min, drag.press)));
        } else if let Some(at) = drag.drop {
            let smart = self.view.word_selection == Some(self.selection);
            self.drop_selection(at, copy, smart);
        }
    }

    /// Text another drag has over the editor (`id`, at `rect`): the caret where it would go (none
    /// over the selection it is, when it's this editor's), and let go there, it goes in.
    pub(super) fn drag_over(&mut self, ui: &Ui, rect: Rect, id: Id) -> bool {
        let ctx = ui.ctx();
        let visible = rect.intersect(ui.clip_rect());
        self.view.drag_over = None;
        if !ui.is_enabled() {
            return false;
        }
        exo_ui::text_drag::accept(ctx, visible);
        let (start, end) = (self.selection.start(), self.selection.end());
        if let Some(over) = exo_ui::text_drag::over(ctx).filter(|over| visible.contains(over.at)) {
            let at = self.hit(rect.min, over.at);
            self.view.drag_over =
                Some(at).filter(|at| over.source != Some(id) || !(start <= *at && *at <= end));
            ui.scroll_to_rect(
                Rect::from_center_size(over.at, Vec2::splat(self.view.body.line())),
                None,
            );
        }
        let Some(dropped) = exo_ui::text_drag::take_drop(ctx, visible, id) else {
            return false;
        };
        let at = self.hit(rect.min, dropped.at);
        self.view.toolbar.reset();
        if dropped.source == Some(id) {
            let smart = self.view.word_selection == Some(self.selection);
            self.drop_selection(at, dropped.copy, smart);
        } else {
            self.drop_text(at, dropped.html.as_deref(), &dropped.text);
        }
        true
    }

    /// The dragged text's picture under the pointer (the selected text without its highlight,
    /// moved with the pointer, at 75%) and the caret where it would land, for an editor at
    /// `origin`; the Mac's drag draws the picture once it's out of the editor.
    pub(super) fn paint_text_drag(&self, ui: &Ui, origin: Pos2, text: Color32) {
        let drag = self.view.text_drag.filter(|drag| drag.dragging);
        if let Some(at) = drag.and_then(|drag| drag.drop).or(self.view.drag_over) {
            ui.painter().rect_filled(
                self.caret_rect_at(origin, ui.pixels_per_point(), at),
                CornerRadius::ZERO,
                text,
            );
        }
        let Some(drag) = drag.filter(|drag| !drag.native) else {
            return;
        };
        let Some(point) = ui.input(|input| input.pointer.latest_pos()) else {
            return;
        };
        let mut painter = ui.ctx().layer_painter(LayerId::new(
            Order::Tooltip,
            ui.id().with("exo_editor_drag"),
        ));
        painter.multiply_opacity(IMAGE_ALPHA);
        self.paint_dragged(&painter, origin, point - drag.press);
    }

    /// Paints the selected text, moved by `delta`, for an editor at `origin`. Returns what it
    /// covers.
    fn paint_dragged(&self, painter: &Painter, origin: Pos2, delta: Vec2) -> Option<Rect> {
        let layout = self.view.layout.as_ref()?;
        let mut covered: Option<Rect> = None;
        for (highlight, _) in self.selection_rects(origin) {
            let clip = highlight.translate(delta).intersect(painter.clip_rect());
            covered = Some(covered.map_or(clip, |covered| covered.union(clip)));
            let painter = painter.with_clip_rect(clip);
            for view in layout.blocks.iter().filter(|view| {
                (origin.y + view.top..=origin.y + view.top + view.height)
                    .contains(&highlight.center().y)
            }) {
                let top_left = origin + Vec2::new(view.left, view.top) + delta;
                let x =
                    (top_left.x * painter.pixels_per_point()).floor() / painter.pixels_per_point();
                if view.rows.is_empty() {
                    exo_ui::paint_text(
                        &painter,
                        pos2(x, top_left.y + view.lines[0].shift),
                        view.galley.clone(),
                        &view.text.style,
                    );
                }
                for (row, line) in view.rows.iter().zip(&view.lines) {
                    exo_ui::paint_text(
                        &painter,
                        pos2(x, top_left.y + line.top + line.shift),
                        row.clone(),
                        &view.text.style,
                    );
                }
            }
        }
        covered.filter(|covered| covered.is_positive())
    }

    /// The picture [`Self::paint_text_drag`] draws, of what shows of the selection (as Chromium's
    /// `DragImageForSelection` is), for the Mac's drag: painted, then drawn into an image as the
    /// GPU would draw it, with where it is in the window.
    fn drag_image(&self, ui: &Ui, origin: Pos2, delta: Vec2) -> Option<(ColorImage, Rect)> {
        let ctx = ui.ctx();
        let layer = LayerId::new(Order::Debug, exo_ui::id(ctx, "exo_editor_drag_image"));
        let mut painter = ctx
            .layer_painter(layer)
            .with_clip_rect(ui.clip_rect().translate(delta));
        painter.multiply_opacity(IMAGE_ALPHA);
        let covered = self.paint_dragged(&painter, origin, delta);
        // Taken out again: it's only for the picture.
        let shapes = ctx
            .graphics_mut(|graphics| graphics.get_mut(layer).map(std::mem::take))
            .map(|list| list.all_entries().cloned().collect())
            .unwrap_or_default();
        let covered = covered?;
        let pixels_per_point = ctx.pixels_per_point();
        let primitives = ctx.tessellate(shapes, pixels_per_point);
        let atlas = ctx.fonts(|fonts| fonts.image());
        Some(rasterize(&primitives, &atlas, covered, pixels_per_point))
    }
}

/// Draws the font texture's triangles of `primitives` into a transparent picture of `bounds`, at
/// `pixels_per_point`, as egui's renderer would: premultiplied vertex colours times the texel
/// nearest each pixel's centre, blended over what's there, each pixel drawn by one triangle of a
/// quad. Emoji and images, drawn from other
/// textures, are left out. Returns the picture and the rect it covers, on whole pixels.
fn rasterize(
    primitives: &[ClippedPrimitive],
    atlas: &ColorImage,
    bounds: Rect,
    pixels_per_point: f32,
) -> (ColorImage, Rect) {
    let (left, top) = (
        (bounds.min.x * pixels_per_point).floor(),
        (bounds.min.y * pixels_per_point).floor(),
    );
    let (width, height) = (
        ((bounds.max.x * pixels_per_point).ceil() - left).max(1.0) as usize,
        ((bounds.max.y * pixels_per_point).ceil() - top).max(1.0) as usize,
    );
    let mut pixels = vec![Color32::TRANSPARENT; width * height];
    let [atlas_width, atlas_height] = atlas.size;
    for primitive in primitives {
        let Primitive::Mesh(mesh) = &primitive.primitive else {
            continue;
        };
        if mesh.texture_id != TextureId::default() {
            continue;
        }
        let clip = Rect::from_min_max(
            primitive.clip_rect.min * pixels_per_point,
            primitive.clip_rect.max * pixels_per_point,
        );
        for triangle in mesh.indices.as_chunks::<3>().0 {
            let [a, b, c] = [0, 1, 2].map(|corner| &mesh.vertices[triangle[corner] as usize]);
            let [pa, pb, pc] =
                [a, b, c].map(|vertex| vertex.pos * pixels_per_point - Vec2::new(left, top));
            let area = (pb - pa).x * (pc - pa).y - (pb - pa).y * (pc - pa).x;
            if area.abs() < f32::EPSILON {
                continue;
            }
            let x_range = (pa
                .x
                .min(pb.x)
                .min(pc.x)
                .max(clip.min.x - left)
                .floor()
                .max(0.0) as usize)
                ..(pa
                    .x
                    .max(pb.x)
                    .max(pc.x)
                    .min(clip.max.x - left)
                    .ceil()
                    .min(width as f32)
                    .max(0.0) as usize);
            let y_range = (pa
                .y
                .min(pb.y)
                .min(pc.y)
                .max(clip.min.y - top)
                .floor()
                .max(0.0) as usize)
                ..(pa
                    .y
                    .max(pb.y)
                    .max(pc.y)
                    .min(clip.max.y - top)
                    .ceil()
                    .min(height as f32)
                    .max(0.0) as usize);
            for y in y_range {
                for x in x_range.clone() {
                    // Just off the centre, so a centre on the edge two triangles share (a
                    // square quad's diagonal) is drawn once.
                    let p = pos2(x as f32 + 0.5 + 1e-3, y as f32 + 0.5 + 2e-3);
                    let edge = |from: Pos2, to: Pos2| {
                        ((to - from).x * (p - from).y - (to - from).y * (p - from).x) / area
                    };
                    let (wa, wb, wc) = (edge(pb, pc), edge(pc, pa), edge(pa, pb));
                    if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                        continue;
                    }
                    let uv = a.uv.to_vec2() * wa + b.uv.to_vec2() * wb + c.uv.to_vec2() * wc;
                    let texel_x = ((uv.x * atlas_width as f32) as usize).min(atlas_width - 1);
                    let texel_y = ((uv.y * atlas_height as f32) as usize).min(atlas_height - 1);
                    let texel = atlas.pixels[texel_y * atlas_width + texel_x].to_array();
                    let color = [a, b, c].map(|vertex| vertex.color.to_array());
                    let source: [f32; 4] = std::array::from_fn(|channel| {
                        let vertex = f32::from(color[0][channel]) * wa
                            + f32::from(color[1][channel]) * wb
                            + f32::from(color[2][channel]) * wc;
                        vertex * f32::from(texel[channel]) / 255.0
                    });
                    let under = pixels[y * width + x].to_array();
                    let over: [u8; 4] = std::array::from_fn(|channel| {
                        (source[channel] + f32::from(under[channel]) * (1.0 - source[3] / 255.0))
                            .round()
                            .clamp(0.0, 255.0) as u8
                    });
                    pixels[y * width + x] =
                        Color32::from_rgba_premultiplied(over[0], over[1], over[2], over[3]);
                }
            }
        }
    }
    let covers = Rect::from_min_size(
        pos2(left, top) / pixels_per_point,
        Vec2::new(width as f32, height as f32) / pixels_per_point,
    );
    (ColorImage::new([width, height], pixels), covers)
}

#[cfg(test)]
mod tests {
    use egui::{Mesh, Rect, epaint::Vertex, pos2, vec2};

    use super::*;

    #[test]
    fn a_textured_quad_is_drawn_with_its_texels_times_its_colour() {
        // A 2×1 atlas: an opaque texel and a clear one.
        let atlas = ColorImage::new([2, 1], vec![Color32::WHITE, Color32::TRANSPARENT]);
        let mut mesh = Mesh::default();
        let red = Color32::from_rgba_premultiplied(128, 0, 0, 128);
        let corners = [
            (pos2(0.0, 0.0), pos2(0.0, 0.0)),
            (pos2(2.0, 0.0), pos2(0.5, 0.0)),
            (pos2(2.0, 2.0), pos2(0.5, 1.0)),
            (pos2(0.0, 2.0), pos2(0.0, 1.0)),
        ];
        mesh.vertices = corners
            .iter()
            .map(|&(pos, uv)| Vertex {
                pos,
                uv,
                color: red,
            })
            .collect();
        mesh.indices = vec![0, 1, 2, 0, 2, 3];
        let primitive = ClippedPrimitive {
            clip_rect: Rect::EVERYTHING,
            primitive: Primitive::Mesh(mesh),
        };
        let (image, covers) = rasterize(
            &[primitive],
            &atlas,
            Rect::from_min_size(pos2(0.0, 0.0), vec2(4.0, 2.0)),
            1.0,
        );
        assert_eq!(image.size, [4, 2]);
        assert_eq!(covers, Rect::from_min_size(pos2(0.0, 0.0), vec2(4.0, 2.0)));
        assert_eq!(image.pixels[0], red, "the opaque texel times the colour");
        assert_eq!(image.pixels[3], Color32::TRANSPARENT, "outside the quad");
    }
}
