//! An event block: a rounded box in the event's colour holding its title and time, clamped to
//! the lines its height allows.

use egui::{
    Color32, CornerRadius, Painter, Rect, Stroke, StrokeKind, Ui, Vec2, text::LayoutJob, vec2,
};
use exo_ui::{BlockColors, BlockPalette, BoxShadow, Family, Font, TextStyle};
use jiff::{SignedDuration, Zoned};

use super::event::{Event, Style};

/// The title: 12px bold, line-height 16px.
const TITLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 16.0);
/// The time: 11px normal.
const TIME: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 11.0, 16.0);
/// Under 30 minutes: 9px/11px, title and time.
const SUB_30_TITLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 9.0, 11.0);
const SUB_30_TIME: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 9.0, 11.0);
/// From 45 to 50 minutes: 10px/13px.
const MID_TITLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 10.0, 13.0);
const MID_TIME: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 10.0, 13.0);
/// A line and the box's vertical padding, for the line count.
const LINE: f32 = 16.0;
const VERTICAL_PADDING: f32 = 8.0;
/// `padding: 4px 5px` inside a 1px border.
const PADDING_TOP: f32 = 4.0;
const PADDING_X: f32 = 5.0;
const BORDER: f32 = 1.0;
/// The gap after the title when the time shares its line: `margin-right: 3px`.
const TIME_GAP: f32 = 3.0;
/// The declined warning icon and its `margin-right`.
const WARNING: f32 = 13.0;
const WARNING_GAP: f32 = 4.0;
/// `transition: box-shadow 100ms ease`.
const GLOW_DURATION: f32 = 0.1;

/// How a block is shown.
#[derive(Clone, Copy, Debug, Default)]
pub struct Look {
    /// Under the pointer, or its details are open, or it's the invite being read: the glow.
    pub glowing: f32,
    /// Ended before now.
    pub past: bool,
    /// A create or edit form is open.
    pub preview: bool,
    /// In the all-day row: its smaller glow.
    pub all_day: bool,
    /// Its time on the title's line.
    pub time_on_same_line: bool,
    /// The event being created, edited or dragged: a solid fill with a shadow.
    pub focused: bool,
    /// A free slot of Find Time's, whose title is its time: no time line.
    pub bare: bool,
    /// A shared slot under an event: its time and cross hidden.
    pub covered: bool,
    /// No text: a collapsed guests' column's bar, or an event under the draft there.
    pub no_text: bool,
    /// No time: a guest's event among three columns or more.
    pub no_time: bool,
    /// The title's lines cut off by the box, with no `…`: the all-day row's guests' columns
    /// (`word-break: break-word`).
    pub clipped: bool,
    /// Colours in place of the event's, with no shadow: the draft blended over a guest's event
    /// (`mix-blend-mode`).
    pub colors: Option<BlockColors>,
}

/// A focused block's shadow: `0 3px 12px rgba(0,0,0,.2), 0 2px 4px rgba(0,0,0,.1)`,
/// white in Carbon (`hsla(0,0%,100%,.2)`, `.1`).
fn focused_shadows(palette: &exo_ui::Palette) -> [BoxShadow; 2] {
    [
        BoxShadow::new([0.0, 3.0], 12.0, 0.0, palette.ink(0.2, 0.2)),
        BoxShadow::new([0.0, 2.0], 4.0, 0.0, palette.ink(0.1, 0.1)),
    ]
}

/// The `{h}:{mm} – {h}:{mm}` a block shows.
pub fn time_range(start: &Zoned, end: &Zoned) -> String {
    format!("{} – {}", start.strftime("%-I:%M"), end.strftime("%-I:%M"))
}

