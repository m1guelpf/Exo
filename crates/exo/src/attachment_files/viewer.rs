//! Exo's attachment viewer: the window darkened to .9 black, the file in the middle (an image as it
//! is, at most 80% of the page either way; a message as a white card), a header with `Download`,
//! the file's type and name and a close cross, chevrons to the previous and next file, and `N of M`
//! for a second after each move. Images and messages show here; PDFs and the rest open in Quick
//! Look.
//!
//! Esc closes it, ← and → move, Enter downloads the file. A click on the image downloads it, on
//! a chevron moves, on the message card does nothing, anywhere else closes. The image drags out
//! as its file. A message's text selects and ⌘C copies it, its links open and its body scrolls
//! under the header.

use std::sync::mpsc;

use egui::{
    Color32, ColorImage, Context, CornerRadius, Mesh, Order, Painter, Pos2, Rect, Sense, Shape,
    Stroke, TextureHandle, TextureOptions, Ui, Vec2, pos2, text::LayoutJob, vec2,
};
use exo_commands::{Command, Commands, Region};
use exo_store::{Attachment, Contact, MessageDetail};
use exo_ui::{BoxShadow, Family, Font, TextStyle, motion};

use super::eml::{self, Eml};
use crate::account::Loader;
use crate::body::{Bodies, Event, Granularity, Hit, Look};

/// What the viewer's keys and clicks ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Close,
    Previous,
    Next,
    /// Download the file shown.
    Download,
    /// Download the message card's `n`th file.
    SaveFile(usize),
    /// ⌘C: the message's selected text to the clipboard.
    Copy,
}

/// What the pointer asked of the viewer: one of its actions, or what the app does for it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Output {
    Action(Action),
    /// A link in a message, clicked (with ⌘ held: opened in the background).
    Link {
        url: String,
        background: bool,
    },
    /// The image, dragged out from this point.
    Drag(Pos2),
}

/// Its keys, while it's open.
pub(crate) fn register(commands: &mut Commands<crate::Action>) {
    const VIEWER: &[Region] = &[Region::AttachmentPreview];
    let key = |id: &'static str, key: &'static str, action: Action| {
        Command::shortcut(id, crate::Action::Preview(action))
            .key(key)
            .regions(VIEWER)
    };
    commands.add(key("ATTACHMENT_PREVIEW_CLOSE", "esc", Action::Close));
    commands.add(key("ATTACHMENT_PREVIEW_PREVIOUS", "left", Action::Previous));
    commands.add(key("ATTACHMENT_PREVIEW_NEXT", "right", Action::Next));
    commands.add(key(
        "ATTACHMENT_PREVIEW_DOWNLOAD",
        "enter",
        Action::Download,
    ));
    // A copy of what's selected in the card.
    commands.add(key("ATTACHMENT_PREVIEW_COPY", "mod+c", Action::Copy));
}

/// The backdrop: black at .9.
const BACKDROP: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 230);
/// The overlay fades in and out over 200ms `ease`; the viewer is gone 150ms into the fade out.
const FADE: f64 = 0.2;
const GONE_AFTER: f64 = 0.15;
/// An item slides over 300ms on `cubic-bezier(0.19, 1, 0.22, 1)`; an image's container scales back
/// up over 250ms on the same curve.
const SLIDE: f64 = 0.3;
const SCALE: f64 = 0.25;
const EXPO: motion::CubicBezier = motion::CubicBezier::new(0.19, 1.0, 0.22, 1.0);
/// The counter shows for a second after a move, then fades over .2s `ease-out`.
const COUNTER_FOR: f64 = 1.0;
const COUNTER_FADE: f64 = 0.2;
/// The header's width, 8px down the page.
const HEADER_WIDTH: f32 = 729.0;
const HEADER_TOP: f32 = 8.0;
/// The close button's 67.5px left margin.
const CLOSE_MARGIN: f32 = 67.5;
/// 12px/12px bold, `top: 3px`: the header's words, the counter and the alt text.
const LABEL: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 12.0);
const NUDGE: f32 = 3.0;
/// The message card's subject: 21px/32px, `top: 3px`, `margin-bottom: 16px`.
const SUBJECT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 21.0, 32.0);
/// A header row: 14px/24px bold, `top: 1px`.
const HEADER_ROW: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 14.0, 24.0);
/// A header row's label is at least 60px wide: the value follows it on the row's first line, and
/// wraps to the row's left edge.
const LABEL_WIDTH: f32 = 60.0;
/// The message card: `padding: 24px 32px 0; width: min(72vw, 720px)`.
const CARD_PADDING_TOP: f32 = 24.0;
const CARD_PADDING_X: f32 = 32.0;
const CARD_WIDTH: f32 = 720.0;
/// The card's body: `margin-top: 20px; padding: 19px 0 32px; border-top: 1px`.
const SCROLL_MARGIN: f32 = 20.0;
const BODY_TOP: f32 = 20.0;
const BODY_BOTTOM: f32 = 32.0;
/// The card's files: `margin-top: 16px; padding-top: 15px; border-top: 1px`, a grid of cards at
/// least 120px wide, 12px apart.
const FILES_TOP: f32 = 32.0;
const FILE_MIN: f32 = 120.0;
const FILE_GAP: f32 = 12.0;
/// `0 15px 12px rgba(0,0,0,.22), 0 19px 38px rgba(0,0,0,.3)`, under the image and the card.
const SHADOWS: [BoxShadow; 2] = [
    BoxShadow::new(
        [0.0, 15.0],
        12.0,
        0.0,
        Color32::from_rgba_premultiplied(0, 0, 0, 56),
    ),
    BoxShadow::new(
        [0.0, 19.0],
        38.0,
        0.0,
        Color32::from_rgba_premultiplied(0, 0, 0, 77),
    ),
];
/// The longest side an image keeps on the GPU.
const MAX_TEXTURE: u32 = 4096;

