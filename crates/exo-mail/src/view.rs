//! A message body laid out and painted with Blitz.
//!
//! [`BodyView`] builds one document per message that stands in for the shadow root a body renders
//! into: its stylesheet, the unquoted part with the original `<body>` attributes, the `…` toggle,
//! and the quoted part when it is shown. It then lays that out at the message column's width and
//! paints it into an RGBA buffer.

use std::{
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyrender::{AntiAliasing, ImageRenderer as _, PaintScene as _, recording::RenderCommand};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{DocumentConfig, NodeData, UserAgentStylesheet, local_name};
use blitz_html::HtmlDocument;
use blitz_traits::shell::{ColorScheme, Viewport};
use peniko::{
    Color, Fill,
    kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape as _},
};

mod find;
mod selection;

use crate::{
    Fonts, Render,
    animation::Player,
    dom::escape_attribute,
    hints,
    net::{Fetch, FetchProvider, MayAnimate},
    remote,
};

/// The stylesheet that precedes the message's own CSS.
const BODY_CSS: &str = include_str!("body.css");
/// The personal-mail restyle that follows it.
const RESTYLE_CSS: &str = include_str!("restyle.css");

/// Shadow-root isolation, recreated in a plain document: a body never sees `html`/`body` rules from
/// the sender's CSS (a shadow root has neither element), and `:host { all: initial }` stops
/// inheritance at the boundary, which `all: initial` on `<body>` does here. User-agent `!important`
/// beats any author rule, so senders can't undo it. The message column's width comes in through
/// `--exo-width` (see [`BodyView::layout`]).
///
/// `.ShadowBody` doesn't clip a too-wide body or scroll it itself: the body is painted whole and
/// the app shows the part scrolled into view (see [`Region::width`]). `flow-root` keeps the block
/// formatting context `overflow-x: auto` would make.
fn host_css(rem: f32) -> String {
    format!(
        "html, body {{ all: initial !important; display: block !important; }}
         html {{ font-size: {rem}px !important; }}
         .ShadowHTML {{ width: var(--exo-width) !important; }}
         .ShadowHTML, .ShadowBody {{ display: flow-root !important; overflow: visible !important; }}"
    )
}

/// The height of a too-wide body's horizontal scrollbar, which sits below its content
/// (`.ShadowBody::-webkit-scrollbar { height: 11px }`: a custom scrollbar takes up space).
pub const SCROLLBAR_HEIGHT: f32 = 11.0;

/// Where relative URLs resolve. The pipeline leaves none in `src`/`href`, but a sender's CSS can
/// still say `url(a.png)`; this reserved domain never resolves, and [`FetchProvider`] drops it.
pub(crate) const BASE_URL: &str = "https://exo.invalid/";

/// The tallest band painted at once, in device pixels (the rasterizer's sizes are 16-bit).
const MAX_BAND_ROWS: u32 = 8192;

/// Bodies up to this much wider than the column are shrunk to fit it…
const MAX_SHRINK: f32 = 1.25;
/// …plus this many px.
const SHRINK_AFFORDANCE: f32 = 2.0;

/// The narrowest window. Email `@media` queries see the window, not the message column, and a
/// window is never narrower than this.
const MIN_WINDOW_WIDTH: f32 = 780.0;

/// The two app themes, which decide the colors of personal mail.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    /// `Theme-light` (also tagged `Theme-snow`).
    #[default]
    Light,
    /// `Theme-dark` (`Theme-carbon`). Marketing mail stays dark-on-white: the app draws the
    /// white card behind it.
    Dark,
}

/// How a body is shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewOptions {
    /// The app theme.
    pub theme: Theme,
    /// Whether the quoted part is expanded below the `…`.
    pub show_quoted: bool,
    /// The app's root font size in px (it scales with the window width). Senders' `rem` lengths
    /// and the `1.25rem` quote indent resolve against it.
    pub rem: f32,
    /// Nothing loads from the network (`Block all remote images`, until the message's images are
    /// shown): remote images show as empty boxes, and CSS images, fonts and stylesheets stay
    /// unloaded. Inline attachments and `data:` images still show.
    pub block_remote_content: bool,
}

impl Default for ViewOptions {
    fn default() -> Self {
        Self {
            theme: Theme::Light,
            show_quoted: false,
            rem: 15.0,
            block_remote_content: false,
        }
    }
}

/// What is under a point of the body.
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    /// A link.
    Link {
        /// Its `href`: absolute after the pipeline, or `#id` for an anchor within the message. The
        /// pipeline only drops script URLs, so any other scheme can come through (`file:`, app URL
        /// schemes); open only the ones you trust.
        href: String,
    },
    /// The `…` that shows or hides the quoted part.
    QuoteToggle,
    /// A detected date (`.sh-date`), which the app offers to make an event of.
    Date {
        /// What it says (`tomorrow`, `Wed Nov 25`); [`crate::date_parts`] reads it.
        text: String,
        /// Its border box, `[left, top, width, height]` in the column's CSS px.
        bounds: [f32; 4],
    },
    /// An inline attachment image (`src="cid:…"`), by content id.
    InlineImage {
        /// The attachment's `Content-ID`, without angle brackets.
        cid: String,
    },
    /// Text that isn't any of the above: Chromium's `cursor: auto` shows the I-beam over it, and
    /// the arrow where there's no hit at all.
    Text,
}

/// What a right-click lands on, for the context menu.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextHit {
    /// The `href` of the link around the point.
    pub link: Option<String>,
    /// The image under the point, once it has its pixels.
    pub image: Option<ContextImage>,
    /// The point is on text (not a link's or an image's).
    pub text: bool,
}

/// An image a right-click landed on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextImage {
    /// Its `src`.
    pub src: String,
    /// Its picture at its own size: straight (not premultiplied) RGBA8.
    pub pixels: Image,
}

/// What a mouse press selects, by how many clicks it ends (Chromium's click count).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Granularity {
    /// One click: a caret, which a drag extends.
    #[default]
    Character,
    /// A double click: the word under the pointer, between word boundaries as UAX #29 defines
    /// them and ICU finds them for Chromium. A space, or a punctuation mark, is a word of its own.
    Word,
    /// A triple click: the paragraph under the pointer, the text of its block between line
    /// breaks (`<br>`, or newlines in preformatted text).
    Paragraph,
}

/// A part of the body to paint, and where it lands on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Region {
    /// The part's top, in CSS px from the top of the body.
    pub top: f32,
    /// Its height in CSS px.
    pub height: f32,
    /// Its left edge, in CSS px from the left of the body.
    pub left: f32,
    /// Its width in CSS px: the column's ([`BodyView::width`]), or the content's
    /// ([`BodyView::content_width`]) for a body that scrolls sideways.
    pub width: f32,
    /// The fractional part of the body's on-screen position, in CSS px. Chromium places glyphs
    /// at subpixel positions of their absolute coordinates, so a body drawn at x = 364.625 has
    /// to be painted 0.625 px to the right of a whole pixel to look the same.
    pub phase: [f32; 2],
}

