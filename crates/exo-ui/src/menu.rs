//! A dropdown of named choices, as a dropdown field opens it: one 13px column in 32px rows, the
//! highlighted row filled, padded 11px, on the base background with 2px corners and two soft
//! shadows. Measured on the event details' menu.

use egui::{Color32, CornerRadius, Id, Order, Pos2, Rect, Sense, Ui, Vec2, pos2, vec2};

use crate::{BoxShadow, Family, Font, TextStyle, app_theme, pixel_snap};

/// A row's name: 13px in 28px lines, padded `4px 16px 0`; 12px in a dialog's.
const ITEM: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 13.0, 28.0);
const DIALOG_ITEM: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 28.0);
/// A dialog's list is as wide as its names at the list's own 13.5px (measured).
const DIALOG_MEASURE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 13.5, 28.0);
/// A name's `min-width`, which a dialog's list keeps (the Reminders dialog's two short names).
const DIALOG_MIN_WIDTH: f32 = 165.0;
const ROW: f32 = 32.0;
/// A tall list: the name's span 36px tall with its line centered, so the text sits where a 32px
/// row's does; the list comes out 6.5px wider (measured on Notifications' `Notify me:`).
const TALL_ROW: f32 = 36.0;
const TALL_EXTRA_WIDTH: f32 = 6.5;
const ROW_PADDING_TOP: f32 = 4.0;
/// A list of addresses: 28px rows, the address 18px in.
const ADDRESS_ROW: f32 = 28.0;
const ADDRESS_PADDING: f32 = 18.0;
const CELL_PADDING: f32 = 16.0;
/// The list: `padding: 11px 0`, 2px corners.
const LIST_PADDING: f32 = 11.0;
const RADIUS: u8 = 2;
/// `0 2px 12px 0 rgba(0,0,0,.05), 0 2px 4px 0 rgba(0,0,0,.1)`.
const SHADOWS: [BoxShadow; 2] = [
    BoxShadow::new(
        [0.0, 2.0],
        12.0,
        0.0,
        Color32::from_rgba_premultiplied(0, 0, 0, 13),
    ),
    BoxShadow::new(
        [0.0, 2.0],
        4.0,
        0.0,
        Color32::from_rgba_premultiplied(0, 0, 0, 26),
    ),
];

/// How a list's rows are drawn: their height, their text's style and ink, and where it sits.
#[derive(Clone, Copy)]
struct Rows {
    height: f32,
    left: f32,
    top: f32,
    style: TextStyle,
    ink: Color32,
    /// The list's padding above and below its rows.
    padding: f32,
}

/// A list of `items`, `highlighted` one filled.
#[derive(Clone, Copy, Debug)]
pub struct Menu<'a> {
    pub items: &'a [&'a str],
    pub highlighted: usize,
}

/// What the pointer did over a [`Menu`] this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuResponse {
    /// The row under the pointer.
    pub hovered: Option<usize>,
    /// The row clicked.
    pub clicked: Option<usize>,
    /// Where the list is.
    pub rect: Rect,
}

