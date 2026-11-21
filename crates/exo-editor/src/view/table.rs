//! The tables the editor keeps whole (a pasted table, a table-built signature, a revealed
//! quote's newsletter): drawn by exo-mail's Blitz body renderer, as the thread view draws a
//! message, in the block that holds one. Its stylesheet is the editor's (the message body's), its
//! colours tagged as the editor's content is ([`exo_mail::render_fragment`]).
//!
//! Its images load as the body's do: each is one of [`crate::Editor::image_requests`], answered
//! with [`crate::Editor::set_image`], whose file the table's document decodes off the UI thread.
//! The document asks for its `<img>`s by their place in the table (`cid:exo-table-image-{n}`), so
//! what it asks for maps back to the `<img>` whatever Blitz makes of the URL; what its CSS or a
//! `background` asks for loads by URL, as a remote or inline image. Remote images the Images
//! setting blocks show as empty boxes: the table is built again with them blocked. A tall
//! table is painted in bands, those on screen.

use std::{
    collections::HashMap,
    hash::{Hash as _, Hasher as _},
    sync::{Arc, Mutex},
};

use egui::{ColorImage, TextureHandle, TextureOptions, Vec2, vec2};
use exo_mail::{BodyView, Done, Fetch, Region, ViewOptions};
use exo_ui::Theme;

use crate::{
    html,
    image::{ImageSource, Pictures, State},
    model::Element,
};

/// What a table's document loads an image by: this and its place in [`Table::images`].
const IMAGE_URL: &str = "cid:exo-table-image-";

thread_local! {
    /// The message bodies' fonts, loaded the first time a table is drawn (the editor lives on the
    /// UI thread).
    static FONTS: exo_mail::Fonts = exo_mail::Fonts::load();
}

/// Which table a block holds, by its markup.
pub(crate) fn key(html: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    html.hash(&mut hasher);
    hasher.finish()
}

/// The height of a painted band, in CSS px.
const BAND: f32 = 1024.0;

/// What a table's document has asked to load and not had: an `<img>` by its place in
/// [`Table::images`], else (CSS, `background`) the image the URL names.
#[derive(Default)]
struct Requests(Mutex<Vec<(Asked, Done)>>);

enum Asked {
    Image(usize),
    Url(ImageSource),
}

impl Fetch for Requests {
    fn fetch(&self, url: &str, done: Done) {
        let asked = match url
            .strip_prefix(IMAGE_URL)
            .and_then(|index| index.parse().ok())
        {
            Some(index) => Asked::Image(index),
            None => match ImageSource::of(&Element::with("img", &[("src", url)])) {
                Some(source @ (ImageSource::Remote(_) | ImageSource::Cid(_))) => Asked::Url(source),
                _ => return,
            },
        };
        if let Ok(mut pending) = self.0.lock() {
            pending.push((asked, done));
        }
    }
}

/// What a band was painted at: the table's generation, and the width, phase and scale's bits.
type Painted = (u64, u32, u32, u32);

/// One table, laid out and painted.
struct Table {
    view: BodyView,
    requests: Arc<Requests>,
    /// Its `<img>`s' sources and elements, in document order, then the images its CSS asked for
    /// (with an empty `<img>`).
    images: Vec<(ImageSource, Element)>,
    theme: Theme,
    /// Built with its remote images blocked.
    blocked: bool,
    /// The width and scale it was last laid out at, and its height there.
    laid_out: Option<(f32, f32, f32)>,
    /// Bumped when what it shows changes (a layout, an image arriving).
    generation: u64,
    /// Its painted bands by index.
    bands: HashMap<u32, (TextureHandle, Painted)>,
    /// Laid out in the editor's latest layout.
    used: bool,
}

