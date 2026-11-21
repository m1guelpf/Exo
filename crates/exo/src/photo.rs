//! Full-window photos (the sign-in screen's, the full-screen alerts'), decoded off the UI thread
//! and painted the way CSS paints `background-size: cover`.

use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};

use egui::{Color32, Mesh, Painter, Rect, Shape, TextureHandle, Vec2, pos2, vec2};

/// The largest texture side every backend takes (egui assumes no more before the first frame).
/// The photos are bigger, so they're cut into tiles.
const MAX_SIDE: u32 = 2048;

/// A photo that is being decoded, or has been: a 4096px JPEG takes a moment (65ms in a release
/// build, two seconds in a debug one).
pub struct Photo {
    tiles: Arc<OnceLock<Option<Tiles>>>,
    decoding: Option<std::thread::JoinHandle<()>>,
}

/// The photos decoded or being decoded, by file, whether with a blurred copy and the context
/// whose textures hold them (its texture manager's address, kept alive by the textures): every
/// screen showing one shares it (each tab's page has the day's photo behind Inbox Zero).
type Decoded = Vec<((PathBuf, bool, usize), Weak<OnceLock<Option<Tiles>>>)>;
static DECODED: Mutex<Decoded> = Mutex::new(Vec::new());

/// A photo as textures no bigger than [`MAX_SIDE`].
struct Tiles {
    size: Vec2,
    /// A small blurred copy, for a photo asked for with one.
    blurred: Option<TextureHandle>,
    /// Each tile owns a range of the photo's columns and rows, and its texture holds one more
    /// texel on each inner side, so sampling at the seam blends the same texels as one big
    /// texture would.
    tiles: Vec<Tile>,
}

struct Tile {
    texture: TextureHandle,
    /// The texels it paints, in photo pixels.
    owned: Rect,
    /// The texels its texture holds.
    held: Rect,
}

impl Photo {
    /// Starts decoding the JPEG at `path` into textures of `ctx`, which repaints when it's done.
    pub fn load(ctx: &egui::Context, path: PathBuf) -> Self {
        Self::start(ctx, path, false)
    }

    /// [`Photo::load`], with a blurred copy too ([`Photo::paint_blurred`]).
    pub fn load_blurred(ctx: &egui::Context, path: PathBuf) -> Self {
        Self::start(ctx, path, true)
    }

    fn start(ctx: &egui::Context, path: PathBuf, blurred: bool) -> Self {
        let key = (
            path.clone(),
            blurred,
            Arc::as_ptr(&ctx.tex_manager()) as usize,
        );
        let mut decoded = DECODED
            .lock()
            .expect("the photo cache's lock is never poisoned");
        decoded.retain(|(_, tiles)| tiles.strong_count() > 0);
        if let Some(tiles) = decoded
            .iter()
            .find(|(known, _)| *known == key)
            .and_then(|(_, tiles)| tiles.upgrade())
        {
            return Self {
                tiles,
                decoding: None,
            };
        }
        let tiles = Arc::new(OnceLock::new());
        decoded.push((key, Arc::downgrade(&tiles)));
        drop(decoded);
        let (done, ctx) = (tiles.clone(), ctx.clone());
        let decode = move || {
            // Set whatever happens, for the screens sharing it that wait ([`Photo::wait`]).
            let decoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                decode(&ctx, &path, blurred)
            }));
            let _ = done.set(decoded.ok().flatten());
            ctx.request_repaint();
        };
        let decoding = std::thread::Builder::new()
            .name("exo-photo".into())
            .spawn(decode)
            .inspect_err(|error| log::error!("decoding a photo: {error}"))
            .ok();
        let mut photo = Self { tiles, decoding };
        if photo.decoding.is_none() {
            photo.give_up();
        }
        photo
    }

    /// Blocks until decoding is over, here or for another screen sharing the photo. A screen with
    /// a photo shows only once it can draw all of it.
    pub fn wait(&mut self) {
        if let Some(decoding) = self.decoding.take()
            && decoding.join().is_err()
        {
            self.give_up();
        }
        self.tiles.wait();
    }

    fn give_up(&mut self) {
        let _ = self.tiles.set(None);
    }

    /// Decoding is over, whether or not it worked. For screenshots and tests.
    pub fn is_loaded(&self) -> bool {
        self.tiles.get().is_some()
    }

    /// Paints the photo scaled to cover `rect`, placed by `position`: given how much wider and
    /// taller the scaled photo is than `rect`, how far left and up to shift it (CSS's
    /// `background-position`). Paints nothing until it's decoded.
    pub fn paint(&self, painter: &Painter, rect: Rect, position: impl FnOnce(Vec2) -> Vec2) {
        let Some(Some(photo)) = self.tiles.get() else {
            return;
        };
        let scale = (rect.width() / photo.size.x).max(rect.height() / photo.size.y);
        let origin = rect.min - position(photo.size * scale - rect.size());
        let on_screen = |texels: Rect| {
            Rect::from_min_max(
                origin + texels.min.to_vec2() * scale,
                origin + texels.max.to_vec2() * scale,
            )
        };
        for tile in &photo.tiles {
            let uv = Rect::from_min_max(
                ((tile.owned.min - tile.held.min) / tile.held.size()).to_pos2(),
                ((tile.owned.max - tile.held.min) / tile.held.size()).to_pos2(),
            );
            // A bare mesh: a rect shape would feather its edges, which shows at the seams.
            let mut mesh = Mesh::with_texture(tile.texture.id());
            mesh.add_rect_with_uv(on_screen(tile.owned), uv, Color32::WHITE);
            painter.add(Shape::mesh(mesh));
        }
    }
}

