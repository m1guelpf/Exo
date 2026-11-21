//! The colour picker: rows of 26px discs, each centred in a 32px circle 3px apart, rows 2px apart,
//! inside a tooltip box padded `6px 7px` (measured; a tooltip's `3px` at the bottom in a calendar's
//! picker); the chosen colour on a white circle (Carbon `rgba(0,0,0,.3)`), a hovered one ringed.
//! The Accounts dialog, a calendar's `…` and the Notifications dialog's swatches open one;
//! [`picker_key`] is its keyboard.

use egui::{Color32, Context, Id, Pos2, Rect, Sense, Vec2, pos2, vec2};

use crate::Theme;

const PADDING: Vec2 = Vec2::new(7.0, 6.0);
const DISC: f32 = 32.0;
const GAP: Vec2 = Vec2::new(3.0, 2.0);

/// What a colour picker did this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picked {
    /// Colour `index` was clicked.
    Color(usize),
    /// A click outside it and `keep`.
    Outside,
    Nothing,
}

/// A key the picker takes while it has the keyboard (`left`/`right`, `tab`/`shift+tab` and
/// `mod+o`/`mod+shift+o` step, `up`/`down` move a row, `mod+` an arrow goes to the first or last
/// colour).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerKey {
    Previous,
    Next,
    Up,
    Down,
    First,
    Last,
    Escape,
    /// Enter or space.
    Enter,
}

/// What a key does to an open picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerMove {
    /// Colour `index` becomes the chosen one (it's applied at once, as a preview).
    Preview(usize),
    /// Colour `index` is applied and the picker closes: Enter on a new colour, or Escape (and
    /// Enter on the colour it opened with) putting back the one it opened with.
    Close(usize),
}

/// The picker's answer to `key` on `len` colours `per_row` to a row, on colour `current`, opened
/// on colour `start`: every move wraps around, rows into each other; ↑ and ↓ are the picker's only
/// with more than one row.
pub fn picker_key(
    key: PickerKey,
    current: usize,
    start: usize,
    len: usize,
    per_row: usize,
) -> PickerMove {
    let step = |by: usize| (current + by) % len.max(1);
    match key {
        PickerKey::Up | PickerKey::Down if len <= per_row => PickerMove::Preview(current),
        PickerKey::Previous => PickerMove::Preview(step(len.saturating_sub(1))),
        PickerKey::Next => PickerMove::Preview(step(1)),
        PickerKey::Up => PickerMove::Preview(step(len - per_row % len)),
        PickerKey::Down => PickerMove::Preview(step(per_row)),
        PickerKey::First => PickerMove::Preview(0),
        PickerKey::Last => PickerMove::Preview(len.saturating_sub(1)),
        PickerKey::Enter if current != start => PickerMove::Close(current),
        PickerKey::Escape | PickerKey::Enter => PickerMove::Close(start),
    }
}

/// A colour picker of `colors`, `per_row` to a row, `chosen` on its white circle, the box
/// padded `padding_bottom` under the last row.
pub struct ColorPicker<'a> {
    pub colors: &'a [Color32],
    pub per_row: usize,
    pub chosen: Option<usize>,
    pub padding_bottom: f32,
}

