//! The editor widget: lays the document out in Chromium's boxes with exo-ui's text, paints it,
//! and turns input into [`Editor`] commands.
//!
//! Layout follows the editor's CSS: every line is Exo Sans Email 14px in a 24px line box; lists
//! indent 40px and keep 16px above and below the outermost list (margins collapse as CSS collapses
//! them); quotes pad 1.25em with a 1px bar; indents are 40px.
//!
//! An inline image is laid out as one wide glyph: a space in its own section, sized so its
//! advance is the image's width, so egui wraps lines around it as it wraps words. Its height
//! then grows its line box as Chromium's does: the image sits on the baseline, and the line is
//! as tall as the image above it and the text's leading and descent below. A table the
//! editor keeps whole is such a glyph too, as wide as its block and as tall as Blitz lays the
//! table out ([`table`]).

mod dates;
mod drag;
mod pill;
mod resizer;
mod table;

use std::sync::Arc;

use egui::{
    Color32, CornerRadius, Event, EventFilter, Galley, Id, ImeEvent, Key, Modifiers, Pos2, Rect,
    Response, Sense, Stroke, TextFormat, Ui, Vec2, pos2,
    text::{CCursor, CharIndex, LayoutJob, TextWrapping},
    vec2,
};
use exo_ui::{Family, Font, TextStyle, Theme};

use crate::{
    clipboard::{Clipboard, SystemClipboard},
    editor::{Color, Editor},
    grapheme, html,
    image::{ImageBox, ImageSource, Look, Pictures, State},
    model::{Block, ContainerKind, Content, Document, Piece, Pos, Selection},
    spelling::Misspelling,
    toolbar,
    trigger::Trigger,
    undo::UndoMark,
};

/// `ol, ul { margin: 16px 0 }`; nested lists have none.
const LIST_MARGIN: f32 = 16.0;
/// The UA's `padding-inline-start` for lists.
const LIST_INDENT: f32 = 40.0;
/// `blockquote { padding: 0 0 0 1.25em !important }` at 14px.
const QUOTE_INDENT: f32 = 17.5;
/// `margin:0 0 0 40px` of an indent.
const INDENT: f32 = 40.0;
/// `hr { margin: 10px 0; border-top: 1px; border-bottom: 1px transparent }`.
const RULE_HEIGHT: f32 = 22.0;
/// Links in user content (`a[href] { color: #54acdc !important }`).
const LINK: Color32 = Color32::from_rgb(0x54, 0xac, 0xdc);
/// Chromium's caret: 1px wide.
const CARET_WIDTH: f32 = 1.0;
/// Exo Sans Email's x-height (OS/2 `sxHeight` 1028 of 2000), which a blocked image's
/// `vertical-align: middle` centres on.
const X_HEIGHT: f32 = 0.514;

/// The size the body's text is set in, and its placeholder's opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyText {
    /// The text's style: its size and its line box.
    pub style: TextStyle,
    /// The font's content area in the line box (where the caret and a selection's text box
    /// are): its top, from the line's top, and its height.
    pub content_top: f32,
    /// See [`BodyText::content_top`].
    pub content_height: f32,
    /// The placeholder's opacity over the text colour (`::before`).
    pub placeholder_opacity: f32,
}

impl BodyText {
    /// Compose's: 14px, forced 24px lines, the content area the ascent and descent at 14px
    /// (12 + 5) centred with the odd pixel of leading below (`y = line top + 3`, `height 17`),
    /// the placeholder at `.45`.
    pub const COMPOSE: Self = Self {
        style: TextStyle::new(Font::new(Family::SansEmail, 400), 14.0, 24.0),
        content_top: 3.0,
        content_height: 17.0,
        placeholder_opacity: 0.45,
    };

    /// 12px in 20px lines, as the Out of Office Reply's message sets it: the content area 2px
    /// down and 15px tall, the placeholder black at `.55`.
    pub const SMALL: Self = Self {
        style: TextStyle::new(Font::new(Family::SansEmail, 400), 12.0, 20.0),
        content_top: 2.0,
        content_height: 15.0,
        placeholder_opacity: 0.55,
    };

    /// A code block's (`pre`): `pre { font-family: "Exo Mono Email"; font-size: 14px;
    /// letter-spacing: -0.02em; line-height: 20px }` (the email CSS), the content area 1px down
    /// and 17px tall.
    pub const CODE: Self = Self {
        style: TextStyle::new(Font::new(Family::MonoEmail, 400), 14.0, 20.0).letter_spacing(-0.28),
        content_top: 1.0,
        content_height: 17.0,
        placeholder_opacity: 0.45,
    };

    fn line(&self) -> f32 {
        self.style.line_height
    }
}

impl Default for BodyText {
    fn default() -> Self {
        Self::COMPOSE
    }
}

/// How the widget looks and sizes itself, beyond the theme.
#[derive(Clone, Debug, Default)]
pub struct EditorStyle {
    /// Shown while the body is empty (`Say hello`, `Write a note`, …); `None` for none.
    pub placeholder: Option<String>,
    /// The least height the widget takes.
    pub min_height: f32,
}

/// What happened in the editor this frame.
pub struct EditorResponse {
    /// The widget's response (focus, hover).
    pub response: Response,
    /// Whether the document changed.
    pub changed: bool,
    /// The inline trigger being typed, with its anchor on screen.
    pub trigger: Option<Trigger>,
    /// Where the caret is on screen.
    pub caret: Rect,
    /// Marks of changes ⌘Z or ⌘⇧Z undid or redid this frame ([`Editor::mark_undo`]).
    pub undone: Vec<UndoMark>,
    /// Marks of changes redone this frame.
    pub redone: Vec<UndoMark>,
    /// Escape the editor didn't need (no dropdown, ghost text or toolbar to close): the compose
    /// form's to handle.
    pub escaped: bool,
    /// ↓ or → at the very end of the body: the compose form moves on to the quote's `…` when it
    /// shows one.
    pub past_end: bool,
    /// A toolbar button clicked this frame (for its hint).
    pub toolbar: Option<crate::ToolbarAction>,
    /// A link the link popover's URL was clicked to open.
    pub open_link: Option<String>,
    /// A right-click, for the app's menu.
    pub context_menu: Option<ContextMenu>,
    /// A word autocorrect's pill learned (`Learned "{word}"`).
    pub learned: Option<String>,
    /// The detected date the caret is on, while the editor has the keyboard: the app shows its day
    /// and moves it with ⌘⇧– and ⌘⇧= ([`Editor::shift_date`]).
    pub date: Option<crate::ActiveDate>,
    /// Text the Mac's drag let go on the editor went in ([`exo_ui::text_drag`]): it takes the
    /// keyboard, as a click would give it.
    pub dropped: bool,
}

/// A right-click in the body and what's under it, which the editable context menu is made of.
#[derive(Clone, Debug, PartialEq)]
pub struct ContextMenu {
    /// Where, on screen.
    pub at: Pos2,
    /// The `href` of the link clicked.
    pub link: Option<String>,
    /// The misspelled word clicked, with its corrections.
    pub misspelling: Option<Misspelling>,
    /// The selected text, once the click has selected the word under it.
    pub selection: String,
    /// Where the selection's first line of text starts on screen: its baseline's left end, which
    /// Look Up's popover lines its own text up with.
    pub selection_baseline: Pos2,
    /// The selection's text size, for Look Up.
    pub text_size: f32,
}

/// What a key did beyond editing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyEffect {
    /// ⌘Z undid a change; its marks.
    Undone(Vec<UndoMark>),
    /// ⌘⇧Z or ⌘Y redid one; its marks.
    Redone(Vec<UndoMark>),
    /// Escape had nothing to close in the editor.
    Escaped,
    /// ↓ or → with nothing after the caret but an empty line.
    PastEnd,
}

/// How a drag extends the selection, by the click that started it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Granularity {
    Character,
    Word,
    Paragraph,
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    origin: Selection,
    granularity: Granularity,
}

/// What the view keeps between frames.
#[derive(Default)]
pub(crate) struct ViewState {
    blink_reset: bool,
    blink_start: f64,
    layout: Option<Layout>,
    drag: Option<Drag>,
    clicks: Option<(f64, Pos2, u8)>,
    preferred_x: Option<f32>,
    pub(crate) preedit: Option<String>,
    pub(crate) toolbar: toolbar::State,
    clipboard: Option<Box<dyn Clipboard>>,
    scroll_to_caret: bool,
    pub(crate) body: BodyText,
    /// The images' pixels.
    pub(crate) pictures: Pictures,
    /// The tables it keeps whole, drawn by Blitz.
    pub(crate) tables: table::Tables,
    /// The document, pictures and tables revisions the images were last asked for at.
    pub(crate) images_scanned: Option<(u64, u64, u64)>,
    /// A resize handle being dragged.
    resize: Option<resizer::Drag>,
    /// A right-click this frame.
    context_menu: Option<ContextMenu>,
    /// Since when the date tooltip has shown.
    date_since: Option<f64>,
    /// A press on the selection, which may drag it.
    text_drag: Option<drag::TextDrag>,
    /// Where text another drag has over the editor would go.
    drag_over: Option<Pos>,
    /// The selection the pointer last made by words (a double click, and its drag): dropped,
    /// it's put in smartly.
    word_selection: Option<Selection>,
}

impl ViewState {
    /// Lays everything out again next frame.
    pub(crate) fn forget_layout(&mut self) {
        self.layout = None;
    }

    /// The caret moved or the document changed: show the caret and restart its blink.
    pub(crate) fn caret_moved(&mut self) {
        self.blink_reset = true;
        self.scroll_to_caret = true;
    }
}

/// The document laid out at one width.
pub(crate) struct Layout {
    key: LayoutKey,
    pub(crate) blocks: Vec<BlockLayout>,
    pub(crate) height: f32,
    bars: Vec<Rect>,
    /// A space's width, which a selection paints past the end of each line it continues from.
    space: f32,
    /// The width it was laid out at.
    width: f32,
    /// Where egui puts the body face's baseline in a row.
    baseline: f32,
}

/// What a layout was made from.
#[derive(Clone, Debug, Default, PartialEq)]
struct LayoutKey {
    revision: u64,
    /// The width at its sub-pixel phase, and the scale.
    size: (u32, u32),
    theme: Theme,
    ghost: Option<String>,
    preedit: Option<String>,
    /// Where the caret is, which matters only while text is spliced in at it.
    caret: Pos,
    pictures: u64,
    tables: u64,
}

/// One block's place and text.
pub(crate) struct BlockLayout {
    /// The top of its first line box, from the editor's top.
    pub(crate) top: f32,
    /// Where its text starts, from the editor's left.
    pub(crate) left: f32,
    pub(crate) height: f32,
    /// The laid-out text (with ghost or composition text spliced in at `insert`).
    pub(crate) galley: Arc<Galley>,
    /// The sub-pixel phase the text was laid out at (`exo_ui::paint_job`).
    phase: f32,
    /// `(offset, length)` of text spliced in that isn't in the document.
    insert: Option<(usize, usize)>,
    marker: Option<Marker>,
    rule: bool,
    /// Underlines and strike-throughs: galley character ranges and their colour.
    decorations: Vec<Decoration>,
    /// For wrapped text, each row laid out on its own at the phase, to paint (the wrapped
    /// galley's later rows were rasterized without it).
    rows: Vec<Arc<Galley>>,
    /// Each galley row's line box.
    lines: Vec<LineBox>,
    /// Its images.
    images: Vec<Placed>,
    /// What the text above was laid out from (`text_key`), to reuse it next time.
    text_key: u64,
    /// Its text's style: the body's, or a code block's.
    text: BodyText,
    /// The dates detected in it ([`crate::dates`]), as character ranges of the block.
    pub(crate) dates: Vec<std::ops::Range<usize>>,
}

/// One line box of a block: where it is below the block's top, how tall, and how far its text
/// sits below where a line without images would put it (an image taller than the text raises
/// the line's baseline).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LineBox {
    top: f32,
    height: f32,
    shift: f32,
}