/// The blurred copy's width: an eighth of a 2880px photo, which a window of half its width shows
/// at twice that size, so a blur of `BLUR` texels is CSS's `blur(8px)` there.
const BLURRED_WIDTH: u32 = 360;
const BLUR: f32 = 2.0;

impl Photo {
    /// Paints the blurred copy as [`Photo::paint`] paints the photo (a photo loaded with
    /// [`Photo::load_blurred`]; nothing otherwise).
    pub fn paint_blurred(
        &self,
        painter: &Painter,
        rect: Rect,
        position: impl FnOnce(Vec2) -> Vec2,
    ) {
        let Some(Some(photo)) = self.tiles.get() else {
            return;
        };
        let Some(texture) = &photo.blurred else {
            return;
        };
        let scale = (rect.width() / photo.size.x).max(rect.height() / photo.size.y);
        let origin = rect.min - position(photo.size * scale - rect.size());
        let mut mesh = Mesh::with_texture(texture.id());
        mesh.add_rect_with_uv(
            Rect::from_min_size(origin, photo.size * scale),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        painter
            .with_clip_rect(rect.intersect(painter.clip_rect()))
            .add(Shape::mesh(mesh));
    }
}

fn decode(ctx: &egui::Context, path: &Path, blurred: bool) -> Option<Tiles> {
    let decoded = std::fs::read(path)
        .ok()
        .and_then(|bytes| image::load_from_memory(&bytes).ok());
    let Some(decoded) = decoded else {
        log::warn!("no photo at {}", path.display());
        return None;
    };
    let blurred = blurred.then(|| {
        let small = decoded
            .thumbnail(
                BLURRED_WIDTH,
                BLURRED_WIDTH * decoded.height() / decoded.width().max(1),
            )
            .blur(BLUR)
            .into_rgb8();
        let image = egui::ColorImage::from_rgb(
            [small.width() as usize, small.height() as usize],
            small.as_raw(),
        );
        ctx.load_texture(
            format!("{} blurred", path.display()),
            image,
            egui::TextureOptions::LINEAR,
        )
    });
    let rgb = decoded.into_rgb8();
    let (width, height) = rgb.dimensions();
    let (columns, rows) = (spans(width), spans(height));
    let mut tiles = Vec::new();
    for (x, columns) in columns.iter().enumerate() {
        for (y, rows) in rows.iter().enumerate() {
            let held = (
                columns.start.saturating_sub(1),
                rows.start.saturating_sub(1),
                (columns.end + 1).min(width),
                (rows.end + 1).min(height),
            );
            // Row by row: a copy per row is far quicker than a generic crop in a debug build.
            let mut pixels =
                Vec::with_capacity(((held.2 - held.0) * (held.3 - held.1) * 3) as usize);
            for row in held.1..held.3 {
                let start = ((row * width + held.0) * 3) as usize;
                pixels.extend_from_slice(
                    &rgb.as_raw()[start..start + ((held.2 - held.0) * 3) as usize],
                );
            }
            let image = egui::ColorImage::from_rgb(
                [(held.2 - held.0) as usize, (held.3 - held.1) as usize],
                &pixels,
            );
            // A scaled-down photo is sampled bilinearly, without mipmaps.
            let texture = ctx.load_texture(
                format!("{} {x},{y}", path.display()),
                image,
                egui::TextureOptions::LINEAR,
            );
            let rect = |left: u32, top: u32, right: u32, bottom: u32| {
                Rect::from_min_max(
                    pos2(left as f32, top as f32),
                    pos2(right as f32, bottom as f32),
                )
            };
            tiles.push(Tile {
                texture,
                owned: rect(columns.start, rows.start, columns.end, rows.end),
                held: rect(held.0, held.1, held.2, held.3),
            });
        }
    }
    Some(Tiles {
        size: vec2(width as f32, height as f32),
        blurred,
        tiles,
    })
}

/// `length` pixels cut into even spans that fit a texture with a spare texel on each side.
fn spans(length: u32) -> Vec<Range<u32>> {
    let count = length.div_ceil(MAX_SIDE - 2);
    (0..count)
        .map(|i| length * i / count..length * (i + 1) / count)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_fit_a_texture_with_their_neighbours_texels() {
        for length in [1, 2046, 2047, 3264, 4096, 4896] {
            let spans = spans(length);
            assert_eq!(spans.first().map(|span| span.start), Some(0));
            assert_eq!(spans.last().map(|span| span.end), Some(length));
            assert!(spans.windows(2).all(|pair| pair[0].end == pair[1].start));
            assert!(
                spans
                    .iter()
                    .all(|span| span.end - span.start + 2 <= MAX_SIDE),
                "{length}: {spans:?}"
            );
        }
        assert_eq!(spans(2046).len(), 1);
        assert_eq!(spans(4096).len(), 3);
    }
}