/// What [`BodyView::animate`] changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Animated {
    /// The boxes (`[left, top, width, height]` in CSS px, like [`BodyView::animations`]) of the
    /// images that moved to a new frame: paint them again.
    pub changed: Vec<[f32; 4]>,
    /// When the next frame of an image on screen is due, on the clock `animate` was given.
    pub next: Option<Duration>,
}

/// An animated image and the elements showing it (all in step, as in Chromium).
struct Animation {
    player: Player,
    nodes: Vec<blitz_dom::NodeId>,
}

/// Painted pixels: premultiplied RGBA8, `width × height` device pixels, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    /// Width in device pixels.
    pub width: u32,
    /// Height in device pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

/// One message body, ready to lay out, paint and hit-test.
///
/// Layout happens in CSS px and the result is scaled when painting, the way Chromium on macOS
/// does it: line boxes and box edges snap to whole CSS px, not device pixels.
///
/// Options are fixed for a view; to show or hide the quoted part, build a new one. A view isn't
/// `Send` (Stylo's font metrics aren't): build, lay out and paint it on one thread, and send the
/// [`Image`]s elsewhere if painting shouldn't block the UI.
pub struct BodyView {
    document: HtmlDocument,
    /// The `.ShadowHTML` element, whose width is the column's and whose height is the body's.
    root: blitz_dom::NodeId,
    /// Its classes as composed, before [`BodyView::layout`] adds `at-1x`.
    root_class: String,
    theme: Theme,
    /// The message column's width, in CSS px.
    width: f32,
    scale: f32,
    /// How much a slightly-too-wide body is shrunk to fit the column (1 when it fits).
    zoom: f32,
    /// [`Self::content_width`] as of the last layout.
    content_width: f32,
    /// Responses the fetch hook has delivered, and how many were laid out.
    arrived: Option<Arc<AtomicUsize>>,
    applied: usize,
    /// Images that arrived and may be animated, not looked at yet.
    may_animate: Option<MayAnimate>,
    animations: Vec<Animation>,
    /// The inline roots the selection starts and ends in, which Blitz's ranges leave out when
    /// no text is selected there.
    selection_ends: Option<(blitz_dom::NodeId, blitz_dom::NodeId)>,
    /// The word or paragraph a double or triple click selected, which a drag extends by whole
    /// words or paragraphs.
    selected_unit: Option<(Granularity, Range<TextPoint>)>,
    /// What the last [`BodyView::find`] found, and which of it is the current match.
    found: Vec<(blitz_dom::NodeId, Range<usize>)>,
    current_match: Option<usize>,
}

impl BodyView {
    /// Builds the body for `render`. `fetch` loads what the message references (remote images,
    /// inline attachments, imported CSS); without it those stay blank.
    pub fn new(
        render: &Render,
        options: ViewOptions,
        fonts: &Fonts,
        fetch: Option<Arc<dyn Fetch>>,
    ) -> Self {
        enable_has_selector();
        let remote = !options.block_remote_content;
        let provider = fetch.map(|fetch| Arc::new(FetchProvider::new(fetch, remote)));
        let root_class = root_class(render, options);
        let html = compose(render, options, &root_class);
        let config = DocumentConfig {
            ua_stylesheets: vec![
                UserAgentStylesheet::default_sheet(),
                UserAgentStylesheet::parse(&host_css(options.rem)),
            ],
            base_url: Some(BASE_URL.to_owned()),
            font_ctx: Some(fonts.context()),
            net_provider: provider.clone().map(|provider| provider as _),
            viewport: Some(viewport(MIN_WINDOW_WIDTH, options.theme)),
            ..DocumentConfig::default()
        };
        let mut document = HtmlDocument::from_html(&html, config);
        // The selection colour: macOS's highlight colour in Snow, rgba(106, 134, 173, .996) in
        // Carbon, as the editor's.
        document.set_text_selection_color(match options.theme {
            Theme::Light => blitz_dom::util::Color::from_rgb8(0xb4, 0xd5, 0xfe),
            Theme::Dark => blitz_dom::util::Color::from_rgba8(106, 134, 173, 254),
        });
        if options.block_remote_content {
            mark_blocked_images(&mut document);
        }
        let root = document
            .query_selector(".ShadowHTML")
            .ok()
            .flatten()
            .expect("the composed document has a .ShadowHTML root");
        Self {
            document,
            root,
            root_class,
            theme: options.theme,
            width: 0.0,
            scale: 0.0,
            zoom: 1.0,
            content_width: 0.0,
            arrived: provider.as_ref().map(|provider| provider.arrived.clone()),
            applied: 0,
            may_animate: provider.map(|provider| provider.may_animate.clone()),
            animations: Vec::new(),
            selection_ends: None,
            selected_unit: None,
            found: Vec::new(),
            current_match: None,
        }
    }

    /// Lays the body out `width` CSS px wide for a display with `scale` device pixels per CSS px,
    /// and returns its height in CSS px.
    ///
    /// A body up to 25% wider than the column is shrunk to fit; wider ones keep their size and
    /// overflow ([`Self::content_width`]). Every coordinate this view takes or returns is in the
    /// column's (shrunk) CSS px.
    pub fn layout(&mut self, width: f32, scale: f32) -> f32 {
        if width != self.width {
            self.width = width;
            self.zoom = 1.0;
            self.document
                .set_viewport(viewport(width.max(MIN_WINDOW_WIDTH), self.theme));
            self.set_document_width(width);
        }
        if scale != self.scale {
            // The body CSS's `@media (max-resolution: 1dppx)` rules. Blitz sees a 1× device
            // because layout happens in CSS px, so the view says which display it's for instead.
            let class = if scale <= 1.0 {
                format!("{} at-1x", self.root_class)
            } else {
                self.root_class.clone()
            };
            self.document
                .mutate()
                .set_attribute(self.root, blitz_dom::qual_name!("class"), &class);
            self.scale = scale;
        }
        self.resolve();
        self.shrink_to_fit();
        self.height()
    }

    /// The column's width in CSS px, as last laid out.
    pub fn width(&self) -> f32 {
        self.width
    }

    /// Shrinks a body that is a little too wide for the column (once: it isn't rescaled after
    /// the images have loaded), and measures what's left wider than it.
    fn shrink_to_fit(&mut self) {
        let content = self.document_content_width();
        if self.zoom == 1.0
            && content > self.width
            && content <= self.width * MAX_SHRINK + SHRINK_AFFORDANCE
        {
            self.zoom = self.width / content;
            self.set_document_width(content);
            self.resolve();
        }
        self.content_width = self.document_content_width() * self.zoom;
    }

    fn set_document_width(&mut self, width: f32) {
        self.document
            .set_style_property(self.root, "--exo-width", &format!("{width}px"));
    }

