//! Message bodies on screen: exo-mail's render pipeline and Blitz view, painted as egui textures.
//! This module is the only place Exo talks to exo-mail.
//!
//! A [`exo_mail::BodyView`] isn't `Send` and laying out or painting one can take tens of
//! milliseconds, so every view lives on one worker thread. The UI sends it requests (render a
//! thread, lay a body out, paint a band of it, hit-test, select) and picks the answers up on a
//! later frame; nothing here blocks a frame. Bodies are painted in horizontal bands so a tall
//! newsletter only costs the bands on screen, and every band is cached until the body's layout,
//! selection or images change. A band covers the body's whole width, so a body wider than its
//! column scrolls sideways without painting again. Animated images play in patches painted over
//! the bands, frame by frame, while they're on screen.

pub(crate) mod fetch;

use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    time::Duration,
};

use egui::{Color32, ColorImage, Painter, Pos2, Rect, TextureHandle, TextureOptions, pos2, vec2};
use exo_mail::{BodyView, Fonts, Region, Render, ViewOptions};
use exo_store::{MessageDetail, RemoteContentBlocking};

use crate::account::Loader;

/// The height of a painted band, in CSS px.
const BAND: f32 = 512.0;
/// Painted bands kept past each end of what's on screen, for scrolling back.
const KEPT_BANDS: u32 = 2;

/// What the pipeline found in a body, for the parts of the thread view around it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// The visible text's first 200 characters (a collapsed message's snippet).
    pub snippet: String,
    /// How many characters the visible text has (the new part's text as it shows).
    pub visible_chars: usize,
    /// There's a quoted part behind a `…`.
    pub has_quoted: bool,
    /// The new part, and the quoted part, look like marketing mail.
    pub unquoted_marketing: bool,
    pub quoted_marketing: bool,
    /// Read-receipt pixels were removed: "Email tracking blocked", unless the Images setting
    /// shows all images ([`Bodies::tracking_blocked`]).
    pub tracking_blocked: bool,
    /// Something in it loads from the network.
    pub remote_images: bool,
    /// The link an Unsubscribe would open when the headers have none.
    pub unsubscribe: Option<String>,
    /// The new part of the message as HTML, which copying the message serializes.
    pub unquoted_html: String,
    /// What hides behind the `…`, which prints when it's shown.
    pub quoted_html: String,
    /// The whole rendered (sanitized) document and its `<body>`'s attributes, which a reply
    /// quotes.
    pub html: String,
    pub body_attributes: exo_mail::BodyAttributes,
    /// Every link, in document order.
    pub links: Vec<exo_mail::Link>,
    /// One-click actions, in document order.
    pub actions: Vec<exo_mail::Action>,
    /// What a Google Calendar notification says about its event.
    pub calendar: exo_mail::CalendarData,
}

/// How a body is shown. Changing any of it lays the body out again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// The column's width in CSS px.
    pub width: f32,
    /// Device pixels per CSS px.
    pub scale: f32,
    /// The app theme.
    pub theme: exo_ui::Theme,
    /// The root font size.
    pub rem: f32,
    /// The quoted part is expanded below the `…`.
    pub show_quoted: bool,
    /// The body is a notification its invite card replaces: nothing shows above the `…`, and the
    /// new part is behind it.
    pub hide_original: bool,
    /// The fractional part of the body's x on screen, in CSS px (glyphs land on quarter device
    /// pixels of their absolute position).
    pub phase: f32,
    /// The card behind the body, painted under it (premultiplied RGBA).
    pub background: [u8; 4],
}

/// What's under the pointer in a body.
pub type Hit = exo_mail::Hit;

/// What a press in a body selects: a caret, a word or a paragraph (one, two or three clicks).
pub type Granularity = exo_mail::Granularity;

/// Something a body request came back with.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A click in message `id` landed on `hit`.
    Clicked { id: String, hit: Hit },
    /// The selected text, for the clipboard.
    Copied(String),
    /// A copy found no text selected.
    NothingSelected,
    /// What conversation `thread`'s messages hold, by message id, rendered for [`Bodies::render`].
    Rendered {
        thread: String,
        facts: Vec<(String, Facts)>,
    },
    /// What an in-message link in message `id` points at starts `top` CSS px down its body
    /// ([`Bodies::find_anchor`]).
    Anchored { id: String, top: f32 },
    /// A right-click at `at` on screen in message `id` ([`Bodies::context_menu`]): what it
    /// landed on, and the text selected then, with where its first line's baseline starts on
    /// screen (roughly: the selection's box, a line's ascent down).
    ContextMenu {
        id: String,
        at: Pos2,
        hit: exo_mail::ContextHit,
        selection: Option<(String, Pos2)>,
    },
    /// The selection changed to most of a message, or more than 500 characters of it.
    SelectedMuch,
}

/// The text selection, as the worker reports it.
#[derive(Clone, Debug, PartialEq)]
struct Selected {
    /// The message it's in.
    message: String,
    /// It as HTML, for a reply that quotes it.
    html: String,
    /// Its text, for ⌘E.
    text: String,
    /// Its bounding box in the body's CSS px.
    bounds: Rect,
}

enum Job {
    /// A new thread, whose messages load their read receipts when first shown
    /// (`read_receipts`, `Show all images`).
    Thread {
        messages: Vec<MessageDetail>,
        own_domains: Vec<String>,
        loader: Loader,
        read_receipts: bool,
        options: exo_mail::RenderOptions,
    },
    /// Lay a body out, loading nothing remote when `block_remote`.
    Layout {
        id: String,
        look: Look,
        block_remote: bool,
    },
    /// Paint band `band`, `width` CSS px wide (the content's, short of the GPU's texture limit).
    Paint {
        id: String,
        generation: u64,
        band: u32,
        width: f32,
    },
    /// Play the animated images showing in `visible` up to `now`.
    Animate {
        id: String,
        generation: u64,
        now: Duration,
        visible: Range<f32>,
    },
    Hover {
        id: String,
        at: Pos2,
    },
    Click {
        id: String,
        at: Pos2,
    },
    SelectFrom {
        id: String,
        at: Pos2,
        granularity: Granularity,
    },
    SelectTo {
        id: String,
        at: Pos2,
    },
    ClearSelection,
    Copy,
    /// Find where `#name` points in message `id`.
    Anchor {
        id: String,
        name: String,
    },
    /// A right-click at `at` in message `id`, `screen` on screen: unless it's on the selection,
    /// the word under it is selected, or the selection goes.
    ContextMenu {
        id: String,
        at: Pos2,
        screen: Pos2,
    },
    /// Render another conversation's `messages`, leaving the open thread be.
    Render {
        thread: String,
        messages: Vec<MessageDetail>,
        own_domains: Vec<String>,
    },
    /// An image a body asked for arrived.
    Wake,
    /// Find (⌘F) searches every body for this, or stops.
    Find(Option<String>),
    /// Find's current match is this one of message `.0`'s (or in none of them).
    CurrentMatch(Option<(String, usize)>),
    /// Find stopped: its current match, of message `id`'s, becomes the selection.
    SelectMatch {
        id: String,
        index: usize,
    },
}

impl Job {
    /// Whether `later`, queued after this job, makes this one pointless.
    fn superseded_by(&self, later: &Self) -> bool {
        match (self, later) {
            // A later layout of the same body paints all of it again.
            (Self::Layout { id, .. } | Self::Paint { id, .. }, Self::Layout { id: later, .. })
            | (Self::Hover { id, .. }, Self::Hover { id: later, .. }) => id == later,
            // A new thread replaces everything about the old one.
            (
                Self::Layout { .. } | Self::Paint { .. } | Self::Hover { .. },
                Self::Thread { .. },
            ) => true,
            (Self::Find(_), Self::Find(_)) | (Self::CurrentMatch(_), Self::CurrentMatch(_)) => true,
            _ => false,
        }
    }

    /// The message the job is about, if it's about one.
    fn message(&self) -> Option<&str> {
        match self {
            Self::Layout { id, .. }
            | Self::Paint { id, .. }
            | Self::Animate { id, .. }
            | Self::Hover { id, .. }
            | Self::Click { id, .. }
            | Self::SelectFrom { id, .. }
            | Self::SelectTo { id, .. }
            | Self::Anchor { id, .. }
            | Self::SelectMatch { id, .. }
            | Self::ContextMenu { id, .. } => Some(id),
            Self::Thread { .. }
            | Self::ClearSelection
            | Self::Copy
            | Self::Render { .. }
            | Self::Wake
            | Self::Find(_)
            | Self::CurrentMatch(_) => None,
        }
    }
}

