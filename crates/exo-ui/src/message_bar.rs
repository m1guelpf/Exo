//! The status bar: one 32px banner across the bottom of the window, under everything else, which
//! shrinks by its height while it shows.
//!
//! `padding: 4px 2rem 4px 3.5rem` (44px on the left at ≤1152px); a bold uppercase title padded
//! 24px on its right, then the description, both Exo Sans 12px/24px nudged down 1px, white; the
//! description padded 32px on its right and faded out over its last 7%
//! (`mask-image: linear-gradient(90deg, #000 93%, transparent)`); links white and underlined;
//! keycaps in a row of their own. The background eases between colors over .25s.

use egui::{
    Color32, CursorIcon, Mesh, Pos2, Rect, Sense, Ui, Vec2, epaint::RectShape, pos2,
    text::LayoutJob,
};

use crate::{
    Family, Font, KeycapColors, Keycaps, TextStyle, layout, motion, paint_job, pixel_snap, rem,
};

/// The bar's height (`max-height: 32px`, its 24px line padded 4px).
pub const MESSAGE_BAR_HEIGHT: f32 = 32.0;
const TITLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 24.0);
const DESCRIPTION: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 24.0);
/// The line's top: `padding-top: 4px`, then `top: 1px`.
const LINE_TOP: f32 = 5.0;
/// A description with keycaps is 26px tall, which the bar's `align-items: center` puts 1px
/// higher, its text with it; the caps sit at its top plus their `margin-top: 1px` (measured).
const KEYED_LINE_TOP: f32 = 4.0;
const KEYCAP_TOP: f32 = 5.0;
/// A cap's `max-height`; its label sits on the description's 24px line, 2px up: its baseline 14px
/// below the cap's top (measured).
const KEYCAP_HEIGHT: f32 = 19.0;
const KEYCAP_BASELINE: f32 = 14.0;

const TITLE_GAP: f32 = 24.0;
const DESCRIPTION_PADDING: f32 = 32.0;
/// Where the description's fade starts, as a share of its box.
const FADE_FROM: f32 = 0.93;
/// The underline: 1px, a pixel below the baseline (measured).
const UNDERLINE_GAP: f32 = 1.0;
/// How far the underline stays from a descender it would cross, before and after the glyph's
/// bitmap (`text-decoration-skip-ink: auto`; Blink dilates the glyph's intercept by the line's
/// thickness, and the bitmap's right edge carries half a point of padding; measured).
const SKIP_BEFORE: f32 = 1.0;
const SKIP_AFTER: f32 = 0.5;

/// The bar's background, the same in Snow and Carbon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MessageBarColor {
    /// The default: `#72c0ea`.
    #[default]
    Blue,
    /// `#f67b7b`.
    Red,
    /// `#6ebd77`.
    Green,
}

impl MessageBarColor {
    fn fill(self) -> Color32 {
        match self {
            Self::Blue => Color32::from_rgb(0x72, 0xc0, 0xea),
            Self::Red => Color32::from_rgb(0xf6, 0x7b, 0x7b),
            Self::Green => Color32::from_rgb(0x6e, 0xbd, 0x77),
        }
    }
}

/// A run of the description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageBarRun<'a> {
    /// Plain text.
    Text(&'a str),
    /// Text linking to a URL: `(text, url)`.
    Link(&'a str, &'a str),
    /// A shortcut's keycaps, `["⌘", "K"]`.
    Keys(&'a [&'a str]),
}

impl<'a> MessageBarRun<'a> {
    /// What the run says; keycaps say their labels, one after the other.
    pub fn text(&self) -> std::borrow::Cow<'a, str> {
        match *self {
            Self::Text(text) | Self::Link(text, _) => text.into(),
            Self::Keys(keys) => keys.concat().into(),
        }
    }
}

/// A stretch of the description between keycaps: one anonymous flex item of the description's
/// `display: flex` (a space at its start stays, measured).
enum Item<'a> {
    Runs(Vec<(&'a str, Option<&'a str>)>),
    Keys(Keycaps),
}

/// A status bar to paint.
#[derive(Clone, Copy, Debug)]
pub struct MessageBar<'a> {
    /// The title, before `text-transform: uppercase`.
    pub title: &'a str,
    /// The description, run by run.
    pub description: &'a [MessageBarRun<'a>],
    /// The background.
    pub color: MessageBarColor,
}