    /// The content's width in the document's own CSS px: the right edge of the furthest box.
    fn document_content_width(&self) -> f32 {
        let mut right: f64 = 0.0;
        let mut stack = vec![self.root];
        while let Some(id) = stack.pop() {
            let Some(node) = self.document.get_node(id) else {
                continue;
            };
            if matches!(node.data, NodeData::Element(_))
                && let Some(rect) = self.document.get_client_bounding_rect(id)
            {
                right = right.max(rect.x + rect.width);
            }
            stack.extend(node.children.iter().copied());
        }
        (right as f32).max(self.width / self.zoom)
    }

    /// Applies loaded resources and lays out, then undoes Taffy's rounding: Chromium keeps
    /// fractional box positions (text lands at subpixel offsets) and snaps only what it paints,
    /// which [`BodyView::paint`] does.
    fn resolve(&mut self) {
        self.document.handle_messages();
        self.adopt_animations();
        self.document.resolve(0.0);
        let ids: Vec<_> = self
            .document
            .tree()
            .iter()
            .filter(|(_, node)| {
                matches!(
                    node.data,
                    NodeData::Element(_) | NodeData::AnonymousBlock(_) | NodeData::Document(_)
                )
            })
            .map(|(id, _)| id)
            .collect();
        for id in ids {
            if let Some(node) = self.document.get_node_mut(id) {
                *node.final_layout_mut() = *node.unrounded_layout();
            }
        }
    }

    /// Starts playing the animated images that arrived, on the `<img>` elements showing them.
    fn adopt_animations(&mut self) {
        let arrived = match self.may_animate.as_ref().map(|pending| pending.lock()) {
            Some(Ok(mut pending)) => std::mem::take(&mut *pending),
            _ => return,
        };
        let base = url::Url::parse(BASE_URL).expect("the base URL parses");
        for (url, bytes) in arrived {
            let nodes: Vec<_> = self
                .document
                .tree()
                .iter()
                .filter(|(_, node)| {
                    node.element_data().is_some_and(|element| {
                        element.name.local == local_name!("img")
                            && element
                                .attr(local_name!("src"))
                                .and_then(|src| base.join(src).ok())
                                .is_some_and(|src| src.as_str() == url)
                    })
                })
                .map(|(id, _)| id)
                .collect();
            if nodes.is_empty() {
                continue;
            }
            if let Some(player) = Player::new(bytes) {
                self.animations.push(Animation { player, nodes });
            }
        }
    }

    /// The boxes of the animated images (`[left, top, width, height]`, CSS px from the body's
    /// top left), as of the last layout.
    pub fn animations(&self) -> Vec<[f32; 4]> {
        self.animations
            .iter()
            .flat_map(|animation| animation.nodes.iter())
            .filter_map(|node| self.client_box(*node))
            .collect()
    }

    /// Moves the animated images showing in `visible` (CSS px from the body's top) to their
    /// frames at `now` (any clock, as long as it's always the same one); the others wait until
    /// they show. Paint the [`Animated::changed`] boxes again.
    pub fn animate(&mut self, now: Duration, visible: Range<f32>) -> Animated {
        let mut animated = Animated::default();
        for at in 0..self.animations.len() {
            let boxes: Vec<[f32; 4]> = self.animations[at]
                .nodes
                .iter()
                .filter_map(|node| self.client_box(*node))
                .collect();
            if !boxes
                .iter()
                .any(|[_, top, _, height]| *top < visible.end && top + height > visible.start)
            {
                continue;
            }
            let animation = &mut self.animations[at];
            if animation.player.advance(now) {
                let frame = animation.player.frame();
                let data = Arc::new(frame.as_raw().clone());
                for node in &animation.nodes {
                    if let Some(image) = self
                        .document
                        .get_node_mut(*node)
                        .and_then(|node| node.element_data_mut())
                        .and_then(|element| element.raster_image_data_mut())
                    {
                        *image = blitz_dom::node::RasterImageData::new(
                            frame.width(),
                            frame.height(),
                            data.clone(),
                        );
                    }
                }
                animated.changed.extend(boxes);
            }
            let next = self.animations[at].player.next_due();
            animated.next = match (animated.next, next) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
        animated
    }

    /// An element's border box in the column's CSS px.
    fn client_box(&self, node: blitz_dom::NodeId) -> Option<[f32; 4]> {
        let rect = self.document.get_client_bounding_rect(node)?;
        Some([rect.x, rect.y, rect.width, rect.height].map(|value| value as f32 * self.zoom))
    }

    /// The laid-out height in CSS px, with the horizontal scrollbar's when the body scrolls
    /// sideways.
    pub fn height(&self) -> f32 {
        let content = self
            .document
            .get_node(self.root)
            .map_or(0.0, |node| node.final_layout().size.height * self.zoom);
        content
            + if self.scrolls() {
                SCROLLBAR_HEIGHT
            } else {
                0.0
            }
    }

    /// The laid-out content width in CSS px: wider than the column when the message is too
    /// wide to shrink, and scrolls sideways.
    pub fn content_width(&self) -> f32 {
        self.content_width
    }

    /// Whether the body is wider than its column, so it scrolls sideways (the two widths compared
    /// in whole px, as Chromium compares them).
    pub fn scrolls(&self) -> bool {
        self.content_width.round() > self.width.round()
    }

    /// Applies resources that finished loading since the last call (see [`Fetch`]) and lays out
    /// again. Returns whether anything changed, i.e. whether to paint again.
    pub fn poll(&mut self) -> bool {
        let arrived = self
            .arrived
            .as_ref()
            .map_or(0, |arrived| arrived.load(Ordering::Acquire));
        if arrived == self.applied {
            return false;
        }
        self.applied = arrived;
        self.resolve();
        self.shrink_to_fit();
        true
    }

    /// Paints `region` of the last layout onto `background` (premultiplied RGBA; `[0; 4]` for
    /// transparent, to composite over the message card).
    pub fn paint(&mut self, region: Region, background: [u8; 4]) -> Image {
        let width = device(region.width + region.phase[0], self.scale);
        let height = device(region.height, self.scale);
        let row = width as usize * 4;
        let mut rgba = vec![0; row * height as usize];
        // The rasterizer takes 16-bit sizes, so tall bodies are painted in bands.
        let mut painted = 0;
        while painted < height {
            let rows = (height - painted).min(MAX_BAND_ROWS);
            let band = &mut rgba[painted as usize * row..(painted + rows) as usize * row];
            let top = f64::from(region.top) + f64::from(painted) / f64::from(self.scale);
            let left = f64::from(region.left);
            self.paint_band([left, top], region.phase, width, rows, background, band);
            painted += rows;
        }
        Image {
            width,
            height,
            rgba,
        }
    }

    /// Paints `width` × `rows` device pixels whose top left is `[left, top]` CSS px into the
    /// body into `out`.
    fn paint_band(
        &mut self,
        [left, top]: [f64; 2],
        phase: [f32; 2],
        width: u32,
        rows: u32,
        background: [u8; 4],
        out: &mut [u8],
    ) {
        // Device pixels per CSS px of the document, which a shrunk body draws smaller.
        let scale = f64::from(self.scale * self.zoom);
        let [phase_x, phase_y] = phase.map(|p| f64::from(p * self.scale));
        let [left, top] = [left, top].map(|edge| edge / f64::from(self.zoom));
        // Blitz paints from a whole CSS px; the fractional rest becomes a translation.
        let [first_column, first_row] = [left, top].map(|edge| edge.max(0.0).floor());
        let css_columns = (f64::from(width) / scale + left - first_column).ceil() as u32 + 1;
        let css_rows = (f64::from(rows) / scale + top - first_row).ceil() as u32 + 1;
        let mut scene = anyrender::Scene::new();
        // Painting from a point is scrolling the viewport to it (Blitz's paint offsets move the
        // content the other way).
        self.document.set_viewport_scroll(blitz_dom::Point {
            x: first_column,
            y: first_row,
        });
        blitz_paint::paint_scene(
            &mut scene,
            &mut self.document,
            1.0,
            css_columns,
            css_rows,
            0,
            0,
        );
        self.document
            .set_viewport_scroll(blitz_dom::Point { x: 0.0, y: 0.0 });
        let transform = Affine::translate((
            phase_x - (left - first_column) * scale,
            phase_y - (top - first_row) * scale,
        )) * Affine::scale(scale);
        for command in &mut scene.commands {
            match_chromium(command, transform, scale);
        }
        let mut renderer = VelloCpuImageRenderer::new(width, rows);
        renderer.render(
            |painter| {
                if background != [0; 4] {
                    let [r, g, b, a] = background;
                    let bounds = Rect::new(0.0, 0.0, f64::from(width), f64::from(rows));
                    painter.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        Color::from_rgba8(r, g, b, a),
                        None,
                        &bounds,
                        AntiAliasing::Enabled,
                    );
                }
                painter.append_scene(scene, transform);
            },
            out,
        );
    }