/// Paints `event`'s block in `rect`, inside `clip`; its duration on the day is `span` (a
/// multi-day event's part of the day it's drawn on).
pub fn paint(ui: &Ui, clip: Rect, rect: Rect, event: &Event, span: SignedDuration, look: Look) {
    if let Some(valid) = event.share {
        paint_slot(ui, clip, rect, event, valid, look);
        return;
    }
    let painter = ui.painter_at(clip);
    let theme = exo_ui::theme(ui.ctx());
    let colors = if let Some(colors) = look.colors {
        colors
    } else if look.focused {
        event.color.block(
            theme,
            BlockPalette {
                focused: true,
                ..BlockPalette::default()
            },
        )
    } else {
        event.color.block(
            theme,
            BlockPalette {
                unaccepted: event.style() == Style::Bordered,
                create: look.preview,
                past: look.past,
                focused: false,
            },
        )
    };
    // The box paints at whole px; the text keeps its place.
    let box_rect = exo_ui::pixel_snap(rect);
    let sub_30 = span < SignedDuration::from_mins(30);
    let radius = if sub_30 {
        CornerRadius::same(2)
    } else {
        CornerRadius::same(4)
    };

    let shadowed = look.colors.is_none();
    if shadowed && look.focused {
        for shadow in focused_shadows(exo_ui::palette(ui.ctx())) {
            shadow.paint(&painter, box_rect, radius);
        }
    } else if shadowed && look.glowing > 0.0 {
        // `box-shadow: 0 0 6px {near}, 0 0 12px {far}` (the all-day row's: 3px, 3px, no spread).
        let (near_color, far_color, spread) = event.color.glow(theme);
        let (near, far, spread) = if look.all_day {
            (3.0, 3.0, 0.0)
        } else {
            (6.0, 12.0, spread)
        };
        for (blur, color) in [(far, far_color), (near, near_color)] {
            BoxShadow::new([0.0, 0.0], blur, spread, color.gamma_multiply(look.glowing))
                .paint(&painter, box_rect, radius);
        }
    }
    if event.style() == Style::Dashed && !look.focused {
        // Dashed: the fill with no border, and an SVG `rect`'s 2px stroke dashed every 6px in
        // the border's colour (white, Carbon `rgb(39, 41, 45)`; previews `rgba(255,255,255,.9)`,
        // Carbon `rgb(97, 104, 109)`), under `padding: 5px 6px`, where the border and padding
        // put the text otherwise.
        painter.rect_filled(box_rect, radius, colors.fill);
        paint_dashes(
            &painter.with_clip_rect(box_rect.intersect(clip)),
            box_rect,
            colors.border,
        );
    } else {
        // A past bordered block's border is half a pixel.
        let border = if look.past && event.style() == Style::Bordered && !look.focused {
            0.5
        } else {
            BORDER
        };
        painter.rect(
            box_rect,
            radius,
            colors.fill,
            Stroke::new(border, colors.border),
            StrokeKind::Inside,
        );
    }

    if look.no_text {
        return;
    }
    // The text, clipped to the padding box.
    let inside = box_rect.shrink(BORDER).intersect(clip);
    let painter = painter.with_clip_rect(inside);
    let exactly_30 = span == SignedDuration::from_mins(30);
    // The 45-to-50-minute sizes only while the time shows.
    let mid = span >= SignedDuration::from_mins(45)
        && span <= SignedDuration::from_mins(50)
        && !look.no_time;
    let (title_style, time_style) = if sub_30 {
        (SUB_30_TITLE, SUB_30_TIME)
    } else if mid {
        (MID_TITLE, MID_TIME)
    } else {
        (TITLE, TIME)
    };
    let padding_top = if sub_30 {
        ((rect.height() - 11.0) / 2.0).max(0.0)
    } else if exactly_30 || mid {
        3.0
    } else {
        PADDING_TOP
    };
    let lines = if sub_30 {
        1
    } else {
        (((rect.height() - VERTICAL_PADDING) / LINE).round() as usize).max(1)
    };
    let nowrap = span < SignedDuration::from_hours(1);
    let origin = rect.min + vec2(BORDER + PADDING_X, BORDER + padding_top);
    let width = rect.width() - 2.0 * (BORDER + PADDING_X);
    let color = colors.text;
    let time = event
        .time_text
        .clone()
        .unwrap_or_else(|| time_range(&event.start, &event.end));
    let declined = event.rsvp == Some(super::event::Rsvp::Declined);

    if look.bare {
        let mut job = LayoutJob::single_section(event.title.clone(), title_style.format(color));
        job.wrap.max_width = width;
        job.wrap.max_rows = 1;
        job.wrap.overflow_character = Some('…');
        paint_rows(&painter, origin, job, &title_style, false, color);
        return;
    }
    let mut job = LayoutJob::default();
    // The declined warning: a 13px warning circle at the text's top, 4px before
    // the title (not on previews).
    let warning = event.everyone_else_declined && !look.preview;
    if warning {
        exo_ui::paint_icon_on(
            &painter,
            "legacy/DEPRECATED_warningCircle",
            Rect::from_min_size(origin, Vec2::splat(WARNING)),
            color,
        );
    }
    if !look.time_on_same_line && !nowrap && !warning {
        // A wrapping title alone: broken the way Chromium breaks it, each line placed on its own
        // (egui's continuation rows land off Chromium's sub-pixel phase), the last of `lines`
        // ending in "…" when more follows.
        let wrapped = title_style.wrap(painter.ctx(), &event.title, width);
        let shown = wrapped.len().min(lines);
        for (row, line) in wrapped.iter().take(shown).enumerate() {
            let text = if row + 1 == shown && wrapped.len() > shown && !look.clipped {
                wrapped[row..].join(" ")
            } else {
                line.clone()
            };
            let mut job = LayoutJob::single_section(text, title_style.format(color));
            job.wrap.max_width = width;
            job.wrap.max_rows = 1;
            job.wrap.overflow_character = Some('…');
            paint_rows(
                &painter,
                origin + vec2(0.0, row as f32 * title_style.line_height),
                job,
                &title_style,
                declined,
                color,
            );
        }
        if shown < lines && !look.no_time {
            let mut job = LayoutJob::single_section(time, time_style.format(color));
            job.wrap.max_width = width;
            job.wrap.max_rows = 1;
            job.wrap.overflow_character = Some('…');
            paint_rows(
                &painter,
                origin + vec2(0.0, shown as f32 * title_style.line_height),
                job,
                &time_style,
                declined,
                color,
            );
        }
        return;
    }
    job.append(
        &event.title,
        if warning { WARNING + WARNING_GAP } else { 0.0 },
        title_style.format(color),
    );
    if look.time_on_same_line && !look.no_time {
        job.append(" ", TIME_GAP, title_style.format(color));
        job.append(&time, 0.0, time_style.format(color));
    }
    job.wrap.max_width = width;
    job.wrap.max_rows = if nowrap { 1 } else { lines };
    // `white-space: nowrap; text-overflow: ellipsis` cuts mid-word.
    job.wrap.break_anywhere = nowrap;
    job.wrap.overflow_character = Some('…');
    let rows = paint_rows(&painter, origin, job, &title_style, declined, color);
    if !look.time_on_same_line && rows < lines && !look.no_time {
        let mut job = LayoutJob::single_section(time, time_style.format(color));
        job.wrap.max_width = width;
        job.wrap.max_rows = 1;
        job.wrap.overflow_character = Some('…');
        paint_rows(
            &painter,
            origin + vec2(0.0, rows as f32 * title_style.line_height),
            job,
            &time_style,
            declined,
            color,
        );
    }
}