impl MessageBar<'_> {
    /// Paints the bar across `window`'s bottom. Returns the URL of a link clicked this frame.
    pub fn show(&self, ui: &mut Ui, window: Rect) -> Option<String> {
        let ctx = ui.ctx().clone();
        let id = crate::id(&ctx, "exo_ui::message_bar");
        let bar = Rect::from_min_max(
            pos2(window.left(), window.bottom() - MESSAGE_BAR_HEIGHT),
            window.max,
        );
        let ease = |color: MessageBarColor| {
            motion::transition(
                &ctx,
                id.with(color),
                self.color == color,
                0.25,
                motion::CubicBezier::EASE,
            )
        };
        let fill = motion::mix(
            motion::mix(
                MessageBarColor::Blue.fill(),
                MessageBarColor::Red.fill(),
                ease(MessageBarColor::Red),
            ),
            MessageBarColor::Green.fill(),
            ease(MessageBarColor::Green),
        );
        let painter = ui.painter_at(bar);
        painter.rect_filled(pixel_snap(bar), 0.0, fill);

        let rem = rem(window.width(), false);
        let left = if window.width() <= 1152.0 {
            44.0
        } else {
            3.5 * rem
        };
        let line_top = bar.top() + LINE_TOP;
        let title = self.title.to_uppercase();
        let title_x = bar.left() + left;
        let title_width = TITLE.paint_line(
            &painter,
            pos2(title_x, line_top),
            &title,
            Color32::WHITE,
            None,
        );

        // The description's box: its items and padding, shrunk to what's left of the bar.
        let x = title_x + title_width + TITLE_GAP;
        let items = items(&ctx, self.description);
        let keyed = items.iter().any(|item| matches!(item, Item::Keys(_)));
        let line_top = bar.top() + if keyed { KEYED_LINE_TOP } else { LINE_TOP };
        let widths: Vec<f32> = items
            .iter()
            .map(|item| match item {
                Item::Runs(runs) => DESCRIPTION.width(
                    &ctx,
                    &runs.iter().map(|(text, _)| *text).collect::<String>(),
                ),
                Item::Keys(caps) => caps.width(),
            })
            .collect();
        let width: f32 = widths.iter().sum();
        let right = (x + width + DESCRIPTION_PADDING)
            .min(bar.right() - 2.0 * rem)
            .max(x);
        let clip = Rect::from_min_max(pos2(x, bar.top()), pos2(right, bar.bottom()));
        let description = painter.with_clip_rect(clip.intersect(bar));

        let mut clicked = None;
        let underline_y = line_top + DESCRIPTION.baseline(&ctx) + UNDERLINE_GAP;
        let mut left = x;
        for (item_index, (item, item_width)) in items.iter().zip(widths).enumerate() {
            let item_left = left;
            left += item_width;
            let runs = match item {
                Item::Keys(caps) => {
                    caps.paint(&description, item_left, bar.top() + KEYCAP_TOP);
                    continue;
                }
                Item::Runs(runs) => runs,
            };
            let mut job = LayoutJob::default();
            for (text, _) in runs {
                job.append(text, 0.0, DESCRIPTION.format(Color32::WHITE));
            }
            paint_job(&description, pos2(item_left, line_top), job, &DESCRIPTION);
            let mut run_left = item_left;
            for (index, (text, url)) in runs.iter().enumerate() {
                let run_width = DESCRIPTION.width(&ctx, text);
                let start = run_left;
                run_left += run_width;
                let Some(url) = url else { continue };
                for (from, to) in underline(&ctx, text) {
                    // Its ends are where the text puts them, anti-aliased as Chromium's are.
                    let line = Rect::from_min_max(
                        pos2(start + from, underline_y),
                        pos2(start + to, underline_y + 1.0),
                    );
                    description.add(
                        RectShape::filled(line, 0.0, Color32::WHITE).with_round_to_pixels(false),
                    );
                }
                let hit = Rect::from_min_size(
                    pos2(start, line_top),
                    Vec2::new(run_width, DESCRIPTION.line_height),
                )
                .intersect(clip);
                if ui
                    .interact(hit, id.with(("link", item_index, index)), Sense::click())
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .clicked()
                {
                    clicked = Some((*url).to_owned());
                }
            }
        }
        fade(
            &description,
            Rect::from_min_max(
                pos2(x + (right - x) * FADE_FROM, bar.top()),
                pos2(right, bar.bottom()),
            ),
            fill,
        );
        clicked
    }
}

/// The description's flex items: runs of text, and keycap rows.
fn items<'a>(ctx: &egui::Context, description: &[MessageBarRun<'a>]) -> Vec<Item<'a>> {
    let mut items: Vec<Item<'a>> = Vec::new();
    for run in description {
        match *run {
            MessageBarRun::Keys(keys) => items.push(Item::Keys(
                Keycaps::new(ctx, keys, KeycapColors::base(crate::palette(ctx)))
                    .height(KEYCAP_HEIGHT)
                    .baseline(KEYCAP_BASELINE),
            )),
            MessageBarRun::Text(text) | MessageBarRun::Link(text, _) => {
                let url = if let MessageBarRun::Link(_, url) = *run {
                    Some(url)
                } else {
                    None
                };
                match items.last_mut() {
                    Some(Item::Runs(runs)) => runs.push((text, url)),
                    _ => items.push(Item::Runs(vec![(text, url)])),
                }
            }
        }
    }
    items
}

/// The underline under `text`, as spans from its start: all of it, less around each descender
/// that reaches down into it.
fn underline(ctx: &egui::Context, text: &str) -> Vec<(f32, f32)> {
    let galley = layout(
        ctx,
        LayoutJob::single_section(text.to_owned(), DESCRIPTION.format(Color32::WHITE)),
    );
    let mut spans = vec![(0.0, galley.size().x)];
    let glyphs = galley.rows.iter().flat_map(|row| row.glyphs.iter());
    for glyph in
        glyphs.filter(|glyph| glyph.uv_rect.offset.y + glyph.uv_rect.size.y > UNDERLINE_GAP)
    {
        let ink = glyph.pos.x + glyph.uv_rect.offset.x;
        let (skip_from, skip_to) = (ink - SKIP_BEFORE, ink + glyph.uv_rect.size.x + SKIP_AFTER);
        spans = spans
            .into_iter()
            .flat_map(|(from, to): (f32, f32)| [(from, to.min(skip_from)), (from.max(skip_to), to)])
            .filter(|(from, to)| to > from)
            .collect();
    }
    spans
}

/// The mask's fade over a solid background: the background, transparent to opaque across `rect`.
fn fade(painter: &egui::Painter, rect: Rect, fill: Color32) {
    if rect.width() <= 0.0 {
        return;
    }
    let mut mesh = Mesh::default();
    let corners: [(Pos2, Color32); 4] = [
        (rect.left_top(), Color32::TRANSPARENT),
        (rect.right_top(), fill),
        (rect.right_bottom(), fill),
        (rect.left_bottom(), Color32::TRANSPARENT),
    ];
    for (pos, color) in corners {
        mesh.colored_vertex(pos, color);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}
