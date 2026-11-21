//! A bare one-line text field: an egui `TextEdit` whose text sits where CSS puts an input's (the
//! palette's and the shortcuts sheet's search fields, dialog inputs).
//!
//! A right-click in one opens the shell's menu for editable text: as on a Mac, it selects the word
//! under it (or keeps the selection it's on), and the field reports it ([`take_field_menu`]) for
//! the app to show the menu over.

use std::ops::Range;

use egui::{
    Color32, Context, Frame, Id, Margin, Pos2, Rect, Shape, Stroke, TextBuffer, TextEdit, Ui, pos2,
    text::{CCursor, CCursorRange},
    text_edit::TextEditState,
    vec2,
};

use crate::TextStyle;

/// Edits `text` in a frameless field whose CSS line box (in `style`) is `line`, in `color` with
/// `selection` behind selected text. Like every input, it draws without ligatures
/// (`font-variant-ligatures: none`).
pub fn text_field(
    ui: &mut Ui,
    id: Id,
    text: &mut String,
    line: Rect,
    style: &TextStyle,
    color: Color32,
    selection: Color32,
) -> egui::Response {
    let style = &style.without_ligatures();
    // TextEdit puts a row's baseline where egui does; shift it to where CSS puts it.
    let probe = style.single_line(ui.painter(), "x", color, None);
    let egui_baseline = probe
        .rows
        .first()
        .and_then(|row| row.row.glyphs.first())
        .map_or(0.0, |glyph| glyph.pos.y);
    // TextEdit paints its text from a whole pixel; start the field there and the text at the
    // line's sub-pixel phase, as `paint_job` does, so the text lands where Chromium puts it.
    let pixels_per_point = ui.ctx().pixels_per_point();
    let phase = (line.left() * pixels_per_point).fract() / pixels_per_point;
    let rect = line.translate(vec2(-phase, style.baseline(ui.ctx()) - egui_baseline));
    let mut layouter = |ui: &Ui, text: &dyn TextBuffer, _wrap: f32| {
        let mut job = style.line_job(text.as_str(), color, None);
        if let Some(first) = job.sections.first_mut() {
            first.leading_space += phase;
        }
        crate::text::layout(ui.ctx(), job)
    };
    // The selection is painted here, over the CSS line box (egui's would follow its own row, off
    // by the baseline shift), behind the text: a slot now, filled once the edit has run.
    let behind = ui.painter().add(Shape::Noop);
    // A right-click's press keeps the selection egui's caret move would lose.
    let ctx = ui.ctx().clone();
    let pressed_id = id.with("exo_ui::field_right_press");
    if ui.input(|input| input.pointer.secondary_pressed()) && ui.rect_contains_pointer(line) {
        let kept = TextEditState::load(&ctx, id)
            .and_then(|state| state.cursor.char_range())
            .filter(|_| ctx.memory(|memory| memory.has_focus(id)));
        ctx.data_mut(|data| data.insert_temp(pressed_id, kept));
    }
    // Chromium draws no caret while text is selected.
    let selecting = TextEditState::load(ui.ctx(), id)
        .and_then(|state| state.cursor.char_range())
        .is_some_and(|range| range.primary != range.secondary);
    let response = ui
        .scope(|ui| {
            let visuals = ui.visuals_mut();
            visuals.selection.bg_fill = Color32::TRANSPARENT;
            visuals.selection.stroke = Stroke::new(0.0, color);
            visuals.text_cursor.stroke = Stroke::new(
                1.0,
                if selecting {
                    Color32::TRANSPARENT
                } else {
                    color
                },
            );
            let edit = TextEdit::singleline(text)
                .id(id)
                .frame(Frame::NONE)
                .margin(Margin::ZERO)
                .desired_width(line.width())
                .layouter(&mut layouter);
            ui.put(rect, edit)
        })
        .inner;
    let pressed = ctx.data(|data| data.get_temp::<Option<CCursorRange>>(pressed_id));
    if let Some(pressed) = pressed {
        let (down, released) = ui.input(|input| {
            (
                input.pointer.secondary_down(),
                input.pointer.secondary_released(),
            )
        });
        if let Some(kept) = pressed.filter(|_| down || released) {
            set_selection(&ctx, id, kept);
        }
        if !down {
            ctx.data_mut(|data| data.remove::<Option<CCursorRange>>(pressed_id));
        }
        if response.secondary_clicked()
            && let Some(point) = response.interact_pointer_pos()
        {
            right_click(&ctx, id, text, line, style, point, pressed);
        }
    }
    let range = TextEditState::load(ui.ctx(), id)
        .and_then(|state| state.cursor.char_range())
        .filter(|range| range.primary != range.secondary && response.has_focus());
    if let Some(range) = range {
        let galley = field_galley(ui.ctx(), text, style);
        let x = |index: usize| {
            line.left()
                + galley
                    .pos_from_cursor(egui::text::CCursor::new(index))
                    .min
                    .x
        };
        let (from, to) = (
            range.primary.index.0.min(range.secondary.index.0),
            range.primary.index.0.max(range.secondary.index.0),
        );
        ui.painter().set(
            behind,
            Shape::rect_filled(
                Rect::from_x_y_ranges(x(from)..=x(to), line.y_range()),
                0.0,
                selection,
            ),
        );
    }
    response
}