/// The remove button's 8px cross: 6px in from a slot's top right.
pub fn slot_remove(rect: Rect) -> Rect {
    Rect::from_min_size(
        egui::pos2(rect.right() - 6.0 - 8.0, rect.top() + 6.0),
        Vec2::splat(8.0),
    )
}

/// A slot of availability being shared: the calendar's colour, or red's when it can't be shared,
/// each paler once past; a dashed border in place of the border (an SVG `rect`'s 2px stroke,
/// dashed every 6px, half of it inside the box), no shadow, `padding: 5px 6px`, its time in white,
/// and the remove cross.
fn paint_slot(
    ui: &Ui,
    clip: Rect,
    rect: Rect,
    event: &Event,
    valid: bool,
    Look { past, covered, .. }: Look,
) {
    let painter = ui.painter_at(clip);
    let theme = exo_ui::theme(ui.ctx());
    let color = if valid {
        event.color
    } else {
        exo_ui::CalendarColor::from_name("red").unwrap_or(event.color)
    };
    let fill = color
        .block(
            theme,
            BlockPalette {
                past,
                ..BlockPalette::default()
            },
        )
        .fill;
    let box_rect = exo_ui::pixel_snap(rect);
    let radius = if rect.height() < 22.0 {
        CornerRadius::same(2)
    } else {
        CornerRadius::same(4)
    };
    painter.rect_filled(box_rect, radius, fill);
    // The dashes white, Carbon `rgb(39, 41, 45)`; the text `#fff`, Carbon `hsla(0,0%,100%,.9)`
    // (`.8` past).
    let (dash, text) = match theme {
        exo_ui::Theme::Snow => (Color32::WHITE, Color32::WHITE),
        exo_ui::Theme::Carbon => (
            Color32::from_rgb(39, 41, 45),
            Color32::WHITE.gamma_multiply(if past { 0.8 } else { 0.9 }),
        ),
    };
    paint_dashes(
        &painter.with_clip_rect(box_rect.intersect(clip)),
        box_rect,
        dash,
    );
    if covered {
        return;
    }
    let origin = rect.min + vec2(6.0, 5.0);
    let mut job = LayoutJob::single_section(event.title.clone(), TITLE.format(text));
    job.wrap.max_width = rect.width() - 12.0 - 14.0;
    job.wrap.max_rows = 1;
    job.wrap.overflow_character = Some('…');
    paint_rows(
        &painter.with_clip_rect(box_rect.intersect(clip)),
        origin,
        job,
        &TITLE,
        false,
        text,
    );
    exo_ui::paint_icon_on(
        &painter,
        "legacy/DEPRECATED_closeV3",
        slot_remove(rect),
        text,
    );
}