    /// What is at `(x, y)` CSS px from the body's top-left corner: a link (anywhere in its box,
    /// padding included), the quote toggle, an inline image, other text, or nothing.
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        let hit = self.document.hit(x / self.zoom, y / self.zoom)?;
        let node = self.document.get_node(hit.node_id)?;
        // Parley matches a point above or below every line box to the first or last line, so a
        // "text" hit can lie in the empty part of its block (down a table cell holding an
        // `&nbsp;`, under a paragraph): that's a hit on the block, as in Chromium.
        let (node, is_text) = match node.inline_root_ancestor() {
            Some(root) if hit.is_text && !on_a_line(root, hit.y) => (root, false),
            _ => (node, hit.is_text),
        };
        let mut node = Some(node);
        while let Some(current) = node {
            if let Some(element) = current.element_data() {
                let name = &element.name.local;
                if *name == local_name!("a")
                    && let Some(href) = element.attr(local_name!("href"))
                {
                    return Some(Hit::Link {
                        href: href.to_owned(),
                    });
                }
                if *name == local_name!("img")
                    && let Some(cid) = element
                        .attr(local_name!("src"))
                        .and_then(|src| src.strip_prefix("cid:"))
                {
                    return Some(Hit::InlineImage {
                        cid: cid.to_owned(),
                    });
                }
                let has_class = |wanted: &str| {
                    element
                        .attr(local_name!("class"))
                        .is_some_and(|class| class.split_ascii_whitespace().any(|c| c == wanted))
                };
                if has_class("Exo-quoting-ellipsis") {
                    return Some(Hit::QuoteToggle);
                }
                if *name == local_name!("span")
                    && has_class("exo-date")
                    && let Some(bounds) = self.client_box(current.id)
                {
                    return Some(Hit::Date {
                        text: current.text_content(),
                        bounds,
                    });
                }
            }
            node = current
                .parent
                .and_then(|parent| self.document.get_node(parent));
        }
        is_text.then_some(Hit::Text)
    }

    /// What a right-click at `(x, y)` CSS px from the body's top-left corner lands on: the link
    /// around it, the image under it, or text.
    pub fn context_hit(&self, x: f32, y: f32) -> ContextHit {
        let mut out = ContextHit::default();
        let Some(hit) = self.document.hit(x / self.zoom, y / self.zoom) else {
            return out;
        };
        let mut node = self.document.get_node(hit.node_id);
        while let Some(current) = node {
            if let Some(element) = current.element_data() {
                let name = &element.name.local;
                if *name == local_name!("a") && out.link.is_none() {
                    out.link = element.attr(local_name!("href")).map(str::to_owned);
                }
                if *name == local_name!("img") && out.image.is_none() {
                    out.image = element.raster_image_data().map(|image| ContextImage {
                        src: element
                            .attr(local_name!("src"))
                            .unwrap_or_default()
                            .to_owned(),
                        pixels: Image {
                            width: image.width,
                            height: image.height,
                            rgba: image.data.as_ref().to_vec(),
                        },
                    });
                }
            }
            node = current
                .parent
                .and_then(|parent| self.document.get_node(parent));
        }
        out.text = hit.is_text && out.link.is_none();
        out
    }

    /// The top of what an in-message link `#name` points at, in the column's CSS px from the
    /// body's top: the first element, in document order, whose `id` or `name` is `name`
    /// (`querySelector("[id='…'], [name='…']")`). `None` if there's none.
    ///
    /// An empty inline element (the usual `<a name="…"></a>`) lays nothing out, so it takes the
    /// top of its block's first line's text, as Chromium puts it on its line (exact when it
    /// starts the block, as it does before a heading), or else of the nearest box around it.
    pub fn anchor_top(&self, name: &str) -> Option<f32> {
        let mut stack = vec![self.root];
        while let Some(id) = stack.pop() {
            let node = self.document.get_node(id)?;
            let named = node.element_data().is_some_and(|element| {
                [local_name!("id"), local_name!("name")]
                    .into_iter()
                    .any(|attribute| element.attr(attribute) == Some(name))
            });
            if named {
                if let Some([_, top, ..]) = self.client_box(id) {
                    return Some(top);
                }
                if let Some(top) = node
                    .inline_root_ancestor()
                    .and_then(|root| self.first_line_top(root))
                {
                    return Some(top);
                }
                return std::iter::successors(Some(node), |node| {
                    node.parent
                        .and_then(|parent| self.document.get_node(parent))
                })
                .find_map(|node| self.client_box(node.id))
                .map(|[_, top, ..]| top);
            }
            // Children in reverse, so the first comes off the stack first.
            stack.extend(node.children.iter().rev().copied());
        }
        None
    }

    /// The top of the text on inline root `root`'s first line (its content area, as Chromium
    /// places an inline box), in the column's CSS px.
    fn first_line_top(&self, root: &blitz_dom::Node) -> Option<f32> {
        let text = root.element_data()?.inline_layout_data.as_deref()?;
        let line = text
            .layout
            .lines()
            .next()?
            .metrics()
            .content_block_min_coord
            / text.layout.scale();
        let [_, top, ..] = self.client_box(root.id)?;
        let layout = root.unrounded_layout();
        Some(top + (layout.padding.top + layout.border.top + text.block_offset + line) * self.zoom)
    }

    /// Starts a text selection at `(x, y)` (a mouse press ending `granularity`'s clicks): a caret,
    /// or the word or paragraph there. Returns whether there was text there.
    pub fn select_from(&mut self, x: f32, y: f32, granularity: Granularity) -> bool {
        let (x, y) = (x / self.zoom, y / self.zoom);
        let selected = match granularity {
            Granularity::Character => self.document.find_text_position(x, y).map(|at| at..at),
            Granularity::Word | Granularity::Paragraph => self.text_around(x, y, granularity),
        };
        self.selected_unit = selected
            .clone()
            .filter(|_| granularity != Granularity::Character)
            .map(|unit| (granularity, unit));
        match selected {
            Some(range) => {
                self.select(range.start, range.end);
                true
            }
            None => {
                self.clear_selection();
                false
            }
        }
    }

    fn select(&mut self, anchor: TextPoint, focus: TextPoint) {
        self.document
            .set_text_selection(anchor.0, anchor.1, focus.0, focus.1);
        self.selection_ends = Some((anchor.0, focus.0));
    }

    /// The word or paragraph at `(x, y)` (the document's CSS px). As in Chromium
    /// (`SelectClosestWordFromHitTestResult`), it's the one after the caret the point puts down,
    /// so the right half of a word's last letter selects the space after it, and past a line's
    /// end its line break. A paragraph, or the break past a block's last line, runs to where
    /// the next block's text starts.
    fn text_around(&self, x: f32, y: f32, granularity: Granularity) -> Option<Range<TextPoint>> {
        let (root, at) = self.document.find_text_position(x, y)?;
        let text = self
            .document
            .get_node(root)?
            .element_data()?
            .inline_layout_data
            .as_deref()?;
        let len = text.text.len();
        let block_end = || {
            self.next_inline_root(root)
                .map_or((root, len), |next| (next, 0))
        };
        let range = match granularity {
            Granularity::Word => match parley::Cluster::from_byte_index(&text.layout, at) {
                Some(cluster) => {
                    let start = if cluster.is_word_boundary() {
                        Some(cluster)
                    } else {
                        cluster.previous_logical_word()
                    };
                    (root, start.map_or(0, |start| start.text_range().start))
                        ..(
                            root,
                            cluster
                                .next_logical_word()
                                .map_or(len, |end| end.text_range().start),
                        )
                }
                None => (root, at)..block_end(),
            },
            Granularity::Paragraph | Granularity::Character => {
                let start = text.text[..at].rfind('\n').map_or(0, |newline| newline + 1);
                let end = text.text[at..]
                    .find('\n')
                    .map(|newline| (root, at + newline + 1));
                (root, start)..end.unwrap_or_else(block_end)
            }
        };
        Some(range)
    }

    /// Extends the selection to `(x, y)` (a mouse drag): to the character there, or after a
    /// double or triple click to the far end of the word or paragraph there, keeping the one
    /// clicked (Chromium's `SelectionController`). Returns whether there was text there.
    pub fn select_to(&mut self, x: f32, y: f32) -> bool {
        let (x, y) = (x / self.zoom, y / self.zoom);
        if let Some((granularity, unit)) = self.selected_unit.clone() {
            let Some(around) = self.text_around(x, y, granularity) else {
                return false;
            };
            if self.precedes(around.start, unit.start) {
                self.select(unit.end, around.start);
            } else {
                self.select(unit.start, around.end);
            }
            return true;
        }
        if let (Some((anchor, _)), Some((focus, _))) =
            (self.selection_ends, self.document.find_text_position(x, y))
        {
            self.selection_ends = Some((anchor, focus));
        }
        self.document.extend_text_selection_to_point(x, y)
    }

    /// Whether `a` comes before `b` in the body's text.
    fn precedes(&self, a: TextPoint, b: TextPoint) -> bool {
        if a.0 == b.0 {
            return a.1 < b.1;
        }
        let document: &blitz_dom::BaseDocument = &self.document;
        let chain = |id| {
            let mut chain: Vec<_> =
                std::iter::successors(Some(id), |&id| document.get_node(id)?.parent).collect();
            chain.reverse();
            chain
        };
        let (a, b) = (chain(a.0), chain(b.0));
        let common = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
        match (
            a.get(common),
            b.get(common),
            common
                .checked_sub(1)
                .and_then(|parent| document.get_node(a[parent])),
        ) {
            (Some(a), Some(b), Some(parent)) => {
                let children = in_layout_order(parent);
                children.iter().position(|id| id == a) < children.iter().position(|id| id == b)
            }
            // One holds the other (an inline block in a line): the outer one's text comes first.
            _ => a.len() < b.len(),
        }
    }

    /// The first inline root after `root` in the body, if any.
    fn next_inline_root(&self, root: blitz_dom::NodeId) -> Option<blitz_dom::NodeId> {
        let document: &blitz_dom::BaseDocument = &self.document;
        fn first_in(
            document: &blitz_dom::BaseDocument,
            id: blitz_dom::NodeId,
        ) -> Option<blitz_dom::NodeId> {
            let node = document.get_node(id)?;
            if node
                .element_data()
                .is_some_and(|data| data.inline_layout_data.is_some())
            {
                return Some(id);
            }
            in_layout_order(node)
                .into_iter()
                .find_map(|child| first_in(document, child))
        }
        let mut current = root;
        while current != self.root {
            let parent = document.get_node(document.get_node(current)?.parent?)?;
            let siblings = in_layout_order(parent);
            let after = siblings.iter().skip_while(|&&id| id != current).skip(1);
            if let Some(next) = after.copied().find_map(|id| first_in(document, id)) {
                return Some(next);
            }
            current = parent.id;
        }
        None
    }

    /// Clears the selection.
    pub fn clear_selection(&mut self) {
        self.document.clear_text_selection();
        self.selection_ends = None;
        self.selected_unit = None;
    }

    /// The selected text, if any.
    pub fn selected_text(&self) -> Option<String> {
        self.document.get_selected_text()
    }
}