impl Table {
    fn new(html: &str, theme: Theme, blocked: bool) -> Self {
        let mut images = Vec::new();
        let markup = html::map_images(html, |image| {
            let Some(source) = ImageSource::of(image) else {
                return;
            };
            images.push((source.clone(), image.clone()));
            // A blocked one keeps its URL, for the view to draw blocked; `data:` loads itself.
            match source {
                ImageSource::Remote(_) if blocked => {}
                ImageSource::Data(_) => {}
                ImageSource::Cid(_) | ImageSource::Remote(_) => {
                    image.set_attr("src", &format!("{IMAGE_URL}{}", images.len() - 1));
                    image.attrs.retain(|(name, _)| name != "srcset");
                }
            }
        });
        let requests = Arc::new(Requests::default());
        let options = ViewOptions {
            theme: match theme {
                Theme::Snow => exo_mail::Theme::Light,
                Theme::Carbon => exo_mail::Theme::Dark,
            },
            block_remote_content: blocked,
            ..ViewOptions::default()
        };
        let render = exo_mail::render_fragment(&markup);
        let view = FONTS.with(|fonts| {
            BodyView::new(
                &render,
                options,
                fonts,
                Some(requests.clone() as Arc<dyn Fetch>),
            )
        });
        Self {
            view,
            requests,
            images,
            theme,
            blocked,
            laid_out: None,
            generation: 0,
            bands: HashMap::new(),
            used: true,
        }
    }

    /// Whether the Images setting blocks one of its images, as last answered.
    fn any_blocked(&self, pictures: &Pictures) -> bool {
        self.images
            .iter()
            .any(|(source, _)| matches!(pictures.get(&source.key()), Some(State::Blocked)))
    }
}

/// Every kept table an editor shows, by [`key`].
#[derive(Default)]
pub(crate) struct Tables {
    tables: HashMap<u64, Table>,
    /// Bumped whenever a table is built or what one shows changes, so the layout looks again.
    pub(crate) revision: u64,
}

impl Tables {
    /// The [`key`] of the table `html` makes, and its box in a block `width` wide at `scale`, in
    /// `theme`, built and laid out as needed.
    pub(crate) fn lay_out(
        &mut self,
        html: &str,
        width: f32,
        scale: f32,
        theme: Theme,
        pictures: &Pictures,
    ) -> (u64, Vec2) {
        let key = key(html);
        let stale = self.tables.get(&key).is_none_or(|table| {
            table.theme != theme || table.blocked != table.any_blocked(pictures)
        });
        if stale {
            let blocked = self
                .tables
                .get(&key)
                .is_some_and(|table| table.any_blocked(pictures));
            self.tables.insert(key, Table::new(html, theme, blocked));
            self.revision += 1;
        }
        let table = self.tables.get_mut(&key).expect("built above");
        table.used = true;
        let height = match table.laid_out {
            Some((w, s, height)) if w == width && s == scale => height,
            _ => {
                let height = table.view.layout(width, scale);
                table.laid_out = Some((width, scale, height));
                table.generation += 1;
                height
            }
        };
        (key, vec2(width, height))
    }

    /// Marks every table unused, before a layout.
    pub(crate) fn begin_layout(&mut self) {
        for table in self.tables.values_mut() {
            table.used = false;
        }
    }

    /// Forgets the tables the latest layout didn't show.
    pub(crate) fn end_layout(&mut self) {
        self.tables.retain(|_, table| table.used);
    }

    /// Every image the tables show, for the app to load.
    pub(crate) fn images(&self) -> impl Iterator<Item = &(ImageSource, Element)> {
        self.tables.values().flat_map(|table| table.images.iter())
    }

