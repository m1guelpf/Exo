//! Colour emoji in text, placed where Chromium puts them.
//!
//! egui draws glyph outlines only, and Apple Color Emoji is all bitmaps. So the font is loaded
//! as a fallback of every family purely for its advances, and draws nothing. [`fit_advances`]
//! then gives each emoji the room Chromium gives it, and [`paint`] paints each emoji cluster's
//! picture (fastframe-emoji, the platform's own artwork) in the font's cell at the start of that
//! room: one em wide, ascent plus descent tall, on the line's baseline, which is the box
//! CoreText draws it in. Pictures are drawn at the cell's size in device pixels and kept per
//! context.

use std::{collections::HashMap, sync::Arc};

use egui::{
    Color32, ColorImage, FontData, Galley, Id, Painter, Pos2, Rect, TextureHandle, TextureOptions,
    epaint::text::Glyph, pos2, vec2,
};
use skrifa::{FontRef, MetadataProvider as _, instance::LocationRef, instance::Size};

use crate::text::{shift_glyph, update_bounds};

/// macOS's emoji font, which Chromium falls back to for emoji.
const SYSTEM_EMOJI: &str = "/System/Library/Fonts/Apple Color Emoji.ttc";
/// egui lays each emoji out with the font's advance (1em) times this, before [`fit_advances`]
/// corrects it: Chromium's advance at the 12px and 16px of most UI text, so the few galleys laid
/// out without [`crate::layout`] are close.
const ADVANCE_SCALE: f32 = 1.25;
/// The egui font name it's registered under.
pub(crate) const FONT_NAME: &str = "AppleColorEmoji";

/// The emoji font's cell, in ems.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Cell {
    ascent: f32,
    descent: f32,
    /// A single emoji's advance (U+1F600's), which every emoji cluster is laid out with.
    advance: f32,
}

/// Maps the system emoji font for egui and measures its cell. `None` where there is none (off
/// macOS), and emoji stay egui's replacement glyph.
pub(crate) fn load() -> Option<(FontData, Cell)> {
    // Mapped: of its 190 MB, only the tables in use are paged in.
    let bytes = crate::fonts::mapped(SYSTEM_EMOJI)?;
    let font = FontRef::from_index(bytes, 0).ok()?;
    let metrics = font.metrics(Size::unscaled(), LocationRef::default());
    let units = f32::from(metrics.units_per_em);
    let grinning = font.charmap().map('\u{1F600}')?;
    let advance = font
        .glyph_metrics(Size::unscaled(), LocationRef::default())
        .advance_width(grinning)?;
    let cell = Cell {
        ascent: metrics.ascent / units,
        descent: -metrics.descent / units,
        advance: advance / units,
    };
    let mut data = FontData::from_static(bytes);
    data.tweak.hinting = Some(false);
    // Only the advance matters: the font draws nothing.
    data.tweak.scale = ADVANCE_SCALE;
    Some((data, cell))
}

fn cell_id() -> Id {
    Id::new("exo_ui::emoji_cell")
}

fn pictures_id() -> Id {
    Id::new("exo_ui::emoji_pictures")
}

/// Pictures uploaded to a context, by cluster and height in device pixels; `None` when no font
/// draws the cluster.
type Pictures = Arc<std::sync::Mutex<HashMap<(String, u32), Option<TextureHandle>>>>;

/// Remembers `cell` for [`paint`].
pub(crate) fn install(ctx: &egui::Context, cell: Cell) {
    ctx.data_mut(|data| data.insert_temp(cell_id(), cell));
}

/// The advance Chromium lays an Apple Color Emoji cluster out with at `size` px: a whole number
/// of px, wider than the font's 1em up to 24px (CoreText's optical advances). 1.25em rounded up
/// to 16px, then `(size + 25) / 2` rounded down, which meets 1em at 24px, then 1em rounded:
/// measured in Chromium from 8px to 48px every 0.05px (in single precision, as Chromium rounds
/// 12.4px's 15.5 down).
fn advance(size: f32) -> f32 {
    (1.25 * size)
        .round()
        .min(((size + 25.0) / 2.0).floor())
        .max(size.round())
}