/// A place in the body's text: an inline root, and a byte offset into its laid-out text.
type TextPoint = (blitz_dom::NodeId, usize);

/// `node`'s children as laid out (an anonymous block standing for the inline children it wraps).
fn in_layout_order(node: &blitz_dom::Node) -> Vec<blitz_dom::NodeId> {
    node.layout_children
        .borrow()
        .as_ref()
        .map_or_else(|| node.children.to_vec(), |children| children.to_vec())
}

/// Whether `y` (CSS px from the top of `root`'s line boxes, as a text hit reports it) lies on one
/// of the inline root's lines.
fn on_a_line(root: &blitz_dom::Node, y: f32) -> bool {
    let Some(text) = root
        .element_data()
        .and_then(|data| data.inline_layout_data.as_deref())
    else {
        return false;
    };
    let y = y * text.layout.scale();
    text.layout.lines().any(|line| {
        let metrics = line.metrics();
        (metrics.block_min_coord..metrics.block_max_coord).contains(&y)
    })
}

/// Adjusts one painted command to where Chromium on macOS would draw it, given the transform
/// that takes it to device pixels: boxes (fills, strokes and clips that are plain rectangles)
/// snap to whole CSS px, glyphs to quarter device pixels.
fn match_chromium(command: &mut RenderCommand, to_device: Affine, scale: f64) {
    match command {
        RenderCommand::GlyphRun(run) => match_chromium_glyphs(run, to_device),
        RenderCommand::Fill(fill) => snap_rect(&mut fill.shape, to_device * fill.transform, scale),
        RenderCommand::PushClipLayer(clip) => {
            snap_rect(&mut clip.clip, to_device * clip.transform, scale)
        }
        RenderCommand::PushLayer(layer) => {
            snap_rect(&mut layer.clip, to_device * layer.transform, scale)
        }
        RenderCommand::Stroke(stroke) => {
            if let Some(line) = date_line(stroke, to_device, scale) {
                *command = line;
            }
        }
        _ => {}
    }
}

