//! One thread row and the date separators.

use egui::{
    Color32, CornerRadius, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2, epaint::PathShape,
    pos2, text::LayoutJob, vec2,
};
use exo_ui::{
    Family, Font, IconButton, KeyTooltip, LabelChip, Placement, TextStyle, chip_hover,
    motion::{self, CubicBezier},
    paint_icon, paint_job,
};
use jiff::Zoned;

use std::ops::Range;

use crate::{
    format::{
        dates, failure,
        names::{NameFormat, display_name},
        row as text,
    },
    mailbox::{Attachments, ThreadSummary},
};
use exo_store::ReminderStatus;

/// Every text in a row: 12px/16px.
const REGULAR: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
const BOLD: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 16.0);
/// The date: `letter-spacing: .017em`.
const DATE: TextStyle = REGULAR.letter_spacing(0.017 * 12.0);
/// A failed or offline draft's note: bolder, no letter spacing, `#ef8e8e` and `#ce9ece` in Snow and
/// Carbon.
const FAILED: TextStyle = BOLD;
const FAILED_INK: Color32 = Color32::from_rgb(0xef, 0x8e, 0x8e);
const OFFLINE_INK: Color32 = Color32::from_rgb(0xce, 0x9e, 0xce);
/// The margin before an offline draft's note.
const OFFLINE_MARGIN: f32 = 6.0;
/// `padding-top: calc(8px + 2px)` above the 16px first row.
const FIRST_ROW: f32 = 10.0;
/// The date's column: at least 52px wide, 4px from what's before it.
const DATE_MIN_WIDTH: f32 = 52.0;
const DATE_MARGIN: f32 = 4.0;
/// Text on a selected row: the subject and the rest.
const ON_SELECTED: Color32 = Color32::WHITE;

/// The from column's names: `rgba(0, 0, 0, .85)`.
fn from_ink(palette: &exo_ui::Palette) -> Color32 {
    palette.ink(0.85, 0.85)
}

/// Row metrics for one list width.
pub(super) struct Geometry {
    padding_left: f32,
    padding_right: f32,
    rem: f32,
    /// The from column: 22.5% of the row.
    from_width: f32,
    /// The from column minus its right padding.
    from_label_width: f32,
    /// The middle column of a row with only a date, which subjects and snippets are cut to.
    center_width: f64,
}

impl Geometry {
    /// The middle column rows cut their subjects and snippets to.
    pub(super) fn center_width(&self) -> f64 {
        self.center_width
    }

    /// Metrics for a list `width` px wide; `compact` is `@media (max-width: 1152px)`.
    pub(super) fn new(width: f32, rem: f32, compact: bool) -> Self {
        let padding_left = if compact { 44.0 } else { 3.5 * rem };
        let padding_right = 2.0 * rem - 6.0;
        let content = width - padding_left - padding_right;
        let from_width = 0.225 * content;
        // The column's `padding-right: 2rem`, in whole px.
        let from_label_width = from_width - (2.0 * rem).trunc();
        let center_width = f64::from(content - from_width - DATE_MIN_WIDTH - DATE_MARGIN);
        Self {
            padding_left,
            padding_right,
            rem,
            from_width,
            from_label_width,
            center_width,
        }
    }

    /// The rows' left padding (`3.5rem`, 44px compact), where text starts.
    pub(super) fn padding_left(&self) -> f32 {
        self.padding_left
    }

    fn content(&self, row: Rect) -> (f32, f32) {
        (
            row.left() + self.padding_left,
            row.right() - self.padding_right,
        )
    }
}

/// A row's state.
pub(super) struct State<'a> {
    /// The list cursor is on it: the hover actions replace the date.
    pub cursor: bool,
    /// It's in the multi-selection.
    pub selected: bool,
    /// The rows above and below join this one into a block (selected, or the cursor), which
    /// squares the corners between them when it's selected.
    pub joined_above: bool,
    pub joined_below: bool,
    /// The view can mark it done: its hover actions include the check (not in Spam).
    pub archivable: bool,
    /// A bulk action waits or runs for it: this shows instead of the date and hover actions.
    pub pending: Option<&'a str>,
    /// Gmail can't be reached: a send waiting to go says so instead of the date.
    pub offline: bool,
    /// A search result with files: their strip under the row, scrolled this far.
    pub strip: Option<f32>,
}

/// What was clicked inside a row, besides the row itself.
pub(super) enum Hit {
    /// The multi-select checkmark.
    Checkmark,
    /// A label chip.
    Label(String),
    /// Mark Done.
    Done,
    /// Remind me.
    Remind,
    /// The command palette.
    Command,
    /// The unread dot or star, over the checkmark: a click on the row.
    Status,
    /// A card of the strip: the file at this index, previewed, or saved with its download
    /// button.
    File { index: usize, download: bool },
    /// The strip's arrow that way (-1, 1).
    Page(isize),
}

