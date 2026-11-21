//! Autocorrect's marks and its suggestion pill on screen.

use egui::{Area, Color32, CornerRadius, Order, Pos2, Rect, Sense, Ui, pos2, text::CCursor, vec2};
use exo_ui::{Family, Font, TextStyle, Theme};

use super::{BlockLayout, BodyText};
use crate::{autocorrect::SuggestionKind, editor::Editor, model::Pos};

/// The pill: 28px tall with 4px of padding.
const HEIGHT: f32 = 28.0;
const PADDING: f32 = 4.0;
/// An item's name: 12px/15px semibold, `padding: 1px 4px 0 4px`, 2px corners; `Learn`'s
/// `padding-right: 3px`.
const NAME: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 12.0, 15.0);
const NAME_PADDING: f32 = 4.0;
const LEARN_PADDING_RIGHT: f32 = 3.0;
/// The divider between items: 1×13px, 2px either side.
const DIVIDER: (f32, f32) = (1.0, 13.0);
const DIVIDER_MARGIN: f32 = 2.0;
/// Classic's undo icon: 10×10, `margin: -2px 0 0 6px`.
const UNDO: f32 = 10.0;
const UNDO_GAP: f32 = 6.0;
/// `Icons.DEPRECATED_undoArrow`, the `legacy` partner of the pill's undo icon.
const UNDO_ICON: &str = "legacy/DEPRECATED_undoArrow";
/// The tooltip's distance from the word (3px below it, 4px above), and from the container's
/// sides.
const BELOW: f32 = 3.0;
const ABOVE: f32 = 4.0;
const SIDE: f32 = 22.5;

/// The pill's colours in Classic: Snow's base rules and `.Theme-carbon`'s.
struct Colors {
    fill: Color32,
    text: Color32,
    learn: Color32,
    selected_text: Color32,
    selected_fill: Color32,
    divider: Color32,
    icon: Color32,
    selected_icon: Color32,
}

impl Colors {
    fn of(theme: Theme) -> Self {
        let carbon_text = |alpha: u8| Color32::from_rgba_unmultiplied(235, 238, 242, alpha);
        match theme {
            Theme::Snow => Self {
                fill: Color32::WHITE,
                text: Color32::from_rgb(0x5b, 0x5b, 0x5d),
                learn: Color32::from_black_alpha(89),
                selected_text: Color32::from_rgb(0x23, 0x23, 0x25),
                selected_fill: Color32::from_rgba_unmultiplied(176, 172, 223, 71),
                divider: Color32::from_black_alpha(38),
                icon: Color32::from_rgb(0x89, 0x89, 0x8c),
                selected_icon: Color32::from_rgb(0x5f, 0x5f, 0x61),
            },
            Theme::Carbon => Self {
                fill: Color32::from_rgb(0x27, 0x29, 0x2d),
                text: carbon_text(191),
                learn: carbon_text(89),
                selected_text: Color32::from_rgb(0xeb, 0xee, 0xf2),
                selected_fill: Color32::from_rgba_unmultiplied(176, 172, 223, 64),
                divider: Color32::from_white_alpha(13),
                icon: carbon_text(191),
                selected_icon: Color32::from_rgb(0xeb, 0xee, 0xf2),
            },
        }
    }
}

/// A marked word's dots: a 2px strip 1px above the word's box's bottom,
/// `radial-gradient(circle at 1px 1px, {colour} 1px, transparent 0)` in 4×2px tiles from 1px in
/// (`background-position-x: 1px`), as many whole ones as round to the word's width
/// (`background-repeat: round`); `#b2b7c3` for a correction, `#f67b7b` for a misspelling, in both
/// themes.
pub(super) fn paint_annotation(
    painter: &egui::Painter,
    view: &BlockLayout,
    chars: std::ops::Range<usize>,
    galley_origin: Pos2,
    body: &BodyText,
    corrected: bool,
) {
    const TILE: f32 = 4.0;
    let color = if corrected {
        Color32::from_rgb(0xb2, 0xb7, 0xc3)
    } else {
        Color32::from_rgb(0xf6, 0x7b, 0x7b)
    };
    let mut start = 0;
    for (row_index, row) in view.galley.rows.iter().enumerate() {
        let count = row.char_count_excluding_newline().0;
        let (from, to) = (chars.start.max(start), chars.end.min(start + count));
        if from < to {
            let x0 = galley_origin.x + view.galley.pos_from_cursor(CCursor::new(from)).min.x;
            let x1 = galley_origin.x + view.galley.pos_from_cursor(CCursor::new(to)).min.x;
            let line = view.lines[row_index.min(view.lines.len() - 1)];
            let bottom =
                galley_origin.y + line.top + line.shift + body.content_top + body.content_height;
            let width = x1 - x0;
            let tiles = (width / TILE).round().max(1.0);
            let tile = width / tiles;
            let strip = Rect::from_x_y_ranges(x0..=x1, bottom - 3.0..=bottom - 1.0);
            for at in 0..tiles as usize {
                paint_dot(
                    painter,
                    pos2(x0 + 2.0 + at as f32 * tile, bottom - 2.0),
                    strip,
                    color,
                );
            }
        }
        start += row.char_count_including_newline().0;
    }
}