/// Draws a glyph run the way Chromium (Skia over CoreText) does on macOS: unhinted (CoreText
/// doesn't hint, and `-webkit-font-smoothing: antialiased` stops it emboldening, which is the
/// only time Blitz turns hinting off), with each glyph's origin on the nearest quarter device
/// pixel across and whole device pixel down (Skia's subpixel positioning of horizontal text,
/// `SkGlyphPositionRoundingSpec`).
///
/// On macOS anyrender_vello_cpu moves an upright run's glyphs to CoreGraphics' grid instead,
/// Safari's (x floored to 1/2 device px at body sizes, the baseline to the pixel below), which
/// put words up to ½ device px left of Chromium's. It leaves skewed runs alone, so the run gets
/// a skew no glyph can show (10⁻³⁰⁰ px per px of height), which Vello's glyph rasterizer treats
/// as none.
fn match_chromium_glyphs(run: &mut anyrender::recording::GlyphRunCommand, to_device: Affine) {
    run.hint = false;
    let transform = to_device * run.transform;
    let [x_scale, b, c, y_scale, ..] = transform.as_coeffs();
    if b != 0.0 || c != 0.0 || x_scale == 0.0 || y_scale == 0.0 {
        return;
    }
    for glyph in &mut run.glyphs {
        let origin = transform * Point::new(f64::from(glyph.x), f64::from(glyph.y));
        glyph.x += (((origin.x * 4.0).round() / 4.0 - origin.x) / x_scale) as f32;
        glyph.y += ((origin.y.round() - origin.y) / y_scale) as f32;
    }
    run.transform *= Affine::skew(UNSEEN_SKEW, 0.0);
}

/// See [`match_chromium_glyphs`].
const UNSEEN_SKEW: f64 = 1e-300;

/// Snaps an axis-aligned rectangle's edges to whole CSS px (`scale` device pixels) of its final
/// position, the way Chromium pixel-snaps box painting.
fn snap_rect(shape: &mut BezPath, full: Affine, scale: f64) {
    let [a, b, c, d, _, _] = full.as_coeffs();
    if b != 0.0 || c != 0.0 || a == 0.0 || d == 0.0 {
        return;
    }
    let Some(rect) = as_rect(shape) else {
        return;
    };
    let device = full.transform_rect_bbox(rect);
    let snap = |v: f64| (v / scale).round() * scale;
    let snapped = Rect::new(
        snap(device.x0),
        snap(device.y0),
        snap(device.x1),
        snap(device.y1),
    );
    *shape = full.inverse() * snapped.to_path(0.1);
}

/// The rectangle a path traces, if it is one (as `Rect::to_path` writes it).
fn as_rect(path: &BezPath) -> Option<Rect> {
    let mut points = Vec::with_capacity(5);
    for element in path.elements() {
        match element {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => points.push(*p),
            PathEl::ClosePath => {}
            _ => return None,
        }
    }
    if points.len() == 5 && points[4] == points[0] {
        points.pop();
    }
    let [p0, p1, p2, p3] = points.as_slice() else {
        return None;
    };
    let horizontal_first = p0.y == p1.y && p1.x == p2.x && p2.y == p3.y && p3.x == p0.x;
    let vertical_first = p0.x == p1.x && p1.y == p2.y && p2.x == p3.x && p3.y == p0.y;
    (horizontal_first || vertical_first).then(|| Rect::from_points(*p0, *p2))
}

/// The color `body.css` gives the line under detected dates.
const DATE_LINE_MARKER: [u8; 4] = [170, 170, 170, 254];

/// Turns the 1 CSS px underline `body.css` puts under detected dates into a hairline: `#aaa`, 0.5px
/// at the line's top, its ends snapped as a box's edges would be. (Blitz floors decoration
/// thickness to whole pixels of the scale it paints at, which is 1×.)
fn date_line(
    stroke: &anyrender::recording::StrokeCommand,
    to_device: Affine,
    scale: f64,
) -> Option<RenderCommand> {
    let anyrender::Paint::Solid(color) = stroke.brush else {
        return None;
    };
    if color.to_rgba8().to_u8_array() != DATE_LINE_MARKER {
        return None;
    }
    let full = to_device * stroke.transform;
    let bounds = full.transform_rect_bbox(stroke.shape.bounding_box());
    let top = bounds.y0 - stroke.style.width * scale / 2.0;
    let snap = |v: f64| (v / scale).round() * scale;
    let rect = Rect::new(snap(bounds.x0), top, snap(bounds.x1), top + scale / 2.0);
    Some(RenderCommand::Fill(anyrender::recording::FillCommand {
        fill: Fill::NonZero,
        transform: to_device.inverse(),
        brush: anyrender::Paint::Solid(Color::from_rgb8(170, 170, 170)),
        brush_transform: None,
        shape: rect.to_path(0.1),
        anti_aliasing: AntiAliasing::Enabled,
    }))
}

/// A viewport `width` CSS px wide at 1×: layout happens in CSS px (see [`BodyView`]).
fn viewport(width: f32, theme: Theme) -> Viewport {
    let scheme = match theme {
        Theme::Light => ColorScheme::Light,
        Theme::Dark => ColorScheme::Dark,
    };
    Viewport::new(width.ceil() as u32, 1000, 1.0, scheme)
}

/// CSS px to whole device pixels.
fn device(css: f32, scale: f32) -> u32 {
    (css * scale).ceil().max(1.0) as u32
}

/// Turns on Stylo's `:has()`, which Blitz leaves off and the body CSS uses (hiding
/// Outlook's `Image removed by sender.` boxes, `body.css`): a process-wide preference, which has
/// to be on before a body's stylesheets are parsed.
fn enable_has_selector() {
    style_config::set_pref!("layout.css.has-selector.enabled", true);
}