impl Menu<'_> {
    /// The list's size: its widest name, padded, rounded up to a whole pixel.
    pub fn size(&self, ctx: &egui::Context) -> Vec2 {
        let widest = self.widest(ctx, &ITEM);
        vec2((widest + 2.0 * CELL_PADDING).ceil(), self.height(ROW))
    }

    fn widest(&self, ctx: &egui::Context, style: &TextStyle) -> f32 {
        self.items
            .iter()
            .map(|item| style.width(ctx, item))
            .fold(0.0, f32::max)
    }

    fn height(&self, row: f32) -> f32 {
        self.items.len() as f32 * row + 2.0 * LIST_PADDING
    }

    /// Paints the list with its top left at `top_left`, above everything else.
    pub fn show(&self, ui: &Ui, id: Id, top_left: Pos2) -> MenuResponse {
        self.paint(ui, id, top_left, false, false, &ITEM)
    }

    /// [`Menu::show`] at least `min_width` wide (a name's `min-width`, which some lists change).
    pub fn show_at_least(&self, ui: &Ui, id: Id, top_left: Pos2, min_width: f32) -> MenuResponse {
        let size = self.size(ui.ctx());
        let rows = Rows {
            height: ROW,
            left: CELL_PADDING,
            top: ROW_PADDING_TOP,
            style: ITEM,
            ink: app_theme(ui.ctx()).palette().text_base_subdued,
            padding: LIST_PADDING,
        };
        self.paint_rows(
            ui,
            id,
            Rect::from_min_size(top_left, vec2(size.x.max(min_width), size.y)),
            rows,
        )
    }

    /// [`Menu::show`] for a dialog's dropdown, whose names are 12px and darker: black or white at
    /// `.7`. The list takes the app's theme, not the dialog's (Carbon's is dark).
    pub fn show_in_dialog(&self, ui: &Ui, id: Id, top_left: Pos2) -> MenuResponse {
        self.paint(ui, id, top_left, true, false, &DIALOG_MEASURE)
    }

    /// [`Menu::show_in_dialog`] in 36px rows.
    pub fn show_tall_in_dialog(&self, ui: &Ui, id: Id, top_left: Pos2) -> MenuResponse {
        self.paint(ui, id, top_left, true, true, &DIALOG_MEASURE)
    }

    /// [`Menu::show_in_dialog`] as wide as its names at their own 12px, padded: Auto Advance's
    /// list of long names, measured 258px.
    pub fn show_fitted_in_dialog(&self, ui: &Ui, id: Id, top_left: Pos2) -> MenuResponse {
        self.paint(ui, id, top_left, true, false, &DIALOG_ITEM)
    }

    /// [`Menu::show_in_dialog`] for a list of addresses (a dropdown of contacts, such as the
    /// Signatures dialog's aliases): 28px rows, the address centred in its line 18px in, and the
    /// list as wide as the longest (measured).
    pub fn show_addresses_in_dialog(&self, ui: &Ui, id: Id, top_left: Pos2) -> MenuResponse {
        let size = vec2(
            (self.widest(ui.ctx(), &DIALOG_ITEM) + 2.0 * ADDRESS_PADDING).round(),
            self.height(ADDRESS_ROW),
        );
        let rows = Rows {
            height: ADDRESS_ROW,
            left: ADDRESS_PADDING,
            top: 0.0,
            style: DIALOG_ITEM,
            ink: app_theme(ui.ctx()).palette().ink(0.7, 0.7),
            padding: LIST_PADDING,
        };
        self.paint_rows(ui, id, Rect::from_min_size(top_left, size), rows)
    }

    /// [`Menu::show`] padded `4px 0` (the calendar form's Repeat lists), its names `cell` in from
    /// either side and the list at least `min_width` wide.
    pub fn show_compact(
        &self,
        ui: &Ui,
        id: Id,
        top_left: Pos2,
        cell: f32,
        min_width: f32,
    ) -> MenuResponse {
        const PADDING: f32 = 4.0;
        let ctx = ui.ctx().clone();
        let width = (self.widest(&ctx, &ITEM) + 2.0 * cell)
            .ceil()
            .max(min_width);
        let size = vec2(width, self.items.len() as f32 * ROW + 2.0 * PADDING);
        let rows = Rows {
            height: ROW,
            left: cell,
            top: ROW_PADDING_TOP,
            style: ITEM,
            ink: app_theme(&ctx).palette().text_base_subdued,
            padding: PADDING,
        };
        self.paint_rows(ui, id, Rect::from_min_size(top_left, size), rows)
    }

    fn paint(
        &self,
        ui: &Ui,
        id: Id,
        top_left: Pos2,
        dialog: bool,
        tall: bool,
        measure: &TextStyle,
    ) -> MenuResponse {
        let ctx = ui.ctx().clone();
        let palette = app_theme(&ctx).palette();
        let (row_height, extra) = if tall {
            (TALL_ROW, TALL_EXTRA_WIDTH)
        } else {
            (ROW, 0.0)
        };
        let (style, name_ink, size) = if dialog {
            (
                DIALOG_ITEM,
                palette.ink(0.7, 0.7),
                vec2(
                    (self.widest(&ctx, measure) + 2.0 * CELL_PADDING + extra)
                        .round()
                        .max(DIALOG_MIN_WIDTH),
                    self.height(row_height),
                ),
            )
        } else {
            (ITEM, palette.text_base_subdued, self.size(&ctx))
        };
        self.paint_rows(
            ui,
            id,
            Rect::from_min_size(top_left, size),
            Rows {
                height: row_height,
                left: CELL_PADDING,
                top: ROW_PADDING_TOP,
                style,
                ink: name_ink,
                padding: LIST_PADDING,
            },
        )
    }

    /// Paints the list at `list`, its rows in `rows`' style.
    fn paint_rows(&self, ui: &Ui, id: Id, list: Rect, rows: Rows) -> MenuResponse {
        let Rows {
            height: row_height,
            left: padding,
            top: padding_top,
            style,
            ink: name_ink,
            padding: list_padding,
        } = rows;
        let ctx = ui.ctx().clone();
        let palette = app_theme(&ctx).palette();
        let mut response = MenuResponse {
            hovered: None,
            clicked: None,
            rect: list,
        };
        egui::Area::new(id)
            .order(Order::Tooltip)
            .fixed_pos(list.min)
            .constrain(false)
            .show(&ctx, |area| {
                let painter = area.painter().clone();
                let snapped = pixel_snap(list);
                for shadow in SHADOWS {
                    shadow.paint(&painter, snapped, CornerRadius::same(RADIUS));
                }
                painter.rect_filled(
                    snapped,
                    CornerRadius::same(RADIUS),
                    palette.background_base_default,
                );
                let _ = area.allocate_rect(list, Sense::click());
                for (index, item) in self.items.iter().enumerate() {
                    let row = Rect::from_min_size(
                        pos2(
                            list.left(),
                            list.top() + list_padding + index as f32 * row_height,
                        ),
                        vec2(list.width(), row_height),
                    );
                    let hit = area
                        .interact(row, id.with(index), Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if hit.hovered() {
                        response.hovered = Some(index);
                    }
                    if hit.clicked() {
                        response.clicked = Some(index);
                    }
                    if index == self.highlighted {
                        painter.rect_filled(pixel_snap(row), 0.0, palette.fill_hover_default);
                    }
                    style.paint_line(
                        &painter,
                        pos2(row.left() + padding, row.top() + padding_top),
                        item,
                        name_ink,
                        None,
                    );
                }
            });
        response
    }
}