/// One file as the viewer has it.
enum Item {
    /// Its bytes are on their way, or didn't come (`Loading…` stays).
    Loading,
    /// They couldn't be had offline (`This image is not available offline.`).
    Offline,
    /// Its natural size in CSS px, and whether it has no transparency.
    Image {
        texture: TextureHandle,
        size: Vec2,
        opaque: bool,
    },
    Eml(Box<Eml>),
    /// A message that couldn't be read (`Unable to load .eml preview.`).
    Unreadable,
}

/// What a fetch for the viewer came back with.
enum Loaded {
    Image(ColorImage, Vec2, bool),
    Eml(Box<Eml>),
    Failed { offline: bool },
    Unreadable,
}

/// The viewer, open over some files.
pub(crate) struct Viewer {
    attachments: Vec<Attachment>,
    items: Vec<Item>,
    position: usize,
    opened_at: f64,
    closing_at: Option<f64>,
    /// When the counter last showed.
    counter_at: f64,
    /// The file that moved into view: how far from its place it started, and when.
    slide: Option<(f32, f64)>,
    /// How far the message card's body is scrolled, and where the card was last drawn.
    scroll: f32,
    card: Option<Rect>,
    /// The page it was last drawn over, which a move slides across.
    page: Rect,
    /// The account's time, which a message's date is shown in.
    now: jiff::Zoned,
    /// The messages' bodies, rendered as the thread's are, by a worker of the viewer's own.
    bodies: Bodies,
    rendered: Vec<String>,
    loaded: Channel<(usize, Loaded)>,
    /// The card's subject and header text, selectable.
    selection: exo_ui::TextSelection,
    /// The last press in the body: when, where, and how many in a row (a double or triple
    /// click).
    clicks: Option<(f64, Pos2, u32)>,
    /// A drag in the body is selecting its text.
    selecting: bool,
    /// ⌘ was held on the last click in the body, which a link it lands on opens in the
    /// background.
    command_click: bool,
    /// The body's scrollbar.
    scrollbar: exo_ui::Scrollbar,
}

/// A channel's two ends.
type Channel<T> = (mpsc::Sender<T>, mpsc::Receiver<T>);

impl Viewer {
    /// The viewer over `attachments` from the `start`th, every file's bytes fetched at once.
    pub(crate) fn open(
        ctx: &Context,
        loader: &Loader,
        attachments: Vec<Attachment>,
        start: usize,
        now: f64,
    ) -> Self {
        let loaded = mpsc::channel();
        for (at, attachment) in attachments.iter().enumerate() {
            let (sender, ctx, is_eml) = (loaded.0.clone(), ctx.clone(), super::is_eml(attachment));
            loader.attachment(attachment, move |bytes| {
                // Decoding a photo takes a while; the account's tasks have better to do.
                std::thread::spawn(move || {
                    let loaded = match bytes {
                        Err(error) => Loaded::Failed {
                            offline: matches!(error, exo_store::Error::Offline(_)),
                        },
                        Ok(bytes) if is_eml => eml::parse(&bytes)
                            .map_or(Loaded::Unreadable, |eml| Loaded::Eml(Box::new(eml))),
                        Ok(bytes) => decode(&bytes).map_or(
                            Loaded::Failed { offline: false },
                            |(image, size, opaque)| Loaded::Image(image, size, opaque),
                        ),
                    };
                    let _ = sender.send((at, loaded));
                    ctx.request_repaint();
                });
            });
        }
        Self {
            items: attachments.iter().map(|_| Item::Loading).collect(),
            position: start.min(attachments.len().saturating_sub(1)),
            attachments,
            opened_at: now,
            closing_at: None,
            counter_at: now,
            slide: None,
            scroll: 0.0,
            card: None,
            page: ctx.content_rect(),
            now: jiff::Zoned::now(),
            bodies: Bodies::new(ctx).cleaned_only(),
            rendered: Vec::new(),
            loaded,
            selection: exo_ui::TextSelection::default(),
            clicks: None,
            selecting: false,
            command_click: false,
            scrollbar: exo_ui::Scrollbar::default(),
        }
    }

    /// The file shown.
    pub(crate) fn current(&self) -> Option<&Attachment> {
        self.attachments.get(self.position)
    }

    /// The message card's `n`th file: its name and bytes.
    pub(crate) fn file(&self, n: usize) -> Option<(String, Vec<u8>)> {
        let Some(Item::Eml(eml)) = self.items.get(self.position) else {
            return None;
        };
        eml.files
            .get(n)
            .map(|(name, _, bytes)| (name.clone(), bytes.clone()))
    }

    /// The viewer is closing (it gave the keyboard back) or has gone.
    pub(crate) fn is_closing(&self) -> bool {
        self.closing_at.is_some()
    }

    /// Its fade out is over.
    pub(crate) fn is_gone(&self, now: f64) -> bool {
        self.closing_at.is_some_and(|at| now >= at + GONE_AFTER)
    }

    /// Does `action` at `now`; downloads are the caller's.
    pub(crate) fn run(&mut self, action: Action, now: f64) {
        let page = self.page;
        match action {
            Action::Close => {
                self.closing_at.get_or_insert(now);
            }
            Action::Previous if self.position > 0 => self.move_to(self.position - 1, page, now),
            Action::Next if self.position + 1 < self.attachments.len() => {
                self.move_to(self.position + 1, page, now)
            }
            Action::Previous
            | Action::Next
            | Action::Download
            | Action::SaveFile(_)
            | Action::Copy => {}
        }
    }

    /// ⌘C: the card's selected header text, else the body's (which [`Self::show`] puts on the
    /// clipboard once the body's worker has it).
    pub(crate) fn copy(&self, ctx: &Context) {
        match self.selection.text() {
            Some(text) => ctx.copy_text(text),
            None => self.bodies.copy(),
        }
    }