/// Marks the images that would load from the network for `body.css`'s blocked-image box (the
/// `img[src^="https://media-blocked"]` rules) and sizes each the way Chromium sizes the broken
/// image (`HTMLImageFallbackHelper::CustomStyleForAltText`): one with both dimension attributes and
/// no alt text keeps their size (and aspect ratio, which Blitz doesn't map from the attributes);
/// one with alt text, or missing a dimension, is an inline box whose width and height are dropped,
/// so it's 16px square unless it isn't laid out inline.
fn mark_blocked_images(document: &mut HtmlDocument) {
    let images = document.query_selector_all("img").unwrap_or_default();
    let blocked: Vec<_> = images
        .into_iter()
        .filter(|&id| {
            document
                .get_node(id)
                .and_then(|node| node.element_data())
                .is_some_and(|element| {
                    let attr = |name| element.attr(name).unwrap_or_default();
                    remote::is_remote(attr(local_name!("src")))
                        || remote::srcset_is_remote(attr(local_name!("srcset")))
                })
        })
        .collect();
    if blocked.is_empty() {
        return;
    }
    let marker = blitz_dom::QualName::new(None, blitz_dom::ns!(), "data-exo-blocked".into());
    let mut mutator = document.mutate();
    for &id in &blocked {
        mutator.set_attribute(id, marker.clone(), "");
    }
    drop(mutator);
    document.resolve_stylist(0.0);
    let sized: Vec<_> = blocked
        .into_iter()
        .filter_map(|id| {
            let element = document.get_node(id)?.element_data()?;
            let attr = |name| element.attr(name).unwrap_or_default();
            let dimensions =
                dimension(attr(local_name!("width"))).zip(dimension(attr(local_name!("height"))));
            let alt = !attr(local_name!("alt")).is_empty();
            let inline = document.resolved_style_value(id, "display") == "inline";
            let size = match dimensions {
                Some((width, height)) if !alt || !inline => {
                    format!("aspect-ratio: {width} / {height}")
                }
                _ if inline => "width: auto !important; height: auto !important".to_owned(),
                _ => return None,
            };
            Some((id, format!("{}; {size}", attr(local_name!("style")))))
        })
        .collect();
    let mut mutator = document.mutate();
    for (id, style) in sized {
        mutator.set_attribute(id, blitz_dom::qual_name!("style"), &style);
    }
}

/// An image dimension attribute in px (the HTML rules for parsing dimension values), or `None`
/// for a percentage, zero or no number.
fn dimension(value: &str) -> Option<f32> {
    let value = value.trim_start();
    let end = value
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(value.len());
    let number: f32 = value[..end].parse().ok()?;
    (number > 0.0 && !value[end..].starts_with('%')).then_some(number)
}

/// The classes of the `.ShadowHTML` element.
pub(crate) fn root_class(render: &Render, options: ViewOptions) -> String {
    let theme = match options.theme {
        Theme::Light => "Theme-light Theme-snow",
        Theme::Dark => "Theme-dark Theme-carbon",
    };
    let marketing = if render.unquoted_is_marketing {
        " isMarketing"
    } else {
        ""
    };
    format!("ShadowHTML {theme} Platform-mac{marketing}")
}

/// The document standing in for a body's shadow root.
fn compose(render: &Render, options: ViewOptions, root_class: &str) -> String {
    let hints = hints::stylesheet(&format!("{}{}", render.unquoted_html, render.quoted_html));
    format!(
        "<!DOCTYPE html><html><head></head><body>{}</body></html>",
        shadow_root(render, options.show_quoted, root_class, &hints, "")
    )
}