/// Where character `index` of `text` starts on screen in a field [`text_field`] lays out on
/// `line` in `style`.
pub fn field_x(ctx: &Context, text: &str, line: Rect, style: &TextStyle, index: usize) -> f32 {
    line.left()
        + field_galley(ctx, text, style)
            .pos_from_cursor(egui::text::CCursor::new(index))
            .min
            .x
}

/// The caret position nearest `x` on screen in a field [`text_field`] lays `text` out in.
pub fn field_index(ctx: &Context, text: &str, line: Rect, style: &TextStyle, x: f32) -> usize {
    field_galley(ctx, text, style)
        .cursor_from_pos(vec2(x - line.left(), 0.0))
        .index
        .0
}

fn field_galley(ctx: &Context, text: &str, style: &TextStyle) -> std::sync::Arc<egui::Galley> {
    crate::text::layout(
        ctx,
        style
            .without_ligatures()
            .line_job(text, Color32::PLACEHOLDER, None),
    )
}

/// A right-click in a field, for the shell's editable menu: where, and what it selected.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldMenu {
    /// The field.
    pub field: Id,
    /// Where, on screen.
    pub at: Pos2,
    /// The selection the click made or kept, as character offsets.
    pub selection: Range<usize>,
    /// The selected text.
    pub selected: String,
    /// Where its text starts on screen, on its baseline (Look Up's popover lines up with it).
    pub baseline: Pos2,
    /// The field's text size, for Look Up.
    pub text_size: f32,
}

/// The right-click a field reported since this was last asked (for `field`'s alone, given one).
pub fn take_field_menu(ctx: &Context, field: Option<Id>) -> Option<FieldMenu> {
    let key = crate::id(ctx, "exo_ui::field_menu");
    ctx.data_mut(|data| {
        let menu = data
            .get_temp::<FieldMenu>(key)
            .filter(|menu| field.is_none_or(|field| field == menu.field))?;
        data.remove::<FieldMenu>(key);
        Some(menu)
    })
}

/// Whether `ch` is part of a word.
fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '\'' | '’')
}