enum Output {
    Rendered(Vec<(String, Facts)>),
    Laid {
        id: String,
        laid: Laid,
    },
    Painted {
        id: String,
        generation: u64,
        band: u32,
        image: exo_mail::Image,
    },
    /// Animated images' new frames, painted over the bands: each a part of the body (`[left,
    /// top, width, height]`, whole CSS px) and its pixels.
    Animated {
        id: String,
        generation: u64,
        patches: Vec<([f32; 4], exo_mail::Image)>,
        next: Option<Duration>,
    },
    Hovered {
        id: String,
        hit: Option<Hit>,
    },
    Clicked {
        id: String,
        hit: Option<Hit>,
    },
    Copied(Option<String>),
    /// The worker panicked outside a job and started again, without the views it had: the
    /// thread is to be sent again.
    Restarted,
    Conversation {
        thread: String,
        facts: Vec<(String, Facts)>,
    },
    /// The selection changed, or went.
    Selected(Option<Selected>),
    Anchored {
        id: String,
        top: Option<f32>,
    },
    /// Find's matches for `query` in message `id`'s body, as boxes in its CSS px.
    Found {
        id: String,
        query: Option<String>,
        boxes: Vec<Rect>,
    },
    /// A right-click's hit, and the selection's text and box then (in the body's CSS px).
    ContextMenu {
        id: String,
        at: Pos2,
        screen: Pos2,
        hit: exo_mail::ContextHit,
        selection: Option<(String, Rect)>,
    },
}

/// The latest layout of a body.
#[derive(Clone)]
struct Laid {
    height: f32,
    generation: u64,
    /// The column width it was laid out for, in CSS px.
    width: f32,
    /// How much wider than the column it is, when it scrolls sideways (else 0), in CSS px.
    overflow: f32,
    /// Where its animated images are.
    animations: Vec<[f32; 4]>,
}

#[derive(Default)]
struct Entry {
    facts: Option<Facts>,
    laid: Option<Laid>,
    /// The last look a layout was asked for, and whether it blocked remote content.
    asked: Option<(Look, bool)>,
    bands: HashMap<u32, (u64, TextureHandle)>,
    asked_bands: HashSet<(u64, u32)>,
    hover: Option<Hit>,
    /// Where the last hover was asked: it's asked again only once the pointer or the body moves.
    hovered_at: Option<Pos2>,
    /// How far a body wider than its column is scrolled sideways, in CSS px.
    scroll_x: f32,
    /// Animated images' latest frames, by the part of the body they cover.
    patches: HashMap<[u32; 2], (u64, [f32; 4], TextureHandle)>,
    animation: AnimationState,
    /// Find's matches in it, in its CSS px, and the query they're for.
    found: Vec<Rect>,
    found_for: Option<String>,
}

/// Where a body's animations are at, on the UI side.
#[derive(Default)]
struct AnimationState {
    /// A request is out.
    asked: bool,
    /// When the next frame is due, on egui's clock.
    due: Option<f64>,
    /// The animated images on screen at the last request.
    shown: Vec<usize>,
}

/// The UI's handle on the body worker, and the textures it painted.
pub struct Bodies {
    jobs: Sender<Job>,
    outputs: Receiver<Output>,
    /// Jobs sent and not finished yet.
    pending: Arc<AtomicUsize>,
    /// The messages last sent to the pipeline, in order.
    messages: Vec<String>,
    entries: HashMap<String, Entry>,
    /// The account's Images setting.
    images: RemoteContentBlocking,
    /// The messages whose images ⌘K Show Images showed, until the setting changes.
    images_shown: HashSet<String>,
    /// The text selection, if any.
    selection: Option<Selected>,
    /// What find searches the bodies for, as last sent to the worker; the current match among
    /// theirs as sent, and as this frame's painting found it.
    searched: Option<String>,
    current_match: Option<(String, usize)>,
    painted_match: Option<(String, usize)>,
    /// What the pipeline does besides the usual: the attachment viewer's messages have no
    /// detected dates, a read-only snippet isn't classified.
    options: exo_mail::RenderOptions,
}