    /// Shows the `to`th file, sliding in from where it waited: an image beside the last one, as
    /// wide as it would show at .85; anything else from the page's left edge, where it had no
    /// place. The counter shows again.
    fn move_to(&mut self, to: usize, page: Rect, now: f64) {
        let vw = page.width();
        let from = match &self.items[to] {
            Item::Image { size, .. } => {
                let scaled = scaled_width(*size, page);
                if to > self.position {
                    (1.0 - (0.8 - 0.37)) * vw + scaled / 2.0
                } else {
                    -(0.37 * vw + scaled / 2.0)
                }
            }
            _ => 0.0,
        };
        self.slide = Some((from - 0.1 * vw, now));
        self.position = to;
        self.scroll = 0.0;
        self.counter_at = now;
        self.selection.clear();
        self.bodies.clear_selection();
    }

    /// Takes in the files that arrived and hands the messages' bodies to the renderer; returns a
    /// link a click in a body landed on.
    fn poll(&mut self, ctx: &Context, loader: &Loader, account: &str) -> Option<Output> {
        while let Ok((at, loaded)) = self.loaded.1.try_recv() {
            self.items[at] = match loaded {
                Loaded::Image(image, size, opaque) => Item::Image {
                    texture: ctx.load_texture(
                        format!("exo-preview-{at}"),
                        image,
                        TextureOptions::LINEAR,
                    ),
                    size,
                    opaque,
                },
                Loaded::Eml(eml) => Item::Eml(eml),
                Loaded::Failed { offline: true } => Item::Offline,
                Loaded::Failed { offline: false } => Item::Loading,
                Loaded::Unreadable => Item::Unreadable,
            };
        }
        let messages: Vec<MessageDetail> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(at, item)| match item {
                Item::Eml(eml) => Some(message(&body_id(at), eml)),
                _ => None,
            })
            .collect();
        let ids: Vec<String> = messages.iter().map(|message| message.id.clone()).collect();
        if ids != self.rendered {
            self.bodies
                .set_thread(&messages.iter().collect::<Vec<_>>(), account, loader);
            self.rendered = ids;
        }
        let mut link = None;
        for event in self.bodies.poll(ctx) {
            match event {
                // `#id` scrolls the card to it; anything else is the app's to open.
                Event::Clicked {
                    id,
                    hit: Hit::Link { href },
                } if href.starts_with('#') => self.bodies.find_anchor(&id, &href[1..]),
                Event::Clicked {
                    hit: Hit::Link { href },
                    ..
                } => {
                    link = Some(Output::Link {
                        url: href,
                        background: self.command_click,
                    })
                }
                // At the scrollable's top, under its 19px of padding.
                Event::Anchored { top, .. } => self.scroll = BODY_TOP - 1.0 + top,
                Event::Copied(text) => ctx.copy_text(text),
                _ => {}
            }
        }
        link
    }

    /// Draws the viewer over `page` (the window under the tab bar; `window_height` is the whole
    /// window's) at `now` (`zoned` in the account's time zone), and returns what a click asked
    /// for.
    #[expect(
        clippy::too_many_arguments,
        reason = "the viewer drawn over the app, from its state"
    )]
    pub(crate) fn show(
        &mut self,
        ctx: &Context,
        page: Rect,
        window_height: f32,
        loader: &Loader,
        account: &str,
        now: f64,
        zoned: &jiff::Zoned,
    ) -> Option<Output> {
        let ctx = ctx.clone();
        self.page = page;
        self.now = zoned.clone();
        let link = self.poll(&ctx, loader, account);
        let opacity = match self.closing_at {
            Some(at) => 1.0 - motion::CubicBezier::EASE.at((((now - at) / FADE) as f32).min(1.0)),
            None => motion::CubicBezier::EASE.at((((now - self.opened_at) / FADE) as f32).min(1.0)),
        };
        if opacity < 1.0 || self.closing_at.is_some() {
            ctx.request_repaint();
        }
        egui::Area::new(exo_ui::id(&ctx, "exo::attachment-preview"))
            .order(Order::Middle)
            .fixed_pos(page.min)
            .show(&ctx, |ui| {
                ui.set_clip_rect(page);
                let backdrop = ui.allocate_rect(page, Sense::click());
                let mut painter = ui.painter().clone();
                painter.multiply_opacity(opacity);
                painter.rect_filled(page, 0.0, BACKDROP);
                let item = self.paint_item(ui, &painter, page, window_height, now);
                paint_shadows(&painter, page);
                let header = self.paint_header(ui, &painter, page);
                let chevrons = self.paint_chevrons(ui, &painter, page);
                self.paint_counter(ui, &painter, page, now);
                let on_card = self
                    .card
                    .zip(ui.ctx().pointer_interact_pos())
                    .is_some_and(|(card, pos)| card.contains(pos));
                // A click on the message card stays there: it doesn't close the viewer.
                let clicked = header
                    .or(chevrons)
                    .map(Output::Action)
                    .or(item)
                    .or_else(|| {
                        (backdrop.clicked() && !on_card).then_some(Output::Action(Action::Close))
                    });
                clicked.or(link)
            })
            .inner
    }

    /// Where the item's left edge is at `now` (10vw in, or on its way there) and the scale an
    /// arriving image's container is at.
    fn item_left(&self, page: Rect, now: f64) -> (f32, f32) {
        let rest = page.left() + 0.1 * page.width();
        match self.slide {
            Some((from, at)) if now - at < SLIDE => {
                let scale = 0.85 + 0.15 * EXPO.at((((now - at) / SCALE) as f32).min(1.0));
                (
                    rest + from * (1.0 - EXPO.at(((now - at) / SLIDE) as f32)),
                    scale,
                )
            }
            _ => (rest, 1.0),
        }
    }

    /// The items' top: a tenth of the page, at least 60px, on the 4px grid.
    fn item_top(page: Rect) -> f32 {
        page.top() + 4.0 * ((0.1 * page.height()).max(60.0) / 4.0).round()
    }

    /// Paints the file shown, and returns what a click or a drag on it asked for.
    fn paint_item(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        page: Rect,
        window_height: f32,
        now: f64,
    ) -> Option<Output> {
        let (left, scale) = self.item_left(page, now);
        if self.slide.is_some_and(|(_, at)| now - at < SLIDE) {
            ui.ctx().request_repaint();
        }
        let item = Rect::from_min_size(
            pos2(left, Self::item_top(page)),
            vec2(0.8 * page.width(), 0.8 * page.height()),
        );
        self.card = None;
        match &self.items[self.position] {
            Item::Image {
                texture,
                size,
                opaque,
            } => {
                // Centred in the item, which is at least 80% of the page tall (`display: table`).
                let shown = fit(*size, page);
                // Chromium draws the image on whole device pixels, the next row down when it's
                // centred between two.
                let rect = Rect::from_center_size(
                    pos2(
                        item.center().x,
                        item.top() + item.height().max(shown.y) / 2.0,
                    ),
                    shown * scale,
                );
                let ppp = ui.ctx().pixels_per_point();
                let snap = |at: Pos2| pos2((at.x * ppp).round() / ppp, (at.y * ppp).ceil() / ppp);
                let rect = Rect::from_min_max(snap(rect.min), snap(rect.max));
                for shadow in SHADOWS {
                    shadow.paint(painter, rect, CornerRadius::ZERO);
                }
                if !opaque {
                    paint_checkerboard(painter, rect);
                }
                painter.image(
                    texture.id(),
                    rect,
                    Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                // The image drags out as its file.
                let response = ui
                    .interact(rect, ui.id().with("preview-image"), Sense::click_and_drag())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if response.drag_started() {
                    return response.interact_pointer_pos().map(Output::Drag);
                }
                response
                    .clicked()
                    .then_some(Output::Action(Action::Download))
            }
            Item::Eml(_) => self.paint_card(ui, painter, page, window_height, left),
            other => {
                // The alt text, centred; its text has no line breaks.
                let text = match other {
                    Item::Offline => {
                        "This image is not available offline. Please check your connection and try again."
                    }
                    Item::Unreadable => "Unable to load .eml preview.",
                    _ => "Loading…",
                };
                let width = LABEL.width(ui.ctx(), text);
                LABEL.paint_line(
                    painter,
                    pos2(
                        item.center().x - width / 2.0,
                        item.center().y - LABEL.line_height / 2.0 + NUDGE,
                    ),
                    text,
                    Color32::WHITE,
                    None,
                );
                None
            }
        }
    }

    /// The message card at the item's top, its left edge `left`: the subject and header, then under
    /// a rule the body and its files, which scroll under the rule once the card reaches 80% of the
    /// window. Its text selects; returns a click on one of its files.
    fn paint_card(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        page: Rect,
        window_height: f32,
        left: f32,
    ) -> Option<Output> {
        let Item::Eml(eml) = &self.items[self.position] else {
            return None;
        };
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let width = (0.72 * page.width()).min(CARD_WIDTH);
        let x = left + (0.8 * page.width() - width) / 2.0;
        let top = Self::item_top(page);
        let content = width - 2.0 * CARD_PADDING_X;
        let content_left = x + CARD_PADDING_X;
        let subject = if eml.subject.is_empty() {
            Vec::new()
        } else {
            SUBJECT.wrap(ctx, &eml.subject, content)
        };
        let rows: Vec<(&str, Vec<LayoutJob>)> = header_rows(eml)
            .into_iter()
            .map(|(label, contacts)| {
                (
                    label,
                    header_value(ctx, content, label, &contacts, eml, &self.now),
                )
            })
            .collect();
        let rows_height: f32 = rows
            .iter()
            .map(|(_, lines)| lines.len() as f32 * HEADER_ROW.line_height)
            .sum();
        let fixed = CARD_PADDING_TOP
            + if subject.is_empty() {
                0.0
            } else {
                subject.len() as f32 * SUBJECT.line_height + 16.0
            }
            + rows_height;
        // The body as the thread renders one, on a white card whatever the theme (`Shadowbox`
        // with `theme: Snow`).
        let id = body_id(self.position);
        let look = Look {
            width: content,
            scale: ctx.pixels_per_point(),
            theme: exo_ui::Theme::Snow,
            rem: exo_ui::rem(page.width(), false),
            show_quoted: true,
            hide_original: false,
            phase: content_left - content_left.floor(),
            background: Color32::WHITE.to_array(),
        };
        let body = self.bodies.height(&id, look).unwrap_or(0.0);
        let files = files_grid(eml.files.len(), content);
        let scrolled =
            BODY_TOP + body + files.map_or(0.0, |(_, height)| FILES_TOP + height) + BODY_BOTTOM;
        let height = (fixed + SCROLL_MARGIN + scrolled).min(0.8 * window_height);
        let card = Rect::from_min_size(pos2(x, top), vec2(width, height));
        self.card = Some(card);
        // The body scrolls: what's under its border scrolls, the border doesn't.
        let viewport = Rect::from_min_max(pos2(x, top + fixed + SCROLL_MARGIN), card.max);
        let most = (scrolled - viewport.height()).max(0.0);
        if ui.rect_contains_pointer(viewport) {
            self.scroll -= ui.input(|input| input.smooth_scroll_delta.y);
        }
        self.scroll = self.scroll.clamp(0.0, most);
        // Over the card the pointer is a text cursor; links and files say otherwise.
        if ui.rect_contains_pointer(card) {
            ctx.set_cursor_icon(egui::CursorIcon::Text);
        }

        for shadow in SHADOWS {
            shadow.paint(painter, card, CornerRadius::same(4));
        }
        painter.rect_filled(card, CornerRadius::same(4), Color32::WHITE);
        self.selection.begin();
        let highlight = exo_ui::palette(ctx).text_selection_default;
        let mut y = top + CARD_PADDING_TOP;
        for line in &subject {
            // Its selection fills the line box, which `top: 3px` doesn't move.
            let galley = SUBJECT.single_line(painter, line, Color32::PLACEHOLDER, None);
            self.selection.add(
                painter,
                pos2(content_left, y),
                galley,
                SUBJECT.line_height,
                0,
                " ",
                highlight,
            );
            SUBJECT.paint_line(
                painter,
                pos2(content_left, y + 3.0),
                line,
                Color32::from_black_alpha(230),
                None,
            );
            y += SUBJECT.line_height;
        }
        if !subject.is_empty() {
            y += 16.0;
        }
        for (label, lines) in rows {
            // The labels are `::before` content, which doesn't select.
            if !label.is_empty() {
                HEADER_ROW.paint_line(
                    painter,
                    pos2(content_left, y + 1.0),
                    label,
                    Color32::from_black_alpha(128),
                    None,
                );
            }
            for (n, line) in lines.into_iter().enumerate() {
                let galley = exo_ui::layout(ctx, line.clone());
                self.selection.add(
                    painter,
                    pos2(content_left, y),
                    galley,
                    HEADER_ROW.line_height,
                    0,
                    if n == 0 { "\n" } else { " " },
                    highlight,
                );
                exo_ui::paint_job(painter, pos2(content_left, y + 1.0), line, &HEADER_ROW);
                y += HEADER_ROW.line_height;
            }
        }
        let (started, _) = self.selection.interact(ui, ui.id().with("preview-text"));
        if started {
            self.bodies.clear_selection();
        }
        painter.rect_filled(
            Rect::from_min_size(pos2(content_left, viewport.top()), vec2(content, 1.0)),
            0.0,
            Color32::from_black_alpha(20),
        );
        let inside = Rect::from_min_max(
            pos2(content_left, viewport.top() + 1.0),
            pos2(content_left + content, viewport.bottom()),
        );
        let clip = inside.intersect(painter.clip_rect());
        let clipped = painter.with_clip_rect(clip);
        let body_top = viewport.top() + BODY_TOP - self.scroll;
        let body_rect =
            Rect::from_min_size(pos2(content_left, body_top), vec2(content, body)).intersect(clip);
        self.body_pointer(ui, &id, body_rect, pos2(content_left, body_top));
        self.bodies
            .paint(&clipped, &id, pos2(content_left, body_top), clip, 1.0);
        // The scrollable's own scrollbar, at its right edge (the card's padding inside).
        self.scrollbar.show(
            ui,
            ui.id().with("preview-scrollbar"),
            inside,
            self.scroll,
            scrolled - 1.0,
        );
        let Item::Eml(eml) = &self.items[self.position] else {
            return None;
        };
        let (columns, _) = files?;
        let files_top = body_top + body + 16.0;
        clipped.rect_filled(
            Rect::from_min_size(pos2(content_left, files_top), vec2(content, 1.0)),
            0.0,
            Color32::from_black_alpha(20),
        );
        let column = (content - FILE_GAP * (columns - 1) as f32) / columns as f32;
        let size = vec2(column.min(154.16), column.min(154.16) * 0.6666);
        let mut clicked = None;
        for (n, (name, kind, _)) in eml.files.iter().enumerate() {
            let at = pos2(
                content_left + (n % columns) as f32 * (column + FILE_GAP),
                files_top + 16.0 + (n / columns) as f32 * (size.y + FILE_GAP),
            );
            let rect = Rect::from_min_size(at, size);
            let response = ui
                .interact(
                    rect.intersect(clip),
                    ui.id().with(("preview-file", n)),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let hover = motion::transition(
                ctx,
                response.id.with("hover"),
                response.hovered(),
                0.25,
                motion::CubicBezier::EASE,
            );
            let mut card_ui = ui.new_child(egui::UiBuilder::new().max_rect(inside));
            card_ui.set_clip_rect(clip);
            exo_ui::paint_attachment_card(&card_ui, rect, name, kind, hover);
            if response.clicked() {
                clicked = Some(Output::Action(Action::SaveFile(n)));
            }
        }
        clicked
    }

    /// The pointer over the message's body, `body` on screen with its top left at `origin`:
    /// the I-beam over text and the hand over links, a click's hit (a link opens), and selecting
    /// with a press and drag or a double or triple click, as in the thread.
    fn body_pointer(&mut self, ui: &Ui, id: &str, body: Rect, origin: Pos2) {
        let response = ui.interact(body, ui.id().with("preview-body"), Sense::click_and_drag());
        let local = |at: Pos2| (at - origin).to_pos2();
        match response.hover_pos() {
            Some(at) => {
                self.bodies.hover(id, local(at));
                if matches!(self.bodies.hovered(id), Some(Hit::Link { .. })) {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            }
            None => self.bodies.unhover(id),
        }
        let (pressed, time) = ui.input(|input| (input.pointer.primary_pressed(), input.time));
        if pressed && let Some(at) = response.hover_pos() {
            let count = match self.clicks {
                Some((then, place, count))
                    if time - then < crate::thread::MULTI_CLICK_DELAY
                        && place.distance(at) < crate::thread::MULTI_CLICK_DISTANCE =>
                {
                    (count + 1).min(3)
                }
                _ => 1,
            };
            self.clicks = Some((time, at, count));
            let granularity = match count {
                1 => None,
                2 => Some(Granularity::Word),
                _ => Some(Granularity::Paragraph),
            };
            if let Some(granularity) = granularity {
                self.bodies.select_from(id, local(at), granularity);
                self.selecting = true;
                self.selection.clear();
            }
        }
        let multi_click = self.clicks.is_some_and(|(.., count)| count > 1);
        // From where the button went down; after a double or triple click, the drag extends that
        // word or paragraph.
        if response.drag_started() && !multi_click {
            self.selecting = false;
            if let Some(at) = ui
                .input(|input| input.pointer.press_origin())
                .filter(|at| body.contains(*at))
            {
                self.bodies
                    .select_from(id, local(at), Granularity::Character);
                self.selecting = true;
                self.selection.clear();
            }
        }
        if response.dragged()
            && self.selecting
            && let Some(at) = response.interact_pointer_pos()
        {
            self.bodies.select_to(id, local(at));
        }
        if response.clicked()
            && !multi_click
            && let Some(at) = response.interact_pointer_pos()
        {
            self.bodies.clear_selection();
            self.command_click = ui.input(|input| input.modifiers.command);
            self.bodies.click(id, local(at));
        }
    }

    /// The header: `Download` at the left, the type and name in the middle, the cross at the right.
    /// Returns a click on `Download` or the cross.
    fn paint_header(&self, ui: &Ui, painter: &Painter, page: Rect) -> Option<Action> {
        let attachment = self.current()?;
        let ctx = ui.ctx();
        let top = page.top() + HEADER_TOP;
        let left = page.left() + (page.width() - HEADER_WIDTH) / 2.0;
        // `Download`: `margin-top: 4px; padding: 12px`, the 13px glyph 6px before the word; .7
        // hovered (150ms).
        let label_width = LABEL.width(ctx, "Download");
        let download = Rect::from_min_size(
            pos2(left, top + 4.0),
            vec2(12.0 + 13.0 + 6.0 + label_width + 12.0, 38.0),
        );
        let download_response = ui
            .interact(download, ui.id().with("preview-download"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let fade = 1.0
            - 0.3
                * motion::transition(
                    ctx,
                    download_response.id.with("hover"),
                    download_response.hovered(),
                    0.15,
                    motion::CubicBezier::EASE,
                );
        let glyph = "legacy/DEPRECATED_download";
        let icon = Rect::from_min_size(download.min + vec2(12.0, 12.0), Vec2::splat(13.0));
        exo_ui::paint_icon_on(
            painter,
            glyph,
            exo_ui::icon_rect(ctx, glyph, icon),
            Color32::WHITE.gamma_multiply(fade),
        );
        LABEL.paint_line(
            painter,
            pos2(icon.right() + 6.0, icon.top() + NUDGE),
            "Download",
            Color32::WHITE.gamma_multiply(fade),
            None,
        );
        // The close button: `margin-top: 5px; padding: 12px` round a 12×13 cross.
        let close = Rect::from_min_size(
            pos2(left + HEADER_WIDTH - 36.0, top + 5.0),
            vec2(36.0, 37.0),
        );
        let close_response = ui
            .interact(close, ui.id().with("preview-close"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let fade = 1.0
            - 0.3
                * motion::transition(
                    ctx,
                    close_response.id.with("hover"),
                    close_response.hovered(),
                    0.15,
                    motion::CubicBezier::EASE,
                );
        paint_cross(
            painter,
            Rect::from_min_size(close.min + vec2(12.0, 12.0), vec2(12.0, 13.0)),
            Color32::WHITE.gamma_multiply(fade),
        );
        // The file's type and name: `margin: 0 auto; padding: 12px`, between the two.
        let kind = extension(&attachment.filename).to_uppercase();
        let name = name_shown(&attachment.filename);
        let kind_width = if kind.is_empty() {
            0.0
        } else {
            LABEL.width(ctx, &kind) + 12.0 + 6.0
        };
        let name_width = LABEL.width(ctx, &name);
        let room = close.left() - CLOSE_MARGIN - download.right();
        let mut x = download.right() + (room - (24.0 + kind_width + name_width)) / 2.0 + 12.0;
        let text_top = top + 12.0 + 4.0 + NUDGE;
        if !kind.is_empty() {
            // The type: black at .9 on a 19px white pill from 5px above it.
            let pill = Rect::from_min_size(pos2(x, text_top - 5.0), vec2(kind_width - 6.0, 19.0));
            painter.rect_filled(
                exo_ui::pixel_snap(pill),
                CornerRadius::same(2),
                Color32::WHITE,
            );
            LABEL.paint_line(
                painter,
                pos2(x + 6.0, text_top),
                &kind,
                Color32::from_black_alpha(230),
                None,
            );
            x += kind_width;
        }
        LABEL.paint_line(painter, pos2(x, text_top), &name, Color32::WHITE, None);
        if download_response.clicked() {
            Some(Action::Download)
        } else {
            close_response.clicked().then_some(Action::Close)
        }
    }

    /// The chevrons: 27×50 strokes centred in 7vw boxes 3vw in from each side,
    /// `top: calc(40% + {top padding})`; shown only where there's somewhere to go (150ms), .7
    /// hovered.
    fn paint_chevrons(&self, ui: &Ui, painter: &Painter, page: Rect) -> Option<Action> {
        let ctx = ui.ctx();
        let vw = page.width();
        let top = page.top() + 0.4 * page.height() + (Self::item_top(page) - page.top()) - 25.0;
        let box_width = 0.07 * vw;
        let margin = 0.03 * box_width;
        let mut clicked = None;
        for (left, previous) in [
            (page.left() + 0.03 * vw, true),
            (page.right() - 0.03 * vw - box_width, false),
        ] {
            let shown = if previous {
                self.position > 0
            } else {
                self.position + 1 < self.attachments.len()
            };
            let target = Rect::from_min_size(pos2(left, top), vec2(box_width, 50.0 + 2.0 * margin));
            let response = ui.interact(
                target,
                ui.id().with(("preview-chevron", previous)),
                if shown {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            let response = if shown {
                response.on_hover_cursor(egui::CursorIcon::PointingHand)
            } else {
                response
            };
            let visible = motion::transition(
                ctx,
                response.id.with("shown"),
                shown,
                0.15,
                motion::CubicBezier::EASE,
            );
            let hovered = motion::transition(
                ctx,
                response.id.with("hover"),
                shown && response.hovered(),
                0.15,
                motion::CubicBezier::EASE,
            );
            let chevron = Rect::from_min_size(
                pos2(left + (box_width - 27.0) / 2.0, top + margin),
                vec2(27.0, 50.0),
            );
            let at = |x: f32, y: f32| chevron.min + vec2(x, y);
            let points = if previous {
                vec![at(26.042, 49.083), at(2.0, 25.042), at(26.042, 1.0)]
            } else {
                vec![at(1.307, 1.0), at(25.348, 25.042), at(1.307, 49.083)]
            };
            painter.add(Shape::line(
                points,
                Stroke::new(
                    2.0,
                    Color32::WHITE.gamma_multiply(visible * (1.0 - 0.3 * hovered)),
                ),
            ));
            if shown && response.clicked() {
                clicked = Some(if previous {
                    Action::Previous
                } else {
                    Action::Next
                });
            }
        }
        clicked
    }

    /// `N of M` at 93% of the page, for a second after a move.
    fn paint_counter(&self, ui: &Ui, painter: &Painter, page: Rect, now: f64) {
        let since = now - self.counter_at;
        let opacity = if since < COUNTER_FOR {
            1.0
        } else {
            1.0 - motion::CubicBezier::EASE_OUT
                .at((((since - COUNTER_FOR) / COUNTER_FADE) as f32).min(1.0))
        };
        if since < COUNTER_FOR + COUNTER_FADE {
            ui.ctx().request_repaint_after_secs(if since < COUNTER_FOR {
                (COUNTER_FOR - since) as f32
            } else {
                0.0
            });
        }
        if opacity <= 0.0 {
            return;
        }
        let text = format!("{} of {}", self.position + 1, self.attachments.len());
        let width = LABEL.width(ui.ctx(), &text) + 12.0;
        let counter = Rect::from_min_size(
            pos2(
                page.center().x - width / 2.0,
                page.top() + 0.93 * page.height() + 6.0,
            ),
            vec2(width, 24.0),
        );
        painter.rect_filled(
            counter,
            CornerRadius::same(2),
            Color32::from_black_alpha(230).gamma_multiply(opacity),
        );
        LABEL.paint_line(
            painter,
            counter.min + vec2(6.0, 6.0),
            &text,
            Color32::WHITE.gamma_multiply(opacity),
            None,
        );
    }
}

/// The id the `at`th file's message renders under.
fn body_id(at: usize) -> String {
    format!("exo-preview-eml-{at}")
}

/// `eml` as a message the body pipeline renders: its HTML and nothing else it reads.
fn message(id: &str, eml: &Eml) -> MessageDetail {
    MessageDetail {
        id: id.to_owned(),
        labels: Vec::new(),
        date: jiff::Timestamp::UNIX_EPOCH,
        from: eml.from.clone(),
        to: Vec::new(),
        cc: Vec::new(),
        bcc: Vec::new(),
        reply_to: Vec::new(),
        subject: eml.subject.clone(),
        snippet: String::new(),
        rfc822_id: None,
        in_reply_to: None,
        references: Vec::new(),
        list_id: None,
        list_unsubscribe: None,
        list_unsubscribe_post: None,
        headers: Vec::new(),
        body: exo_store::Body {
            html: Some(eml.html.clone()),
            text: None,
        },
        attachments: Vec::new(),
        failure: None,
        scheduled: None,
        sending: false,
    }
}

/// The header's rows (those with something in them), then the date's (an empty label).
fn header_rows(eml: &Eml) -> Vec<(&'static str, Vec<&Contact>)> {
    let mut rows: Vec<(&'static str, Vec<&Contact>)> = Vec::new();
    if let Some(from) = &eml.from {
        rows.push(("From", vec![from]));
    }
    for (label, list) in [("To", &eml.to), ("Cc", &eml.cc), ("Bcc", &eml.bcc)] {
        if !list.is_empty() {
            rows.push((label, list.iter().collect()));
        }
    }
    if eml.date.is_some() {
        rows.push(("", Vec::new()));
    }
    rows
}

/// A header row's value broken into lines `width` wide: each contact's name at .8 and address
/// at .5, after the label's 60px on the first line and from the row's left edge on the next; or the
/// date at .5 (in `now`'s zone) for the date's row (no label).
fn header_value(
    ctx: &Context,
    width: f32,
    label: &str,
    contacts: &[&Contact],
    eml: &Eml,
    now: &jiff::Zoned,
) -> Vec<LayoutJob> {
    let (light, dark) = (
        Color32::from_black_alpha(128),
        Color32::from_black_alpha(204),
    );
    let mut job = LayoutJob::default();
    if label.is_empty() {
        let date = match &eml.date {
            Some(eml::Date::At(at)) => crate::format::dates::expanded(*at, now),
            Some(eml::Date::Unparsed(text)) => text.clone(),
            None => String::new(),
        };
        job.append(&date, 0.0, HEADER_ROW.format(light));
    }
    for (n, contact) in contacts.iter().enumerate() {
        if n > 0 {
            job.append(", ", 0.0, HEADER_ROW.format(dark));
        }
        match &contact.name {
            Some(name) => {
                job.append(name, 0.0, HEADER_ROW.format(dark));
                job.append(
                    &format!(" <{}>", contact.email),
                    0.0,
                    HEADER_ROW.format(light),
                );
            }
            None => job.append(&contact.email, 0.0, HEADER_ROW.format(dark)),
        }
    }
    let indent = if label.is_empty() { 0.0 } else { LABEL_WIDTH };
    lines(ctx, &job, width, indent)
}

/// `job` broken into lines `width` wide, the first `indent` in, each a job of its own: egui's
/// own wrapping puts continuation rows off the sub-pixel position Chromium gives them.
fn lines(ctx: &Context, job: &LayoutJob, width: f32, indent: f32) -> Vec<LayoutJob> {
    let mut wrapped = job.clone();
    if let Some(first) = wrapped.sections.first_mut() {
        first.leading_space = indent;
    }
    wrapped.wrap = egui::text::TextWrapping::wrap_at_width(width);
    let galley = exo_ui::layout(ctx, wrapped);
    let bytes: Vec<usize> = job
        .text
        .char_indices()
        .map(|(at, _)| at)
        .chain(std::iter::once(job.text.len()))
        .collect();
    let mut start = 0;
    let mut lines = Vec::with_capacity(galley.rows.len());
    for row in &galley.rows {
        let end = (start + row.char_count_including_newline().0).min(bytes.len() - 1);
        let (from, to) = (bytes[start], bytes[end]);
        let mut line = LayoutJob::default();
        for section in &job.sections {
            let (a, b) = (
                section.byte_range.start.0.max(from),
                section.byte_range.end.0.min(to),
            );
            if a < b {
                line.append(&job.text[a..b], 0.0, section.format.clone());
            }
        }
        if let Some(first) = line.sections.first_mut().filter(|_| lines.is_empty()) {
            first.leading_space = indent;
        }
        lines.push(line);
        start = end;
    }
    lines
}

/// How the message card's files sit `width` wide: columns, and the grid's height.
fn files_grid(count: usize, width: f32) -> Option<(usize, f32)> {
    if count == 0 {
        return None;
    }
    let columns = (((width + FILE_GAP) / (FILE_MIN + FILE_GAP)).floor() as usize).max(1);
    let column = (width - FILE_GAP * (columns - 1) as f32) / columns as f32;
    let rows = count.div_ceil(columns) as f32;
    Some((
        columns,
        rows * column.min(154.16) * 0.6666 + (rows - 1.0) * FILE_GAP,
    ))
}

/// `filename`'s extension.
fn extension(filename: &str) -> String {
    filename
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| !extension.contains(' '))
        .unwrap_or_default()
        .to_owned()
}

/// The name the header shows: without its extension, cut at 75 characters with "…".
fn name_shown(filename: &str) -> String {
    let stem = match filename.rsplit_once('.') {
        Some((stem, extension)) if !extension.contains(' ') => stem,
        _ => filename,
    };
    if stem.chars().count() <= 75 {
        stem.to_owned()
    } else {
        format!("{}…", stem.chars().take(75).collect::<String>())
    }
}

/// An image `size` (CSS px) as it shows: whole, or shrunk to 80% of the page's width or height
/// (`max-width: 80vw; max-height: calc(.8*(100vh - tab bar))`).
fn fit(size: Vec2, page: Rect) -> Vec2 {
    size * (0.8 * page.width() / size.x)
        .min(0.8 * page.height() / size.y)
        .min(1.0)
}

/// How wide an image waiting beside the one shown is reckoned.
fn scaled_width(size: Vec2, page: Rect) -> f32 {
    let (most_wide, most_tall) = (0.8 * page.width(), 0.8 * page.height());
    if size.x < most_wide && size.y < most_tall {
        return 0.85 * size.x;
    }
    let scale = most_tall / size.y;
    if scale < most_wide / size.x {
        scale * size.x * 0.85
    } else {
        0.85 * most_wide
    }
}

/// `bytes` decoded, at most [`MAX_TEXTURE`] a side, with its natural size and whether it's
/// opaque.
fn decode(bytes: &[u8]) -> Option<(ColorImage, Vec2, bool)> {
    let image = image::load_from_memory(bytes).ok()?;
    let size = vec2(image.width() as f32, image.height() as f32);
    let image = if image.width().max(image.height()) > MAX_TEXTURE {
        image.thumbnail(MAX_TEXTURE, MAX_TEXTURE)
    } else {
        image
    };
    let opaque = !image.color().has_alpha();
    let rgba = image.to_rgba8();
    Some((
        ColorImage::from_rgba_unmultiplied(
            [rgba.width() as usize, rgba.height() as usize],
            rgba.as_raw(),
        ),
        size,
        opaque,
    ))
}

/// The side shadows: 3vw of black at .5 fading in from each side.
fn paint_shadows(painter: &Painter, page: Rect) {
    let width = 0.03 * page.width();
    let shadow = Color32::from_black_alpha(128);
    for (from, to) in [
        (page.left(), page.left() + width),
        (page.right(), page.right() - width),
    ] {
        let mut mesh = Mesh::default();
        let (top, bottom) = (page.top(), page.bottom());
        mesh.colored_vertex(pos2(from, top), shadow);
        mesh.colored_vertex(pos2(to, top), Color32::TRANSPARENT);
        mesh.colored_vertex(pos2(to, bottom), Color32::TRANSPARENT);
        mesh.colored_vertex(pos2(from, bottom), shadow);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(mesh);
    }
}

/// An image's background: 8px squares of `#d0d0d0` on white, behind an image with transparency.
fn paint_checkerboard(painter: &Painter, rect: Rect) {
    painter.rect_filled(rect, 0.0, Color32::WHITE);
    let clipped = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let (columns, rows) = (
        (rect.width() / 8.0).ceil() as usize,
        (rect.height() / 8.0).ceil() as usize,
    );
    for row in 0..rows {
        for column in (row % 2..columns).step_by(2) {
            clipped.rect_filled(
                Rect::from_min_size(
                    rect.min + vec2(column as f32 * 8.0, row as f32 * 8.0),
                    Vec2::splat(8.0),
                ),
                0.0,
                Color32::from_gray(0xd0),
            );
        }
    }
}

/// The close cross: two 1px strokes corner to corner of `rect`.
fn paint_cross(painter: &Painter, rect: Rect, color: Color32) {
    let stroke = Stroke::new(1.0, color);
    painter.line_segment(
        [rect.min + vec2(0.35, 0.85), rect.max - vec2(0.35, 0.85)],
        stroke,
    );
    painter.line_segment(
        [
            pos2(rect.left() + 0.35, rect.bottom() - 0.85),
            pos2(rect.right() - 0.35, rect.top() + 0.85),
        ],
        stroke,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_shows_whole_or_within_80_percent_of_the_page() {
        let page = Rect::from_min_size(pos2(0.0, 41.0), vec2(1440.0, 859.0));
        assert_eq!(fit(vec2(600.0, 400.0), page), vec2(600.0, 400.0));
        let big = fit(vec2(3000.0, 1000.0), page);
        assert!(
            (big.x - 1152.0).abs() < 0.01 && (big.y - 384.0).abs() < 0.01,
            "{big:?}"
        );
        // The next item waiting: 0.57vw + 0.85 × 600 / 2.
        assert_eq!(scaled_width(vec2(600.0, 400.0), page), 510.0);
        assert_eq!(Viewer::item_top(page) - page.top(), 84.0);
    }

    #[test]
    fn the_header_names_the_file_without_its_extension() {
        assert_eq!(name_shown("millbrook-lake.png"), "millbrook-lake");
        assert_eq!(extension("Room block confirmed.eml"), "eml");
        assert_eq!(
            name_shown(&format!("{}.pdf", "a".repeat(80))),
            format!("{}…", "a".repeat(75))
        );
    }
}
