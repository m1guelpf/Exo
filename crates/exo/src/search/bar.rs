//! The search bar's look: the bar in the header with its lozenges and completion drawn under the
//! text, the suggestion list or history below it, and the hint an empty search shows.

use std::sync::LazyLock;

use egui::{
    Color32, CornerRadius, Id, Painter, Pos2, Rect, Sense, Ui, Vec2, pos2,
    text::{CCursor, CCursorRange, LayoutJob},
    vec2,
};
use exo_store::SearchHistoryItem;
use exo_ui::{Family, Font, IconButton, KeyTooltip, TextStyle, paint_job};
use regex::Regex;

use super::{
    Effect, Search,
    autocomplete::{self, Kind, OPERATORS},
    history_rows,
};

/// The bar's 28px top padding, above its 28px line.
const BAR_TOP: f32 = 28.0;
const BAR_HEIGHT: f32 = 28.0;
/// The bar's text: 16px/24px, `top: 2px`.
const INPUT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 24.0);
const INPUT_TOP: f32 = 2.0;
/// An inline span's background covers the 16px font's 20px content area, 2px inside the line.
const CONTENT_INSET: f32 = 2.0;
/// The lozenges' `box-shadow: 0 0 0 2px` and `border-radius: 1px`.
const RING: f32 = 2.0;
/// Classic's lozenges: the word being typed pink, the operators typed before it blue.
const CURRENT_LOZENGE: [u8; 3] = [249, 150, 255];
const LOZENGE: [u8; 3] = [84, 172, 220];
/// A lozenge's fill and ring: `rgba(…, .2)` both in Snow; in Carbon the fill solid (`#f996ff`,
/// `#54acdc`), the ring still `.2`.
fn lozenge_colors(ctx: &egui::Context, current: bool) -> (Color32, Color32) {
    let [r, g, b] = if current { CURRENT_LOZENGE } else { LOZENGE };
    let faint = Color32::from_rgba_unmultiplied(r, g, b, 51);
    match exo_ui::theme(ctx) {
        exo_ui::Theme::Snow => (faint, faint),
        exo_ui::Theme::Carbon => (Color32::from_rgb(r, g, b), faint),
    }
}
/// The layer under the text is at .6 opacity, and at 1 in Carbon.
fn underlay_opacity(ctx: &egui::Context) -> f32 {
    match exo_ui::theme(ctx) {
        exo_ui::Theme::Snow => 0.6,
        exo_ui::Theme::Carbon => 1.0,
    }
}
/// The 3px focus bar.
const FOCUS_BAR: f32 = 3.0;
/// Suggestion and history rows: 36px, text 12px/16px bold `padding-top: 11px; top: 1px`.
const ROW: f32 = 36.0;
const ROW_TEXT: f32 = 12.0;
const ITEM: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 16.0);
/// The suggestions' and the history's 21px top padding (the suggestions have 2px under them).
const LIST_TOP: f32 = 21.0;
const LIST_BOTTOM: f32 = 2.0;
/// The suggestions' divider: `margin: 6.5px 0 1.5px`, the history's `6.5px 0 4.5px`, each a 1px
/// border.
const DIVIDER: [f32; 2] = [6.5, 1.5];
const HISTORY_DIVIDER: [f32; 2] = [6.5, 4.5];
/// The dividers: 1px `#ededed`, `#5f6165` in Carbon.
fn divider_color(ctx: &egui::Context) -> Color32 {
    match exo_ui::theme(ctx) {
        exo_ui::Theme::Snow => Color32::from_rgb(0xed, 0xed, 0xed),
        exo_ui::Theme::Carbon => Color32::from_rgb(0x5f, 0x61, 0x65),
    }
}
/// The empty suggestion list's padding: 16px above, 4px below.
const EMPTY: f32 = 20.0;
/// The name column: 22.5% of the row's content box.
const NAME_COLUMN: f32 = 0.225;
/// The empty search's hint: 12px/20px, `top: 3px`, after the block's `padding-top: 8px`.
const HINT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
const HINT_BOLD: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 20.0);

/// The header's height in search mode: the bar, then the suggestions, the history or an empty
/// gap.
pub(crate) fn header_height(search: &Search, history: &[SearchHistoryItem]) -> f32 {
    BAR_TOP + BAR_HEIGHT + below_bar(search, history)
}