/// Paints a date separator.
pub(super) fn separator(painter: &Painter, rect: Rect, geometry: &Geometry, label: &str) {
    let palette = exo_ui::palette(painter.ctx());
    // `padding-top: calc(10px + 12px)`, `top: 1px`; `rgba(0, 0, 0, .35)`.
    BOLD.paint_line(
        painter,
        pos2(rect.left() + geometry.padding_left, rect.top() + 23.0),
        label,
        palette.text_base_accent,
        None,
    );
}

/// Paints `thread`'s row in `rect` and reports what inside it was clicked.
pub(super) fn paint(
    ui: &mut Ui,
    painter: &Painter,
    rect: Rect,
    geometry: &Geometry,
    thread: &ThreadSummary,
    state: &State<'_>,
    now: &Zoned,
) -> Option<Hit> {
    let palette = exo_ui::palette(ui.ctx());
    let (left, right) = geometry.content(rect);
    let first_row = rect.top() + FIRST_ROW;
    let id = ui.id().with(("row", &thread.id));
    let mut hit = None;
    // The cursor's and selected rows draw their status animated, with its tooltip while it's
    // hovered (the rest change at once, with no tooltip).
    let lively = state.cursor || state.selected;
    let origin = pos2(left, first_row + 0.5);

    if state.selected {
        // A selected row: its 2px corners square where selected rows meet.
        let (top, bottom) = (
            if state.joined_above { 0 } else { 2 },
            if state.joined_below { 0 } else { 2 },
        );
        painter.rect_filled(
            exo_ui::pixel_snap(rect),
            CornerRadius {
                nw: top,
                ne: top,
                sw: bottom,
                se: bottom,
            },
            palette.select_fill_default,
        );
    }
    let (strong, subdued) = if state.selected {
        (ON_SELECTED, ON_SELECTED)
    } else {
        (palette.text_base_default, palette.text_base_subdued)
    };

    let background = if state.selected {
        palette.select_fill_default
    } else if state.cursor {
        palette.background_brand_subdued
    } else {
        palette.background_base_default
    };
    if checkmark(
        ui,
        painter,
        pos2(left - geometry.rem - 36.0, rect.top()),
        id,
        state.selected,
        background,
    ) {
        hit = Some(Hit::Checkmark);
    }
    // The status sits over the checkmark's clickable: where it shows, the checkmark isn't hovered
    // or clicked. A failed draft's dot is red, with no star.
    let status = thread
        .failure
        .is_none()
        .then(|| status(thread, lively, ui.ctx(), id));
    // The failed dot's box is the unread dot's.
    let area = status
        .unwrap_or_else(|| exo_ui::Status::resting(exo_ui::Unread::Yes, false))
        .rect(origin);
    if lively && let Some(area) = area {
        let response = ui.interact(area, id.with("status"), Sense::click());
        if let Some(text) = status_tooltip(thread) {
            // The tooltip's left edge on the status's 0×0 box, its middle 8px below it.
            KeyTooltip { text, keys: &[] }.show_at(
                ui,
                &response,
                Placement::Beside(origin + vec2(0.0, 8.0)),
            );
        }
        if response.clicked() {
            hit = Some(Hit::Status);
        }
    }
    match &status {
        Some(status) => exo_ui::paint_status(painter, origin, status),
        None => exo_ui::paint_failed_status(painter, origin),
    }

    // From.
    from_label(
        painter,
        pos2(left, first_row + 1.0),
        geometry.from_label_width,
        thread,
        state.selected,
    );

    // A bulk action's status, metadata on the right, or the hover actions on the cursor's row.
    let middle_right = if let Some(pending) = state.pending {
        // A bulk action's status: bold, `top: 1px`, `margin-left: 1rem`, `#f67b7b`, or white on a
        // selected row.
        let color = if state.selected {
            ON_SELECTED
        } else {
            Color32::from_rgb(0xf6, 0x7b, 0x7b)
        };
        let width = BOLD.width(ui.ctx(), pending);
        BOLD.paint_line(
            painter,
            pos2(right - width, first_row + 1.0),
            pending,
            color,
            None,
        );
        right - width - geometry.rem
    } else if state.cursor && !state.selected && !editable_draft(thread) {
        let (actions_left, action) = actions(ui, rect, right, id, thread, state.archivable, now);
        hit = hit.or(action);
        actions_left
    } else {
        metadata(
            ui,
            painter,
            pos2(right, first_row),
            geometry.rem,
            thread,
            state,
            now,
        )
    };

    if let Some(scrolled) = state.strip {
        hit = strip(ui, painter, rect, geometry, thread, state, scrolled).or(hit);
    }

    // Labels, subject and snippet, cut to the estimated width of the middle column.
    let middle_left = left + geometry.from_width;
    let mut x = middle_left;
    for label in &thread.labels {
        let chip = LabelChip::new(painter, label, state.selected);
        let width = chip.width();
        // A label's clickable fills the row's height, so the whole strip is clickable.
        let strip = Rect::from_min_size(pos2(x, rect.top()), vec2(width, rect.height()));
        let response = ui.interact(strip, id.with(("label", label)), Sense::click());
        let hover = chip_hover(
            ui.ctx(),
            response.id,
            response.hovered(),
            response.is_pointer_button_down_on(),
        );
        chip.paint(painter, pos2(x, first_row), hover);
        if response.clicked() {
            hit = Some(Hit::Label(label.clone()));
        }
        x += width + 6.0;
    }
    // A search result's highlights are cut with the text.
    let (subject_marks, snippet_marks) =
        thread.search.as_ref().map_or((&[][..], &[][..]), |marks| {
            (&marks.subject[..], &marks.snippet[..])
        });
    let subject = text::subject(
        &text::marked(&thread.subject, subject_marks, text::MARKERS),
        geometry.center_width,
        &thread.labels,
    );
    let (shown, marks) = text::unmark(&subject);
    let subject_width = paint_marked(
        painter,
        pos2(x, first_row + 1.0),
        &REGULAR,
        &shown,
        &marks,
        strong,
        None,
    );
    if let Some((snippet, cut)) = text::snippet(
        &thread.snippet,
        snippet_marks,
        geometry.center_width,
        &thread.labels,
        &subject,
        thread.done,
    ) {
        let (snippet, marks) = text::unmark(&snippet);
        let snippet = if cut {
            format!("{snippet}…")
        } else {
            snippet
        };
        let snippet_left = x + subject_width + geometry.rem;
        paint_marked(
            painter,
            pos2(snippet_left, first_row + 1.0),
            &REGULAR,
            &snippet,
            &marks,
            subdued,
            Some((middle_right - snippet_left).max(0.0)),
        );
    }
    hit
}