/// Gives each emoji in `galley` the advance Chromium gives it ([`advance`]) instead of egui's
/// (1.25em): what follows on its row moves by the difference, on whole device pixels.
pub(crate) fn fit_advances(ctx: &egui::Context, galley: &mut Arc<Galley>) {
    let job = galley.job.clone();
    let clusters = clusters(&job.text);
    let Some(cell) = ctx
        .data(|data| data.get_temp::<Cell>(cell_id()))
        .filter(|_| !clusters.is_empty())
    else {
        return;
    };
    let pixels_per_point = ctx.pixels_per_point();
    let correction = |glyph: &Glyph| {
        let size = glyph.advance_width / (cell.advance * ADVANCE_SCALE);
        let correction =
            ((advance(size) - glyph.advance_width) * pixels_per_point).round() / pixels_per_point;
        (glyph.advance_width > 0.0 && cluster_of(&clusters, glyph).is_some()).then_some(correction)
    };
    if galley.rows.iter().all(|placed| {
        placed
            .row
            .glyphs
            .iter()
            .all(|glyph| correction(glyph).is_none_or(|c| c == 0.0))
    }) {
        return;
    }
    let galley = Arc::make_mut(galley);
    for placed in &mut galley.rows {
        if placed
            .glyphs
            .iter()
            .all(|glyph| correction(glyph).is_none())
        {
            continue;
        }
        let row = Arc::make_mut(&mut placed.row);
        let mut offset = 0.0;
        for index in 0..row.glyphs.len() {
            shift_glyph(row, index, vec2(offset, 0.0));
            offset += correction(&row.glyphs[index]).unwrap_or(0.0);
        }
        row.size.x += offset;
        row.visuals.mesh_bounds = row.visuals.mesh.calc_bounds();
    }
    update_bounds(galley);
}

/// The emoji clusters of `text` (the start of each, in bytes, and the cluster).
fn clusters(text: &str) -> Vec<(usize, &str)> {
    // Emoji start at U+00A9; ASCII text has none, and most text is ASCII.
    if text.is_ascii() {
        return Vec::new();
    }
    fastframe_emoji::pieces(text)
        .into_iter()
        .filter_map(|piece| match piece {
            fastframe_emoji::Piece::Emoji(cluster) => {
                Some((cluster.as_ptr() as usize - text.as_ptr() as usize, cluster))
            }
            fastframe_emoji::Piece::Text(_) => None,
        })
        .collect()
}

/// The cluster `glyph` starts, if it's one of `clusters`.
fn cluster_of<'a>(clusters: &[(usize, &'a str)], glyph: &Glyph) -> Option<&'a str> {
    clusters
        .iter()
        .find(|(start, _)| *start == glyph.cluster as usize)
        .map(|(_, cluster)| *cluster)
}

/// Paints the pictures of the emoji in `galley`, with its top left at `origin` and its glyphs on
/// their line's baseline ([`crate::align_baselines`]).
pub(crate) fn paint(painter: &Painter, origin: Pos2, galley: &Galley) {
    let clusters = clusters(&galley.job.text);
    if clusters.is_empty() {
        return;
    }
    let Some(cell) = painter.ctx().data(|data| data.get_temp::<Cell>(cell_id())) else {
        return;
    };
    for row in &galley.rows {
        for glyph in row.glyphs.iter().filter(|glyph| glyph.advance_width > 0.0) {
            let Some(cluster) = cluster_of(&clusters, glyph) else {
                continue;
            };
            let em = glyph.advance_width / (cell.advance * ADVANCE_SCALE);
            let baseline = origin.y + row.pos.y + glyph.pos.y;
            let rect = Rect::from_min_size(
                pos2(
                    origin.x + row.pos.x + glyph.pos.x,
                    baseline - cell.ascent * em,
                ),
                vec2(cell.advance * em, (cell.ascent + cell.descent) * em),
            );
            picture(painter, cluster, rect);
        }
    }
}

/// Paints `cluster`'s picture filling `rect`, drawing it first if this context hasn't yet: an
/// emoji as an image (an `<img>` of one), not a glyph in a run of text.
pub fn picture(painter: &Painter, cluster: &str, rect: Rect) {
    let ctx = painter.ctx();
    let height = (rect.height() * painter.pixels_per_point())
        .round()
        .max(1.0) as u32;
    let pictures = ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Pictures>(pictures_id())
            .clone()
    });
    let mut pictures = pictures
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let texture = pictures
        .entry((cluster.to_owned(), height))
        .or_insert_with(|| {
            // A millisecond or two, once per emoji and size.
            let picture = fastframe_emoji::get().render(cluster, height)?;
            let image = ColorImage::from_rgba_premultiplied(picture.size, &picture.rgba);
            Some(ctx.load_texture(
                format!("emoji-{cluster}-{height}"),
                image,
                TextureOptions::LINEAR,
            ))
        });
    if let Some(texture) = texture {
        painter.image(
            texture.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
}