fn below_bar(search: &Search, history: &[SearchHistoryItem]) -> f32 {
    if let Some(suggestions) = search
        .suggestions
        .as_ref()
        .filter(|suggestions| !suggestions.is_empty())
    {
        return LIST_TOP
            + ROW * suggestions.len() as f32
            + DIVIDER[0]
            + 1.0
            + DIVIDER[1]
            + LIST_BOTTOM;
    }
    if search.history_open() {
        let people = history
            .iter()
            .filter(|item| item.kind == exo_store::SearchHistoryKind::Contact)
            .count();
        let divider = if people > 0 && people < history.len() {
            HISTORY_DIVIDER[0] + 1.0 + HISTORY_DIVIDER[1]
        } else {
            0.0
        };
        return LIST_TOP + ROW * history.len() as f32 + divider;
    }
    EMPTY
}

/// What happened in the bar this frame.
#[derive(Default)]
pub(crate) struct BarResponse {
    /// What the bar asks of the app.
    pub effects: Vec<Effect>,
    /// The close button was clicked.
    pub close: bool,
    /// The bar was clicked while the results had the keyboard.
    pub focus: bool,
}

/// Where the text starts: `calc(3.5rem - 2px)` in (44px - 2px at most 1152px wide) and the
/// container's `padding-left: 2px`.
fn text_left(view: Rect, rem: f32, compact: bool) -> f32 {
    view.left() + if compact { 44.0 } else { 3.5 * rem }
}

/// Paints the search header over the top of `view` (the list view's padding box) and handles
/// its input.
pub(crate) fn show_bar(
    ui: &mut Ui,
    view: Rect,
    rem: f32,
    compact: bool,
    search: &mut Search,
    history: &[SearchHistoryItem],
) -> BarResponse {
    let palette = exo_ui::palette(ui.ctx());
    let height = header_height(search, history);
    let header = Rect::from_min_size(view.min, vec2(view.width(), height));
    ui.painter()
        .rect_filled(header, 0.0, palette.background_base_default);
    let mut response = BarResponse::default();
    let id = ui.id().with("search-bar");

    let bar_top = view.top() + BAR_TOP;
    if search.focused {
        let bar = Rect::from_min_size(pos2(view.left(), bar_top), vec2(FOCUS_BAR, BAR_HEIGHT));
        ui.painter()
            .rect_filled(bar, CornerRadius::same(1), palette.border_base_active);
    }
    // The bar leaves `calc(48px + 2.875rem)` on its right.
    let left = text_left(view, rem, compact);
    let right = view.right() - 48.0 - 2.875 * rem;
    let line = Rect::from_min_max(
        pos2(left, bar_top + INPUT_TOP),
        pos2(right, bar_top + INPUT_TOP + INPUT.line_height),
    );
    let clip = Rect::from_min_max(
        pos2(left - RING - 2.0, bar_top),
        pos2(right, bar_top + BAR_HEIGHT),
    );
    paint_underlay(&ui.painter_at(clip), line, search);

    let mut text = search.value.clone();
    let replaced = text != search.field_text;
    paste_search(ui.ctx(), id.with("field"), &text);
    // The field's text: `rgba(0, 0, 0, .7)`, `#fff` in Carbon.
    let field = exo_ui::text_field(
        ui,
        id.with("field"),
        &mut text,
        line,
        &INPUT,
        palette.ink(0.7, 1.0),
        palette.text_selection_default,
    );
    if replaced {
        let end = CCursor::new(text.chars().count());
        exo_ui::focus_field(ui.ctx(), field.id, CCursorRange::two(end, end));
        if !search.focused {
            field.surrender_focus();
        }
    }
    if field.clicked() || field.gained_focus() {
        response.focus = !search.focused;
    }
    if search.focused && !field.has_focus() {
        field.request_focus();
    } else if !search.focused && field.has_focus() && !response.focus {
        field.surrender_focus();
    }
    if std::mem::take(&mut search.select_all) {
        exo_ui::focus_field(
            ui.ctx(),
            field.id,
            CCursorRange::two(CCursor::new(0), CCursor::new(text.chars().count())),
        );
    }
    // Where the caret is after this frame's typing: suggestions for it show only with the caret at
    // the end (checked once the field has taken the keystroke).
    let chars = text.chars().count();
    search.caret_at_end = egui::text_edit::TextEditState::load(ui.ctx(), field.id)
        .and_then(|state| state.cursor.char_range())
        .is_none_or(|range| range.primary.index.0 == chars && range.secondary.index.0 == chars);
    if text != search.value {
        response.effects.extend(search.edit(&text));
    }
    search.field_text = search.value.clone();

    // The close button where the search button was:
    // `top: 35px; right: calc(2rem - 5px); padding: 10px; margin: -10px`, a 14px icon.
    let icon = pos2(view.right() - 2.0 * rem + 5.0 - 14.0, view.top() + 35.0);
    let glyph = Rect::from_min_size(icon, Vec2::splat(14.0));
    let close =
        IconButton::new("legacy/close").show(ui, id.with("close"), glyph.expand(10.0), glyph);
    KeyTooltip {
        text: "Close",
        keys: &["esc"],
    }
    .show_below(ui, &close, 8.0);
    if close.clicked() {
        response.close = true;
    }

    let below = Rect::from_min_max(
        pos2(view.left(), bar_top + BAR_HEIGHT),
        pos2(view.right(), header.bottom()),
    );
    // A row's `padding-right: 2rem`.
    let columns = Columns {
        left,
        right: view.right() - 2.0 * rem,
        rem,
    };
    if search.list_open() {
        response
            .effects
            .extend(suggestion_list(ui, id, below, &columns, search));
    } else if search.history_open() {
        response
            .effects
            .extend(history_list(ui, id, below, &columns, search, history));
    }
    response
}