/// An image in a block's layout.
#[derive(Clone, Debug)]
struct Placed {
    /// Its position in the block.
    offset: usize,
    /// Its content box, from the block's text origin (`left`, `top`).
    rect: Rect,
    /// Its column of its line box (its padding box across, the line box down), where a
    /// selection paints its tint rather than the text's highlight.
    column: Rect,
    /// Its pixels' key ([`ImageSource::key`]).
    key: Option<String>,
    look: Look,
}

/// A `text-decoration` line over a run of characters (the run of the `u` or `s` element).
#[derive(Clone, Debug)]
struct Decoration {
    chars: std::ops::Range<usize>,
    kind: DecorationKind,
    color: Color32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DecorationKind {
    Underline,
    LineThrough,
}

#[derive(Clone, Debug)]
enum Marker {
    Disc,
    Circle,
    Square,
    Text(String),
}

/// The theme's colours for user content.
struct Colors {
    text: Color32,
    /// A quote's text: `blockquote:not([style*="border:none"])` in `.sh-unquoted-content`.
    quote_text: Color32,
    quote: Color32,
    bar: Color32,
    rule: Color32,
    selection: Color32,
    /// The selection's tint over a selected image.
    image_selection: Color32,
    ghost: Color32,
}

impl Colors {
    fn of(ctx: &egui::Context) -> Self {
        let palette = exo_ui::palette(ctx);
        match exo_ui::theme(ctx) {
            // `.sh-color-black`: rgba(0,0,0,.9); the quote bar rgb(237,237,237); `::selection`
            // #b4d5fe; the prediction rgba(0,0,0,.45).
            Theme::Snow => Self {
                text: Color32::from_rgba_unmultiplied(0, 0, 0, 230),
                quote_text: Color32::from_rgba_unmultiplied(0, 0, 0, 153),
                quote: palette.text_base_subdued,
                bar: Color32::from_rgb(237, 237, 237),
                rule: Color32::from_rgb(0xed, 0xed, 0xed),
                selection: Color32::from_rgb(0xb4, 0xd5, 0xfe),
                image_selection: crate::image::selection_tint(Color32::from_rgb(0xb4, 0xd5, 0xfe)),
                ghost: Color32::from_rgba_unmultiplied(0, 0, 0, 115),
            },
            // The dark themes fall through to Classic carbon: hsla(0,0%,100%,.9), bar
            // rgb(95,97,101), `hr` #5f6165, selection rgba(106,134,173,.996).
            Theme::Carbon => Self {
                text: Color32::from_rgba_unmultiplied(255, 255, 255, 230),
                quote_text: Color32::from_rgba_unmultiplied(255, 255, 255, 153),
                quote: palette.text_base_subdued,
                bar: Color32::from_rgb(95, 97, 101),
                rule: Color32::from_rgb(0x5f, 0x61, 0x65),
                selection: Color32::from_rgba_unmultiplied(106, 134, 173, 254),
                // Carbon's selection over an image, `color-mix(…, transparent 50%)`, for every
                // image: an opaque one would hide it whole.
                image_selection: Color32::from_rgba_unmultiplied(106, 134, 173, 127),
                ghost: Color32::from_rgba_unmultiplied(255, 255, 255, 102),
            },
        }
    }
}

impl Editor {
    /// Uses `clipboard` for copy, cut and paste instead of the system's.
    pub fn set_clipboard(&mut self, clipboard: Box<dyn Clipboard>) {
        self.view.clipboard = Some(clipboard);
    }

    /// Sets the body's text in `body` rather than compose's size.
    pub fn set_body_text(&mut self, body: BodyText) {
        self.view.body = body;
        self.view.layout = None;
    }

    /// Whether Escape has something to close in the editor (a trigger's dropdown, ghost text,
    /// the toolbar, the link popover); otherwise it's the compose form's.
    pub fn wants_escape(&self) -> bool {
        self.trigger.is_some()
            || self.ghost.is_some()
            || self.pill_wants_escape()
            || self.view.toolbar.is_open()
            || (!self.selection.is_collapsed() && self.view.toolbar.visible)
            || self.view.toolbar.popover
            || self.view.text_drag.is_some_and(|drag| drag.dragging())
    }

    /// Whether Tab is the editor's now (ghost text to take, a suggestion chosen) rather than the
    /// form's.
    pub fn wants_tab(&self) -> bool {
        self.ghost.is_some() || self.pill_wants_tab()
    }

    /// The document's height at `width`, its left edge at `left` on screen: what [`Self::ui`]
    /// will lay it out to (the layout is kept for it), for callers placing things below.
    pub fn height(&mut self, ui: &Ui, width: f32, left: f32) -> f32 {
        let colors = Colors::of(ui.ctx());
        self.layout(ui, width, &colors, left).height
    }

