//! Inline images in the body: what the app is to load, putting images in (Insert Image, a pasted
//! screenshot, pasted HTML with `data:` images), the resizer's sizes, and the keys around a focused
//! image.

use egui::{Key, Modifiers};

use super::Editor;
use crate::{
    image::{self, ImageLoad, ImageSource, State},
    model::{Block, Content, Document, Element, Piece, Pos, Run, Selection},
};

/// A `data:` image pasted in HTML, which becomes one of the draft's inline attachments: its
/// bytes, shown in the body by Content-ID `cid`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upload {
    /// The Content-ID the body now shows it by; also its file name.
    pub cid: String,
    /// The `data:` URL's type.
    pub mime_type: String,
    /// The image.
    pub bytes: Vec<u8>,
}

/// The resizer's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// `Small`: half of Best Fit, or Best Fit if that's under 32px.
    Small,
    /// `Best Fit`.
    BestFit,
}

impl Editor {
    /// The images the body shows that the editor has no pixels for yet, each once, those in the
    /// tables it keeps whole too (once they're laid out): the app loads them and answers each
    /// with [`Self::set_image`]. Each comes with its `<img>`, for what the app decides by its
    /// attributes (a read receipt's size).
    pub fn image_requests(&mut self) -> Vec<(ImageSource, Element)> {
        let scanned = (
            self.revision,
            self.view.pictures.revision,
            self.view.tables.revision,
        );
        if self.view.images_scanned == Some(scanned) {
            return Vec::new();
        }
        self.view.images_scanned = Some(scanned);
        let mut out = Vec::new();
        for run in self.doc.blocks.iter().flat_map(Block::runs) {
            if let Piece::Image(element) = &run.piece
                && let Some(source) = ImageSource::of(element)
                && self.view.pictures.ask(source.key())
            {
                out.push((source, element.clone()));
            }
        }
        // The kept tables' images, once a layout has built them.
        for (source, element) in self.view.tables.images() {
            if self.view.pictures.ask(source.key()) {
                out.push((source.clone(), element.clone()));
            }
        }
        out
    }

    /// What became of loading the image [`ImageSource::key`] `key` names. An image just put in
    /// gets its resolution and Best Fit's width once its pixels come (`_onImageLoaded`), without
    /// an undo step of its own.
    pub fn set_image(&mut self, key: &str, load: ImageLoad) {
        let fit = match &load {
            ImageLoad::Loaded(picture) => key
                .strip_prefix("cid:")
                .filter(|cid| self.fit_on_load.contains(*cid))
                .map(|cid| (cid.to_owned(), picture.retina)),
            _ => None,
        };
        self.view.pictures.set(key, load);
        let Some((cid, retina)) = fit else { return };
        let natural = self.view.pictures.natural(key).unwrap_or_default();
        let mut fitted = false;
        for block in &mut self.doc.blocks {
            let Content::Inline(runs) = &mut block.content else {
                continue;
            };
            for run in runs {
                if let Piece::Image(element) = &mut run.piece
                    && element.attr("src-cid") == Some(cid.as_str())
                {
                    image::fit_on_load(element, natural, retina);
                    fitted = true;
                }
            }
        }
        if fitted {
            self.fit_on_load.remove(&cid);
            self.changed();
        }
    }

    /// Forgets what the remote images loaded to, so they're asked for again: the Images setting
    /// changed, or Show Images was asked for.
    pub fn reload_remote_images(&mut self) {
        self.view
            .pictures
            .forget(|key| key.starts_with("http:") || key.starts_with("https:"));
    }

    /// Puts the picture the draft keeps as inline attachment `cid` at the caret, over the
    /// selection, one undo step: an `<img>` shown by Content-ID, Best Fit wide once it loads.
    pub fn insert_image(&mut self, cid: &str) {
        self.fit_on_load.insert(cid.to_owned());
        let image = Run {
            piece: Piece::Image(image::inserted(cid)),
            marks: Vec::new(),
        };
        self.insert_fragment(
            Document::new(vec![Block::div(Vec::new(), vec![image])]),
            false,
        );
    }

    /// The `data:` images pasted since the last call, now shown by Content-ID: the app keeps
    /// each as an inline attachment of the draft.
    pub fn take_uploads(&mut self) -> Vec<Upload> {
        std::mem::take(&mut self.uploads)
    }

    /// While pasting, each `data:` image in `fragment` is shown by a new Content-ID instead, its
    /// bytes kept for [`Self::take_uploads`]; one that doesn't decode is left empty.
    pub(super) fn take_pasted_images(&mut self, fragment: &mut Document) {
        for block in &mut fragment.blocks {
            let Content::Inline(runs) = &mut block.content else {
                continue;
            };
            for run in runs {
                let Piece::Image(element) = &mut run.piece else {
                    continue;
                };
                let Some(url) = element
                    .attr("src")
                    .filter(|src| {
                        src.get(..5)
                            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:"))
                    })
                    .map(str::to_owned)
                else {
                    continue;
                };
                match exo_mail::decode_data_url(&url) {
                    Some(bytes) => {
                        let cid = element
                            .attr("src-cid")
                            .filter(|cid| !cid.is_empty())
                            .map_or_else(crate::uuid_v4, str::to_owned);
                        image::pasted(element, &cid);
                        let mime_type = url[5..]
                            .split([';', ','])
                            .next()
                            .filter(|mime| !mime.is_empty())
                            .unwrap_or("text/plain")
                            .to_ascii_lowercase();
                        self.fit_on_load.insert(cid.clone());
                        self.uploads.push(Upload {
                            cid,
                            mime_type,
                            bytes,
                        });
                    }
                    None => {
                        element.set_attr("src", "");
                        element.attrs.retain(|(name, _)| name != "src-cid");
                    }
                }
            }
        }
    }