/// Pasting a subject copied from a conversation types the search that finds it, unless the text
/// before the selection already ends in `subject:`. The caret ends after the paste.
fn paste_search(ctx: &egui::Context, field: Id, text: &str) {
    static AFTER_SUBJECT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)subject:\s*"?$"#).expect("valid regex"));
    let pasting = ctx.input(|input| {
        input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::Paste(_)))
    });
    if !pasting || !ctx.memory(|memory| memory.has_focus(field)) {
        return;
    }
    let Some(search) = crate::clipboard::read_search(ctx) else {
        return;
    };
    let start = egui::text_edit::TextEditState::load(ctx, field)
        .and_then(|state| state.cursor.char_range())
        .map_or(usize::MAX, |range| {
            range.primary.index.0.min(range.secondary.index.0)
        });
    let before: String = text.chars().take(start).collect();
    if AFTER_SUBJECT.is_match(&before) {
        return;
    }
    ctx.input_mut(|input| {
        for event in &mut input.events {
            if let egui::Event::Paste(pasted) = event {
                pasted.clone_from(&search);
            }
        }
    });
}

/// A row's text box: from the text's left to the row's `padding-right: 2rem`.
struct Columns {
    left: f32,
    right: f32,
    rem: f32,
}

impl Columns {
    /// Where the second column of a person's row starts.
    fn second(&self) -> f32 {
        self.left + NAME_COLUMN * (self.right - self.left)
    }

    /// The name column's text width, inside its `padding-right: 1rem`.
    fn name_width(&self) -> f32 {
        self.second() - self.left - self.rem
    }
}