/// A gradient's 1px-radius disc at `center`, inside `clip`, as Chromium rasterizes its hard stop:
/// each device pixel in `color` at the share of it the disc covers, with no feathering past it.
fn paint_dot(painter: &egui::Painter, center: Pos2, clip: Rect, color: Color32) {
    const SAMPLES: usize = 4;
    let ppp = painter.pixels_per_point();
    let pixel = 1.0 / ppp;
    let area = Rect::from_center_size(center, vec2(2.0, 2.0)).intersect(clip);
    let (left, top) = (
        (area.left() * ppp).floor() as i32,
        (area.top() * ppp).floor() as i32,
    );
    let (right, bottom) = (
        (area.right() * ppp).ceil() as i32,
        (area.bottom() * ppp).ceil() as i32,
    );
    // A mesh, which egui doesn't feather.
    let mut mesh = egui::Mesh::default();
    for y in top..bottom {
        for x in left..right {
            let mut inside = 0;
            for sample in 0..SAMPLES * SAMPLES {
                let at = pos2(
                    (x as f32 + ((sample % SAMPLES) as f32 + 0.5) / SAMPLES as f32) * pixel,
                    (y as f32 + ((sample / SAMPLES) as f32 + 0.5) / SAMPLES as f32) * pixel,
                );
                inside += usize::from(clip.contains(at) && at.distance(center) <= 1.0);
            }
            if inside > 0 {
                let rect = Rect::from_min_size(
                    pos2(x as f32 * pixel, y as f32 * pixel),
                    vec2(pixel, pixel),
                );
                mesh.add_colored_rect(
                    rect,
                    color.gamma_multiply(inside as f32 / (SAMPLES * SAMPLES) as f32),
                );
            }
        }
    }
    painter.add(mesh);
}

/// A slot in the pill: an item, or `Learn`.
struct Slot {
    text: String,
    learn: bool,
    width: f32,
}