/// A right-click at `point` in field `id` (`SelectClosestWordOrLinkFromMouseEvent`): on the
/// selection its press found (`pressed`) it keeps it; else it selects the word under it, or puts
/// the caret there off a word. Then the field has the keyboard and the menu is the app's.
fn right_click(
    ctx: &Context,
    id: Id,
    text: &str,
    line: Rect,
    style: &TextStyle,
    point: Pos2,
    pressed: Option<CCursorRange>,
) {
    let chars: Vec<char> = text.chars().collect();
    let at = field_index(ctx, text, line, style, point.x);
    // The character under the pointer: before the caret position when left of it.
    let under = if at > 0 && point.x < field_x(ctx, text, line, style, at) {
        at - 1
    } else {
        at
    };
    let selected = pressed.map(|range| {
        let (a, b) = (range.primary.index.0, range.secondary.index.0);
        a.min(b)..a.max(b)
    });
    let range = match selected {
        Some(range) if range.start <= under && under < range.end => range,
        _ if chars.get(under).copied().is_some_and(is_word_char) => {
            let start = (0..under)
                .rev()
                .take_while(|at| is_word_char(chars[*at]))
                .last()
                .unwrap_or(under);
            let end = (under..chars.len())
                .take_while(|at| is_word_char(chars[*at]))
                .last()
                .map_or(under, |last| last + 1);
            start..end
        }
        _ => at..at,
    };
    set_selection(
        ctx,
        id,
        CCursorRange::two(CCursor::new(range.start), CCursor::new(range.end)),
    );
    ctx.memory_mut(|memory| memory.request_focus(id));
    let menu = FieldMenu {
        field: id,
        at: point,
        selected: chars[range.clone()].iter().collect(),
        baseline: pos2(
            field_x(ctx, text, line, style, range.start),
            line.top() + style.baseline(ctx),
        ),
        selection: range,
        text_size: style.size,
    };
    let key = crate::id(ctx, "exo_ui::field_menu");
    ctx.data_mut(|data| data.insert_temp(key, menu));
    ctx.request_repaint();
}

fn set_selection(ctx: &Context, id: Id, range: CCursorRange) {
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(range));
    state.store(ctx, id);
}

/// Focuses the field `id` with `selection` of its text selected.
pub fn focus_field(ctx: &Context, id: Id, selection: CCursorRange) {
    ctx.memory_mut(|memory| memory.request_focus(id));
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(selection));
    state.store(ctx, id);
}

#[cfg(test)]
mod tests {
    use egui::{Shape, epaint::TextShape, pos2};

    use super::*;
    use crate::{Family, Font};

    /// The pixel the tessellator draws a text shape's first glyph from: it rounds the shape's
    /// position to a whole pixel (`round_text_to_pixels`); where the glyph sits inside that pixel
    /// is in its raster, the same for the same layout.
    fn first_glyph_x(text: &TextShape, pixels_per_point: f32) -> f32 {
        let row = &text.galley.rows[0];
        (text.pos.x * pixels_per_point).round() / pixels_per_point + row.pos.x + row.glyphs[0].pos.x
    }

    #[test]
    fn a_fields_text_lands_where_a_painted_lines_does() {
        let ctx = Context::default();
        crate::install(&ctx).expect("the system fonts load");
        let style = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 24.0).without_ligatures();
        ctx.set_pixels_per_point(2.0);
        // 3.5rem at 1280px wide: a quarter pixel past a whole one at 2×.
        let line = |top: f32| Rect::from_min_size(pos2(47.25, top), vec2(300.0, 24.0));
        let mut text = "has:attachment".to_owned();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            text_field(
                ui,
                Id::new("field"),
                &mut text,
                line(10.0),
                &style,
                Color32::BLACK,
                Color32::BLUE,
            );
            style.paint_line(
                ui.painter(),
                line(50.0).min,
                "has:attachment",
                Color32::BLACK,
                None,
            );
        });
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        let texts: Vec<&TextShape> = shapes
            .iter()
            .filter_map(|clipped| {
                if let Shape::Text(text) = &clipped.shape {
                    Some(text)
                } else {
                    None
                }
            })
            .collect();
        let [field, painted] = texts[..] else {
            panic!("two lines of text, got {}", texts.len())
        };
        assert_eq!(first_glyph_x(field, 2.0), first_glyph_x(painted, 2.0));
    }
}