impl ColorPicker<'_> {
    /// The tooltip box's size.
    pub fn size(&self) -> Vec2 {
        let rows = self.colors.len().div_ceil(self.per_row);
        let columns = self.per_row.min(self.colors.len());
        vec2(
            columns as f32 * DISC + columns.saturating_sub(1) as f32 * GAP.x,
            rows as f32 * DISC + rows.saturating_sub(1) as f32 * GAP.y,
        ) + vec2(2.0 * PADDING.x, PADDING.y + self.padding_bottom)
    }

    /// Paints it as a tooltip with its top left at `at` (on a whole pixel); a click outside it and
    /// `keep` (the control that opened it) is [`Picked::Outside`].
    pub fn show(&self, ctx: &Context, id: Id, at: Pos2, keep: Rect) -> Picked {
        let rect = Rect::from_min_size(at.round(), self.size());
        let mut picked = Picked::Nothing;
        egui::Area::new(id)
            .order(egui::Order::Tooltip)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                let painter = ui.painter();
                crate::paint_tooltip_box(painter, rect);
                for (index, color) in self.colors.iter().enumerate() {
                    let (row, column) = (index / self.per_row, index % self.per_row);
                    let outer = Rect::from_min_size(
                        rect.min
                            + PADDING
                            + vec2(column as f32 * (DISC + GAP.x), row as f32 * (DISC + GAP.y)),
                        Vec2::splat(DISC),
                    );
                    let response = ui
                        .interact(outer, id.with(index), Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    paint_disc(
                        painter,
                        outer,
                        *color,
                        self.chosen == Some(index),
                        response.hovered(),
                    );
                    if response.clicked() {
                        picked = Picked::Color(index);
                    }
                }
                let outside = ui.input(|input| {
                    input.pointer.any_click()
                        && input
                            .pointer
                            .interact_pos()
                            .is_some_and(|at| !rect.contains(at) && !keep.contains(at))
                });
                if outside && picked == Picked::Nothing {
                    picked = Picked::Outside;
                }
            });
        picked
    }
}

/// One colour: a 26px disc 3px into its 32px circle, on a white circle when chosen
/// (`rgba(0, 0, 0, .3)` in Carbon), ringed `0 0 0 .6px` when hovered.
fn paint_disc(painter: &egui::Painter, outer: Rect, color: Color32, chosen: bool, hovered: bool) {
    let carbon = crate::theme(painter.ctx()) == Theme::Carbon;
    if chosen {
        painter.circle_filled(
            outer.center(),
            DISC / 2.0,
            if carbon {
                Color32::from_black_alpha(77)
            } else {
                Color32::WHITE
            },
        );
    } else if hovered {
        let ring = if carbon {
            Color32::from_rgb(0x81, 0x81, 0x81)
        } else {
            Color32::from_rgb(0x90, 0x93, 0x97)
        };
        painter.circle_stroke(outer.center(), DISC / 2.0, egui::Stroke::new(0.6, ring));
    }
    // `top: 9.375%` of the 32px circle.
    painter.circle_filled(
        pos2(outer.center().x, outer.top() + DISC * 0.093_75 + 13.0),
        13.0,
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_wrap_around_the_colours_and_enter_or_escape_close() {
        // 25 colours in rows of 5, on the third, opened on the third.
        assert_eq!(
            picker_key(PickerKey::Previous, 0, 2, 25, 5),
            PickerMove::Preview(24)
        );
        assert_eq!(
            picker_key(PickerKey::Next, 24, 2, 25, 5),
            PickerMove::Preview(0)
        );
        assert_eq!(
            picker_key(PickerKey::Up, 2, 2, 25, 5),
            PickerMove::Preview(22)
        );
        assert_eq!(
            picker_key(PickerKey::Down, 22, 2, 25, 5),
            PickerMove::Preview(2)
        );
        assert_eq!(
            picker_key(PickerKey::Last, 2, 2, 25, 5),
            PickerMove::Preview(24)
        );
        assert_eq!(
            picker_key(PickerKey::First, 9, 2, 25, 5),
            PickerMove::Preview(0)
        );
        assert_eq!(
            picker_key(PickerKey::Enter, 9, 2, 25, 5),
            PickerMove::Close(9)
        );
        assert_eq!(
            picker_key(PickerKey::Enter, 2, 2, 25, 5),
            PickerMove::Close(2)
        );
        assert_eq!(
            picker_key(PickerKey::Escape, 9, 2, 25, 5),
            PickerMove::Close(2)
        );
        // The Accounts dialog's two rows of six: a row's end goes on to the next.
        assert_eq!(
            picker_key(PickerKey::Next, 5, 0, 12, 6),
            PickerMove::Preview(6)
        );
        assert_eq!(
            picker_key(PickerKey::Up, 2, 0, 12, 6),
            PickerMove::Preview(8),
            "up from the top row wraps to the bottom"
        );
        // One row: ↑ and ↓ aren't the picker's.
        assert_eq!(
            (
                picker_key(PickerKey::Up, 1, 0, 4, 4),
                picker_key(PickerKey::Down, 1, 0, 4, 4)
            ),
            (PickerMove::Preview(1), PickerMove::Preview(1))
        );
    }
}