impl Bodies {
    /// Starts the worker; it loads the email fonts and repaints `ctx` whenever it has something
    /// new.
    pub fn new(ctx: &egui::Context) -> Self {
        let (jobs, job_receiver) = channel();
        let (output_sender, outputs) = channel();
        let pending = Arc::new(AtomicUsize::new(0));
        let (wake, worker_pending, ctx) = (jobs.clone(), pending.clone(), ctx.clone());
        // Views aren't `Send`, so the worker is built on its own thread. A panic outside a job
        // starts it again, as a background is (`recovery`).
        let spawned = std::thread::Builder::new()
            .name("exo-bodies".into())
            .spawn(move || {
                let restarts = crate::recovery::Restarts::new(ctx.clone());
                loop {
                    let worker = Worker {
                        fonts: Fonts::load(),
                        renders: HashMap::new(),
                        views: HashMap::new(),
                        fetch: None,
                        read_receipts: false,
                        receipts_loaded: HashSet::new(),
                        selected: None,
                        query: None,
                        current_match: None,
                        generation: 0,
                        wake: wake.clone(),
                        pending: worker_pending.clone(),
                    };
                    let run = std::panic::AssertUnwindSafe(|| {
                        worker.run(&job_receiver, &output_sender, &ctx)
                    });
                    if std::panic::catch_unwind(run).is_ok() {
                        return;
                    }
                    let Some(wait) = restarts.failed() else {
                        return;
                    };
                    std::thread::sleep(wait);
                    worker_pending.fetch_add(1, Ordering::SeqCst);
                    if output_sender.send(Output::Restarted).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            });
        if let Err(error) = spawned {
            log::error!("starting the body renderer: {error}");
        }
        Self {
            jobs,
            outputs,
            pending,
            messages: Vec::new(),
            entries: HashMap::new(),
            images: RemoteContentBlocking::default(),
            images_shown: HashSet::new(),
            selection: None,
            searched: None,
            current_match: None,
            painted_match: None,
            options: exo_mail::RenderOptions::default(),
        }
    }

    fn send(&self, job: Job) {
        self.pending.fetch_add(1, Ordering::SeqCst);
        if self.jobs.send(job).is_err() {
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Renders as the attachment viewer shows a message, cleaned and nothing more: no dates
    /// marked, nothing behind a `…` ([`exo_mail::RenderOptions`]).
    #[must_use]
    pub fn cleaned_only(mut self) -> Self {
        self.options = exo_mail::RenderOptions {
            detect_dates: false,
            split_quoted: false,
            ..self.options
        };
        self
    }

    /// Renders bodies unclassified: never marketing, no colors tagged for the restyle, as the
    /// read-only snippet view shows a snippet ([`exo_mail::RenderOptions::classify`]).
    #[must_use]
    pub fn unclassified(mut self) -> Self {
        self.options.classify = false;
        self
    }

    /// Nothing is in flight, so what's on screen is final. For screenshots and tests.
    pub fn is_idle(&self) -> bool {
        self.pending.load(Ordering::SeqCst) == 0
    }

    /// Renders `messages` (one thread's, oldest first) unless they're what's rendered already.
    /// `account` is the account's address, whose domain is the user's own; `loader` fetches the
    /// messages' images. Messages that stay keep their layout and painted bands until the new
    /// render replaces them, so a reply arriving neither blanks the thread nor moves its scroll.
    pub fn set_thread(&mut self, messages: &[&MessageDetail], account: &str, loader: &Loader) {
        if messages
            .iter()
            .map(|message| &message.id)
            .eq(&self.messages)
        {
            return;
        }
        self.messages = messages.iter().map(|message| message.id.clone()).collect();
        self.entries.retain(|id, _| self.messages.contains(id));
        for entry in self.entries.values_mut() {
            entry.asked = None;
            entry.hover = None;
            entry.hovered_at = None;
        }
        let own_domains = account
            .rsplit_once('@')
            .map(|(_, domain)| domain.to_lowercase())
            .into_iter()
            .collect();
        let read_receipts = self.images == RemoteContentBlocking::None;
        self.send(Job::Thread {
            messages: messages.iter().map(|message| (*message).clone()).collect(),
            own_domains,
            loader: loader.clone(),
            read_receipts,
            options: self.options,
        });
    }

    /// Follows the account's Images setting. Changing it forgets which messages' images were
    /// shown.
    pub fn set_images(&mut self, images: RemoteContentBlocking) {
        if images != self.images {
            self.images = images;
            self.images_shown.clear();
        }
    }

    /// Shows message `id`'s remote images.
    pub fn show_images(&mut self, id: &str) {
        self.images_shown.insert(id.to_owned());
    }

    /// Whether `id`'s remote content is blocked: `Block all remote images`, until shown.
    fn blocks_images(&self, id: &str) -> bool {
        self.images == RemoteContentBlocking::All && !self.images_shown.contains(id)
    }

    /// Whether ⌘K Show Images would show anything in `id`.
    pub fn can_show_images(&self, id: &str) -> bool {
        self.blocks_images(id) && self.facts(id).is_some_and(|facts| facts.remote_images)
    }

    /// Whether `id`'s header says `Email tracking blocked`: it had read receipts, which only
    /// `Show all images` loads.
    pub fn tracking_blocked(&self, id: &str) -> bool {
        self.images != RemoteContentBlocking::None
            && self.facts(id).is_some_and(|facts| facts.tracking_blocked)
    }

    /// What the pipeline found in message `id`, once it ran.
    pub fn facts(&self, id: &str) -> Option<&Facts> {
        self.entries.get(id)?.facts.as_ref()
    }

    /// Message `id`'s body as it prints (`exo_mail::printable_body`), with its quoted part when
    /// `show_quoted` and its remote content blocked as on screen; `None` until it's rendered.
    pub fn printable(&self, id: &str, show_quoted: bool) -> Option<String> {
        let facts = self.facts(id)?;
        // The parts of the render a body is composed from, which is all printing reads.
        let render = Render {
            unquoted_html: facts.unquoted_html.clone(),
            quoted_html: facts.quoted_html.clone(),
            has_quoted_content: facts.has_quoted,
            body_attributes: facts.body_attributes.clone(),
            unquoted_is_marketing: facts.unquoted_marketing,
            quoted_is_marketing: facts.quoted_marketing,
            ..Render::default()
        };
        Some(exo_mail::printable_body(
            &render,
            show_quoted,
            self.blocks_images(id),
        ))
    }

    /// The body of `id`'s height when shown with `look`, asking for a layout when the look
    /// changed. Until one arrives it's the last layout's height, or `None` the first time.
    pub fn height(&mut self, id: &str, look: Look) -> Option<f32> {
        let block_remote = self.blocks_images(id);
        let entry = self.entries.entry(id.to_owned()).or_default();
        entry.facts.as_ref()?;
        if entry.asked != Some((look, block_remote)) {
            entry.asked = Some((look, block_remote));
            let job = Job::Layout {
                id: id.to_owned(),
                look,
                block_remote,
            };
            self.send(job);
        }
        self.entries.get(id)?.laid.as_ref().map(|laid| laid.height)
    }

    /// How much wider than its column `id`'s body is, in CSS px: how far it scrolls sideways.
    pub fn overflow(&self, id: &str) -> f32 {
        self.entries
            .get(id)
            .and_then(|entry| entry.laid.as_ref())
            .map_or(0.0, |laid| laid.overflow)
    }

    /// The column width `id`'s body was last laid out for, and its content's width there, in CSS
    /// px.
    pub fn laid_width(&self, id: &str) -> Option<(f32, f32)> {
        let laid = self.entries.get(id)?.laid.as_ref()?;
        Some((laid.width, laid.width + laid.overflow))
    }

    /// How far `id`'s body is scrolled sideways, in CSS px.
    pub fn scroll_x(&self, id: &str) -> f32 {
        self.entries.get(id).map_or(0.0, |entry| entry.scroll_x)
    }

    /// Scrolls `id`'s body sideways to `x` CSS px (kept within its overflow), on a device pixel
    /// of `scale`.
    pub fn set_scroll_x(&mut self, id: &str, x: f32, scale: f32) {
        let overflow = self.overflow(id);
        if let Some(entry) = self.entries.get_mut(id) {
            entry.scroll_x = (x.clamp(0.0, overflow) * scale).round() / scale;
        }
    }

    /// Whether `id`'s body has been laid out at least once.
    pub fn height_known(&self, id: &str) -> bool {
        self.entries
            .get(id)
            .is_some_and(|entry| entry.laid.is_some())
    }

    /// Paints the part of `id`'s body inside `clip`, its top left at `origin` (less how far it's
    /// scrolled sideways), asking for the bands that aren't painted yet and dropping those far off
    /// screen, and plays its animated images on screen. `opacity` fades the whole body (a
    /// marketing card in dark mode is 90% opaque).
    pub fn paint(&mut self, painter: &Painter, id: &str, origin: Pos2, clip: Rect, opacity: f32) {
        let Some(entry) = self.entries.get_mut(id) else {
            return;
        };
        let Some(laid) = entry.laid.clone() else {
            return;
        };
        let ppp = painter.pixels_per_point();
        // The worker painted the fractional x into the band; the texture sits on the whole px
        // below it, and on a device pixel vertically (glyphs don't move vertically in Chromium).
        let left = origin.x.floor() - entry.scroll_x;
        let top = (origin.y * ppp).round() / ppp;
        let shown = (clip.y_range().min - top)..(clip.y_range().max - top);
        let visible = visible_bands(shown.start, shown.end, laid.height);
        if visible.is_empty() {
            // Out of view (painted only for find to search the thread): what's painted stays.
            return;
        }
        let kept = visible.start.saturating_sub(KEPT_BANDS)..visible.end.saturating_add(KEPT_BANDS);
        entry.bands.retain(|band, _| kept.contains(band));
        entry.asked_bands.retain(|(_, band)| kept.contains(band));
        let alpha = (opacity * 255.0).round() as u8;
        let tint = Color32::from_rgba_premultiplied(alpha, alpha, alpha, alpha);
        let whole = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
        // A band covers the content's whole width, as far as a texture can go.
        let max_width = painter.ctx().input(|input| input.max_texture_side) as f32 / ppp - 1.0;
        let width = (laid.width + laid.overflow).min(max_width);
        let mut jobs = Vec::new();
        for band in visible {
            let band_top = top + band as f32 * BAND;
            let stale = match entry.bands.get(&band) {
                Some((generation, texture)) => {
                    let [w, h] = texture.size();
                    let at = Rect::from_min_size(
                        pos2(left, band_top),
                        vec2(w as f32 / ppp, h as f32 / ppp),
                    );
                    painter
                        .with_clip_rect(clip)
                        .image(texture.id(), at, whole, tint);
                    *generation != laid.generation
                }
                None => true,
            };
            if stale && entry.asked_bands.insert((laid.generation, band)) {
                jobs.push(Job::Paint {
                    id: id.to_owned(),
                    generation: laid.generation,
                    band,
                    width,
                });
            }
        }
        for (generation, [x, y, ..], texture) in entry.patches.values() {
            if *generation == laid.generation {
                let [w, h] = texture.size();
                let at = Rect::from_min_size(
                    pos2(left + x, top + y),
                    vec2(w as f32 / ppp, h as f32 / ppp),
                );
                painter
                    .with_clip_rect(clip)
                    .image(texture.id(), at, whole, tint);
            }
        }
        // Animated images play while they show: the worker is asked for their next frames when
        // they're due, and again whenever another one scrolls into view.
        let on_screen: Vec<usize> = laid
            .animations
            .iter()
            .enumerate()
            .filter(|(_, [_, y, _, h])| *y < shown.end && y + h > shown.start)
            .map(|(at, _)| at)
            .collect();
        if !on_screen.is_empty() && !entry.animation.asked {
            let now = painter.ctx().input(|input| input.time);
            let due = entry.animation.due.is_some_and(|due| due <= now);
            if due || on_screen != entry.animation.shown {
                entry.animation.asked = true;
                entry.animation.shown = on_screen;
                jobs.push(Job::Animate {
                    id: id.to_owned(),
                    generation: laid.generation,
                    now: Duration::from_secs_f64(now),
                    visible: shown,
                });
            } else if let Some(due) = entry.animation.due {
                painter.ctx().request_repaint_after_secs((due - now) as f32);
            }
        }
        for job in jobs {
            self.send(job);
        }
    }

    /// Hit-tests `at` (CSS px from the body's top left) in `id`, unless that's what it last did
    /// and the body hasn't changed since; [`Bodies::hovered`] has the answer a frame or two later.
    pub fn hover(&mut self, id: &str, at: Pos2) {
        let Some(entry) = self.entries.get_mut(id) else {
            return;
        };
        let at = at + vec2(entry.scroll_x, 0.0);
        if entry.hovered_at.replace(at) != Some(at) {
            self.send(Job::Hover {
                id: id.to_owned(),
                at,
            });
        }
    }

    /// What was under the pointer at the last [`Bodies::hover`] of `id`.
    pub fn hovered(&self, id: &str) -> Option<&Hit> {
        self.entries.get(id)?.hover.as_ref()
    }

    /// Forgets the hover over `id` (the pointer left it).
    pub fn unhover(&mut self, id: &str) {
        if let Some(entry) = self.entries.get_mut(id) {
            entry.hover = None;
            entry.hovered_at = None;
        }
    }

    /// A click at `at` in `id`; [`Bodies::poll`] reports what it hit.
    pub fn click(&self, id: &str, at: Pos2) {
        self.send(Job::Click {
            id: id.to_owned(),
            at: self.scrolled(id, at),
        });
    }

    /// Starts a text selection at `at` in `id`, clearing any other: a caret for one click, the
    /// word or paragraph there for a double or triple click.
    pub fn select_from(&self, id: &str, at: Pos2, granularity: Granularity) {
        self.send(Job::SelectFrom {
            id: id.to_owned(),
            at: self.scrolled(id, at),
            granularity,
        });
    }

    /// Extends the selection to `at` in `id`.
    pub fn select_to(&self, id: &str, at: Pos2) {
        self.send(Job::SelectTo {
            id: id.to_owned(),
            at: self.scrolled(id, at),
        });
    }

    /// `at` (CSS px from the body's top left on screen) in the body's own coordinates.
    fn scrolled(&self, id: &str, at: Pos2) -> Pos2 {
        at + vec2(self.scroll_x(id), 0.0)
    }

    /// Clears the text selection.
    pub fn clear_selection(&self) {
        self.send(Job::ClearSelection);
    }

    /// Asks for the selected text; [`Bodies::poll`] reports it if there is any.
    pub fn copy(&self) {
        self.send(Job::Copy);
    }

    /// The text selected in message `id`, as HTML ([`exo_mail::BodyView::selected_html`]), for
    /// a reply to quote.
    pub fn selected_html(&self, id: &str) -> Option<&str> {
        self.selection
            .as_ref()
            .filter(|selected| selected.message == id)
            .map(|selected| selected.html.as_str())
    }

    /// The selected text, if any.
    pub fn selected_text(&self) -> Option<&str> {
        self.selection
            .as_ref()
            .map(|selected| selected.text.as_str())
    }

    /// The message holding the text selection, and the selection's bounding box in its body's
    /// CSS px.
    pub fn selection_bounds(&self) -> Option<(&str, Rect)> {
        self.selection
            .as_ref()
            .map(|selected| (selected.message.as_str(), selected.bounds))
    }

    /// A right-click at `at` in `id`, `screen` on screen. [`Bodies::poll`] reports what it
    /// landed on.
    pub fn context_menu(&self, id: &str, at: Pos2, screen: Pos2) {
        self.send(Job::ContextMenu {
            id: id.to_owned(),
            at: self.scrolled(id, at),
            screen,
        });
    }

    /// Finds where in-message link `#name` points in message `id`; [`Bodies::poll`] reports it
    /// if it points anywhere.
    pub fn find_anchor(&self, id: &str, name: &str) {
        self.send(Job::Anchor {
            id: id.to_owned(),
            name: name.to_owned(),
        });
    }

    /// Renders conversation `thread`'s `messages` (oldest first) for what they hold (their links,
    /// their new parts), which [`Bodies::poll`] reports; `account` as for [`Bodies::set_thread`].
    /// The open thread's bodies stay as they are.
    pub fn render(&self, thread: &str, messages: &[&MessageDetail], account: &str) {
        let own_domains = account
            .rsplit_once('@')
            .map(|(_, domain)| domain.to_lowercase())
            .into_iter()
            .collect();
        self.send(Job::Render {
            thread: thread.to_owned(),
            messages: messages.iter().map(|message| (*message).clone()).collect(),
            own_domains,
        });
    }

    /// Adds find's matches in `id`'s body, painted with its top left at `origin`, at this point in
    /// the page's order ([`exo_ui::find::add_matches`]), or says they're still to come while the
    /// worker hasn't searched it (or rendered it) yet.
    pub fn add_matches(&mut self, ctx: &egui::Context, id: &str, origin: Pos2) {
        let entry = self
            .entries
            .get(id)
            .filter(|entry| entry.found_for == self.searched);
        let Some(entry) = entry else {
            // With the worker idle nothing is coming (a message it has no render for).
            if !self.is_idle() {
                exo_ui::find::pending(ctx);
            }
            return;
        };
        let offset = origin.to_vec2() - vec2(entry.scroll_x, 0.0);
        let boxes: Vec<Rect> = entry
            .found
            .iter()
            .map(|found| found.translate(offset))
            .collect();
        if let Some(index) = exo_ui::find::add_matches(ctx, exo_ui::id(ctx, ("body", id)), &boxes) {
            self.painted_match = Some((id.to_owned(), index));
        }
    }

    /// Hands the worker what find searches for, and where its current match is, as the last
    /// frame left them. When find stops, its current match in a body becomes the selection.
    fn sync_find(&mut self, ctx: &egui::Context) {
        let painted = self.painted_match.take();
        let query = exo_ui::find::query(ctx);
        if query != self.searched {
            if let (None, Some((id, index))) = (&query, self.current_match.take()) {
                self.send(Job::SelectMatch { id, index });
            }
            for entry in self.entries.values_mut() {
                entry.found.clear();
            }
            self.searched.clone_from(&query);
            self.current_match = None;
            self.send(Job::Find(query));
        } else if painted != self.current_match {
            self.current_match.clone_from(&painted);
            self.send(Job::CurrentMatch(painted));
        }
    }

    /// Takes in what the worker finished, uploading painted bands as textures. Call once a frame,
    /// before painting.
    pub fn poll(&mut self, ctx: &egui::Context) -> Vec<Event> {
        self.sync_find(ctx);
        let mut events = Vec::new();
        while let Ok(output) = self.outputs.try_recv() {
            match output {
                Output::Rendered(facts) => {
                    for (id, facts) in facts {
                        self.entries.entry(id).or_default().facts = Some(facts);
                    }
                }
                Output::Laid { id, laid } => {
                    if let Some(entry) = self.entries.get_mut(&id) {
                        entry
                            .asked_bands
                            .retain(|(asked, _)| *asked == laid.generation);
                        entry.hovered_at = None;
                        entry
                            .patches
                            .retain(|_, (generation, ..)| *generation == laid.generation);
                        entry.animation.shown.clear();
                        entry.scroll_x = entry.scroll_x.min(laid.overflow);
                        entry.laid = Some(laid);
                    }
                }
                Output::Painted {
                    id,
                    generation,
                    band,
                    image,
                } => {
                    let Some(entry) = self.entries.get_mut(&id) else {
                        continue;
                    };
                    if entry
                        .laid
                        .as_ref()
                        .is_none_or(|laid| laid.generation != generation)
                    {
                        continue;
                    }
                    let color = ColorImage::from_rgba_premultiplied(
                        [image.width as usize, image.height as usize],
                        &image.rgba,
                    );
                    let texture = ctx.load_texture(
                        format!("body {id} {band}"),
                        color,
                        TextureOptions::NEAREST,
                    );
                    entry.bands.insert(band, (generation, texture));
                }
                Output::Animated {
                    id,
                    generation,
                    patches,
                    next,
                } => {
                    let Some(entry) = self.entries.get_mut(&id) else {
                        continue;
                    };
                    entry.animation.asked = false;
                    entry.animation.due = next.map(|next| next.as_secs_f64());
                    if entry
                        .laid
                        .as_ref()
                        .is_none_or(|laid| laid.generation != generation)
                    {
                        continue;
                    }
                    for (region, image) in patches {
                        let color = ColorImage::from_rgba_premultiplied(
                            [image.width as usize, image.height as usize],
                            &image.rgba,
                        );
                        let key = [region[0], region[1]].map(|edge| edge as u32);
                        // A new frame goes into the texture the last one is in, rather than a new
                        // texture every frame.
                        match entry.patches.get_mut(&key) {
                            Some((patch_generation, patch_region, texture))
                                if texture.size() == color.size =>
                            {
                                texture.set(color, TextureOptions::NEAREST);
                                (*patch_generation, *patch_region) = (generation, region);
                            }
                            _ => {
                                let texture = ctx.load_texture(
                                    format!("body {id} frame {key:?}"),
                                    color,
                                    TextureOptions::NEAREST,
                                );
                                entry.patches.insert(key, (generation, region, texture));
                            }
                        }
                    }
                }
                Output::Hovered { id, hit } => {
                    if let Some(entry) = self.entries.get_mut(&id) {
                        entry.hover = hit;
                    }
                }
                Output::Clicked { id, hit } => {
                    events.extend(hit.map(|hit| Event::Clicked { id, hit }))
                }
                Output::Copied(text) => {
                    events.push(text.map_or(Event::NothingSelected, Event::Copied))
                }
                Output::Restarted => {
                    self.messages.clear();
                    self.entries.clear();
                    self.selection = None;
                    (self.searched, self.current_match) = (None, None);
                }
                Output::Conversation { thread, facts } => {
                    events.push(Event::Rendered { thread, facts })
                }
                Output::Selected(selection) => {
                    if selection != self.selection
                        && let Some(selected) = &selection
                        && let Some(visible) = self
                            .facts(&selected.message)
                            .map(|facts| facts.visible_chars)
                            .filter(|visible| *visible > 0)
                    {
                        let chars = selected.text.chars().count();
                        if chars > 500 || chars as f64 / visible as f64 > 0.8 {
                            events.push(Event::SelectedMuch);
                        }
                    }
                    self.selection = selection;
                }
                Output::Anchored { id, top } => {
                    events.extend(top.map(|top| Event::Anchored { id, top }))
                }
                Output::Found { id, query, boxes } => {
                    // An answer for an earlier query is outdated: the worker searches again.
                    if query == self.searched {
                        let entry = self.entries.entry(id).or_default();
                        (entry.found, entry.found_for) = (boxes, query);
                    }
                }
                Output::ContextMenu {
                    id,
                    at,
                    screen,
                    hit,
                    selection,
                } => {
                    // A line's ascent down the selection's box: about where its baseline is.
                    let baseline = |bounds: Rect| {
                        screen + (bounds.min - at) + vec2(0.0, 0.75 * bounds.height().min(20.0))
                    };
                    events.push(Event::ContextMenu {
                        id,
                        at: screen,
                        hit,
                        selection: selection.map(|(text, bounds)| (text, baseline(bounds))),
                    });
                }
            }
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
        events
    }
}

/// One message's view on the worker.
struct View {
    body: BodyView,
    look: Look,
    block_remote: bool,
    height: f32,
    generation: u64,
}

impl View {
    /// Its latest layout, for message `id`.
    fn laid(&self, id: &str) -> Output {
        let overflow = if self.body.scrolls() {
            self.body.content_width() - self.body.width()
        } else {
            0.0
        };
        let laid = Laid {
            height: self.height,
            generation: self.generation,
            width: self.body.width(),
            overflow,
            animations: self.body.animations(),
        };
        Output::Laid {
            id: id.to_owned(),
            laid,
        }
    }
}

/// The worker thread's state: every render and view of the open thread.
struct Worker {
    fonts: Fonts,
    renders: HashMap<String, Render>,
    views: HashMap<String, View>,
    fetch: Option<Arc<dyn exo_mail::Fetch>>,
    /// The thread's messages load their read receipts when first shown.
    read_receipts: bool,
    /// The messages that have, since the thread was opened.
    receipts_loaded: HashSet<String>,
    /// The view holding the text selection.
    selected: Option<String>,
    /// What find searches for, and its current match (a message's, and which of its matches).
    query: Option<String>,
    current_match: Option<(String, usize)>,
    generation: u64,
    /// Lets image fetches wake the worker.
    wake: Sender<Job>,
    pending: Arc<AtomicUsize>,
}

impl Worker {
    fn run(mut self, jobs: &Receiver<Job>, outputs: &Sender<Output>, ctx: &egui::Context) {
        while let Ok(first) = jobs.recv() {
            // While a window resizes, layouts queue up faster than they run: only the newest
            // layout (and hover) of each body in the queue is worth doing.
            let mut batch = vec![first];
            batch.extend(jobs.try_iter());
            let superseded: Vec<bool> = batch
                .iter()
                .enumerate()
                .map(|(at, job)| batch[at + 1..].iter().any(|later| job.superseded_by(later)))
                .collect();
            for (job, skip) in batch.into_iter().zip(superseded) {
                // A job answers with as many outputs as it changed; each output is one unit of
                // `pending` on the UI side, so the job's own unit is swapped for them.
                let produced = if skip {
                    Vec::new()
                } else {
                    self.handle_caught(job)
                };
                self.pending.fetch_add(produced.len(), Ordering::SeqCst);
                for output in produced {
                    if outputs.send(output).is_err() {
                        return;
                    }
                }
                self.pending.fetch_sub(1, Ordering::SeqCst);
            }
            ctx.request_repaint();
        }
    }

    /// [`Self::handle`], surviving a panic in the body engine: the message the job was about
    /// loses its view and render (its body stays a placeholder) and the other bodies carry on.
    /// Otherwise one hostile body would leave every body blank until Exo restarts.
    fn handle_caught(&mut self, job: Job) -> Vec<Output> {
        let id = job.message().map(str::to_owned);
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.handle(job))).unwrap_or_else(
            |_| {
                log::error!("the body renderer panicked on message {id:?}");
                if let Some(id) = id {
                    self.views.remove(&id);
                    self.renders.remove(&id);
                }
                Vec::new()
            },
        )
    }

    fn next_generation(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    fn handle(&mut self, job: Job) -> Vec<Output> {
        match job {
            Job::Thread {
                messages,
                own_domains,
                loader,
                read_receipts,
                options,
            } => {
                self.views.clear();
                self.current_match = None;
                let had_selection = self.selected.take().is_some();
                self.read_receipts = read_receipts;
                self.receipts_loaded.clear();
                self.renders = render_thread(&messages, &own_domains, options);
                let inline = messages
                    .iter()
                    .flat_map(|message| message.attachments.iter().cloned())
                    .collect();
                self.fetch = Some(Arc::new(fetch::Fetcher::new(
                    loader,
                    inline,
                    self.wake.clone(),
                    self.pending.clone(),
                )));
                let facts = messages
                    .iter()
                    .filter_map(|message| {
                        Some((message.id.clone(), facts(self.renders.get(&message.id)?)))
                    })
                    .collect();
                let mut outputs = vec![Output::Rendered(facts)];
                outputs.extend(had_selection.then(|| self.selection()));
                outputs
            }
            Job::Layout {
                id,
                look,
                block_remote,
            } => {
                let selected = self.selected.clone();
                let mut outputs: Vec<Output> =
                    self.layout(&id, look, block_remote).into_iter().collect();
                outputs.extend(self.find(&id));
                // A new view has no selection.
                if selected.is_some() && self.selected.is_none() {
                    outputs.push(self.selection());
                }
                outputs
            }
            Job::Paint {
                id,
                generation,
                band,
                width,
            } => {
                let Some(view) = self
                    .views
                    .get_mut(&id)
                    .filter(|view| view.generation == generation)
                else {
                    return Vec::new();
                };
                let top = band as f32 * BAND;
                let region = Region {
                    top,
                    height: BAND.min(view.height - top).max(1.0),
                    left: 0.0,
                    width,
                    phase: [view.look.phase, 0.0],
                };
                let image = view.body.paint(region, view.look.background);
                vec![Output::Painted {
                    id,
                    generation,
                    band,
                    image,
                }]
            }
            Job::Animate {
                id,
                generation,
                now,
                visible,
            } => {
                // Always answered: the UI asks again only once this one is.
                let Some(view) = self.views.get_mut(&id) else {
                    return vec![Output::Animated {
                        id,
                        generation,
                        patches: Vec::new(),
                        next: None,
                    }];
                };
                let animated = view.body.animate(now, visible);
                let patches = if view.generation == generation {
                    animated
                        .changed
                        .into_iter()
                        .map(|[x, y, w, h]| {
                            let [left, top] = [x.floor(), y.floor()];
                            let region = [left, top, (x + w).ceil() - left, (y + h).ceil() - top];
                            let [left, top, width, height] = region;
                            (
                                region,
                                view.body.paint(
                                    Region {
                                        top,
                                        height,
                                        left,
                                        width,
                                        phase: [view.look.phase, 0.0],
                                    },
                                    view.look.background,
                                ),
                            )
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                vec![Output::Animated {
                    id,
                    generation,
                    patches,
                    next: animated.next,
                }]
            }
            Job::Hover { id, at } => {
                let hit = self
                    .views
                    .get(&id)
                    .and_then(|view| view.body.hit(at.x, at.y));
                vec![Output::Hovered { id, hit }]
            }
            Job::Click { id, at } => {
                let hit = self
                    .views
                    .get(&id)
                    .and_then(|view| view.body.hit(at.x, at.y));
                vec![Output::Clicked { id, hit }]
            }
            Job::SelectFrom {
                id,
                at,
                granularity,
            } => {
                let mut changed = self.clear_selection();
                if let Some(view) = self.views.get_mut(&id) {
                    view.body.select_from(at.x, at.y, granularity);
                    self.selected = Some(id.clone());
                    changed.extend(self.repaint(&id));
                }
                changed.push(self.selection());
                changed
            }
            Job::SelectTo { id, at } => {
                let moved = self.selected.as_deref() == Some(&id)
                    && self
                        .views
                        .get_mut(&id)
                        .is_some_and(|view| view.body.select_to(at.x, at.y));
                if moved {
                    self.repaint(&id)
                        .into_iter()
                        .chain([self.selection()])
                        .collect()
                } else {
                    Vec::new()
                }
            }
            Job::ContextMenu { id, at, screen } => {
                let Some(view) = self.views.get(&id) else {
                    return Vec::new();
                };
                let hit = view.body.context_hit(at.x, at.y);
                let on_selection = self.selected.as_deref() == Some(&id)
                    && view
                        .body
                        .selection_bounds()
                        .is_some_and(|[left, top, width, height]| {
                            Rect::from_min_size(pos2(left, top), vec2(width, height)).contains(at)
                        });
                let mut out = Vec::new();
                if !on_selection {
                    // A Mac's right-click selects the word under it (`SelectClosestWordOrLinkFromMouseEvent`).
                    out = self.clear_selection();
                    if hit.text
                        && let Some(view) = self.views.get_mut(&id)
                    {
                        view.body.select_from(at.x, at.y, Granularity::Word);
                        self.selected = Some(id.clone());
                        out.extend(self.repaint(&id));
                    }
                    out.push(self.selection());
                }
                let selection = self
                    .selected
                    .as_ref()
                    .and_then(|selected| self.views.get(selected))
                    .and_then(|view| {
                        let text = view.body.selected_text().filter(|text| !text.is_empty())?;
                        let [left, top, width, height] = view.body.selection_bounds()?;
                        Some((
                            text,
                            Rect::from_min_size(pos2(left, top), vec2(width, height)),
                        ))
                    });
                out.push(Output::ContextMenu {
                    id,
                    at,
                    screen,
                    hit,
                    selection,
                });
                out
            }
            Job::ClearSelection => self.clear_selection(),
            Job::Copy => {
                let text = self
                    .selected
                    .as_ref()
                    .and_then(|id| self.views.get(id))
                    .and_then(|view| view.body.selected_text())
                    .filter(|text| !text.is_empty());
                vec![Output::Copied(text)]
            }
            Job::Anchor { id, name } => {
                let top = self
                    .views
                    .get(&id)
                    .and_then(|view| view.body.anchor_top(&name));
                vec![Output::Anchored { id, top }]
            }
            Job::Render {
                thread,
                messages,
                own_domains,
            } => {
                let renders =
                    render_thread(&messages, &own_domains, exo_mail::RenderOptions::default());
                let facts = messages
                    .iter()
                    .filter_map(|message| {
                        Some((message.id.clone(), facts(renders.get(&message.id)?)))
                    })
                    .collect();
                vec![Output::Conversation { thread, facts }]
            }
            Job::Wake => {
                let ids: Vec<String> = self.views.keys().cloned().collect();
                let mut changed = Vec::new();
                for id in ids {
                    let Some(view) = self.views.get_mut(&id) else {
                        continue;
                    };
                    if view.body.poll() {
                        view.height = view.body.height();
                        changed.extend(self.find(&id));
                        changed.extend(self.repaint(&id));
                    }
                }
                changed
            }
            Job::Find(query) => {
                self.query = query;
                self.current_match = None;
                let ids: Vec<String> = self.views.keys().cloned().collect();
                ids.iter()
                    .flat_map(|id| self.find(id).into_iter().chain(self.repaint(id)))
                    .collect()
            }
            Job::CurrentMatch(current) => {
                let before = std::mem::replace(&mut self.current_match, current.clone());
                let mut changed = Vec::new();
                for (id, index) in [
                    before.map(|(id, _)| (id, None)),
                    current.map(|(id, index)| (id, Some(index))),
                ]
                .into_iter()
                .flatten()
                {
                    if let Some(view) = self.views.get_mut(&id) {
                        view.body.set_current_match(index);
                        changed.extend(self.repaint(&id));
                    }
                }
                changed
            }
            Job::SelectMatch { id, index } => {
                let mut changed = self.clear_selection();
                if let Some(view) = self.views.get_mut(&id) {
                    view.body.select_match(index);
                    self.selected = Some(id.clone());
                    changed.extend(self.repaint(&id));
                }
                changed.push(self.selection());
                changed
            }
        }
    }

    /// Searches message `id`'s body for find's query (or takes its highlights away), keeping the
    /// current match if it's there. `None` without a view.
    fn find(&mut self, id: &str) -> Option<Output> {
        let view = self.views.get_mut(id)?;
        let boxes = match &self.query {
            Some(query) => view.body.find(|text| exo_ui::find::matches(text, query)),
            None => {
                view.body.clear_find();
                Vec::new()
            }
        };
        if let Some((_, index)) = self
            .current_match
            .as_ref()
            .filter(|(current, _)| current == id)
        {
            view.body.set_current_match(Some(*index));
        }
        let boxes = boxes
            .into_iter()
            .map(|[left, top, width, height]| {
                Rect::from_min_size(pos2(left, top), vec2(width, height))
            })
            .collect();
        Some(Output::Found {
            id: id.to_owned(),
            query: self.query.clone(),
            boxes,
        })
    }

    /// Lays message `id` out with `look`, building its view if the look (or `block_remote`)
    /// needs a new one. Under `Show all images`, a message's first view since its thread opened
    /// loads its read receipts.
    fn layout(&mut self, id: &str, look: Look, block_remote: bool) -> Option<Output> {
        let render = self.renders.get(id)?;
        let fonts = &self.fonts;
        let rebuild = self.views.get(id).is_none_or(|view| {
            (
                view.look.theme,
                view.look.rem,
                view.look.show_quoted,
                view.look.hide_original,
                view.block_remote,
            ) != (
                look.theme,
                look.rem,
                look.show_quoted,
                look.hide_original,
                block_remote,
            )
        });
        if rebuild {
            let options = ViewOptions {
                theme: match look.theme {
                    exo_ui::Theme::Snow => exo_mail::Theme::Light,
                    exo_ui::Theme::Carbon => exo_mail::Theme::Dark,
                },
                show_quoted: look.show_quoted,
                rem: look.rem,
                block_remote_content: block_remote,
            };
            let hidden = look.hide_original.then(|| hidden(render));
            let body = BodyView::new(
                hidden.as_ref().unwrap_or(render),
                options,
                fonts,
                self.fetch.clone(),
            );
            if self.selected.as_deref() == Some(id) {
                self.selected = None;
            }
            if self.read_receipts
                && self.receipts_loaded.insert(id.to_owned())
                && let Some(fetch) = &self.fetch
            {
                exo_mail::load_read_receipts(render, fetch.as_ref());
            }
            self.views.insert(
                id.to_owned(),
                View {
                    body,
                    look,
                    block_remote,
                    height: 0.0,
                    generation: 0,
                },
            );
        }
        let generation = self.next_generation();
        let view = self.views.get_mut(id)?;
        view.height = view.body.layout(look.width, look.scale);
        view.look = look;
        view.generation = generation;
        Some(view.laid(id))
    }

    /// Marks `id`'s painted bands stale (its selection or images changed).
    fn repaint(&mut self, id: &str) -> Option<Output> {
        let generation = self.next_generation();
        let view = self.views.get_mut(id)?;
        view.generation = generation;
        Some(view.laid(id))
    }

    fn clear_selection(&mut self) -> Vec<Output> {
        let Some(id) = self.selected.take() else {
            return Vec::new();
        };
        if let Some(view) = self.views.get_mut(&id) {
            view.body.clear_selection();
        }
        self.repaint(&id)
            .into_iter()
            .chain([self.selection()])
            .collect()
    }

    /// The selection as the UI keeps it.
    fn selection(&self) -> Output {
        let selected = self.selected.as_ref().and_then(|id| {
            let body = &self.views.get(id)?.body;
            let [left, top, width, height] = body.selection_bounds()?;
            Some(Selected {
                message: id.clone(),
                html: body.selected_html()?,
                text: body.selected_text().unwrap_or_default(),
                bounds: Rect::from_min_size(pos2(left, top), vec2(width, height)),
            })
        });
        Output::Selected(selected)
    }
}

/// The bands of a body `height` px tall that show between `from` and `to` px below its top.
fn visible_bands(from: f32, to: f32, height: f32) -> Range<u32> {
    let count = (height / BAND).ceil().max(1.0);
    let band = |y: f32| y.clamp(0.0, count) as u32;
    band((from / BAND).floor())..band((to / BAND).ceil())
}

/// Runs the pipeline on every message, oldest first, each against the one it replies to: the
/// message its `In-Reply-To` names, else the one before it.
fn render_thread(
    sources: &[MessageDetail],
    own_domains: &[String],
    options: exo_mail::RenderOptions,
) -> HashMap<String, Render> {
    let messages: Vec<exo_mail::Message> = sources
        .iter()
        .map(|source| exo_mail::Message {
            html: body_html(&source.body),
            from_email: source
                .from
                .as_ref()
                .map(|from| from.email.clone())
                .unwrap_or_default(),
            from_name: source
                .from
                .as_ref()
                .and_then(|from| from.name.clone())
                .unwrap_or_default(),
            rfc822_id: source.rfc822_id.clone().unwrap_or_default(),
            attachment_names: source
                .attachments
                .iter()
                .filter(|attachment| !attachment.inline)
                .map(|attachment| attachment.filename.clone())
                .collect(),
        })
        .collect();
    let mut renders: Vec<Render> = Vec::with_capacity(messages.len());
    for (at, message) in messages.iter().enumerate() {
        let parent = sources[at]
            .in_reply_to
            .as_ref()
            .and_then(|parent| {
                sources[..at]
                    .iter()
                    .position(|earlier| earlier.rfc822_id.as_ref() == Some(parent))
            })
            .or(at.checked_sub(1));
        let replying_to = parent.map(|parent| (&messages[parent], &renders[parent]));
        renders.push(exo_mail::render_with(
            message,
            replying_to,
            own_domains,
            options,
        ));
    }
    sources
        .iter()
        .map(|source| source.id.clone())
        .zip(renders)
        .collect()
}

/// The text a reader sees of each of `sources`, each read against the one it replies to as
/// [`render_thread`] reads them: what the search index holds (`Store::index_text_with`).
pub(crate) fn visible_texts(sources: &[exo_store::TextSource]) -> Vec<String> {
    let messages: Vec<exo_mail::Message> = sources
        .iter()
        .map(|source| exo_mail::Message {
            html: body_html(&source.body),
            from_email: source.from_email.clone(),
            from_name: source.from_name.clone(),
            rfc822_id: source.rfc822_id.clone(),
            attachment_names: source.attachment_names.clone(),
        })
        .collect();
    let mut renders: Vec<Render> = Vec::with_capacity(messages.len());
    for (at, message) in messages.iter().enumerate() {
        let replying_to = sources[at]
            .replying_to
            .filter(|parent| *parent < at)
            .map(|parent| (&messages[parent], &renders[parent]));
        renders.push(exo_mail::render(message, replying_to, &[]));
    }
    renders
        .into_iter()
        .map(|render| render.visible_text)
        .collect()
}

/// A body as the pipeline takes it: its HTML, or its plain text as HTML.
fn body_html(body: &exo_store::Body) -> String {
    match (&body.html, &body.text) {
        (Some(html), _) => html.clone(),
        (None, Some(text)) => exo_mail::text_to_html(text),
        (None, None) => String::new(),
    }
}

fn facts(render: &Render) -> Facts {
    Facts {
        snippet: render.snippet.clone(),
        visible_chars: render.visible_text.chars().count(),
        has_quoted: render.has_quoted_content,
        unquoted_marketing: render.unquoted_is_marketing,
        quoted_marketing: render.quoted_is_marketing,
        tracking_blocked: !render.read_status_urls.is_empty(),
        remote_images: render.has_remote_images,
        unsubscribe: render
            .unsubscribe_link
            .as_ref()
            .map(|link| link.url.clone()),
        unquoted_html: render.unquoted_html.clone(),
        quoted_html: render.quoted_html.clone(),
        html: render.html.clone(),
        body_attributes: render.body_attributes.clone(),
        links: render.links.clone(),
        actions: render.actions.clone(),
        calendar: render.calendar_data.clone(),
    }
}

/// The render of a body its invite card replaces: no new part and no `<body>` attributes, and the
/// new part, when it has anything, as the quoted one behind the `…`.
fn hidden(render: &Render) -> Render {
    let has_body = !render.unquoted_html.trim().is_empty();
    Render {
        unquoted_html: String::new(),
        quoted_html: if has_body {
            render.unquoted_html.clone()
        } else {
            String::new()
        },
        has_quoted_content: has_body,
        body_attributes: exo_mail::BodyAttributes::default(),
        ..render.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_images_setting_decides_what_shows_and_changing_it_forgets_shown_messages() {
        let mut bodies = Bodies::new(&egui::Context::default());
        let facts = Facts {
            tracking_blocked: true,
            remote_images: true,
            ..Facts::default()
        };
        bodies.entries.entry("m".into()).or_default().facts = Some(facts);
        bodies.set_images(RemoteContentBlocking::None);
        assert!(
            !bodies.tracking_blocked("m") && !bodies.can_show_images("m"),
            "everything loads"
        );
        bodies.set_images(RemoteContentBlocking::ReduceTracking);
        assert!(
            bodies.tracking_blocked("m") && !bodies.can_show_images("m"),
            "the read receipt alone is blocked"
        );
        bodies.set_images(RemoteContentBlocking::All);
        assert!(
            bodies.tracking_blocked("m")
                && bodies.blocks_images("m")
                && bodies.can_show_images("m")
        );
        bodies.show_images("m");
        assert!(
            !bodies.blocks_images("m") && !bodies.can_show_images("m"),
            "shown"
        );
        assert!(
            bodies.tracking_blocked("m"),
            "its read receipt stays blocked"
        );
        bodies.set_images(RemoteContentBlocking::ReduceTracking);
        bodies.set_images(RemoteContentBlocking::All);
        assert!(
            bodies.blocks_images("m"),
            "a new setting forgets what was shown"
        );
    }

    #[test]
    fn the_index_text_leaves_out_quotes_and_signatures() {
        let source = |id: &str, html: &str, replying_to: Option<usize>| exo_store::TextSource {
            id: id.into(),
            body: exo_store::Body {
                html: Some(html.into()),
                text: None,
            },
            from_email: "ann@example.com".into(),
            from_name: "Ann Lee".into(),
            rfc822_id: format!("{id}@example.com"),
            attachment_names: Vec::new(),
            replying_to,
        };
        let first = "<div>Shall we try the boathouse on Friday? The walrus exhibit opens then.</div><div class=\"gmail_signature\">Ann Lee · Zanzibar Shipping</div>";
        let reply = format!(
            "<div>Noon works too.</div><div class=\"gmail_quote\"><div class=\"gmail_attr\">On Mon, Nov 16, 2026 at 9:00 AM Ann Lee &lt;ann@example.com&gt; wrote:</div><blockquote class=\"gmail_quote\">{first}</blockquote></div>"
        );
        let texts = visible_texts(&[source("a", first, None), source("b", &reply, Some(0))]);
        assert!(
            texts[0].contains("walrus") && !texts[0].contains("Zanzibar"),
            "{texts:?}"
        );
        assert!(
            texts[1].contains("Noon works too")
                && !texts[1].contains("walrus")
                && !texts[1].contains("Zanzibar"),
            "{texts:?}"
        );
        // Read alone, the reply keeps what it quotes.
        assert!(visible_texts(&[source("b", &reply, None)])[0].contains("walrus"));
    }

    /// A store on gmail-mock with the app's visible text, synced.
    async fn indexed_store(
        server: &gmail_mock::MockServer,
        dir: &std::path::Path,
    ) -> exo_store::Store {
        let options = exo_store::Options {
            quota: None,
            ..exo_store::Options::default()
        };
        let store = exo_store::Store::open_with(
            dir,
            "maya@brightmoor.co",
            server.endpoints(),
            gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()),
            options,
        )
        .await
        .expect("opens");
        store.index_text_with(Arc::new(visible_texts));
        while store.sync().await.expect("syncs").backfilling {}
        store
    }

    #[test]
    fn search_finds_what_a_message_shows_and_not_what_it_quotes() {
        let runtime = tokio::runtime::Runtime::new().expect("a runtime");
        runtime.block_on(async {
            let server = gmail_mock::MockServer::start(gmail_mock::Config::default()).await.expect("the mock starts");
            let dir = std::env::temp_dir().join(format!("exo-index-text-search-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let store = indexed_store(&server, &dir).await;
            let http = reqwest::Client::new();
            let deliver = server.url().join("/__mock/deliver").expect("a URL");
            // Ann asks, signing with her company; Ben answers above her message, quoted whole.
            let first = "<div>Shall we try the boathouse on Friday? The walrus exhibit opens then.</div><div class=\"gmail_signature\">Ann Lee · Zanzibar Shipping</div>";
            let asked: serde_json::Value = http.post(deliver.clone()).json(&serde_json::json!({ "from": "Ann Lee <ann@example.com>", "subject": "Friday lunch", "html": first })).send().await.expect("delivers").json().await.expect("a message");
            let reply = format!("<div>Noon by the pergola works too.</div><div class=\"gmail_quote\"><div class=\"gmail_attr\">On Mon, Nov 16, 2026 at 9:00 AM Ann Lee &lt;ann@example.com&gt; wrote:</div><blockquote class=\"gmail_quote\">{first}</blockquote></div>");
            http.post(deliver).json(&serde_json::json!({ "from": "Ben Ortiz <ben@example.com>", "html": reply, "threadId": asked["threadId"] })).send().await.expect("delivers");
            while store.sync().await.expect("syncs").backfilling {}
            store.index_texts().await.expect("indexes");
            // The local index alone (offline): online, Gmail's search adds what it finds.
            let found = async |query: &str| -> Vec<String> {
                let page = store.search(&mut exo_store::Search::new(query), 50, false, 15).await.expect("searches");
                page.results.into_iter().map(|result| result.thread.subject).collect()
            };
            assert_eq!(found("pergola").await, ["Friday lunch"], "the reply's own words");
            assert_eq!(found("walrus").await, ["Friday lunch"], "Ann's, which the reply quotes");
            assert!(found("zanzibar").await.is_empty(), "not her signature, there or quoted");
            drop(store);
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    /// What working out the search index's visible text costs (`Store::index_text_with`): the
    /// pipeline's time per 100 conversations, the mock's mail as it is, then 100 conversations of
    /// two messages (a reply quoting the first) at sizes of real mail.
    ///
    /// `cargo test --release -p exo --lib -- --ignored --nocapture index_text_cost`
    #[test]
    #[ignore = "a measurement"]
    fn index_text_cost() {
        type Spent = Arc<std::sync::Mutex<(std::time::Duration, usize, usize)>>;
        let runtime = tokio::runtime::Runtime::new().expect("a runtime");
        runtime.block_on(async {
            let server = gmail_mock::MockServer::start(gmail_mock::Config::default()).await.expect("the mock starts");
            let dir = std::env::temp_dir().join(format!("exo-index-text-cost-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let options = exo_store::Options { quota: None, ..exo_store::Options::default() };
            let store = exo_store::Store::open_with(&dir, "maya@brightmoor.co", server.endpoints(), gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()), options).await.expect("opens");
            let spent = Spent::default();
            let counted = spent.clone();
            store.index_text_with(Arc::new(move |sources: &[exo_store::TextSource]| {
                let clock = std::time::Instant::now();
                let texts = visible_texts(sources);
                let mut spent = counted.lock().expect("lock");
                *spent = (spent.0 + clock.elapsed(), spent.1 + 1, spent.2 + sources.len());
                texts
            }));
            // What the pipeline took for what's queued.
            let drain = async |label: &str| {
                store.index_texts().await.expect("indexes");
                let (pipeline, threads, messages) = std::mem::take(&mut *spent.lock().expect("lock"));
                let per = |time: std::time::Duration, n: usize| time / u32::try_from(n.max(1)).unwrap_or(1);
                println!("{label}: {threads} conversations ({messages} messages read): {pipeline:.0?}, {:.0?} per 100 conversations, {:.1?} a message", per(pipeline * 100, threads), per(pipeline, messages));
            };
            while store.sync().await.expect("syncs").backfilling {}
            for thread in store.list(&exo_store::ListId::All, None, 1000).await.expect("lists").threads {
                store.load_bodies(&thread.id).await.expect("fetches");
            }
            drain("the mock's mail").await;
            let http = reqwest::Client::new();
            let deliver = server.url().join("/__mock/deliver").expect("a URL");
            for (label, pad) in [("personal mail, 15 KB", 13_000), ("newsletters, 40 KB", 37_000)] {
                const CELL: &str = r#"<td style="padding:0 0 16px;font-family:-apple-system,Helvetica,Arial,sans-serif;font-size:16px;line-height:24px;color:#374151"></td>"#;
                let padding = format!("<div style=\"display:none\"><table><tr>{}</tr></table></div>", CELL.repeat(pad / CELL.len()));
                for n in 0..100 {
                    let first = format!("<div>Can we move the review of item {n} to Thursday? The deck is nearly done.</div>{padding}<div class=\"gmail_signature\">Ann Lee</div>");
                    let asked: serde_json::Value = http.post(deliver.clone()).json(&serde_json::json!({ "from": "Ann Lee <ann@example.com>", "subject": format!("Review {n}"), "html": first })).send().await.expect("delivers").json().await.expect("a message");
                    let reply = format!("<div>Thursday works.</div><div class=\"gmail_quote\"><div class=\"gmail_attr\">On Mon, Ann Lee wrote:</div><blockquote class=\"gmail_quote\">{first}</blockquote></div>");
                    http.post(deliver.clone()).json(&serde_json::json!({ "from": "Ben Ortiz <ben@example.com>", "html": reply, "threadId": asked["threadId"] })).send().await.expect("delivers");
                }
                while store.sync().await.expect("syncs").backfilling {}
                drain(label).await;
            }
            drop(store);
            let _ = std::fs::remove_dir_all(&dir);
        });
    }

    #[test]
    fn paints_only_the_bands_on_screen() {
        assert_eq!(
            visible_bands(0.0, 300.0, 100.0),
            0..1,
            "a short body is one band"
        );
        assert_eq!(visible_bands(600.0, 1400.0, 5000.0), 1..3);
        assert_eq!(visible_bands(-200.0, -10.0, 5000.0), 0..0, "above the body");
        assert_eq!(
            visible_bands(9000.0, 9800.0, 5000.0),
            10..10,
            "below the body"
        );
        // A hostile `height: 1e11px` costs the bands on screen, not 200 million of them.
        assert!(visible_bands(5e10, 5e10 + 800.0, 1e11).len() <= 3);
    }
}