/// A dialog's dropdown button: `value` at 12px/16px with its line box's top left at
/// `value_top_left`, black at `.7`, and the down caret 10px after it, 10px wide, black at `.5`, the
/// icon's 12×6.7 viewBox fitted to that, 5.4px down the line. Returns the button, for the pointer.
pub fn paint_dialog_dropdown(ui: &Ui, value_top_left: Pos2, value: &str) -> Rect {
    let palette = crate::palette(ui.ctx());
    paint_dialog_dropdown_with(
        ui,
        value_top_left,
        value,
        DialogDropdown {
            value: palette.ink(0.7, 0.7),
            caret: palette.ink(0.5, 0.5),
            gap: 10.0,
        },
    )
}

/// A dialog's dropdown button's inks, and the gap before its caret, where a dialog's rules change
/// them (the Reminders dialog's `rgba(0,0,0,.9)` value and `.3` caret, `margin-left: 4px`).
#[derive(Clone, Copy, Debug)]
pub struct DialogDropdown {
    pub value: Color32,
    pub caret: Color32,
    pub gap: f32,
}

/// [`paint_dialog_dropdown`] in `style`.
pub fn paint_dialog_dropdown_with(
    ui: &Ui,
    value_top_left: Pos2,
    value: &str,
    style: DialogDropdown,
) -> Rect {
    const VALUE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
    const CARET: Vec2 = vec2(10.0, 10.0 * 6.7 / 12.0);
    const CARET_TOP: f32 = 5.4;
    let width = VALUE.paint_line(ui.painter(), value_top_left, value, style.value, None);
    let caret = Rect::from_min_size(value_top_left + vec2(width + style.gap, CARET_TOP), CARET);
    crate::paint_icon(ui, "legacy/DEPRECATED_downCaret", caret, style.caret);
    Rect::from_min_size(
        value_top_left - vec2(0.0, 2.0),
        vec2(width + style.gap + CARET.x, 20.0),
    )
}