    /// Draws the editor in the available width and handles its input.
    pub fn ui(&mut self, ui: &mut Ui, style: &EditorStyle) -> EditorResponse {
        let id = ui.id().with("exo_editor");
        let width = ui.available_width();
        let origin = ui.cursor().min;
        let colors = Colors::of(ui.ctx());
        let layout_height = self.layout(ui, width, &colors, origin.x).height;
        let height = layout_height.max(style.min_height);
        let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
        let response = response.on_hover_cursor(egui::CursorIcon::Text);
        // A click on the toolbar or the link popover keeps the editor's focus, as their
        // `mousedown`'s `preventDefault` does; egui would surrender it to the click.
        let overlays = std::mem::take(&mut self.view.toolbar.rects);
        if !response.has_focus()
            && ui.memory(|memory| memory.had_focus_last_frame(response.id))
            && ui.input(|input| {
                input.pointer.any_click()
                    && input
                        .pointer
                        .interact_pos()
                        .is_some_and(|point| overlays.iter().any(|overlay| overlay.contains(point)))
            })
        {
            response.request_focus();
        }
        let focused = response.has_focus();
        let revision = self.revision;
        let mut out = EditorResponse {
            response: response.clone(),
            changed: false,
            trigger: None,
            caret: Rect::NOTHING,
            undone: Vec::new(),
            redone: Vec::new(),
            escaped: false,
            past_end: false,
            toolbar: None,
            open_link: None,
            context_menu: None,
            learned: None,
            date: None,
            dropped: false,
        };
        self.pointer(ui, &response, rect, id);
        if self.drag_over(ui, rect, id) {
            response.request_focus();
            out.dropped = true;
        }
        out.context_menu = self.view.context_menu.take();
        if response.has_focus() || focused {
            self.keyboard(ui, &mut out);
        } else {
            // A composition ends with focus: its commit or cancel goes to whatever has it now.
            self.view.preedit = None;
        }
        // Input may have changed the document: lay out again before painting.
        self.layout(ui, width, &colors, rect.min.x);
        if self.update_spelling(response.has_focus()) {
            ui.ctx().request_repaint();
        }
        let now = ui.input(|input| input.time);
        if std::mem::take(&mut self.view.blink_reset) {
            self.view.blink_start = now;
        }
        // Autocorrect's pause-dependent work, the caret having rested since the blink restarted.
        if let Some(wait) = self.assist(response.has_focus(), now - self.view.blink_start) {
            ui.ctx().request_repaint_after_secs(wait as f32);
        }
        // What autocorrect did is laid out before painting.
        self.layout(ui, width, &colors, rect.min.x);
        // Images on screen have textures, and show the frame that's due.
        let shown = self.images_on_screen(rect.min, ui.clip_rect());
        self.view
            .pictures
            .upload(ui.ctx(), |key| shown.contains(key));
        if let Some(wait) = self.view.pictures.animate(now, |key| shown.contains(key)) {
            ui.ctx().request_repaint_after_secs(wait as f32);
        }
        self.paint_tables(ui.ctx(), rect.min, ui.clip_rect());
        self.paint(ui, rect, &colors, style, response.has_focus(), now);
        self.paint_text_drag(ui, rect.min, colors.text);
        if response.has_focus() {
            self.resizer(ui, rect.min, id);
            // A resize changed the document: what's laid out next frame is what it painted.
            self.layout(ui, width, &colors, rect.min.x);
        }
        out.caret = self.caret_rect(rect.min, ui.pixels_per_point());
        // The toolbar hangs from the selected text's box (`getBoundingClientRect`).
        let mut text_boxes =
            self.text_boxes(rect.min, self.selection.start(), self.selection.end());
        text_boxes.extend(
            self.selected_images()
                .into_iter()
                .filter_map(|at| self.image_rect(rect.min, at)),
        );
        let selection = toolbar::anchor(&text_boxes);
        out.toolbar = self.show_toolbar(ui, selection, response.has_focus());
        // Without it, a caret in a link shows the link popover under the link.
        let link = if response.has_focus() && !self.view.toolbar.visible {
            self.popover_link()
        } else {
            None
        };
        self.view.toolbar.popover = link.is_some();
        if let Some((block, range, href)) = link {
            let anchor = toolbar::anchor(&self.text_boxes(
                rect.min,
                Pos::new(block, range.start),
                Pos::new(block, range.end),
            ));
            match self.link_popover(ui, anchor, rect.min.x, &href) {
                Some(toolbar::PopoverClick::Open(href)) => out.open_link = Some(href),
                Some(toolbar::PopoverClick::Remove) => {
                    out.toolbar = Some(crate::ToolbarAction::LinkRemoved)
                }
                None => {}
            }
        }
        if let Some(mut trigger) = self.trigger.clone() {
            trigger.anchor = self.char_rect(rect.min, trigger.start);
            out.trigger = Some(trigger);
        }
        // A date the caret is on has its tooltip, and hides the assistant's dropdowns,
        // autocorrect's pill among them.
        out.date = if response.has_focus() {
            self.active_date()
        } else {
            None
        };
        match &out.date {
            Some(date) => self.show_date_tooltip(ui, rect.min, date, now),
            None => self.view.date_since = None,
        }
        if out.date.is_none()
            && let Some(picked) = self.show_pill(ui, rect.min)
        {
            self.pick_suggestion(picked);
            self.layout(ui, width, &colors, rect.min.x);
        }
        out.learned = self.autocorrect.learned.take();
        if response.has_focus() {
            ui.output_mut(|output| {
                output.ime = Some(egui::output::IMEOutput {
                    purpose: egui::IMEPurpose::Normal,
                    rect,
                    cursor_rect: out.caret,
                    should_interrupt_composition: false,
                });
            });
            if std::mem::take(&mut self.view.scroll_to_caret) {
                ui.scroll_to_rect(out.caret.expand2(vec2(0.0, 4.0)), None);
            }
        }
        if response.has_focus() {
            // The keys egui leaves the editor next frame, kept by the widget's own id: Tab and the
            // arrows always (egui's focus never moves on with them, to autocorrect's pill, say),
            // and Escape while the editor has something to close with it.
            let escape = self.wants_escape();
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    response.id,
                    EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape,
                    },
                )
            });
        }
        out.changed = self.revision != revision;
        out
    }

    /// Lays the document out (cached until it, the width, the theme or the caret's splice
    /// changes) and returns it.
    fn layout(&mut self, ui: &Ui, width: f32, colors: &Colors, left: f32) -> &Layout {
        self.view.tables.update(&self.view.pictures, ui.ctx());
        let pixels_per_point = ui.pixels_per_point();
        let phase = {
            let x = left * pixels_per_point;
            (x - x.floor()) / pixels_per_point
        };
        let key = LayoutKey {
            revision: self.revision,
            // Fonts are rasterized per scale, so a new one lays everything out again.
            size: (
                (width * 64.0) as u32 ^ ((phase * 64.0) as u32) << 24,
                pixels_per_point.to_bits(),
            ),
            theme: exo_ui::theme(ui.ctx()),
            ghost: self.ghost.clone(),
            preedit: self.view.preedit.clone(),
            caret: self.selection.focus,
            pictures: self.view.pictures.revision,
            tables: self.view.tables.revision,
        };
        let stale = self.view.layout.as_ref().is_none_or(|layout| {
            let spliced = key.ghost.is_some() || key.preedit.is_some();
            layout.key
                != LayoutKey {
                    caret: if spliced { key.caret } else { layout.key.caret },
                    ..key.clone()
                }
        });
        if stale {
            // Blocks laid out at this width, scale and theme keep their text, so an edit lays
            // out only the blocks it changed.
            let previous = self
                .view
                .layout
                .take()
                .filter(|layout| layout.key.size == key.size && layout.key.theme == key.theme);
            let tables = self.view.tables.revision;
            let layout = lay_out(
                ui,
                &self.doc,
                width,
                phase,
                colors,
                &self.view.body,
                self.splice(),
                previous.as_ref().map_or(&[], |layout| &layout.blocks),
                &self.view.pictures,
                &mut self.view.tables,
                !self.settings.skip_dates,
            );
            // A table built has images for the app to load, which it asks for next frame.
            if self.view.tables.revision != tables {
                ui.ctx().request_repaint();
            }
            self.view.layout = Some(Layout {
                key: LayoutKey {
                    tables: self.view.tables.revision,
                    ..key
                },
                ..layout
            });
        }
        self.view.layout.as_ref().expect("laid out above")
    }

    /// Text spliced in at the caret that isn't in the document: the IME composition or the
    /// ghost text.
    fn splice(&self) -> Option<(Pos, String, bool)> {
        let caret = self.selection.focus;
        if let Some(preedit) = self.view.preedit.clone().filter(|text| !text.is_empty()) {
            return Some((caret, preedit, true));
        }
        self.ghost.clone().map(|ghost| (caret, ghost, false))
    }

    fn paint(
        &self,
        ui: &Ui,
        rect: Rect,
        colors: &Colors,
        style: &EditorStyle,
        focused: bool,
        now: f64,
    ) {
        let Some(layout) = &self.view.layout else {
            return;
        };
        let body = &self.view.body;
        let painter = ui.painter_at(rect.expand(2.0));
        for bar in &layout.bars {
            painter.rect_filled(
                exo_ui::pixel_snap(bar.translate(rect.min.to_vec2())),
                CornerRadius::ZERO,
                colors.bar,
            );
        }
        // Selection, under the text; a focused image shows the resizer instead.
        let focused_image = self.focused_image();
        if !self.selection.is_collapsed() && focused_image.is_none() {
            for (highlight, _) in self.selection_rects(rect.min) {
                painter.rect_filled(highlight, CornerRadius::ZERO, colors.selection);
            }
        }
        if let Some(image) = focused_image.and_then(|at| self.image_rect(rect.min, at)) {
            resizer::paint_shadow(ui, image);
        }
        for (index, (block, view)) in self.doc.blocks.iter().zip(&layout.blocks).enumerate() {
            let top_left = rect.min + vec2(view.left, view.top);
            if view.rule {
                let line = Rect::from_min_size(
                    pos2(rect.min.x + view.left, top_left.y + 10.0),
                    vec2(rect.width() - view.left, 1.0),
                );
                painter.rect_filled(exo_ui::pixel_snap(line), CornerRadius::ZERO, colors.rule);
                continue;
            }
            if let Some(marker) = &view.marker {
                paint_marker(
                    &painter,
                    marker,
                    top_left,
                    colors_for(block, colors),
                    &view.text,
                );
            }
            let x = (top_left.x * painter.pixels_per_point()).floor() / painter.pixels_per_point();
            // Autocorrect's dots, under the text (`z-index: -1`).
            for (range, corrected) in self
                .dotted_words(index)
                .filter(|(range, _)| !range.is_empty())
            {
                let chars = view.galley_index(range.start)..view.galley_index(range.end - 1) + 1;
                pill::paint_annotation(
                    &painter,
                    view,
                    chars,
                    pos2(x, top_left.y),
                    &view.text,
                    corrected,
                );
            }
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
            for decoration in &view.decorations {
                paint_decoration(&painter, view, decoration, pos2(x, top_left.y), &view.text);
            }
            for range in self.misspellings(index).filter(|range| !range.is_empty()) {
                let chars = view.galley_index(range.start)..view.galley_index(range.end - 1) + 1;
                paint_misspelling(&painter, view, chars, pos2(x, top_left.y), &view.text);
            }
            self.paint_images(
                &painter,
                view,
                index,
                top_left,
                colors,
                focused_image.is_none(),
            );
            for range in view.dates.iter().filter(|range| !range.is_empty()) {
                let chars = view.galley_index(range.start)..view.galley_index(range.end - 1) + 1;
                dates::paint_line(&painter, view, chars, pos2(x, top_left.y), &view.text);
            }
        }
        if self.doc.is_empty()
            && let Some(placeholder) = &style.placeholder
            && self.view.preedit.is_none()
        {
            // `::before { opacity: .45 }` over the text colour; laid out like the body so a
            // fallback glyph (`⌘`) sits on the same baseline.
            let color = colors.text.gamma_multiply(body.placeholder_opacity);
            let first = &layout.blocks[0];
            let mut job = LayoutJob::single_section(placeholder.clone(), body.style.format(color));
            job.sections[0].leading_space = first.phase;
            let mut galley = exo_ui::layout(ui.ctx(), job);
            align_rows(&mut galley, layout.baseline, 0.0);
            let left = rect.min.x + first.left;
            let x = (left * painter.pixels_per_point()).floor() / painter.pixels_per_point();
            exo_ui::paint_text(
                &painter,
                pos2(x, rect.min.y + first.top),
                galley,
                &body.style,
            );
        }
        if focused && self.selection.is_collapsed() {
            let cursor = &ui.visuals().text_cursor;
            let elapsed = now - self.view.blink_start;
            let period = f64::from(cursor.on_duration + cursor.off_duration);
            let visible = !cursor.blink
                || period <= 0.0
                || (elapsed % period) < f64::from(cursor.on_duration);
            if visible {
                painter.rect_filled(
                    self.caret_rect(rect.min, painter.pixels_per_point()),
                    CornerRadius::ZERO,
                    colors.text,
                );
            }
            if cursor.blink && period > 0.0 {
                let phase = elapsed % period;
                let next = if phase < f64::from(cursor.on_duration) {
                    f64::from(cursor.on_duration) - phase
                } else {
                    period - phase
                };
                ui.ctx().request_repaint_after_secs(next as f32);
            }
        }
    }

    /// The caret's box on screen, for an editor whose top left is `origin`: Chromium's caret
    /// rect, the font's content area in the line box (ascent + descent, 17px, 3px down), 1px
    /// wide on a whole device pixel. Beside an image with no text on either side, the caret is
    /// the image's box side, its whole line box tall.
    fn caret_rect(&self, origin: Pos2, pixels_per_point: f32) -> Rect {
        self.caret_rect_at(origin, pixels_per_point, self.selection.focus)
    }

    /// [`Self::caret_rect`] for a caret at `pos`.
    fn caret_rect_at(&self, origin: Pos2, pixels_per_point: f32, pos: Pos) -> Rect {
        let (rect, shift) = self.line_at(origin, pos);
        let x = (rect.min.x * pixels_per_point).round() / pixels_per_point;
        let view = self
            .view
            .layout
            .as_ref()
            .and_then(|layout| layout.blocks.get(pos.block));
        let text = view.map_or(self.view.body, |view| view.text);
        if view.is_some_and(|view| view.rule) || self.caret_on_image_side(pos) {
            return Rect::from_min_size(pos2(x, rect.min.y), vec2(CARET_WIDTH, rect.height()));
        }
        Rect::from_min_size(
            pos2(x, rect.min.y + shift + text.content_top),
            vec2(CARET_WIDTH, text.content_height),
        )
    }

    /// Whether the caret at `pos` is an image's box side (Chromium's caret beside an atomic
    /// inline): an image on one side and no text on either.
    fn caret_on_image_side(&self, pos: Pos) -> bool {
        let block = &self.doc.blocks[pos.block];
        // A kept table is a block-level atom: the caret is its box's side.
        if matches!(block.content, Content::Raw { .. }) {
            return true;
        }
        let before = pos
            .offset
            .checked_sub(1)
            .and_then(|offset| block.slice(offset..pos.offset).pop())
            .map(|run| run.piece);
        let after = block
            .slice(pos.offset..pos.offset + 1)
            .pop()
            .map(|run| run.piece);
        let text = |piece: &Option<Piece>| matches!(piece, Some(Piece::Text(_)));
        let image = |piece: &Option<Piece>| matches!(piece, Some(Piece::Image(_)));
        !text(&before) && !text(&after) && (image(&before) || image(&after))
    }

    /// The box of the line position `pos` is at: zero wide, a line box tall.
    fn char_rect(&self, origin: Pos2, pos: Pos) -> Rect {
        self.line_at(origin, pos).0
    }

    /// The line box position `pos` is at (zero wide, at its x) and how far down its text is
    /// moved for its images.
    fn line_at(&self, origin: Pos2, pos: Pos) -> (Rect, f32) {
        let Some(layout) = &self.view.layout else {
            return (Rect::NOTHING, 0.0);
        };
        let Some(view) = layout.blocks.get(pos.block) else {
            return (Rect::NOTHING, 0.0);
        };
        // The caret goes before ghost text (`span.sh-autocomplete-prediction` after it), after a
        // composition.
        let ghost = self.ghost.is_some()
            && self.view.preedit.is_none()
            && view.insert.is_some_and(|(at, _)| at == pos.offset);
        let index = if ghost {
            pos.offset
        } else {
            view.galley_index(pos.offset)
        };
        let cursor = view.galley.pos_from_cursor(CCursor::new(index));
        let x = origin.x + view.left + cursor.min.x - view.phase;
        if view.rule {
            return (
                Rect::from_min_size(pos2(x, origin.y + view.top), vec2(0.0, RULE_HEIGHT)),
                0.0,
            );
        }
        let line = view.lines[view.row_of(cursor.center().y, view.text.line())];
        (
            Rect::from_min_size(
                pos2(x, origin.y + view.top + line.top),
                vec2(0.0, line.height),
            ),
            line.shift,
        )
    }

    /// The image at `at`'s box on screen, for an editor at `origin`.
    pub(crate) fn image_rect(&self, origin: Pos2, at: Pos) -> Option<Rect> {
        let view = self.view.layout.as_ref()?.blocks.get(at.block)?;
        let placed = view
            .images
            .iter()
            .find(|placed| placed.offset == at.offset)?;
        Some(
            placed
                .rect
                .translate(origin.to_vec2() + vec2(view.left, view.top)),
        )
    }

    /// The image under `point` that can be focused, if any.
    fn image_under(&self, origin: Pos2, point: Pos2) -> Option<Pos> {
        let layout = self.view.layout.as_ref()?;
        layout.blocks.iter().enumerate().find_map(|(index, view)| {
            let local = point - origin - vec2(view.left, view.top);
            view.images
                .iter()
                .find(|placed| {
                    placed.look == Look::Picture && placed.rect.contains(local.to_pos2())
                })
                .map(|placed| Pos::new(index, placed.offset))
        })
    }

    /// Paints block `index`'s images (laid out in `view`, its text origin at `top_left`), and the
    /// selection's tint over those selected, if `tint`.
    /// The keys of the images whose boxes are in `clip`, for an editor at `origin`.
    fn images_on_screen(&self, origin: Pos2, clip: Rect) -> std::collections::HashSet<String> {
        let Some(layout) = &self.view.layout else {
            return Default::default();
        };
        layout
            .blocks
            .iter()
            .flat_map(|view| {
                view.images.iter().filter(move |placed| {
                    clip.intersects(
                        placed
                            .rect
                            .translate((origin + vec2(view.left, view.top)).to_vec2()),
                    )
                })
            })
            .filter_map(|placed| placed.key.clone())
            .collect()
    }

    fn paint_images(
        &self,
        painter: &egui::Painter,
        view: &BlockLayout,
        index: usize,
        top_left: Pos2,
        colors: &Colors,
        tint: bool,
    ) {
        let (start, end) = (self.selection.start(), self.selection.end());
        for placed in &view.images {
            let rect = exo_ui::pixel_snap(placed.rect.translate(top_left.to_vec2()));
            match placed.look {
                Look::Picture => {
                    let texture =
                        placed
                            .key
                            .as_deref()
                            .and_then(|key| match self.view.pictures.get(key) {
                                Some(State::Loaded {
                                    texture: Some(texture),
                                    ..
                                }) => Some(texture.id()),
                                _ => None,
                            });
                    if let Some(texture) = texture {
                        painter.image(
                            texture,
                            rect,
                            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                }
                // Its texture holds the sub-pixel phase, on the whole pixel left of the text's
                // origin ([`Self::paint_tables`]).
                Look::Table(key) => {
                    let ppp = painter.pixels_per_point();
                    for (top, texture) in self.view.tables.bands(key) {
                        let at = pos2(
                            (top_left.x * ppp).floor() / ppp,
                            ((top_left.y + placed.rect.min.y + top) * ppp).round() / ppp,
                        );
                        let [width, height] = texture.size();
                        painter.image(
                            texture.id(),
                            Rect::from_min_size(at, vec2(width as f32 / ppp, height as f32 / ppp)),
                            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                }
                // `img[src^="https://media-blocked"]::after`: white, a 1px `rgba(0, 0, 0, .2)`
                // border inside it.
                Look::Blocked => {
                    painter.rect_filled(rect, CornerRadius::ZERO, Color32::WHITE);
                    painter.rect_stroke(
                        rect,
                        CornerRadius::ZERO,
                        Stroke::new(1.0, Color32::from_rgba_unmultiplied(0, 0, 0, 51)),
                        egui::StrokeKind::Inside,
                    );
                }
            }
            // Chromium tints a selected image's column of its line box, over the image.
            let at = Pos::new(index, placed.offset);
            if tint && at >= start && at < end {
                let column = exo_ui::pixel_snap(placed.column.translate(top_left.to_vec2()));
                painter.rect_filled(column, CornerRadius::ZERO, colors.image_selection);
            }
        }
    }

    /// Paints the parts of the layout's kept tables inside `clip`, for an editor at `origin`,
    /// where their pictures are out of date.
    fn paint_tables(&mut self, ctx: &egui::Context, origin: Pos2, clip: Rect) {
        let ViewState {
            layout: Some(layout),
            tables,
            ..
        } = &mut self.view
        else {
            return;
        };
        let ppp = ctx.pixels_per_point();
        for view in &layout.blocks {
            for placed in &view.images {
                let Look::Table(key) = placed.look else {
                    continue;
                };
                let left = origin.x + view.left;
                let top = origin.y + view.top + placed.rect.min.y;
                tables.paint(
                    key,
                    placed.rect.width(),
                    left - (left * ppp).floor() / ppp,
                    ppp,
                    (clip.top() - top)..(clip.bottom() - top),
                    ctx,
                );
            }
        }
    }

    /// The selection's highlight boxes: per line, the selected text's span in a full line box,
    /// plus a space's width past the end of each line the selection continues past; each with how
    /// far its line's text is moved down for its images.
    fn selection_rects(&self, origin: Pos2) -> Vec<(Rect, f32)> {
        self.range_rects(origin, self.selection.start(), self.selection.end())
    }

    /// The text's boxes from `start` to `end`: the font's content area in each line box (the body's
    /// 17px, 3px down).
    fn text_boxes(&self, origin: Pos2, start: Pos, end: Pos) -> Vec<Rect> {
        let body = self.view.body;
        self.range_rects(origin, start, end)
            .into_iter()
            .map(|(line, shift)| {
                Rect::from_min_size(
                    line.min + vec2(0.0, shift + body.content_top),
                    vec2(line.width(), body.content_height),
                )
            })
            .collect()
    }

    /// [`Self::selection_rects`] for the range from `start` to `end`.
    fn range_rects(&self, origin: Pos2, start: Pos, end: Pos) -> Vec<(Rect, f32)> {
        let Some(layout) = &self.view.layout else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for index in start.block..=end.block {
            let view = &layout.blocks[index];
            let block = &self.doc.blocks[index];
            let from = if index == start.block {
                start.offset
            } else {
                0
            };
            let to = if index == end.block {
                end.offset
            } else {
                block.len()
            };
            let left = origin.x + view.left - view.phase;
            let top = origin.y + view.top;
            if view.rule {
                if from < to {
                    out.push((
                        Rect::from_min_size(
                            pos2(origin.x + view.left, top),
                            vec2(4.0, RULE_HEIGHT),
                        ),
                        0.0,
                    ));
                }
                continue;
            }
            let mut offset = 0;
            for (row_index, row) in view.galley.rows.iter().enumerate() {
                let count = row.char_count_including_newline().0;
                let (row_start, row_end) = (offset, offset + row.char_count_excluding_newline().0);
                offset += count;
                let (a, b) = (
                    view.document_offset(row_start),
                    view.document_offset(row_end),
                );
                let (s, e) = (from.max(a), to.min(b));
                // Past a hard line end the selection paints the line break as a space; past a
                // soft wrap it paints nothing more.
                let last_row = row_index + 1 == view.galley.rows.len();
                let continues =
                    (to > b || index < end.block) && (last_row || row.ends_with_newline);
                if s > e || (s == e && !continues) {
                    continue;
                }
                let x0 = row.x_offset(CharIndex(view.galley_index(s) - row_start));
                let mut x1 = row.x_offset(CharIndex(view.galley_index(e) - row_start));
                if continues {
                    // Chromium highlights the line break as a space.
                    x1 += layout.space;
                }
                let line = view.lines[row_index];
                let y = top + line.top;
                // Chromium paints the whole line box, its edges on whole CSS pixels, inside the
                // block; an image's column gets the image's tint instead ([`Self::paint_images`]).
                let right = (left + x1).min(origin.x + layout.width);
                let mut from_x = left + x0;
                for placed in view
                    .images
                    .iter()
                    .filter(|placed| (s..e).contains(&placed.offset))
                {
                    let column = placed.column.translate(vec2(origin.x + view.left, 0.0));
                    if column.min.x > from_x {
                        out.push((
                            exo_ui::pixel_snap(Rect::from_min_max(
                                pos2(from_x, y),
                                pos2(column.min.x, y + line.height),
                            )),
                            line.shift,
                        ));
                    }
                    from_x = from_x.max(column.max.x);
                }
                if right > from_x {
                    out.push((
                        exo_ui::pixel_snap(Rect::from_min_max(
                            pos2(from_x, y),
                            pos2(right, y + line.height),
                        )),
                        line.shift,
                    ));
                }
            }
        }
        out
    }

    /// The document position under `point` (screen), for an editor at `origin`.
    fn hit(&self, origin: Pos2, point: Pos2) -> Pos {
        let Some(layout) = &self.view.layout else {
            return Pos::default();
        };
        let local = point - origin;
        let index = layout
            .blocks
            .iter()
            .position(|view| local.y < view.top + view.height)
            .unwrap_or(layout.blocks.len() - 1);
        let view = &layout.blocks[index];
        if view.rule {
            return Pos::new(index, 0);
        }
        let x = local.x - view.left + view.phase;
        let y = (local.y - view.top).clamp(0.0, view.height - 1.0);
        // The galley's rows are a plain line apart; find the line box `y` is in.
        let line = view.text.line();
        let row = view
            .lines
            .iter()
            .rposition(|line| y >= line.top)
            .unwrap_or(0);
        let cursor = view
            .galley
            .cursor_from_pos(vec2(x, row as f32 * line + line / 2.0));
        Pos::new(index, view.document_offset(cursor.index.0))
    }

    fn pointer(&mut self, ui: &Ui, response: &Response, rect: Rect, id: Id) {
        if response.secondary_clicked()
            && let Some(point) = response.interact_pointer_pos()
        {
            self.right_click(ui, rect.min, point);
            response.request_focus();
            return;
        }
        if self.view.text_drag.is_some() {
            self.drag_text(ui, rect, id);
            return;
        }
        let (pressed, pointer, shift, time) = ui.input(|input| {
            (
                input.pointer.primary_pressed(),
                input.pointer.interact_pos(),
                input.modifiers.shift,
                input.time,
            )
        });
        if response.hovered()
            && pressed
            && let Some(point) = pointer
            && let Some(at) = self.image_under(rect.min, point)
        {
            // A click on an image focuses it.
            response.request_focus();
            self.view.drag = None;
            self.view.preferred_x = None;
            if self.focus_image(at) {
                return;
            }
        }
        if response.hovered()
            && pressed
            && let Some(point) = pointer
        {
            response.request_focus();
            let count = match self.view.clicks {
                Some((at, place, count)) if time - at < 0.5 && place.distance(point) < 4.0 => {
                    count % 3 + 1
                }
                _ => 1,
            };
            self.view.clicks = Some((time, point, count));
            let pos = self.hit(rect.min, point);
            // A press on the selection may drag it (`FrameSelection::Contains`).
            if count == 1
                && !shift
                && !self.selection.is_collapsed()
                && self.selection.start() <= pos
                && pos <= self.selection.end()
            {
                self.view.text_drag = Some(drag::TextDrag::new(point));
                self.view.drag = None;
                return;
            }
            let granularity = match count {
                2 => Granularity::Word,
                3 => Granularity::Paragraph,
                _ => Granularity::Character,
            };
            let origin = match granularity {
                Granularity::Character if shift => Selection::new(self.selection.anchor, pos),
                Granularity::Character => Selection::caret(pos),
                Granularity::Word => self.word_at(pos),
                Granularity::Paragraph => Selection::new(
                    Pos::new(pos.block, 0),
                    Pos::new(pos.block, self.doc.blocks[pos.block].len()),
                ),
            };
            self.view.drag = Some(Drag {
                origin,
                granularity,
            });
            self.view.preferred_x = None;
            self.view.toolbar.reset();
            self.set_selection(origin);
            self.view.word_selection = (granularity == Granularity::Word).then_some(self.selection);
        }
        if let Some(drag) = self.view.drag {
            if response.dragged()
                && let Some(point) = pointer
            {
                let pos = self.hit(rect.min, point);
                let span = match drag.granularity {
                    Granularity::Character => Selection::caret(pos),
                    Granularity::Word => self.word_at(pos),
                    Granularity::Paragraph => Selection::new(
                        Pos::new(pos.block, 0),
                        Pos::new(pos.block, self.doc.blocks[pos.block].len()),
                    ),
                };
                let selection = if span.end() <= drag.origin.start() {
                    Selection::new(drag.origin.end(), span.start())
                } else {
                    Selection::new(drag.origin.start(), span.end().max(drag.origin.end()))
                };
                self.set_selection(selection);
                if drag.granularity == Granularity::Word {
                    self.view.word_selection = Some(self.selection);
                }
                ui.scroll_to_rect(
                    Rect::from_center_size(point, Vec2::splat(self.view.body.line())),
                    None,
                );
            }
            if ui.input(|input| input.pointer.primary_released()) {
                self.view.drag = None;
            }
        }
    }

    /// A right-click at `point`, for an editor at `origin`: a click outside the selection selects
    /// the word under it, or puts the caret there off a word, as a Mac's does
    /// (`SelectClosestWordOrLinkFromMouseEvent`), then the menu is the app's to show.
    fn right_click(&mut self, ui: &Ui, origin: Pos2, point: Pos2) {
        let pos = self.hit(origin, point);
        // The character under the pointer: before the caret position when left of it.
        let before = pos.offset > 0 && point.x < self.char_rect(origin, pos).min.x;
        let under = Pos::new(pos.block, pos.offset - usize::from(before));
        let over_selection = self.selection.start() <= under
            && under < self.selection.end()
            && under.offset < self.doc.blocks[under.block].len();
        if !over_selection {
            let words = Words::of(&self.doc.blocks[under.block]);
            let on_word = words
                .clusters
                .iter()
                .position(|(from, to)| *from <= under.offset && under.offset < *to)
                .is_some_and(|at| words.is_word(at));
            self.view.toolbar.reset();
            self.set_selection(if on_word {
                self.word_at(under)
            } else {
                Selection::caret(pos)
            });
        }
        let selection = self.selection;
        let misspelling = self.misspelling_at(selection.start()).filter(|word| {
            Pos::new(word.block, word.range.start) == selection.start()
                && Pos::new(word.block, word.range.end) == selection.end()
        });
        let block = &self.doc.blocks[selection.start().block];
        let link = block
            .slice(selection.start().offset..selection.start().offset + 1)
            .pop()
            .and_then(|run| run.marks.into_iter().find(|mark| mark.is("a")))
            .and_then(|link| link.attr("href").map(str::to_owned));
        let text = self
            .view
            .layout
            .as_ref()
            .and_then(|layout| layout.blocks.get(selection.start().block))
            .map_or(self.view.body, |view| view.text);
        let (line, shift) = self.line_at(origin, selection.start());
        let selection_baseline = pos2(
            line.min.x,
            line.min.y + shift + text.style.baseline(ui.ctx()),
        );
        self.view.context_menu = Some(ContextMenu {
            at: point,
            link,
            misspelling,
            selection: self.selected_text(),
            selection_baseline,
            text_size: text.style.size,
        });
    }

    /// The word around `pos` (double click): letters and digits, or a run of spaces, or one
    /// other character, in whole graphemes.
    fn word_at(&self, pos: Pos) -> Selection {
        let words = Words::of(&self.doc.blocks[pos.block]);
        let Some(at) = words
            .clusters
            .iter()
            .position(|(_, end)| *end > pos.offset)
            .or(words.clusters.len().checked_sub(1))
        else {
            return Selection::caret(pos);
        };
        let kind = words.class(at);
        let same = |index: &usize| kind != 2 && words.class(*index) == kind;
        let first = (0..at).rev().take_while(same).last().unwrap_or(at);
        let last = (at + 1..words.clusters.len())
            .take_while(same)
            .last()
            .unwrap_or(at);
        Selection::new(
            Pos::new(pos.block, words.clusters[first].0),
            Pos::new(pos.block, words.clusters[last].1),
        )
    }

    fn keyboard(&mut self, ui: &Ui, out: &mut EditorResponse) {
        let events = ui.input(|input| input.events.clone());
        for event in events {
            match event {
                Event::Text(text) if self.view.preedit.is_none() => {
                    self.view.preferred_x = None;
                    self.type_text(&text);
                }
                Event::Ime(ImeEvent::Preedit { text, .. }) => {
                    self.view.preedit = (!text.is_empty()).then_some(text);
                    self.view.caret_moved();
                }
                Event::Ime(ImeEvent::Commit(text)) => {
                    self.view.preedit = None;
                    self.type_text(&text);
                }
                Event::Ime(ImeEvent::DeleteSurrounding {
                    before_chars,
                    after_chars,
                }) => {
                    let caret = self.selection.focus;
                    let len = self.doc.blocks[caret.block].len();
                    let start = Pos::new(caret.block, caret.offset.saturating_sub(before_chars));
                    let end = Pos::new(
                        caret.block,
                        caret.offset.saturating_add(after_chars).min(len),
                    );
                    self.set_selection(Selection::new(start, end));
                    self.delete_back_to(start);
                }
                Event::Copy => self.copy(false),
                Event::Cut => self.copy(true),
                Event::Paste(text) => {
                    let plain = ui.input(|input| input.modifiers.shift);
                    let html = if plain {
                        None
                    } else {
                        self.clipboard().read_html()
                    };
                    match html {
                        Some(html) => self.paste_html(&html),
                        None => self.paste_text(&text),
                    }
                }
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => match self.press(key, modifiers) {
                    Some(KeyEffect::Undone(marks)) => out.undone.extend(marks),
                    Some(KeyEffect::Redone(marks)) => out.redone.extend(marks),
                    Some(KeyEffect::Escaped) => out.escaped = true,
                    Some(KeyEffect::PastEnd) => out.past_end = true,
                    None => {}
                },
                _ => {}
            }
        }
    }

    fn clipboard(&mut self) -> &mut Box<dyn Clipboard> {
        self.view
            .clipboard
            .get_or_insert_with(|| Box::new(SystemClipboard::default()))
    }

    /// ⌘C / ⌘X: the selection as HTML (bare list items in their lists) and as text.
    fn copy(&mut self, cut: bool) {
        if self.selection.is_collapsed() {
            return;
        }
        let markup = self.selection_markup();
        let text = self.selected_text();
        self.clipboard().write(&markup, &text);
        if cut {
            self.cut();
        }
    }

    /// The selection as a copy writes it in HTML: bare list items in their lists.
    pub(crate) fn selection_markup(&self) -> String {
        let blocks = self
            .doc
            .fragment(self.selection.start(), self.selection.end());
        let mut out = String::new();
        if blocks.len() == 1 && blocks[0].path.is_empty() {
            html::write_runs(blocks[0].runs(), false, &mut out);
        } else {
            html::write_blocks(&blocks, &mut out);
        }
        out
    }

    /// Handles a key press as the widget does when it has focus: what a keystroke script (or a
    /// test) drives the editor with. Text keys go through [`Editor::type_text`].
    pub fn press(&mut self, key: Key, modifiers: Modifiers) -> Option<KeyEffect> {
        // A modifier pressed on its own (egui reports Shift, ⌘ and the rest as keys) does
        // nothing, as in a browser: it mustn't forget the caret's column before Shift+↓.
        if matches!(
            key,
            Key::ShiftLeft
                | Key::ShiftRight
                | Key::ControlLeft
                | Key::ControlRight
                | Key::AltLeft
                | Key::AltRight
                | Key::SuperLeft
                | Key::SuperRight
        ) {
            return None;
        }
        if modifiers.is_none() && self.pill_key(key) {
            return None;
        }
        if self.ghost.is_none() && self.image_key(key, modifiers) {
            self.view.preferred_x = None;
            return None;
        }
        let past_end = matches!(key, Key::ArrowDown | Key::ArrowRight)
            && !modifiers.shift
            && (key == Key::ArrowDown || self.selection.is_collapsed())
            && self.ghost.is_none()
            && self.trigger.is_none()
            && self.nothing_after();
        let mac = modifiers.mac_cmd || modifiers.command;
        let caret = self.selection.focus;
        let vertical = matches!(key, Key::ArrowUp | Key::ArrowDown);
        if !vertical {
            self.view.preferred_x = None;
        }
        match key {
            Key::Enter if !mac => self.enter(modifiers.shift),
            Key::Backspace if modifiers.alt && self.selection.is_collapsed() => {
                self.delete_back_to(self.word_start(caret))
            }
            Key::Backspace if mac && self.selection.is_collapsed() => {
                self.delete_back_to(self.line_start(caret))
            }
            Key::Backspace => {
                self.backspace();
                self.deleted_for_assistance();
            }
            Key::Delete if modifiers.alt && self.selection.is_collapsed() => {
                self.delete_forward_to(self.word_end(caret))
            }
            Key::Delete => {
                self.delete_forward();
                self.recheck_edited_word();
            }
            Key::A if mac => self.select_all(),
            Key::Z if mac && modifiers.shift => return Some(KeyEffect::Redone(self.redo())),
            Key::Z if mac => return Some(KeyEffect::Undone(self.undo())),
            Key::Y if mac => return Some(KeyEffect::Redone(self.redo())),
            Key::ArrowRight if self.ghost.is_some() && !modifiers.shift => {
                self.accept_ghost_text();
            }
            Key::Tab if self.ghost.is_some() => {
                self.accept_ghost_text();
            }
            Key::Escape => {
                if self.trigger.is_some() {
                    self.dismiss_trigger();
                } else if self.ghost.is_some() {
                    self.ghost = None;
                } else if self.view.toolbar.is_open()
                    || !self.selection.is_collapsed() && self.view.toolbar.visible
                {
                    self.view.toolbar.reset();
                    self.set_selection(Selection::caret(self.selection.focus));
                } else if self.view.toolbar.popover {
                    self.view.toolbar.popover = false;
                    self.view.toolbar.popover_closed = Some((self.selection, self.revision));
                } else {
                    return Some(KeyEffect::Escaped);
                }
            }
            Key::ArrowLeft
            | Key::ArrowRight
            | Key::ArrowUp
            | Key::ArrowDown
            | Key::Home
            | Key::End => self.move_caret(key, modifiers),
            _ => {}
        }
        past_end.then_some(KeyEffect::PastEnd)
    }

    /// Whether nothing shows after the selection's end: no text, image or atom, and at most
    /// one line break.
    fn nothing_after(&self) -> bool {
        let end = self.selection.end();
        let mut breaks = 0;
        for (index, block) in self.doc.blocks.iter().enumerate().skip(end.block) {
            if !block.is_inline() {
                return false;
            }
            let from = if index == end.block { end.offset } else { 0 };
            for run in block.slice(from..block.len()) {
                match run.piece {
                    Piece::Break => breaks += 1,
                    _ => return false,
                }
            }
            breaks += usize::from(block.trailing_break);
        }
        breaks <= 1
    }

    fn move_caret(&mut self, key: Key, modifiers: Modifiers) {
        let mac = modifiers.mac_cmd || modifiers.command;
        let (selection, caret) = (self.selection, self.selection.focus);
        let collapsing = !selection.is_collapsed() && !modifiers.shift;
        let target = match key {
            Key::ArrowLeft if collapsing && !modifiers.alt && !mac => selection.start(),
            Key::ArrowRight if collapsing && !modifiers.alt && !mac => selection.end(),
            Key::ArrowLeft if mac => self.line_start(caret),
            Key::ArrowRight if mac => self.line_end(caret),
            Key::ArrowLeft if modifiers.alt => self.word_start(caret),
            Key::ArrowRight if modifiers.alt => self.word_end(caret),
            Key::ArrowLeft => self.step(caret, false),
            Key::ArrowRight => self.step(caret, true),
            Key::Home => self.line_start(caret),
            Key::End => self.line_end(caret),
            Key::ArrowUp if mac => Pos::default(),
            Key::ArrowDown if mac => self.doc.end(),
            Key::ArrowUp if modifiers.alt => {
                if caret.offset > 0 {
                    Pos::new(caret.block, 0)
                } else {
                    Pos::new(caret.block.saturating_sub(1), 0)
                }
            }
            Key::ArrowDown if modifiers.alt => {
                let len = self.doc.blocks[caret.block].len();
                if caret.offset < len {
                    Pos::new(caret.block, len)
                } else {
                    self.doc.clamp(Pos::new(caret.block + 1, usize::MAX))
                }
            }
            Key::ArrowUp | Key::ArrowDown => self.vertical(
                if collapsing {
                    if key == Key::ArrowUp {
                        selection.start()
                    } else {
                        selection.end()
                    }
                } else {
                    caret
                },
                key == Key::ArrowDown,
            ),
            _ => caret,
        };
        let keep_x = self.view.preferred_x;
        let next = if modifiers.shift {
            Selection::new(selection.anchor, target)
        } else {
            Selection::caret(target)
        };
        self.set_selection(next);
        self.view.preferred_x = keep_x;
        self.view.blink_reset = true;
        self.view.scroll_to_caret = true;
    }

    /// One grapheme left or right, or across a block boundary.
    fn step(&self, pos: Pos, forward: bool) -> Pos {
        let block = &self.doc.blocks[pos.block];
        if forward {
            if pos.offset >= block.len() {
                return self.doc.clamp(Pos::new(pos.block + 1, 0)).max(pos);
            }
            Pos::new(pos.block, grapheme::next(&block.text(), pos.offset))
        } else {
            if pos.offset == 0 {
                return if pos.block == 0 {
                    pos
                } else {
                    Pos::new(pos.block - 1, self.doc.blocks[pos.block - 1].len())
                };
            }
            Pos::new(pos.block, grapheme::previous(&block.text(), pos.offset))
        }
    }

    /// ⌥←: the start of the word before `pos` (or the previous block's end at a block start).
    fn word_start(&self, pos: Pos) -> Pos {
        if pos.offset == 0 {
            return self.step(pos, false);
        }
        let words = Words::of(&self.doc.blocks[pos.block]);
        let before = (0..words.clusters.len())
            .rev()
            .skip_while(|index| words.clusters[*index].1 > pos.offset);
        let start = before
            .skip_while(|index| !words.is_word(*index))
            .take_while(|index| words.is_word(*index))
            .last();
        Pos::new(pos.block, start.map_or(0, |index| words.clusters[index].0))
    }

    /// ⌥→: the end of the word after `pos`.
    fn word_end(&self, pos: Pos) -> Pos {
        let block = &self.doc.blocks[pos.block];
        if pos.offset >= block.len() {
            return self.step(pos, true);
        }
        let words = Words::of(block);
        let after =
            (0..words.clusters.len()).skip_while(|index| words.clusters[*index].0 < pos.offset);
        let end = after
            .skip_while(|index| !words.is_word(*index))
            .take_while(|index| words.is_word(*index))
            .last();
        Pos::new(
            pos.block,
            end.map_or(block.len(), |index| words.clusters[index].1),
        )
    }

    /// The start of the visual line `pos` is on.
    fn line_start(&self, pos: Pos) -> Pos {
        self.row_bounds(pos).0
    }

    /// The end of the visual line `pos` is on.
    fn line_end(&self, pos: Pos) -> Pos {
        self.row_bounds(pos).1
    }

    fn row_bounds(&self, pos: Pos) -> (Pos, Pos) {
        let Some(view) = self
            .view
            .layout
            .as_ref()
            .and_then(|layout| layout.blocks.get(pos.block))
        else {
            return (
                Pos::new(pos.block, 0),
                Pos::new(pos.block, self.doc.blocks[pos.block].len()),
            );
        };
        let index = view.galley_index(pos.offset);
        let mut start = 0;
        for row in &view.galley.rows {
            let end = start + row.char_count_excluding_newline().0;
            if index <= end {
                return (
                    Pos::new(pos.block, view.document_offset(start)),
                    Pos::new(pos.block, view.document_offset(end)),
                );
            }
            start += row.char_count_including_newline().0;
        }
        (
            Pos::new(pos.block, 0),
            Pos::new(pos.block, self.doc.blocks[pos.block].len()),
        )
    }

    /// The position a line up or down from `pos`, keeping the caret's x.
    fn vertical(&mut self, pos: Pos, down: bool) -> Pos {
        let origin = Pos2::ZERO;
        let here = self.char_rect(origin, pos);
        let x = *self.view.preferred_x.get_or_insert(here.min.x);
        let half = self.view.body.line() / 2.0;
        let y = if down {
            here.max.y + half
        } else {
            here.min.y - half
        };
        let Some(layout) = &self.view.layout else {
            return pos;
        };
        if y < 0.0 {
            return Pos::default();
        }
        if y > layout.height {
            return self.doc.end();
        }
        // Skip margins between blocks: land in the nearest line box.
        let target_y = layout
            .blocks
            .iter()
            .map(|view| (view.top, view.top + view.height))
            .find(|(top, bottom)| y < *bottom && (down || y >= *top))
            .map_or(y, |(top, bottom)| y.clamp(top + 1.0, bottom - 1.0));
        let target_y = if !down {
            layout
                .blocks
                .iter()
                .rev()
                .map(|view| (view.top, view.top + view.height))
                .find(|(top, _)| y >= *top)
                .map_or(target_y, |(top, bottom)| y.clamp(top + 1.0, bottom - 1.0))
        } else {
            target_y
        };
        self.hit(origin, pos2(x, target_y))
    }
}

/// A block's text in grapheme clusters, for moving by words.
struct Words {
    chars: Vec<char>,
    clusters: Vec<(usize, usize)>,
}

impl Words {
    fn of(block: &Block) -> Self {
        let text = block.text();
        Self {
            clusters: grapheme::clusters(&text),
            chars: text.chars().collect(),
        }
    }

    /// Cluster `index`'s class, by its first character: 0 for a word's, 1 for whitespace, 2
    /// for anything else.
    fn class(&self, index: usize) -> u8 {
        let ch = self.chars[self.clusters[index].0];
        if ch.is_alphanumeric() || ch == '_' || ch == '\'' {
            0
        } else if ch.is_whitespace() {
            1
        } else {
            2
        }
    }

    fn is_word(&self, index: usize) -> bool {
        self.class(index) == 0
    }
}

impl BlockLayout {
    /// The galley row at galley `y` (its rows are `line` apart).
    fn row_of(&self, y: f32, line: f32) -> usize {
        ((y / line).floor().max(0.0) as usize).min(self.lines.len() - 1)
    }

    /// The galley's character index for document offset `offset`.
    fn galley_index(&self, offset: usize) -> usize {
        match self.insert {
            Some((at, len)) if offset > at => offset + len,
            Some((at, len)) if offset == at => at + len,
            _ => offset,
        }
    }

    /// The document offset for galley index `index`.
    fn document_offset(&self, index: usize) -> usize {
        match self.insert {
            Some((at, len)) if index >= at + len => index - len,
            Some((at, _)) if index > at => at,
            _ => index,
        }
    }
}

/// The colour a block's text is drawn in: a quote's at 60% (measured: `rgba(0, 0, 0, .6)`, as
/// in the email restyle), the reply's quoted part subdued.
fn colors_for(block: &Block, colors: &Colors) -> Color32 {
    if block.in_quote() {
        colors.quote_text
    } else if block.path.iter().any(|container| {
        container
            .element
            .attr("class")
            .is_some_and(|class| class.split_whitespace().any(|class| class == "gmail_quote"))
    }) {
        colors.quote
    } else {
        colors.text
    }
}

/// The hash of what a block's text layout depends on.
fn text_key(
    block: &Block,
    color: Color32,
    width: f32,
    splice: Option<&(Pos, String, bool)>,
    images: &[ImageBox],
) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (
        &block.content,
        block.in_code(),
        color,
        width.to_bits(),
        splice.map(|(pos, text, composing)| (pos.offset, text, composing)),
    )
        .hash(&mut hasher);
    for image in images {
        image.hash(&mut hasher);
    }
    hasher.finish()
}

/// The text's vertical metrics an image's line box is built from.
#[derive(Clone, Copy)]
struct LineMetrics {
    /// The baseline below a plain line box's top.
    baseline: f32,
    x_height: f32,
}

/// What a block's text is laid out with: its style and what's measured from its face.
struct Face {
    text: BodyText,
    metrics: LineMetrics,
    /// A space's advance per px of font size, which sizes an image's glyph.
    space_em: f32,
    /// Where egui puts the face's baseline in a row: every face's glyphs go there.
    baseline: f32,
}

impl Face {
    fn of(ui: &Ui, text: BodyText) -> Self {
        let first_glyph = |job: LayoutJob| {
            ui.fonts_mut(|fonts| fonts.layout_job(job))
                .rows
                .first()
                .and_then(|row| row.glyphs.first())
                .copied()
        };
        let format = text.style.format(Color32::WHITE);
        let space = TextFormat {
            font_id: egui::FontId::new(100.0, format.font_id.family.clone()),
            ..Default::default()
        };
        Self {
            metrics: LineMetrics {
                baseline: text.style.baseline(ui.ctx()),
                x_height: X_HEIGHT * text.style.size,
            },
            space_em: first_glyph(LayoutJob::single_section(" ".to_owned(), space))
                .map_or(0.25, |glyph| glyph.advance_width / 100.0),
            baseline: first_glyph(LayoutJob::single_section("x".to_owned(), format))
                .map_or(0.0, |glyph| glyph.pos.y),
            text,
        }
    }
}

/// `pre { margin: 1em 0 }` (the UA's) at the code's 14px.
const CODE_MARGIN: f32 = 14.0;

/// The `margin-left` a code block's `style` gives it (`20px` for one made in the editor).
fn code_indent(element: &crate::model::Element) -> f32 {
    element
        .attr("style")
        .and_then(|style| {
            style.split(';').find_map(|declaration| {
                declaration
                    .split_once(':')
                    .filter(|(name, _)| name.trim() == "margin-left")
                    .map(|(_, value)| {
                        value
                            .trim()
                            .trim_end_matches("px")
                            .trim()
                            .parse::<f32>()
                            .ok()
                    })
            })
        })
        .flatten()
        .unwrap_or(0.0)
}

/// The boxes of a block's images, in order.
fn image_boxes(block: &Block, pictures: &Pictures, container: f32) -> Vec<ImageBox> {
    block
        .runs()
        .iter()
        .filter_map(|run| match &run.piece {
            Piece::Image(image) => Some(ImageBox::of(
                image,
                ImageSource::of(image).and_then(|source| pictures.get(&source.key())),
                container,
            )),
            _ => None,
        })
        .collect()
}

/// Each galley row's line box, grown for its images (CSS 2.1 section 10.8): the strut's line box,
/// and each image's box from the baseline, the line as tall as both reach.
fn line_boxes(
    galley: &Galley,
    images: &[(usize, ImageBox)],
    line: f32,
    metrics: LineMetrics,
) -> Vec<LineBox> {
    let mut out = Vec::with_capacity(galley.rows.len());
    let (mut start, mut top) = (0, 0.0);
    for row in &galley.rows {
        let end = start + row.char_count_including_newline().0;
        // A `display: block` image alone on its row is a block box: no strut, no line around it.
        let alone = row.char_count_excluding_newline().0 == 1
            && images
                .iter()
                .any(|(index, image)| *index == start && image.block);
        let (mut above, mut below) = if alone {
            (0.0, 0.0)
        } else {
            (metrics.baseline, line - metrics.baseline)
        };
        for (_, image) in images
            .iter()
            .filter(|(index, _)| (start..end).contains(index))
        {
            let (image_top, image_bottom) = image.extent(metrics.x_height);
            above = above.max(-image_top);
            below = below.max(image_bottom);
        }
        out.push(LineBox {
            top,
            height: above + below,
            shift: above - metrics.baseline,
        });
        top += above + below;
        start = end;
    }
    if out.is_empty() {
        out.push(LineBox {
            top: 0.0,
            height: line,
            shift: 0.0,
        });
    }
    out
}

/// Where each image lands: where the pen is at its glyph, on its line's baseline. The pen is
/// the sum of the advances before it in its row (each row is painted from the text's origin):
/// egui puts a glyph on the whole pixel left of it, which would put an image up to a pixel left
/// of where Chromium draws it.
fn place_images(
    block: &Block,
    galley: &Galley,
    glyphs: &[(usize, usize)],
    boxes: &[ImageBox],
    lines: &[LineBox],
    metrics: LineMetrics,
) -> Vec<Placed> {
    // Each image's pixels' key; a kept table's are its own.
    let keys: Vec<Option<String>> = match &block.content {
        Content::Raw { .. } => vec![None],
        _ => block
            .runs()
            .iter()
            .filter_map(|run| match &run.piece {
                Piece::Image(image) => Some(ImageSource::of(image).map(|source| source.key())),
                _ => None,
            })
            .collect(),
    };
    keys.into_iter()
        .zip(glyphs.iter().zip(boxes))
        .map(|(key, (&(index, offset), image))| {
            let mut row_start = 0;
            let row = galley.rows.iter().position(|row| {
                let end = row_start + row.char_count_including_newline().0;
                let here = index < end;
                if !here {
                    row_start = end;
                }
                here
            });
            let row = row
                .unwrap_or(galley.rows.len().saturating_sub(1))
                .min(lines.len() - 1);
            let pen = layout_unit(galley.rows.get(row).map_or(0.0, |placed| {
                placed
                    .glyphs
                    .iter()
                    .take(index.saturating_sub(row_start))
                    .map(|glyph| glyph.advance_width)
                    .sum()
            }));
            let baseline = lines[row].top + lines[row].shift + metrics.baseline;
            let (image_top, _) = image.extent(metrics.x_height);
            Placed {
                offset,
                rect: Rect::from_min_size(
                    pos2(pen + image.padding, baseline + image_top),
                    image.size,
                ),
                column: Rect::from_min_size(
                    pos2(pen, lines[row].top),
                    vec2(image.advance(), lines[row].height),
                ),
                key,
                look: image.look,
            }
        })
        .collect()
}

/// `px` as Chromium's layout holds a length: to the nearest 1/64 px (`LayoutUnit`), which puts
/// what follows text (an image, a button's edge) where Chromium snaps it.
fn layout_unit(px: f32) -> f32 {
    (px * 64.0).round() / 64.0
}

/// Lays `doc` out, taking each block's text from `previous` when nothing it depends on changed.
#[expect(
    clippy::too_many_arguments,
    reason = "the layout's inputs, each its own"
)]
fn lay_out(
    ui: &Ui,
    doc: &Document,
    width: f32,
    phase: f32,
    colors: &Colors,
    body: &BodyText,
    splice: Option<(Pos, String, bool)>,
    previous: &[BlockLayout],
    pictures: &Pictures,
    tables: &mut table::Tables,
    detect_dates: bool,
) -> Layout {
    let cached: std::collections::HashMap<u64, &BlockLayout> =
        previous.iter().map(|view| (view.text_key, view)).collect();
    let (theme, scale) = (exo_ui::theme(ui.ctx()), ui.pixels_per_point());
    tables.begin_layout();
    let faces = [Face::of(ui, *body), Face::of(ui, BodyText::CODE)];
    let mut blocks = Vec::with_capacity(doc.blocks.len());
    let mut y = 0.0;
    // Margins pending above the next block (collapsed as CSS does: the largest wins).
    let mut margin: f32 = 0.0;
    let mut bars: Vec<Rect> = Vec::new();
    // Quotes whose bar is still running: `(depth, top, left)`.
    let mut open_bars: Vec<(usize, f32, f32)> = Vec::new();
    let mut ordinals: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    for (index, block) in doc.blocks.iter().enumerate() {
        let previous = index.checked_sub(1).map(|i| &doc.blocks[i]);
        let shared = previous.map_or(0, |previous| {
            previous
                .path
                .iter()
                .zip(&block.path)
                .take_while(|(a, b)| a.id == b.id)
                .count()
        });
        // Containers closing after the previous block, then those opening before this one.
        if let Some(previous) = previous {
            for container in &previous.path[shared..] {
                margin = margin.max(container_margin(previous, container));
            }
            // Quotes that end here draw their bar down to the end of the previous block.
            let (closed, still_open): (Vec<_>, Vec<_>) = open_bars
                .into_iter()
                .partition(|(depth, _, _)| *depth >= shared);
            open_bars = still_open;
            bars.extend(
                closed
                    .into_iter()
                    .map(|(_, top, left)| Rect::from_min_max(pos2(left, top), pos2(left + 1.0, y))),
            );
        }
        for container in &block.path[shared..] {
            margin = margin.max(container_margin(block, container));
        }
        if previous.is_some_and(|previous| previous.element.is("p") && previous.path == block.path)
            && block.element.is("p")
        {
            margin = margin.max(16.0);
        }
        if block.element.is("pre") || previous.is_some_and(|previous| previous.element.is("pre")) {
            margin = margin.max(CODE_MARGIN);
        }
        y += margin;
        margin = 0.0;
        let mut left = 0.0;
        for (depth, container) in block.path.iter().enumerate() {
            match container.element.container_kind() {
                ContainerKind::List { .. } => left += LIST_INDENT,
                ContainerKind::Quote => {
                    if depth >= shared {
                        open_bars.push((depth, y, left));
                    }
                    left += QUOTE_INDENT;
                }
                ContainerKind::Indent => left += INDENT,
                ContainerKind::Code => left += code_indent(&container.element),
                ContainerKind::Other => {}
            }
        }
        if block.element.is("pre") {
            left += code_indent(&block.element);
        }
        let face = &faces[usize::from(block.in_code())];
        let text_width = (width - left).max(1.0);
        let color = colors_for(block, colors);
        let splice_here = splice.as_ref().filter(|(pos, _, _)| pos.block == index);
        // A kept table is one block-level atom, drawn by Blitz, as tall as its table.
        let boxes = match &block.content {
            Content::Raw { html, .. } => {
                let (key, size) = tables.lay_out(
                    &html::in_containers(&block.path, html),
                    text_width,
                    scale,
                    theme,
                    pictures,
                );
                vec![ImageBox {
                    size,
                    padding: 0.0,
                    look: Look::Table(key),
                    block: true,
                }]
            }
            _ => image_boxes(block, pictures, text_width),
        };
        let key = text_key(block, color, text_width, splice_here, &boxes);
        let (galley, insert, decorations, row_galleys, lines, images, dates) = match cached
            .get(&key)
        {
            Some(view) => (
                view.galley.clone(),
                view.insert,
                view.decorations.clone(),
                view.rows.clone(),
                view.lines.clone(),
                view.images.clone(),
                view.dates.clone(),
            ),
            None => {
                let dates = if detect_dates {
                    crate::dates::detect(block)
                } else {
                    Vec::new()
                };
                let BlockJob {
                    job,
                    insert,
                    decorations,
                    images: glyphs,
                } = block_job(
                    block,
                    color,
                    colors,
                    &face.text,
                    text_width,
                    phase,
                    splice_here,
                    &boxes,
                    face.space_em,
                    &dates,
                );
                let mut galley = exo_ui::layout(ui.ctx(), job.clone());
                align_rows(&mut galley, face.baseline, phase);
                let rows = if galley.rows.len() > 1 {
                    row_jobs(&job, &galley, phase, &face.text)
                        .into_iter()
                        .map(|row| {
                            let mut row = exo_ui::layout(ui.ctx(), row);
                            align_rows(&mut row, face.baseline, 0.0);
                            row
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                let sized: Vec<(usize, ImageBox)> = glyphs
                    .iter()
                    .map(|(index, _)| *index)
                    .zip(boxes.iter().copied())
                    .collect();
                let lines = line_boxes(&galley, &sized, face.text.line(), face.metrics);
                let images = place_images(block, &galley, &glyphs, &boxes, &lines, face.metrics);
                (galley, insert, decorations, rows, lines, images, dates)
            }
        };
        let rule = matches!(block.content, Content::Rule);
        let height = if rule {
            RULE_HEIGHT
        } else {
            lines.iter().map(|line| line.height).sum()
        };
        let marker = list_marker(block, &mut ordinals);
        blocks.push(BlockLayout {
            top: y,
            left,
            height,
            galley,
            phase,
            insert,
            marker,
            rule,
            decorations,
            rows: row_galleys,
            lines,
            images,
            text_key: key,
            text: face.text,
            dates,
        });
        y += height;
    }
    // Containers still open at the end close here.
    if let Some(last) = doc.blocks.last() {
        for container in &last.path {
            margin = margin.max(container_margin(last, container));
        }
        if last.element.is("pre") {
            margin = margin.max(CODE_MARGIN);
        }
    }
    bars.extend(
        open_bars
            .into_iter()
            .map(|(_, top, left)| Rect::from_min_max(pos2(left, top), pos2(left + 1.0, y))),
    );
    tables.end_layout();
    let space = ui
        .fonts_mut(|fonts| {
            fonts.layout_job(LayoutJob::single_section(
                " ".to_owned(),
                body.style.format(colors.text),
            ))
        })
        .size()
        .x;
    Layout {
        key: LayoutKey::default(),
        blocks,
        height: y + margin,
        bars,
        space,
        width,
        baseline: faces[0].baseline,
    }
}

/// Puts every glyph on the body's baseline (where egui would hang an italic run higher than the
/// text around it; `exo_ui::align_baselines`) and every wrapped row at the text's sub-pixel
/// phase: only the first row carries the phase `exo_ui::paint_job` lays text out at, so later
/// rows would start half a pixel off.
fn align_rows(galley: &mut Arc<Galley>, baseline: f32, phase: f32) {
    exo_ui::align_baselines(galley, baseline);
    if galley.rows.len() > 1 && phase != 0.0 {
        for placed in Arc::make_mut(galley).rows.iter_mut().skip(1) {
            placed.pos.x += phase;
        }
    }
}

/// One unwrapped job per row of `galley` (laid out from `job`), each starting at `phase`.
fn row_jobs(job: &LayoutJob, galley: &Galley, phase: f32, body: &BodyText) -> Vec<LayoutJob> {
    let bytes: Vec<usize> = job
        .text
        .char_indices()
        .map(|(at, _)| at)
        .chain(std::iter::once(job.text.len()))
        .collect();
    let mut start = 0;
    let mut out = Vec::with_capacity(galley.rows.len());
    for row in &galley.rows {
        let end = start + row.char_count_excluding_newline().0;
        let range = bytes[start.min(bytes.len() - 1)]..bytes[end.min(bytes.len() - 1)];
        let mut row_job = LayoutJob {
            wrap: TextWrapping::no_max_width(),
            ..Default::default()
        };
        for (index, section) in job.sections.iter().enumerate() {
            let (from, to) = (
                section.byte_range.start.0.max(range.start),
                section.byte_range.end.0.min(range.end),
            );
            if from >= to {
                continue;
            }
            // An image's room (its section's leading space) stays where it starts.
            let own = if from == section.byte_range.start.0 && index > 0 {
                section.leading_space
            } else {
                0.0
            };
            let leading = if row_job.sections.is_empty() {
                phase + own
            } else {
                own
            };
            row_job.append(&job.text[from..to], leading, section.format.clone());
        }
        if row_job.sections.is_empty() {
            row_job.append("", phase, body.style.format(Color32::TRANSPARENT));
        }
        out.push(row_job);
        start += row.char_count_including_newline().0;
    }
    out
}

/// A container's vertical margin: 16px for a list not inside another list, a code block's.
fn container_margin(block: &Block, container: &crate::model::Container) -> f32 {
    match container.element.container_kind() {
        ContainerKind::Code => return CODE_MARGIN,
        ContainerKind::List { .. } => {}
        _ => return 0.0,
    }
    let depth = block
        .path
        .iter()
        .position(|c| c.id == container.id)
        .unwrap_or(0);
    let nested = block.path[..depth]
        .iter()
        .any(|c| matches!(c.element.container_kind(), ContainerKind::List { .. }));
    if nested { 0.0 } else { LIST_MARGIN }
}

/// A list item's marker: from its `list-style-type`, else by nesting (`ul ul` circle, deeper
/// square); numbers count the items before it in its list.
fn list_marker(
    block: &Block,
    ordinals: &mut std::collections::HashMap<u32, u32>,
) -> Option<Marker> {
    if !block.is_list_item() {
        return None;
    }
    let (_, list) = block.list()?;
    let ordered = matches!(
        list.element.container_kind(),
        ContainerKind::List { ordered: true }
    );
    let style = block.element.attr("style").and_then(|style| {
        style.split(';').find_map(|declaration| {
            declaration
                .trim()
                .strip_prefix("list-style-type:")
                .map(|value| value.trim().to_owned())
        })
    });
    let count = ordinals.entry(list.id).or_insert_with(|| {
        list.element
            .attr("start")
            .and_then(|start| start.parse::<u32>().ok())
            .unwrap_or(1)
            .saturating_sub(1)
    });
    *count = block
        .element
        .attr("value")
        .and_then(|value| value.parse().ok())
        .unwrap_or(*count + 1);
    let depth = block.list_depth();
    let style = style.unwrap_or_else(|| {
        if ordered {
            "decimal".to_owned()
        } else {
            ["disc", "circle", "square"][(depth.max(1) - 1).min(2)].to_owned()
        }
    });
    Some(match style.as_str() {
        "disc" => Marker::Disc,
        "circle" => Marker::Circle,
        "square" => Marker::Square,
        "lower-alpha" => Marker::Text(format!("{}.", alpha(*count))),
        "lower-roman" => Marker::Text(format!("{}.", roman(*count))),
        _ => Marker::Text(format!("{count}.")),
    })
}

fn alpha(mut n: u32) -> String {
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        out.push(char::from(b'a' + (n % 26) as u8));
        n /= 26;
    }
    out.iter().rev().collect()
}

fn roman(mut n: u32) -> String {
    const TABLE: [(u32, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut out = String::new();
    for (value, symbol) in TABLE {
        while n >= value {
            out.push_str(symbol);
            n -= value;
        }
    }
    out
}

/// Paints a list marker in the first line box at `line_top_left` (the item's text start):
/// Chromium's shapes for disc, circle and square, text for numbers.
fn paint_marker(
    painter: &egui::Painter,
    marker: &Marker,
    line_top_left: Pos2,
    color: Color32,
    body: &BodyText,
) {
    let baseline = line_top_left.y + body.style.baseline(painter.ctx());
    match marker {
        Marker::Text(text) => {
            // `1. `: the marker's text and its trailing space end where the item's text starts.
            let galley = body
                .style
                .single_line(painter, &format!("{text} "), color, None);
            let width = galley.size().x;
            exo_ui::paint_text(
                painter,
                pos2(line_top_left.x - width, line_top_left.y),
                galley,
                &body.style,
            );
        }
        shape => {
            // `ListMarker::RelativeSymbolMarkerRect`: a (ascent·2/3 + 1)/2 = 4px bullet whose box
            // ends 11.375px before the text and sits 2px above the baseline at every nesting
            // level, on whole device pixels.
            let pixels = painter.pixels_per_point();
            let left = ((line_top_left.x - 15.375) * pixels).round() / pixels;
            let rect = Rect::from_min_size(pos2(left, baseline - 6.0), Vec2::splat(4.0));
            match shape {
                Marker::Disc => painter.circle_filled(rect.center(), 2.0, color),
                Marker::Circle => {
                    painter.circle_stroke(rect.center(), 2.0, Stroke::new(1.0, color))
                }
                _ => painter.rect_filled(rect, CornerRadius::ZERO, color),
            };
        }
    }
}

/// Paints an underline (1px, 1px under the baseline) or a strike-through (1px, 5px above it)
/// across `decoration`'s characters, one segment per row, as Chromium draws them for 14px
/// text (the post table's underline position −75, OS/2 strikeout 250, both 50 thick).
fn paint_decoration(
    painter: &egui::Painter,
    view: &BlockLayout,
    decoration: &Decoration,
    galley_origin: Pos2,
    body: &BodyText,
) {
    let baseline = body.style.baseline(painter.ctx());
    let offset = match decoration.kind {
        DecorationKind::Underline => 1.0,
        DecorationKind::LineThrough => -5.0,
    };
    let mut start = 0;
    for (row_index, row) in view.galley.rows.iter().enumerate() {
        let count = row.char_count_excluding_newline().0;
        let (from, to) = (
            decoration.chars.start.max(start),
            decoration.chars.end.min(start + count),
        );
        if from < to {
            let x0 = view.galley.pos_from_cursor(CCursor::new(from)).min.x;
            let x1 = view.galley.pos_from_cursor(CCursor::new(to)).min.x;
            let line = view.lines[row_index.min(view.lines.len() - 1)];
            let y = galley_origin.y + line.top + line.shift + baseline + offset;
            let rect = Rect::from_min_max(
                pos2(galley_origin.x + x0, y),
                pos2(galley_origin.x + x1, y + 1.0),
            );
            painter.rect_filled(rect, CornerRadius::ZERO, decoration.color);
        }
        start += row.char_count_including_newline().0;
    }
}

/// Chromium's misspelling marker on a Mac ([`exo_ui::paint_misspelling`]) under characters
/// `chars` of `view`, on each row they're on.
fn paint_misspelling(
    painter: &egui::Painter,
    view: &BlockLayout,
    chars: std::ops::Range<usize>,
    galley_origin: Pos2,
    body: &BodyText,
) {
    let top = exo_ui::misspelling_top(&body.style, painter.ctx());
    let mut start = 0;
    for (row_index, row) in view.galley.rows.iter().enumerate() {
        let count = row.char_count_excluding_newline().0;
        let (from, to) = (chars.start.max(start), chars.end.min(start + count));
        if from < to {
            let x0 = galley_origin.x + view.galley.pos_from_cursor(CCursor::new(from)).min.x;
            let x1 = galley_origin.x + view.galley.pos_from_cursor(CCursor::new(to)).min.x;
            let line = view.lines[row_index.min(view.lines.len() - 1)];
            let ppp = painter.pixels_per_point();
            exo_ui::paint_misspelling(
                painter,
                x0..x1,
                galley_origin.y + line.top + line.shift + top,
                (x0 * ppp).floor() / ppp,
            );
        }
        start += row.char_count_including_newline().0;
    }
}

/// A block's text as an egui job.
struct BlockJob {
    job: LayoutJob,
    /// `(offset, length)` of the splice.
    insert: Option<(usize, usize)>,
    decorations: Vec<Decoration>,
    /// Each image's glyph: its galley index and its offset in the block.
    images: Vec<(usize, usize)>,
}

/// One block's text as an egui job, with `splice` (IME or ghost text) inserted at its offset,
/// and each image (`images`, in order) a space whose advance is the image's width (a space
/// `space_em` px of font size wide), transparent, in a line box of the text's. A line doesn't
/// break inside one of its `dates` (`.sh-date { white-space: nowrap }`).
#[expect(
    clippy::too_many_arguments,
    reason = "the layout's inputs, each its own"
)]
fn block_job(
    block: &Block,
    color: Color32,
    colors: &Colors,
    body: &BodyText,
    width: f32,
    phase: f32,
    splice: Option<&(Pos, String, bool)>,
    images: &[ImageBox],
    space_em: f32,
    dates: &[std::ops::Range<usize>],
) -> BlockJob {
    let mut job = LayoutJob {
        wrap: TextWrapping::wrap_at_width(width + phase),
        ..Default::default()
    };
    let mut insert = None;
    let runs = match &block.content {
        Content::Inline(runs) => runs.clone(),
        Content::Raw { .. } => vec![crate::model::Run {
            piece: Piece::Image(crate::model::Element::new("table")),
            marks: Vec::new(),
        }],
        Content::Rule => Vec::new(),
    };
    // Spaces a browser would collapse are `&nbsp;`s, which egui mustn't break a line at; a
    // code block's are all plain.
    let mut spaces = if block.in_code() {
        vec![false; runs.iter().map(|run| run.piece.len()).sum()]
    } else {
        html::space_kinds(&runs)
    };
    for at in dates.iter().flat_map(Clone::clone) {
        if let Some(space) = spaces.get_mut(at) {
            *space = true;
        }
    }
    let mut position = 0;
    // The first section starts at the sub-pixel phase (`exo_ui::paint_job`).
    let push = |job: &mut LayoutJob, text: &str, format: TextFormat| {
        let leading = if job.sections.is_empty() { phase } else { 0.0 };
        job.append(text, leading, format);
    };
    let splice_at = splice.map(|(pos, _, _)| pos.offset);
    let mut spliced = false;
    let mut decorations: Vec<Decoration> = Vec::new();
    let mut boxes = images.iter();
    let mut placed = Vec::new();
    let emit_splice = |job: &mut LayoutJob,
                       position: usize,
                       insert: &mut Option<(usize, usize)>,
                       spliced: &mut bool| {
        if let Some((_, text, composing)) = splice
            && splice_at == Some(position)
            && !*spliced
        {
            *spliced = true;
            let mut format = body
                .style
                .format(if *composing { color } else { colors.ghost });
            if *composing {
                format.underline = Stroke::new(1.0, color);
            }
            *insert = Some((position, text.chars().count()));
            push(job, text, format);
        }
    };
    for run in &runs {
        let (format, lines) = run_format(&run.marks, color, body);
        let before = job.text.chars().count();
        match &run.piece {
            Piece::Text(text) => {
                let mut chunk = String::new();
                for ch in text.chars() {
                    if splice_at == Some(position) && !spliced && !chunk.is_empty() {
                        push(&mut job, &chunk, format.clone());
                        chunk.clear();
                    }
                    emit_splice(&mut job, position, &mut insert, &mut spliced);
                    chunk.push(if ch == ' ' && spaces[position] {
                        '\u{A0}'
                    } else {
                        ch
                    });
                    position += 1;
                }
                if !chunk.is_empty() {
                    push(&mut job, &chunk, format.clone());
                }
            }
            Piece::Break => {
                emit_splice(&mut job, position, &mut insert, &mut spliced);
                push(&mut job, "\n", format.clone());
                position += 1;
            }
            Piece::Image(_) => {
                emit_splice(&mut job, position, &mut insert, &mut spliced);
                let advance = boxes.next().map_or(0.0, ImageBox::advance);
                let mut glyph = body.style.format(Color32::TRANSPARENT);
                glyph.font_id.size = (advance / space_em).max(0.01);
                glyph.line_height = Some(body.line());
                placed.push((job.text.chars().count(), position));
                push(&mut job, " ", glyph);
                position += 1;
                // Chromium draws no text decoration across an image.
                continue;
            }
        }
        // The run's decorations cover what it added (a splice inside it too, as the browser
        // composes inside the element).
        let galley_chars = job.text.chars().count();
        for (kind, line_color) in lines {
            match decorations.last_mut() {
                Some(last)
                    if last.kind == kind
                        && last.color == line_color
                        && last.chars.end == before =>
                {
                    last.chars.end = galley_chars
                }
                _ => decorations.push(Decoration {
                    chars: before..galley_chars,
                    kind,
                    color: line_color,
                }),
            }
        }
    }
    emit_splice(&mut job, position, &mut insert, &mut spliced);
    if job.sections.is_empty() {
        // An empty line still has a line box.
        job.append("", phase, body.style.format(color));
    }
    BlockJob {
        job,
        insert,
        decorations,
        images: placed,
    }
}

/// The egui format for text inside `marks`, and the decoration lines over it. A line takes the
/// colour of the element that draws it (`<s><span class="colour">` strikes in the text colour).
fn run_format(
    marks: &[crate::model::Element],
    base: Color32,
    body: &BodyText,
) -> (TextFormat, Vec<(DecorationKind, Color32)>) {
    let bold = marks.iter().any(|mark| {
        mark.is("b")
            || mark.attr("style").is_some_and(|style| {
                style.contains("font-weight:700") || style.contains("font-weight: bold")
            })
    });
    let italic = marks.iter().any(|mark| {
        mark.is("i")
            || mark
                .attr("style")
                .is_some_and(|style| style.contains("font-style:italic"))
    });
    let mut font = Font::new(body.style.font.family, if bold { 700 } else { 400 });
    if italic {
        font = font.italic();
    }
    let style = TextStyle { font, ..body.style };
    let mut color = base;
    let mut lines = Vec::new();
    for mark in marks {
        if mark.is("span")
            && mark
                .attr("class")
                .is_some_and(|class| class.split_whitespace().any(|class| class == "colour"))
        {
            color = match Color::of(mark) {
                Some(Color::None) => base,
                _ => mark.attr("style").and_then(css_color).unwrap_or(color),
            };
        } else if let Some(value) = mark.attr("style").and_then(css_color) {
            color = value;
        }
        // The `sh-` spelling comes in senders' signatures and in drafts begun elsewhere.
        if mark.is("a")
            && mark.attr("href").is_some()
            && !mark.attr("class").is_some_and(|class| {
                class.contains("exo-preserve-color") || class.contains("sh-preserve-color")
            })
        {
            color = LINK;
        }
        if mark.is("u")
            && !lines
                .iter()
                .any(|(kind, _)| *kind == DecorationKind::Underline)
        {
            lines.push((DecorationKind::Underline, color));
        }
        if mark.is("s")
            && !lines
                .iter()
                .any(|(kind, _)| *kind == DecorationKind::LineThrough)
        {
            lines.push((DecorationKind::LineThrough, color));
        }
    }
    (style.format(color), lines)
}

/// The `color` of a `style` attribute.
fn css_color(style: &str) -> Option<Color32> {
    let value = style.split(';').find_map(|declaration| {
        let (property, value) = declaration.split_once(':')?;
        (property.trim() == "color").then(|| value.trim().trim_end_matches("!important").trim())
    })?;
    let [r, g, b, a] = exo_mail::css_color(value)?;
    Some(Color32::from_rgba_unmultiplied(r, g, b, a))
}