impl Editor {
    /// Draws the suggestion pill under its word (above it without room below), centred on it,
    /// for an editor at `origin`; returns the item pressed (mouse-down takes it), the mouse
    /// moving over one choosing it.
    pub(super) fn show_pill(&mut self, ui: &Ui, origin: Pos2) -> Option<usize> {
        let (block, range) = self.pill_word()?;
        let suggestions = self.suggestions()?.clone();
        let ctx = ui.ctx().clone();
        let target = self
            .text_boxes(
                origin,
                Pos::new(block, range.start),
                Pos::new(block, range.end),
            )
            .into_iter()
            .reduce(Rect::union)?;
        let undo = suggestions.kind == SuggestionKind::Undo;
        let mut slots: Vec<Slot> = suggestions
            .items
            .iter()
            .map(|item| Slot {
                text: item.clone(),
                learn: false,
                width: 2.0 * NAME_PADDING
                    + NAME.width(&ctx, item)
                    + if undo { UNDO_GAP + UNDO } else { 0.0 },
            })
            .collect();
        if suggestions.learn {
            let text = format!("Learn \"{}\"", suggestions.word);
            let width = NAME_PADDING + NAME.width(&ctx, &text) + LEARN_PADDING_RIGHT;
            slots.push(Slot {
                text,
                learn: true,
                width,
            });
        }
        let divider = DIVIDER.0 + 2.0 * DIVIDER_MARGIN;
        let width = 2.0 * PADDING
            + slots.iter().map(|slot| slot.width).sum::<f32>()
            + divider * slots.len().saturating_sub(1) as f32;
        // Kept clear of the window's sides, and above the word when the body's visible part ends
        // under it.
        let window = ctx.content_rect();
        let mut left = ((target.left() + target.right()) / 2.0 - width / 2.0).ceil();
        if left < window.left() + SIDE {
            left = window.left() + SIDE;
        } else if left > window.right() - SIDE - width {
            left = (window.right() - SIDE - width).max(window.left() + SIDE);
        }
        let top = if ui.clip_rect().bottom() < target.bottom() + HEIGHT {
            target.top() - ABOVE - HEIGHT
        } else {
            target.bottom() + BELOW
        };
        let card = Rect::from_min_size(pos2(left, top), vec2(width, HEIGHT));
        let colors = Colors::of(exo_ui::theme(&ctx));
        let pointer = ctx.input(|input| {
            (
                input.pointer.hover_pos(),
                input.pointer.primary_pressed(),
                input.pointer.is_moving(),
            )
        });
        let mut picked = None;
        let mut hovered = None;
        Area::new(exo_ui::id(
            &ctx,
            ("exo_editor_autocorrect", origin.x.to_bits()),
        ))
        .order(Order::Foreground)
        .fixed_pos(card.min)
        .show(&ctx, |area| {
            // It takes clicks off the body under it, but never the keyboard (egui would move it
            // there with ↓).
            area.allocate_rect(card, Sense::CLICK);
            let painter = area.painter();
            exo_ui::paint_editor_tooltip(painter, card, colors.fill);
            let mut x = card.left() + PADDING;
            let inner_top = card.top() + PADDING;
            for (index, slot) in slots.iter().enumerate() {
                let item_left = x;
                let selected = suggestions.selected == Some(index);
                if index > 0 {
                    // Hidden beside the chosen item.
                    let beside = selected || suggestions.selected == Some(index - 1);
                    if !beside {
                        let rect = Rect::from_min_size(
                            pos2(
                                x + DIVIDER_MARGIN,
                                inner_top + (HEIGHT - 2.0 * PADDING - DIVIDER.1) / 2.0,
                            ),
                            vec2(DIVIDER.0, DIVIDER.1),
                        );
                        painter.rect_filled(rect, CornerRadius::ZERO, colors.divider);
                    }
                    x += divider;
                }
                let name = Rect::from_min_size(
                    pos2(x, inner_top),
                    vec2(slot.width, HEIGHT - 2.0 * PADDING),
                );
                if selected {
                    painter.rect_filled(name, CornerRadius::same(2), colors.selected_fill);
                }
                let ink = match (selected, slot.learn) {
                    (true, _) => colors.selected_text,
                    (false, true) => colors.learn,
                    (false, false) => colors.text,
                };
                // `align-items: center` under the 1px of top padding.
                let text_top = name.top() + 1.0 + (name.height() - 1.0 - NAME.line_height) / 2.0;
                let text_width = NAME.paint_line(
                    painter,
                    pos2(name.left() + NAME_PADDING, text_top),
                    &slot.text,
                    ink,
                    None,
                );
                if undo {
                    // The 10px box's margin box centred, 2px of it above the box; the arrow from
                    // its top left at its own size (10×11, measured), `[fill*=red]`.
                    let icon_top =
                        name.top() + 1.0 + (name.height() - 1.0 - (UNDO - 2.0)) / 2.0 - 2.0;
                    let at = pos2(name.left() + NAME_PADDING + text_width + UNDO_GAP, icon_top);
                    let size = exo_ui::icon_size(area.ctx(), UNDO_ICON).unwrap_or(vec2(UNDO, UNDO));
                    exo_ui::paint_icon(
                        area,
                        UNDO_ICON,
                        Rect::from_min_size(at, size),
                        if selected {
                            colors.selected_icon
                        } else {
                            colors.icon
                        },
                    );
                }
                x += slot.width;
                let item = Rect::from_x_y_ranges(item_left..=x, card.y_range());
                if let (Some(at), pressed, moving) = pointer
                    && item.contains(at)
                {
                    if moving {
                        hovered = Some(index);
                    }
                    if pressed {
                        picked = Some(index);
                    }
                }
            }
        });
        if let Some(index) = hovered
            && suggestions.selected != Some(index)
        {
            self.hover_suggestion(index);
        }
        picked
    }
}