/// `thread`'s unread dot and star, animated on a `lively` row.
fn status(
    thread: &ThreadSummary,
    lively: bool,
    ctx: &egui::Context,
    id: egui::Id,
) -> exo_ui::Status {
    let unread = if thread.reminder_unread() {
        exo_ui::Unread::Reminder
    } else if thread.unread {
        exo_ui::Unread::Yes
    } else {
        exo_ui::Unread::No
    };
    if lively {
        exo_ui::animated_status(ctx, id.with("status"), unread, thread.starred)
    } else {
        exo_ui::Status::resting(unread, thread.starred)
    }
}

/// A file card's width and the gap after it.
pub(super) const CARD_STEP: f32 = 184.0 + 8.0;

impl Geometry {
    /// The strip's width (the content's) and, for `files` cards, the width of what it scrolls (the
    /// from column's 22.5%, then the cards 8px apart).
    pub(super) fn strip_widths(&self, files: usize) -> (f32, f32) {
        let outer = self.from_width / 0.225;
        (outer, self.from_width + files as f32 * CARD_STEP - 8.0)
    }
}

/// The strip of a search result's files under its first row, scrolled `scrolled` px: 184px cards
/// 8px apart from the subject column, cut off at the content's edges, fading out where more lie
/// beyond into the row's background; on the cursor's row (not a selected one), round buttons that
/// page it.
fn strip(
    ui: &Ui,
    painter: &Painter,
    rect: Rect,
    geometry: &Geometry,
    thread: &ThreadSummary,
    state: &State<'_>,
    scrolled: f32,
) -> Option<Hit> {
    let ctx = ui.ctx();
    let id = ui.id().with(("row-files", &thread.id));
    let palette = exo_ui::palette(ctx);
    let dark = palette.theme == exo_ui::Theme::Carbon;
    let (left, right) = geometry.content(rect);
    let (outer, inner) = geometry.strip_widths(thread.files.len());
    // `padding: 2px 0 4px` from 26px down, the cards 4px into it.
    let area = Rect::from_min_max(
        pos2(left, rect.top() + 26.0),
        pos2(right, rect.top() + 26.0 + 41.0),
    );
    let clip = painter.with_clip_rect(painter.clip_rect().intersect(area));
    // It slides over 100ms, on whole px.
    let shown = motion::transition_value(
        ctx,
        id.with("scroll"),
        scrolled,
        0.1,
        0.0,
        CubicBezier::EASE,
    )
    .floor();
    let mut hit = None;
    for (index, file) in thread.files.iter().enumerate() {
        let card = Rect::from_min_size(
            pos2(
                left + geometry.from_width - shown + index as f32 * CARD_STEP,
                rect.top() + 32.0,
            ),
            exo_ui::SMALL_CARD_SIZE,
        );
        if !card.intersects(area) {
            continue;
        }
        let response = ui.interact(
            card.intersect(area),
            id.with(("card", index)),
            Sense::click(),
        );
        let download = ui.interact(
            exo_ui::small_card_download(card).intersect(area),
            id.with(("download", index)),
            Sense::click(),
        );
        let hovered = response.hovered() || download.hovered();
        let hover = motion::transition(
            ctx,
            id.with(("hover", index)),
            hovered && !response.is_pointer_button_down_on(),
            0.25,
            CubicBezier::EASE,
        );
        exo_ui::SmallAttachmentCard {
            filename: &file.filename,
            mime_type: &file.mime_type,
            hover,
            hovered,
            download_hovered: download.hovered(),
        }
        .paint(ui, &clip, card);
        if hovered {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if download.clicked() && hovered {
            hit = Some(Hit::File {
                index,
                download: true,
            });
        } else if response.clicked() {
            hit = Some(Hit::File {
                index,
                download: false,
            });
        }
    }
    // The fades go to the row's background: white or `#27292d`; the cursor's, at 80% (20% for the
    // left one); a selected row's blue.
    let background = if state.selected {
        Color32::from_rgb(84, 172, 220)
    } else if state.cursor {
        // `hsl(220, 45%, 97%)`, `rgb(70, 75, 83)`: the highlight's.
        palette.background_brand_subdued
    } else if dark {
        Color32::from_rgb(39, 41, 45)
    } else {
        Color32::WHITE
    };
    let solid = if state.cursor && !state.selected {
        0.2
    } else {
        0.0
    };
    // `linear-gradient(to right, transparent, {background})` (`{background} 80%` on the cursor's
    // row) and its mirror for the left (`{background} 20%`).
    let fade = |rect: Rect, towards_right: bool| {
        let mut mesh = egui::Mesh::default();
        let steps = 12;
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            let x = rect.left() + rect.width() * t;
            let towards = if towards_right { t } else { 1.0 - t };
            let color = background.gamma_multiply((towards / (1.0 - solid)).clamp(0.0, 1.0));
            mesh.colored_vertex(pos2(x, rect.top()), color);
            mesh.colored_vertex(pos2(x, rect.bottom()), color);
            if step > 0 {
                let base = 2 * step as u32;
                mesh.add_triangle(base - 2, base - 1, base);
                mesh.add_triangle(base - 1, base, base + 1);
            }
        }
        painter.add(Shape::mesh(mesh));
    };
    let has_previous = scrolled > 0.0;
    let has_next = inner - outer > scrolled;
    let fades_top = rect.bottom() - 8.0 - 28.0;
    if has_next {
        // `right: calc(2rem - 6px); bottom: 8px`, 30×28.
        fade(
            Rect::from_min_size(pos2(right - 30.0, fades_top), vec2(30.0, 28.0)),
            true,
        );
    }
    if has_previous {
        // `left: 3.5rem`.
        fade(
            Rect::from_min_size(pos2(left, fades_top), vec2(30.0, 28.0)),
            false,
        );
    }
    if state.cursor && !state.selected {
        // `-navigation-right-clickable { top: 30px; padding: 2px calc(2rem - 7px) 8px 10px }` and
        // the left one's `padding: 2px 10px 8px calc(3.5rem - 12px)`: 28px white circles, their
        // chevrons 15px.
        let buttons = [
            (-1, has_previous, left - 12.0),
            (
                1,
                has_next,
                rect.right() - (geometry.padding_right - 1.0) - 28.0,
            ),
        ];
        for (direction, shown, circle_left) in buttons {
            if !shown {
                continue;
            }
            let circle = exo_ui::pixel_snap(Rect::from_min_size(
                pos2(circle_left, rect.top() + 32.0),
                Vec2::splat(28.0),
            ));
            let clickable = if direction < 0 {
                Rect::from_min_max(
                    pos2(rect.left(), circle.top() - 2.0),
                    pos2(circle.right() + 10.0, circle.bottom() + 8.0),
                )
            } else {
                Rect::from_min_max(
                    pos2(circle.left() - 10.0, circle.top() - 2.0),
                    pos2(rect.right(), circle.bottom() + 8.0),
                )
            };
            let response = ui
                .interact(clickable, id.with(("page", direction)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let lift = motion::transition(
                ctx,
                id.with(("page-hover", direction)),
                response.hovered(),
                0.15,
                CubicBezier::EASE,
            );
            let shadow = |y: f32, blur: f32, alpha: f32| {
                exo_ui::BoxShadow::new(
                    [0.0, y],
                    blur,
                    0.0,
                    Color32::from_black_alpha((alpha * 255.0).round() as u8),
                )
            };
            shadow(1.0 + lift, 6.0 + 2.0 * lift, 0.1 + 0.15 * lift).paint(
                painter,
                circle,
                CornerRadius::same(14),
            );
            painter.circle_filled(circle.center(), 14.0, Color32::WHITE);
            // The chevrons' `opacity: .4` (`.6` hovered) over their path's own `.4`.
            let ink = Color32::from_black_alpha(((0.4 + 0.2 * lift) * 0.4 * 255.0).round() as u8);
            let glyph_left = if direction < 0 {
                circle.left() + 8.0
            } else {
                circle.left() + 11.0
            };
            chevron(
                painter,
                pos2(glyph_left, circle.top() + 6.5),
                direction,
                ink,
            );
            if response.clicked() {
                hit = Some(Hit::Page(direction));
            }
        }
    }
    hit
}

/// A strip button's chevron: the 8×14 path of `-left-chevron-icon` / `-right-chevron-icon`, a
/// 1px stroke scaled to its 15px box, pointing `direction`.
fn chevron(painter: &Painter, at: Pos2, direction: isize, color: Color32) {
    let scale = 15.0 / 14.0;
    let (tip, tail) = if direction < 0 {
        (1.0, 7.0)
    } else {
        (7.0, 1.0)
    };
    let points = vec![
        at + vec2(tail, 1.0) * scale,
        at + vec2(tip, 7.0) * scale,
        at + vec2(tail, 13.0) * scale,
    ];
    painter.add(Shape::Path(PathShape::line(
        points,
        Stroke::new(scale, color),
    )));
}

/// The status's tooltip: a failed draft's failure, else what the dot and the star mean. Exo's
/// reminders are all set by hand, so none is `an Auto Reminder`.
fn status_tooltip(thread: &ThreadSummary) -> Option<&'static str> {
    if let Some(reason) = thread.failure {
        return Some(failure::information(reason).notification);
    }
    match (thread.reminder_unread(), thread.unread, thread.starred) {
        (true, _, true) => Some("Conversation is Starred and returned from a Reminder"),
        (true, _, false) => Some("Conversation returned from a Reminder"),
        (false, true, true) => Some("Conversation is Starred and Unread"),
        (false, true, false) => Some("Conversation is Unread"),
        (false, false, true) => Some("Conversation is Starred"),
        (false, false, false) => None,
    }
}

/// A draft of its own that can still be edited (not sending, not failed): its row keeps its date
/// under the cursor rather than the hover actions.
fn editable_draft(thread: &ThreadSummary) -> bool {
    thread.draft_only && !thread.sending && thread.failure.is_none()
}

/// Paints one line of `text` (cut, maybe ending in `…`) with its line box at `at`, with what a
/// search matched (`marks`, byte ranges of `text`) highlighted as `Snippet.highlight`
/// does: its background behind the 12px font's 15px content area, and its text color. Returns the
/// text's width.
fn paint_marked(
    painter: &Painter,
    at: Pos2,
    style: &TextStyle,
    text: &str,
    marks: &[Range<usize>],
    color: Color32,
    max_width: Option<f32>,
) -> f32 {
    let palette = exo_ui::palette(painter.ctx());
    let mut job = LayoutJob::default();
    let mut done = 0;
    let limit = max_width.unwrap_or(f32::INFINITY);
    for range in marks {
        let (start, end) = (range.start.min(text.len()), range.end.min(text.len()));
        if start < done
            || start >= end
            || !text.is_char_boundary(start)
            || !text.is_char_boundary(end)
        {
            continue;
        }
        let left = style.width(painter.ctx(), &text[..start]);
        let right = style.width(painter.ctx(), &text[..end]).min(limit);
        if left < right {
            let behind =
                Rect::from_min_max(pos2(at.x + left, at.y), pos2(at.x + right, at.y + 15.0));
            painter.rect_filled(
                exo_ui::pixel_snap(behind),
                0.0,
                palette.highlight_pro_default,
            );
        }
        job.append(&text[done..start], 0.0, style.format(color));
        // A highlight's text.
        job.append(&text[start..end], 0.0, style.format(palette.ink(0.85, 0.8)));
        done = end;
    }
    job.append(&text[done..], 0.0, style.format(color));
    if let Some(width) = max_width {
        job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    }
    paint_job(painter, at, job, style)
}

/// Where a search result's names show highlighted: a participant whose address the query names,
/// whole; otherwise each word of a name the query has as a word.
fn name_marks(names: &str, thread: &ThreadSummary, words: &[String]) -> Vec<Range<usize>> {
    let mut marks: Vec<Range<usize>> = Vec::new();
    let format = if thread.participants.len() == 1 {
        NameFormat::Full
    } else {
        NameFormat::First
    };
    let mut from = 0;
    for participant in &thread.participants {
        let shown = display_name(
            &participant.person,
            participant.is_me,
            format,
            thread.participants.len() > 1,
        );
        let Some(at) = names[from..].find(&shown).map(|at| at + from) else {
            continue;
        };
        if words
            .iter()
            .any(|word| word.eq_ignore_ascii_case(&participant.person.email))
        {
            marks.push(at..at + shown.len());
        } else {
            let mut offset = at;
            for part in shown.split(' ') {
                if words.iter().any(|word| word.eq_ignore_ascii_case(part)) {
                    marks.push(offset..offset + part.len());
                }
                offset += part.len() + 1;
            }
        }
        from = at + shown.len();
    }
    marks
}

/// The multi-select checkmark in the row's left padding. Returns whether it was clicked.
/// `background` is the row's color under it.
fn checkmark(
    ui: &Ui,
    painter: &Painter,
    origin: Pos2,
    id: egui::Id,
    selected: bool,
    background: Color32,
) -> bool {
    // The checkmark's clickable: a 15px box with margins 11/6/11/11 and 10px of right padding.
    let area = Rect::from_min_size(origin, vec2(11.0 + 15.0 + 6.0 + 10.0, 11.0 + 15.0 + 11.0));
    let response = ui.interact(area, id.with("checkmark"), Sense::click());
    let target = match (selected, response.hovered()) {
        (true, true) => 1.0,
        (true, false) => 0.85,
        (false, true) => 0.2,
        (false, false) => 0.0,
    };
    // `transition: opacity 150ms ease`, approximated by egui's linear value animation.
    let opacity = ui
        .ctx()
        .animate_value_with_time(id.with("checkmark-opacity"), target, 0.15);
    if opacity > 0.0 {
        let palette = exo_ui::palette(ui.ctx());
        // Fading the drawing as a whole, by mixing with the background, keeps the tick's
        // overlapping strokes from doubling up. Its strokes: black (white in Carbon), white when
        // selected.
        let color = if selected {
            ON_SELECTED
        } else {
            palette.ink(1.0, 1.0)
        };
        paint_checkmark(
            painter,
            origin + vec2(11.0, 11.0),
            selected,
            exo_ui::motion::mix(background, color, opacity),
        );
    }
    response.clicked()
}

/// The 15×14 checkmark: a rounded square open at its top-right corner, with a tick when checked.
/// The SVG strokes 2px inside a mask of its own shape, which leaves the inner pixel.
pub(super) fn paint_checkmark(painter: &Painter, at: Pos2, checked: bool, color: Color32) {
    // An SVG: it paints on the whole px (see `pixel_snap`).
    let at = exo_ui::pixel_snap(Rect::from_min_size(at, Vec2::splat(15.0))).min;
    let corner = |center: Pos2, from: f32| {
        (0..=4).map(move |i| center + 1.5 * Vec2::angled((from + i as f32 * 22.5).to_radians()))
    };
    let mut outline = vec![at + vec2(8.875, 0.5)];
    outline.extend(
        corner(at + vec2(2.0, 2.0), 180.0)
            .collect::<Vec<_>>()
            .into_iter()
            .rev(),
    );
    outline.extend(
        corner(at + vec2(2.0, 12.0), 90.0)
            .collect::<Vec<_>>()
            .into_iter()
            .rev(),
    );
    outline.extend(
        corner(at + vec2(12.0, 12.0), 0.0)
            .collect::<Vec<_>>()
            .into_iter()
            .rev(),
    );
    outline.push(at + vec2(13.5, 8.0));
    painter.add(Shape::Path(PathShape::line(
        outline,
        Stroke::new(1.0, color),
    )));
    if checked {
        let tick = vec![
            at + vec2(4.0, 6.142),
            at + vec2(6.858, 9.0),
            at + vec2(13.445, 2.413),
        ];
        painter.add(Shape::Path(PathShape::line(tick, Stroke::new(2.0, color))));
    }
}

fn from_label(painter: &Painter, at: Pos2, width: f32, thread: &ThreadSummary, selected: bool) {
    let palette = exo_ui::palette(painter.ctx());
    // A search result counts its messages after the names (` (N)`, bold, subdued,
    // `padding-left: 1px`), inside the from column.
    let count =
        (thread.search.is_some() && thread.messages > 1).then(|| format!(" ({})", thread.messages));
    let count_width = count.as_ref().map_or(0.0, |count| {
        1.0 + BOLD
            .single_line(painter, count, Color32::PLACEHOLDER, None)
            .size()
            .x
    });
    let ink = if selected {
        ON_SELECTED
    } else {
        from_ink(palette)
    };
    let width = width - count_width;
    let searched = thread
        .search
        .as_ref()
        .map_or(&[][..], |marks| &marks.names[..]);
    let names = text::participants(
        &thread.participants,
        (!thread.recipients).then_some(f64::from(width)),
        searched,
    );
    let mut job = LayoutJob::default();
    if thread.drafts > 0 {
        let marker = if thread.drafts == 1 {
            "Draft "
        } else {
            "Drafts "
        };
        let color = if selected {
            ON_SELECTED
        } else {
            palette.actions_draft
        };
        job.append(marker, 0.0, BOLD.format(color));
        if !thread.participants.is_empty() {
            job.append("to ", 0.0, BOLD.format(ink));
        }
    }
    let names_width = match &thread.search {
        Some(marks) if thread.drafts == 0 => paint_marked(
            painter,
            at,
            &BOLD,
            &names,
            &name_marks(&names, thread, &marks.names),
            ink,
            Some(width),
        ),
        _ => {
            job.append(&names, 0.0, BOLD.format(ink));
            job.wrap = egui::text::TextWrapping::truncate_at_width(width);
            paint_job(painter, at, job, &BOLD)
        }
    };
    if let Some(count) = count {
        // The message count: `rgba(0, 0, 0, .4)`.
        let color = if selected {
            ON_SELECTED
        } else {
            palette.ink(0.4, 0.4)
        };
        BOLD.paint_line(
            painter,
            pos2(at.x + names_width + 1.0, at.y),
            &count,
            color,
            None,
        );
    }
}

/// The attachment icon and date, right-aligned at `top_right` (the first row's top right).
/// Returns the column's left edge.
fn metadata(
    ui: &Ui,
    painter: &Painter,
    top_right: Pos2,
    rem: f32,
    thread: &ThreadSummary,
    state: &State<'_>,
    now: &Zoned,
) -> f32 {
    let (right, first_row) = (top_right.x, top_right.y);
    let selected = state.selected;
    let palette = exo_ui::palette(ui.ctx());
    // A failed draft's failure, offline a send waiting to go, or a scheduled send's time takes the
    // date's place, white on a selected row as the date is. A scheduled one is `Schedule:` until
    // Gmail has it, then `Scheduled:`, styled as the offline one.
    let ink = |color| if selected { ON_SELECTED } else { color };
    let (date, style, color, date_margin) = match (thread.failure, thread.scheduled) {
        (Some(reason), _) => (
            failure::information(reason).title.to_owned(),
            FAILED,
            ink(FAILED_INK),
            DATE_MARGIN,
        ),
        (None, None) if thread.sending && state.offline => (
            "Send: when online".to_owned(),
            FAILED,
            ink(OFFLINE_INK),
            OFFLINE_MARGIN,
        ),
        (None, Some((at, persisted))) => {
            let label = if persisted { "Scheduled" } else { "Schedule" };
            (
                format!("{label}: {}", dates::row(at, now)),
                FAILED,
                ink(OFFLINE_INK),
                OFFLINE_MARGIN,
            )
        }
        // A reminder's time is styled as the scheduled one is.
        (None, None) if let Some(ReminderStatus::Set { at, .. }) = thread.reminder => (
            format!("Remind me: {}", dates::row(at, now)),
            FAILED,
            ink(OFFLINE_INK),
            OFFLINE_MARGIN,
        ),
        (None, None) => (
            dates::row(thread.received, now),
            DATE,
            ink(palette.text_base_subdued),
            DATE_MARGIN,
        ),
    };
    let text_width = style.single_line(painter, &date, color, None).size().x;
    let date_width = text_width.max(DATE_MIN_WIDTH);
    // `letter-spacing` also follows the last character, inside the right-aligned box.
    style.paint_line(
        painter,
        pos2(right - text_width - style.letter_spacing, first_row + 1.0),
        &date,
        color,
        None,
    );
    // Right to left: the date, the done check (not with a failed draft), the attachment icon,
    // each with its margin-left.
    let mut left = right - date_width;
    let mut margin = date_margin;
    if thread.done && thread.failure.is_none() {
        // The done check: 16×14, `margin-left: 4px` (10px after the paperclip, 12px after the
        // calendar), its stroke `#8892b9` 2 units wide, white on a selected row.
        let icon_rect =
            Rect::from_min_size(pos2(left - margin - 16.0, first_row), vec2(16.0, 14.0));
        exo_ui::paint_icon_stroked(
            ui,
            DONE_ICON,
            exo_ui::pixel_snap(icon_rect),
            if selected {
                ON_SELECTED
            } else {
                palette.actions_done_default
            },
            2.0,
        );
        left = icon_rect.left();
        margin = 4.0;
    }
    // `margin-left: 1rem`; the paperclip (19×8, 4px down) keeps 1px after it, the calendar (14×14,
    // 1px down) 4px; black at 40%, white on a selected row.
    let icon = match thread.attachment {
        Attachments::Files => Some((
            "legacy/DEPRECATED_paperclip",
            vec2(19.0, 8.0),
            4.0,
            1.0,
            10.0,
        )),
        Attachments::CalendarInvite => Some((
            "legacy/DEPRECATED_roundedCalendar",
            vec2(14.0, 14.0),
            1.0,
            4.0,
            12.0,
        )),
        Attachments::None => None,
    };
    if let Some((name, size, top, margin_right, done_margin)) = icon {
        let gap = margin_right + if thread.done { done_margin } else { margin };
        let icon_rect = Rect::from_min_size(pos2(left - gap - size.x, first_row + top), size);
        paint_icon(
            ui,
            name,
            exo_ui::pixel_snap(icon_rect),
            if selected {
                palette.select_content_default
            } else {
                palette.ink(0.4, 0.4)
            },
        );
        left = icon_rect.left();
        margin = rem;
    }
    left - margin
}

/// Mark Done (unless the view can't archive), Remind Me (not for a draft) and ⌘K, right-aligned 4px
/// past `right`: 24px boxes 4px apart (`gap: 5px`, `margin-left: -1px`), the 14px clock and command
/// glyphs 5px into theirs; the 16×14 check at the left of a 20px box; every glyph 10px below the
/// row's top. A done thread's check is `#8892b9`, `#5f74aa` hovered; a thread with a reminder's
/// clock is the bolder one in `#e0bfe0`, `#cd7acf` hovered. Returns the group's left edge and what
/// was clicked.
fn actions(
    ui: &Ui,
    row: Rect,
    right: f32,
    id: egui::Id,
    thread: &ThreadSummary,
    archivable: bool,
    now: &Zoned,
) -> (f32, Option<Hit>) {
    let done = thread.done;
    let mut hit = None;
    let mut left = right + 4.0;
    let remind = if thread.has_reminder() {
        REMIND_SET_ICON
    } else {
        REMIND_ICON
    };
    let buttons = [
        ("legacy/DEPRECATED_threadActionsCommand", Hit::Command),
        (remind, Hit::Remind),
        (DONE_ICON, Hit::Done),
    ];
    let shown = |action: &Hit| match action {
        Hit::Done => archivable,
        Hit::Remind => !thread.draft_only,
        _ => true,
    };
    for (icon, action) in buttons.into_iter().filter(|(_, action)| shown(action)) {
        let width = if matches!(action, Hit::Done) {
            20.0
        } else {
            24.0
        };
        let button = Rect::from_min_size(pos2(left - width, row.top() + 5.0), vec2(width, 24.0));
        let icon_rect = match action {
            Hit::Done => Rect::from_min_size(button.min + vec2(0.0, 5.0), vec2(16.0, 14.0)),
            _ => Rect::from_min_size(button.min + vec2(5.0, 5.0), Vec2::splat(14.0)),
        };
        let response = if done && matches!(action, Hit::Done) {
            let response = ui
                .interact(button, id.with(icon), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let color = if response.hovered() {
                Color32::from_rgb(0x5f, 0x74, 0xaa)
            } else {
                exo_ui::palette(ui.ctx()).actions_done_default
            };
            exo_ui::paint_icon_stroked(ui, icon, exo_ui::pixel_snap(icon_rect), color, 2.0);
            response
        } else if icon == REMIND_SET_ICON {
            IconButton::new(icon)
                .colors(
                    Color32::from_rgb(0xe0, 0xbf, 0xe0),
                    Color32::from_rgb(0xcd, 0x7a, 0xcf),
                )
                .show(ui, id.with(icon), button, icon_rect)
        } else {
            IconButton::new(icon).show(ui, id.with(icon), button, icon_rect)
        };
        // Under the button, 10px right of its centre.
        let tooltip = match action {
            Hit::Done if done => KeyTooltip {
                text: "Mark not Done",
                keys: &["shift", "E"],
            },
            Hit::Done => KeyTooltip {
                text: "Mark Done",
                keys: &["E"],
            },
            Hit::Remind => KeyTooltip {
                text: "Remind Me",
                keys: &["H"],
            },
            _ => KeyTooltip {
                text: "Exo Command",
                keys: &["⌘", "K"],
            },
        };
        let placement = Placement::Below {
            gap: 8.0,
            shift: 10.0,
        };
        match thread
            .reminder
            .as_ref()
            .filter(|_| matches!(action, Hit::Remind))
        {
            Some(reminder) => tooltip.show_at_with_status(
                ui,
                &response,
                placement,
                &crate::remind::status(reminder, now),
                egui::Rangef::new(row.left(), row.right()),
            ),
            None => tooltip.show_at(ui, &response, placement),
        }
        if response.clicked() {
            hit = Some(action);
        }
        left = button.left() - 4.0;
    }
    (left + 4.0, hit)
}

/// The done check.
const DONE_ICON: &str = "legacy/DEPRECATED_threadActionsDone";
/// The clock, bolder when a reminder is set.
pub(crate) const REMIND_ICON: &str = "legacy/DEPRECATED_threadActionsRemind-unset";
pub(crate) const REMIND_SET_ICON: &str = "legacy/DEPRECATED_threadActionsRemind";