    /// Hands the tables' documents the images that have loaded (decoded on a thread of their
    /// own, which repaints `ctx` once done), and applies what they decoded. Returns whether a
    /// table changed.
    pub(crate) fn update(&mut self, pictures: &Pictures, ctx: &egui::Context) -> bool {
        let mut changed = false;
        for table in self.tables.values_mut() {
            let mut ready = Vec::new();
            if let Ok(mut pending) = table.requests.0.lock() {
                for (asked, done) in std::mem::take(&mut *pending) {
                    let source = match &asked {
                        Asked::Image(index) => {
                            table.images.get(*index).map(|(source, _)| source.clone())
                        }
                        Asked::Url(source) => {
                            if !table.images.iter().any(|(known, _)| known == source) {
                                // A new image for the app to load.
                                table.images.push((source.clone(), Element::new("img")));
                                changed = true;
                            }
                            Some(source.clone())
                        }
                    };
                    match source.and_then(|source| pictures.get(&source.key())) {
                        Some(State::Loaded { file, .. }) => ready.push((done, file.clone())),
                        None | Some(State::Pending) => pending.push((asked, done)),
                        // Blocked (the table is built again) or broken: it never loads.
                        Some(State::Blocked | State::Broken) => {}
                    }
                }
            }
            if !ready.is_empty() {
                let ctx = ctx.clone();
                let spawned = std::thread::Builder::new()
                    .name("exo-editor-table-images".into())
                    .spawn(move || {
                        for (done, file) in ready {
                            done.finish(file.to_vec());
                        }
                        ctx.request_repaint();
                    });
                if let Err(error) = spawned {
                    log::warn!("decoding a table's images: {error}");
                }
            }
            if table.view.poll() {
                table.laid_out = table
                    .laid_out
                    .map(|(width, scale, _)| (width, scale, table.view.height()));
                table.generation += 1;
                changed = true;
            }
        }
        if changed {
            self.revision += 1;
        }
        changed
    }

    /// Paints the bands of table `key` (`width` wide, `phase` px right of the whole pixel its
    /// textures go on) that `shown` (CSS px from its top) reaches and aren't up to date; those
    /// further than a band from it are dropped.
    pub(crate) fn paint(
        &mut self,
        key: u64,
        width: f32,
        phase: f32,
        scale: f32,
        shown: std::ops::Range<f32>,
        ctx: &egui::Context,
    ) {
        let Some(table) = self.tables.get_mut(&key) else {
            return;
        };
        let Some((_, _, height)) = table.laid_out else {
            return;
        };
        let painted = (
            table.generation,
            width.to_bits(),
            phase.to_bits(),
            scale.to_bits(),
        );
        let band = |y: f32| (y.max(0.0) / BAND) as u32;
        let visible = band(shown.start)..band(shown.end.min(height - 0.5)) + 1;
        table
            .bands
            .retain(|index, _| *index + 1 >= visible.start && *index <= visible.end);
        for index in visible.filter(|index| (*index as f32) * BAND < height) {
            if table
                .bands
                .get(&index)
                .is_some_and(|(_, at)| *at == painted)
            {
                continue;
            }
            let top = index as f32 * BAND;
            let image = table.view.paint(
                Region {
                    top,
                    height: BAND.min(height - top),
                    left: 0.0,
                    width,
                    phase: [phase, 0.0],
                },
                [0; 4],
            );
            let pixels = ColorImage::from_rgba_premultiplied(
                [image.width as usize, image.height as usize],
                &image.rgba,
            );
            match table.bands.get_mut(&index) {
                Some((texture, at)) => {
                    texture.set(pixels, TextureOptions::NEAREST);
                    *at = painted;
                }
                None => {
                    let texture = ctx.load_texture(
                        format!("exo_editor::table {key:x} {index}"),
                        pixels,
                        TextureOptions::NEAREST,
                    );
                    table.bands.insert(index, (texture, painted));
                }
            }
        }
    }

    /// Table `key`'s painted bands: each one's top (CSS px from the table's) and texture.
    pub(crate) fn bands(&self, key: u64) -> impl Iterator<Item = (f32, &TextureHandle)> {
        self.tables.get(&key).into_iter().flat_map(|table| {
            table
                .bands
                .iter()
                .map(|(index, (texture, _))| (*index as f32 * BAND, texture))
        })
    }
}