/// The dashes of an SVG `rect`'s stroke (`stroke-dasharray: 6`) around `rect`, from its top left
/// clockwise, the inner pixel of each.
fn paint_dashes(painter: &Painter, rect: Rect, color: Color32) {
    let edges = [
        (rect.left_top(), Vec2::X, rect.width()),
        (rect.right_top(), Vec2::Y, rect.height()),
        (rect.right_bottom(), -Vec2::X, rect.width()),
        (rect.left_bottom(), -Vec2::Y, rect.height()),
    ];
    // Where the path is in the 12px on/off pattern.
    let mut along = 0.0f32;
    for (start, direction, length) in edges {
        let mut at = 0.0;
        while at < length {
            let phase = along % 12.0;
            let step = if phase < 6.0 {
                6.0 - phase
            } else {
                12.0 - phase
            }
            .min(length - at);
            if phase < 6.0 {
                let from = start + direction * at;
                let to = start + direction * (at + step);
                // The inner half of the 2px stroke.
                let inward = Vec2::new(-direction.y, direction.x);
                painter.rect_filled(Rect::from_two_pos(from, to + inward), 0.0, color);
            }
            at += step;
            along += step;
        }
    }
}

/// Paints a block's text job at `origin`, struck through for a declined event. Returns its row
/// count.
fn paint_rows(
    painter: &Painter,
    origin: egui::Pos2,
    job: LayoutJob,
    style: &TextStyle,
    struck: bool,
    color: Color32,
) -> usize {
    let galley = exo_ui::layout(painter.ctx(), job.clone());
    let rows = galley.rows.len();
    exo_ui::paint_job(painter, origin, job, style);
    if struck {
        for row in &galley.rows {
            let y = origin.y + row.pos.y + style.line_height / 2.0;
            painter.hline(
                origin.x..=origin.x + row.rect().width(),
                y,
                Stroke::new(1.0, color),
            );
        }
    }
    rows
}

/// How far the glow has come on for `id` (100ms ease).
pub fn glow(ctx: &egui::Context, id: egui::Id, on: bool) -> f32 {
    exo_ui::motion::transition(
        ctx,
        id,
        on,
        GLOW_DURATION,
        exo_ui::motion::CubicBezier::EASE,
    )
}