    /// The image the resizer is on: one clicked or arrowed into, selected whole.
    pub fn focused_image(&self) -> Option<Pos> {
        let at = self.image_focus?;
        let selected = self.selection.start() == at
            && self.selection.end() == Pos::new(at.block, at.offset + 1);
        (selected && self.image_element(at).is_some()).then_some(at)
    }

    /// Focuses the image at `at` (selecting it whole) when it can take the resizer: it has
    /// loaded, and the Images setting doesn't block it.
    pub fn focus_image(&mut self, at: Pos) -> bool {
        if !self.can_focus(at) {
            return false;
        }
        self.set_selection(Selection::new(at, Pos::new(at.block, at.offset + 1)));
        self.image_focus = Some(at);
        self.view.toolbar.reset();
        true
    }

    fn can_focus(&self, at: Pos) -> bool {
        self.image_element(at)
            .and_then(ImageSource::of)
            .is_some_and(|source| {
                matches!(
                    self.view.pictures.get(&source.key()),
                    Some(State::Loaded { .. })
                )
            })
    }

    /// The `<img>` at `at`, if one is there.
    pub(crate) fn image_element(&self, at: Pos) -> Option<&Element> {
        let mut start = 0;
        for run in self.doc.blocks.get(at.block)?.runs() {
            if start == at.offset
                && let Piece::Image(element) = &run.piece
            {
                return Some(element);
            }
            start += run.piece.len();
            if start > at.offset {
                break;
            }
        }
        None
    }

    /// Where the selected images are.
    pub(crate) fn selected_images(&self) -> Vec<Pos> {
        let (start, end) = (self.selection.start(), self.selection.end());
        let mut out = Vec::new();
        for (index, block) in self
            .doc
            .blocks
            .iter()
            .enumerate()
            .take(end.block + 1)
            .skip(start.block)
        {
            let mut offset = 0;
            for run in block.runs() {
                let at = Pos::new(index, offset);
                if matches!(run.piece, Piece::Image(_)) && at >= start && at < end {
                    out.push(at);
                }
                offset += run.piece.len();
            }
        }
        out
    }

    /// The natural size of the image at `at`, once it has loaded.
    pub(crate) fn image_natural(&self, at: Pos) -> Option<egui::Vec2> {
        let source = ImageSource::of(self.image_element(at)?)?;
        self.view.pictures.natural(&source.key())
    }

    /// Sets the focused image's width as Small or Best Fit do, one undo step after it.
    pub fn fit_image(&mut self, fit: Fit) {
        let Some(at) = self.focused_image() else {
            return;
        };
        let (Some(natural), Some(element)) = (self.image_natural(at), self.image_element(at))
        else {
            return;
        };
        let best = image::best_fit(natural, image::retina(element));
        let width = match fit {
            Fit::BestFit => best,
            Fit::Small if best < 32.0 => best,
            Fit::Small => best / 2.0,
        };
        self.resize_focused(width);
        self.undo.record(&self.doc, self.selection, false);
    }

    /// A drag of a resize handle starts: the state before it is an undo step.
    pub(crate) fn begin_resize(&mut self) {
        self.undo.record(&self.doc, self.selection, false);
    }

    /// A drag of a resize handle ends: the state it left is one too.
    pub(crate) fn end_resize(&mut self) {
        self.undo.record(&self.doc, self.selection, false);
    }

    /// Makes the focused image `width` wide, its height following its picture's ratio.
    pub(crate) fn resize_focused(&mut self, width: f32) {
        let Some(at) = self.focused_image() else {
            return;
        };
        let Some(natural) = self.image_natural(at) else {
            return;
        };
        let Content::Inline(runs) = &mut self.doc.blocks[at.block].content else {
            return;
        };
        let mut start = 0;
        for run in runs {
            if start == at.offset
                && let Piece::Image(element) = &mut run.piece
            {
                image::resize(element, width, natural);
                break;
            }
            start += run.piece.len();
        }
        self.changed();
    }

    /// The keys around images: ← or → beside an image focuses it; on a focused one, ←/↑/Home/Page
    /// Up put the caret before it and →/↓/End/Page Down after it, and anything else acts on it,
    /// selected (Backspace deletes it, a letter replaces it, ⌘C copies it). Returns whether the key
    /// is spent.
    pub(crate) fn image_key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        if let Some(at) = self.focused_image() {
            let before = matches!(key, Key::ArrowLeft | Key::ArrowUp | Key::Home | Key::PageUp);
            let after = matches!(
                key,
                Key::ArrowRight | Key::ArrowDown | Key::End | Key::PageDown
            );
            if (before || after) && !modifiers.shift {
                let caret = if before {
                    at
                } else {
                    Pos::new(at.block, at.offset + 1)
                };
                self.set_selection(Selection::caret(caret));
                return true;
            }
            self.image_focus = None;
            return false;
        }
        if modifiers.shift || !matches!(key, Key::ArrowLeft | Key::ArrowRight) {
            return false;
        }
        let beside = if key == Key::ArrowLeft {
            let start = self.selection.start();
            start
                .offset
                .checked_sub(1)
                .map(|offset| Pos::new(start.block, offset))
        } else {
            Some(self.selection.end())
        };
        beside.is_some_and(|at| self.focus_image(at))
    }
}