/// The underlay: what was typed and what a suggestion would add, with the operators already typed
/// in lozenges and the word being typed in the current one.
fn paint_underlay(painter: &Painter, line: Rect, search: &Search) {
    static LOZENGED: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(&format!(r"{OPERATORS}\s*\S+")).expect("valid regex"));
    static OPERATOR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(OPERATORS).expect("valid regex"));
    let palette = exo_ui::palette(painter.ctx());
    let ctx = painter.ctx();
    let opacity = underlay_opacity(ctx);
    let (head, current) = match search
        .underlay
        .as_deref()
        .filter(|underlay| !underlay.is_empty())
    {
        Some(underlay) => {
            let at = search.lozenge_start.unwrap_or_default().min(underlay.len());
            let at = (0..=at)
                .rev()
                .find(|at| underlay.is_char_boundary(*at))
                .unwrap_or_default();
            (&underlay[..at], &underlay[at..])
        }
        None => ("Search", ""),
    };
    let underlay = format!("{head}{current}");
    // The underlay is set as the field is: no ligatures, so it lines up with the field's text.
    let style = INPUT.without_ligatures();
    let x = |offset: usize| line.left() + style.width(ctx, &underlay[..offset]);
    let lozenge = |from: usize, to: usize, current: bool| {
        let rect = Rect::from_min_max(
            pos2(x(from), line.top() + CONTENT_INSET),
            pos2(x(to), line.bottom() - CONTENT_INSET),
        );
        let (fill, ring) = lozenge_colors(ctx, current);
        // A box shadow paints only outside the box.
        painter.rect_filled(rect, CornerRadius::same(1), fill.gamma_multiply(opacity));
        painter.rect_stroke(
            rect,
            CornerRadius::same(1),
            egui::Stroke::new(RING, ring.gamma_multiply(opacity)),
            egui::StrokeKind::Outside,
        );
    };
    // Each operator and its value in a lozenge; a quoted value runs to its closing quote.
    let mut offset = 0;
    while offset < head.len() {
        let Some(found) = LOZENGED.find(&head[offset..]) else {
            break;
        };
        let start = offset + found.start();
        let mut end = offset + found.end();
        if found.as_str().contains('"') {
            let rest = &head[end..];
            match rest.find('"') {
                Some(close) => end += close + 1,
                None => end = head.len(),
            }
        }
        lozenge(start, end.min(head.len()), false);
        offset = end;
    }
    let needs_lozenge = OPERATOR.is_match(current)
        || (current.contains(char::is_whitespace)
            && !current.trim().is_empty()
            && !current.contains('"'));
    if needs_lozenge && !current.is_empty() {
        lozenge(head.len(), underlay.len(), true);
    }
    // The underlay: `rgba(0, 0, 0, .7)`, `hsla(0, 0%, 100%, .7)` in Carbon.
    style.paint_line(
        painter,
        line.min,
        &underlay,
        palette.ink(0.7, 0.7).gamma_multiply(opacity),
        None,
    );
}

/// The suggestion list: the suggestions, then a divider.
fn suggestion_list(
    ui: &mut Ui,
    id: Id,
    below: Rect,
    columns: &Columns,
    search: &mut Search,
) -> Vec<Effect> {
    let palette = exo_ui::palette(ui.ctx());
    let suggestions = search.suggestions.clone().unwrap_or_default();
    let moved = ui.input(|input| input.pointer.is_moving());
    let mut effects = Vec::new();
    let mut top = below.top() + LIST_TOP;
    let mut hovered = None;
    for (at, suggestion) in suggestions.iter().enumerate() {
        let row = Rect::from_min_size(pos2(below.left(), top), vec2(below.width(), ROW));
        let response = ui.interact(row, id.with(("suggestion", at)), Sense::click());
        if response.hovered() {
            hovered = Some(at);
        }
        if search.selected == Some(at) {
            // The picked row: `hsl(220, 45%, 97%)`, `#464b53` in Carbon.
            ui.painter()
                .rect_filled(row, 0.0, palette.fill_hover_default);
        }
        let text_top = top + ROW_TEXT;
        match &suggestion.kind {
            Kind::Contact { name, email, .. } => {
                let shown = name.as_deref().unwrap_or(email);
                paint_highlighted(
                    ui.painter(),
                    pos2(columns.left, text_top),
                    shown,
                    &suggestion.original,
                    Some(columns.name_width()),
                );
                paint_highlighted(
                    ui.painter(),
                    pos2(columns.second(), text_top),
                    email,
                    &suggestion.original,
                    Some(columns.right - columns.second()),
                );
            }
            _ => paint_highlighted(
                ui.painter(),
                pos2(columns.left, text_top),
                &suggestion.replacement,
                &suggestion.original,
                Some(columns.right - columns.left),
            ),
        }
        if response.is_pointer_button_down_on() && ui.input(|input| input.pointer.any_pressed()) {
            effects.extend(search.click_suggestion(at));
            return effects;
        }
        top += ROW;
    }
    let divider = top + DIVIDER[0];
    ui.painter().hline(
        below.x_range(),
        divider + 0.5,
        egui::Stroke::new(1.0, divider_color(ui.ctx())),
    );
    // Only a mouse that moved picks a suggestion, and leaving the list unpicks it.
    if moved {
        match hovered {
            Some(at) if search.selected != Some(at) => search.hover_suggestion(Some(at)),
            None if search.selected_by_mouse && ui.rect_contains_pointer(below) => {}
            None if search.selected_by_mouse => search.hover_suggestion(None),
            _ => {}
        }
    }
    effects
}