/// What a body's shadow root holds, `.ShadowHTML` down: the body stylesheet after
/// `first_css`, the unquoted part with the original `<body>` attributes, the `…` and the quoted
/// part when it's shown, then the restyle sheet and `last_css` (when there is one).
pub(crate) fn shadow_root(
    render: &Render,
    show_quoted: bool,
    root_class: &str,
    first_css: &str,
    last_css: &str,
) -> String {
    let mut wrapper = String::from("exo-color-transparent-bg ShadowWrapper");
    if !render.quoted_is_marketing {
        wrapper.push_str(" styleQuotedContent");
    }
    if !render.unquoted_is_marketing {
        wrapper.push_str(" styleUnquotedContent");
    }
    let body = &render.body_attributes;
    let mut unquoted_class = String::from("unquoted-content exo-color-black");
    if !body.has_background {
        unquoted_class.push_str(" exo-color-transparent-bg");
    }
    if let Some(class) = &body.class {
        unquoted_class.push(' ');
        unquoted_class.push_str(class);
    }
    let mut unquoted_attributes = format!(" class=\"{}\"", escape_attribute(&unquoted_class));
    if let Some(id) = &body.id {
        unquoted_attributes.push_str(&format!(" id=\"{}\"", escape_attribute(id)));
    }
    if let Some(style) = &body.style {
        unquoted_attributes.push_str(&format!(" style=\"{}\"", escape_attribute(style)));
    }
    let quoted = if !render.has_quoted_content {
        String::new()
    } else if show_quoted {
        format!(
            "<div class=\"Exo-quoting-ellipsis isOpen\"><span></span></div><div id=\"quoted-content\">{}</div>",
            render.quoted_html
        )
    } else {
        "<div class=\"Exo-quoting-ellipsis\"><span></span></div>".to_owned()
    };
    let last = if last_css.is_empty() {
        String::new()
    } else {
        format!("<style>{last_css}</style>")
    };
    format!(
        "<div class=\"{root_class}\"><div class=\"ShadowBody\">\
         <style>{first_css}</style><style>{BODY_CSS}</style>\
         <div class=\"{wrapper}\"><div{unquoted_attributes}>{unquoted}</div>{quoted}</div>\
         <style>{RESTYLE_CSS}</style>{last}\
         </div></div>",
        unquoted = render.unquoted_html,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use regex::Regex;

    use super::*;
    use crate::{Message, dom, purify, render};

    /// A small xorshift generator, so failures reproduce.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }

        fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
            items[self.below(items.len())]
        }

        fn pick_word<'a>(&mut self, words: &'a str) -> &'a str {
            let words: Vec<&str> = words.split_ascii_whitespace().collect();
            self.pick(&words)
        }
    }

    const TAGS: &str = "\
        div p span a b table tr td blockquote img br style script svg math mtext mglyph \
        malignmark mi foreignObject annotation-xml title textarea noscript xmp iframe template \
        form select option noembed plaintext desc image use o:p base link meta font pre li hr \
        video object embed body html head";
    const ATTRIBUTES: &[(&str, &[&str])] = &[
        (
            "href",
            &[
                "javascript:alert(1)",
                "&#1;javascript:x",
                " https://a.co ",
                "a@b.co",
                "x.io",
                "#q",
                "vbscript:x",
            ],
        ),
        (
            "src",
            &[
                "https://t.yesware.com/t.gif",
                "cid:a@b",
                "x.png",
                "data:image/gif;base64,R0lG",
                "\tjavascript:x",
                "cid:1f600@goomoji.gmail",
            ],
        ),
        ("xlink:href", &["javascript:x", "data:text/html,x"]),
        ("onerror", &["alert(1)"]),
        (
            "style",
            &[
                "color:red",
                "position:fixed;width:100vw",
                "display:none",
                "border:1px solid",
                "font-size:30px",
                "background:url(x)",
            ],
        ),
        (
            "class",
            &[
                "gmail_quote",
                "gmail_signature",
                "exo-date",
                "sh-date",
                "Exo-quoting-ellipsis",
                "ShadowHTML",
                "exo-external-warning-banner",
            ],
        ),
        ("align", &["center", "left"]),
        ("width", &["1", "200"]),
        ("height", &["1"]),
        ("face", &["</style><img src=x onerror=1>", "Arial\"; x"]),
        ("color", &["#ff0000", "red;}*{color:blue"]),
        ("cellpadding", &["4"]),
        ("background", &["https://x.io/b.png", "javascript:x"]),
        ("attributename", &["href"]),
        ("title", &["--><img src=x onerror=1>"]),
        ("srcset", &["https://x.io/a.png 1x"]),
    ];
    const TEXTS: &[&str] = &[
        "hello there",
        "<img src=x onerror=alert(1)>",
        "&lt;script&gt;",
        "</style>",
        "<!--",
        "-->",
        "On Mon, Nov 16, 2026 at 5:54 PM Ann Lee <ann@x.co> wrote:",
        "see you tomorrow",
        "a.com",
        "--",
        "Sent from my iPhone",
        "Ann Lee",
        "\u{0}",
        "<![CDATA[x]]>",
        "&#0;",
        "\u{2028}",
        "é✓",
        "From: Ann\nSent: Mon\nTo: Bob\nSubject: Hi",
        "Best,",
        "> quoted",
        "unsubscribe",
        "\n\n",
        // Known mutation XSS: markup that only becomes elements when parsed a second time.
        "<math><mtext><table><mglyph><style><img src=x onerror=alert(1)>",
        "<form><math><mtext></form><form><mglyph><style></math><img src onerror=alert(1)>",
        "<svg></p><style><a id=\"</style><img src=1 onerror=alert(1)>\">",
    ];

    fn html(rng: &mut Rng, depth: usize) -> String {
        let mut out = String::new();
        for _ in 0..=rng.below(4) {
            match rng.below(7) {
                0 | 1 => out.push_str(rng.pick(TEXTS)),
                2 => out.push_str(&format!("<!--{}-->", rng.pick(TEXTS))),
                3 => out.push_str(&format!("</{}>", rng.pick_word(TAGS))),
                _ => {
                    let tag = rng.pick_word(TAGS);
                    out.push_str(&format!("<{tag}"));
                    for _ in 0..rng.below(3) {
                        let (name, values) = ATTRIBUTES[rng.below(ATTRIBUTES.len())];
                        out.push_str(&format!(" {name}=\"{}\"", rng.pick(values)));
                    }
                    out.push('>');
                    if depth < 6 {
                        out.push_str(&html(rng, depth + 1));
                    }
                    if rng.below(4) != 0 {
                        out.push_str(&format!("</{tag}>"));
                    }
                }
            }
        }
        out
    }

    /// What a URL parser sees of an attribute value.
    fn compact(value: &str) -> String {
        value
            .chars()
            .filter(|&c| c > ' ' && !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase()
    }

    /// Everything the composed document gives Blitz is something the sanitizer allows.
    fn assert_sanitized(html: &str) {
        static SCRIPT_URL: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^[0-9a-z_]+script:").expect("valid regex"));
        let document = dom::parse(html);
        for element in dom::select(&document, |_| true) {
            let tag = dom::tag(&element).unwrap_or_default().to_ascii_lowercase();
            assert!(purify::TAGS.contains(tag.as_str()), "<{tag}> in {html}");
            for name in ["onerror", "onload", "onclick"] {
                assert!(!dom::has_attr(&element, name), "{name} in {html}");
            }
            for name in [
                "href",
                "src",
                "xlink:href",
                "action",
                "background",
                "poster",
            ] {
                if let Some(value) = dom::attr(&element, name) {
                    assert!(
                        !SCRIPT_URL.is_match(&compact(&value)),
                        "{name}={value:?} in {html}"
                    );
                }
            }
            if tag == "img"
                && let Some(src) = dom::attr(&element, "src")
            {
                let fine = src.is_empty()
                    || ["http:", "https:", "data:", "cid:"]
                        .iter()
                        .any(|scheme| src.starts_with(scheme));
                assert!(fine, "img src={src:?} in {html}");
            }
        }
    }

    #[test]
    fn hostile_html_stays_sanitized() {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        // `FUZZ_CASES=100000` searches harder.
        let cases = std::env::var("FUZZ_CASES").map_or(300, |n| n.parse().expect("a count"));
        for _ in 0..cases {
            let previous = Message {
                html: html(&mut rng, 0),
                from_email: "ann@x.co".into(),
                from_name: "Ann Lee".into(),
                ..Message::default()
            };
            let message = Message {
                html: format!(
                    "{}<blockquote>{}</blockquote>",
                    html(&mut rng, 0),
                    previous.html
                ),
                from_email: "bob@y.co".into(),
                from_name: "Bob".into(),
                attachment_names: vec!["a.png".into()],
                ..Message::default()
            };
            println!("{}", message.html);
            let first = render(&previous, None, &[]);
            let reply = render(&message, Some((&previous, &first)), &[]);
            for rendered in [&first, &reply] {
                let options = ViewOptions {
                    show_quoted: true,
                    ..ViewOptions::default()
                };
                assert_sanitized(&compose(rendered, options, &root_class(rendered, options)));
            }
        }
    }

    #[test]
    fn glyphs_land_on_chromiums_grid() {
        let mut run = anyrender::recording::GlyphRunCommand {
            font_data: peniko::FontData::new(peniko::Blob::new(Arc::new(Vec::<u8>::new())), 0),
            font_size: 14.0,
            hint: true,
            normalized_coords: Vec::new(),
            embolden: Default::default(),
            style: Fill::NonZero.into(),
            brush: Color::BLACK.into(),
            brush_alpha: 1.0,
            transform: Affine::translate((10.0, 20.0)),
            glyph_transform: None,
            glyphs: vec![anyrender::Glyph {
                id: 1,
                x: 0.4,
                y: 0.3,
            }],
            anti_aliasing: AntiAliasing::Enabled,
        };
        let to_device = Affine::translate((0.6, 0.0)) * Affine::scale(2.0);
        match_chromium_glyphs(&mut run, to_device);
        let transform = to_device * run.transform;
        let glyph = &run.glyphs[0];
        let origin = transform * Point::new(f64::from(glyph.x), f64::from(glyph.y));
        // 2 × 10.4 + 0.6 = 21.4 is 21.5 for Skia (CoreGraphics' grid would make it 21), and the
        // baseline's 40.6 is 41.
        assert!(
            (origin.x - 21.5).abs() < 1e-4 && (origin.y - 41.0).abs() < 1e-4,
            "{origin:?}"
        );
        assert!(!run.hint);
        // anyrender_vello_cpu only regrids runs without a skew.
        assert_ne!(transform.as_coeffs()[2], 0.0);
    }
}