/// The history: recent people, a divider, recent searches.
fn history_list(
    ui: &mut Ui,
    id: Id,
    below: Rect,
    columns: &Columns,
    search: &mut Search,
    history: &[SearchHistoryItem],
) -> Vec<Effect> {
    let palette = exo_ui::palette(ui.ctx());
    let rows = history_rows(history);
    let moved = ui.input(|input| input.pointer.is_moving());
    let mut top = below.top() + LIST_TOP;
    let mut previous = None;
    for (at, item) in rows.iter().enumerate() {
        if previous == Some(exo_store::SearchHistoryKind::Contact)
            && item.kind == exo_store::SearchHistoryKind::Query
        {
            ui.painter().hline(
                below.x_range(),
                top + HISTORY_DIVIDER[0] + 0.5,
                egui::Stroke::new(1.0, divider_color(ui.ctx())),
            );
            top += HISTORY_DIVIDER[0] + 1.0 + HISTORY_DIVIDER[1];
        }
        previous = Some(item.kind);
        let row = Rect::from_min_size(pos2(below.left(), top), vec2(below.width(), ROW));
        let response = ui.interact(row, id.with(("history", at)), Sense::click());
        if moved && response.hovered() {
            search.hover_history(Some(at));
        }
        if search.history_selected == Some(at) {
            // The picked row: `hsl(220, 45%, 97%)`, `#464b53` in Carbon.
            ui.painter()
                .rect_filled(row, 0.0, palette.fill_hover_default);
        }
        let text_top = top + ROW_TEXT;
        // A history row's text: .85 in both; a person's address: .4.
        let item_color = palette.ink(0.85, 0.85);
        if item.kind == exo_store::SearchHistoryKind::Contact {
            // The name column shows the address when there's no contact by it (Exo keeps no
            // contacts with names yet).
            ITEM.paint_line(
                ui.painter(),
                pos2(columns.left, text_top),
                &item.value,
                item_color,
                Some(columns.name_width()),
            );
            ITEM.paint_line(
                ui.painter(),
                pos2(columns.second(), text_top),
                &item.value,
                palette.ink(0.4, 0.4),
                Some(columns.right - columns.second()),
            );
        } else {
            ITEM.paint_line(
                ui.painter(),
                pos2(columns.left, text_top),
                &item.value,
                item_color,
                Some(columns.right - columns.left),
            );
        }
        if response.is_pointer_button_down_on() && ui.input(|input| input.pointer.any_pressed()) {
            return search.click_history(at, history);
        }
        top += ROW;
    }
    Vec::new()
}

/// A suggestion's text with what was typed darker (.75 in both) than the rest (.4).
fn paint_highlighted(
    painter: &Painter,
    at: Pos2,
    text: &str,
    original: &str,
    max_width: Option<f32>,
) {
    let palette = exo_ui::palette(painter.ctx());
    let mut job = LayoutJob::default();
    for (part, matched) in autocomplete::highlight(text, original) {
        job.append(
            part,
            0.0,
            ITEM.format(if matched {
                palette.ink(0.75, 0.75)
            } else {
                palette.ink(0.4, 0.4)
            }),
        );
    }
    if let Some(width) = max_width {
        job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    }
    paint_job(painter, at, job, &ITEM);
}

/// What an empty search with no history shows in the list's place.
pub(crate) fn show_empty(painter: &Painter, list: Rect, rem: f32, compact: bool) {
    let palette = exo_ui::palette(painter.ctx());
    let left = list.left() + if compact { 44.0 } else { 3.5 * rem };
    let mut job = LayoutJob::default();
    for (text, bold) in [
        ("Search even faster with ", false),
        ("in:sent", true),
        (", ", false),
        ("is:unread", true),
        (", and ", false),
        ("has:attachment", true),
        (".", false),
    ] {
        // The hint: .7 in both.
        job.append(
            text,
            0.0,
            if bold { HINT_BOLD } else { HINT }.format(palette.ink(0.7, 0.7)),
        );
    }
    paint_job(painter, pos2(left, list.top() + 8.0 + 3.0), job, &HINT);
}
