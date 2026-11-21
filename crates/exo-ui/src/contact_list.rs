//! The contact list: the list of contacts under a recipient field or a contact picker, each row an
//! avatar, a name and an address with the typed text darker in each, the highlighted row filled.
//! Compose's From list is drawn in its box too.

use egui::{Color32, CornerRadius, Id, Painter, Pos2, Rect, Sense, pos2, vec2};

use crate::{BoxShadow, KeyTooltip, Palette, Placement, TextStyle, Theme, paint_job, pixel_snap};

/// A row's remove button: a 42px cell at the row's right, its `button` 32px tall and padded 16px a
/// side around a 10px cross.
const REMOVE_WIDTH: f32 = 42.0;
const REMOVE_HEIGHT: f32 = 32.0;
const REMOVE_INSET: f32 = 16.0;
const REMOVE_ICON: f32 = 10.0;

/// The list's colors. Lists forced into Snow (the contact pickers') keep Snow's in Carbon.
#[derive(Clone, Copy, Debug)]
pub struct ContactListColors {
    /// The list's background.
    pub background: Color32,
    /// The highlighted row.
    pub selected: Color32,
    /// A name, and the typed text in it.
    pub name: (Color32, Color32),
    /// An address, and the typed text in it.
    pub email: (Color32, Color32),
    /// A row that can't be picked.
    pub disabled: Color32,
}

impl ContactListColors {
    /// The list's colors in `palette`'s theme, or always Snow's when `force_snow`.
    pub fn new(palette: &Palette, force_snow: bool) -> Self {
        // `hsl(220, 45%, 97%)`.
        let selected = Color32::from_rgb(0xf4, 0xf6, 0xfb);
        let black = |opacity: f32| Color32::from_black_alpha((opacity * 255.0).round() as u8);
        if force_snow || palette.theme == Theme::Snow {
            // A list forced into Snow has Snow's colors in Carbon too.
            return Self {
                background: Color32::WHITE,
                selected,
                name: (black(0.5), black(0.85)),
                email: (
                    black(0.45),
                    if force_snow { black(0.85) } else { black(0.75) },
                ),
                disabled: black(0.4),
            };
        }
        Self {
            background: palette.background_base_default,
            selected: Color32::from_rgb(0x46, 0x4b, 0x53),
            name: (palette.ink(0.5, 0.5), Color32::WHITE),
            email: (palette.ink(0.45, 0.45), palette.ink(0.75, 0.75)),
            disabled: palette.ink(0.4, 0.4),
        }
    }
}

/// The list's box: the background, 2px corners, and
/// `0 2px 12px 0 rgba(0,0,0,.05), 0 2px 4px 0 rgba(0,0,0,.1)`. Returns it pixel-snapped.
pub fn paint_contact_list_box(painter: &Painter, rect: Rect, background: Color32) -> Rect {
    let snapped = pixel_snap(rect);
    let radius = CornerRadius::same(2);
    BoxShadow::new([0.0, 2.0], 12.0, 0.0, Color32::from_black_alpha(13))
        .paint(painter, snapped, radius);
    BoxShadow::new([0.0, 2.0], 4.0, 0.0, Color32::from_black_alpha(26))
        .paint(painter, snapped, radius);
    painter.rect_filled(snapped, radius, background);
    snapped
}

/// Paints `text` on the line at `at`, cut at `width`: its first case-insensitive match of `query`
/// in `matched`, the rest in `rest`, unkerned (`font-kerning: none`). Returns its width.
#[expect(
    clippy::too_many_arguments,
    reason = "one cell's text and its two colors"
)]
pub fn paint_matched(
    painter: &Painter,
    at: Pos2,
    text: &str,
    query: &str,
    style: &TextStyle,
    rest: Color32,
    matched: Color32,
    width: f32,
) -> f32 {
    let query = query.trim();
    let found = (!query.is_empty())
        .then(|| text.to_lowercase().find(&query.to_lowercase()))
        .flatten();
    let mut job = egui::text::LayoutJob::default();
    match found {
        Some(start)
            if text.is_char_boundary(start) && text.is_char_boundary(start + query.len()) =>
        {
            job.append(&text[..start], 0.0, style.format(rest));
            job.append(
                &text[start..start + query.len()],
                0.0,
                style.format(matched),
            );
            job.append(&text[start + query.len()..], 0.0, style.format(rest));
        }
        _ => job.append(text, 0.0, style.format(rest)),
    }
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(0.0));
    paint_job(painter, at, crate::text::unkerned(job), style)
}

/// A row's avatar: a 20px circle with the initial in black, or nothing in its cell (a domain's).
#[derive(Clone, Debug)]
pub struct RowAvatar {
    /// The letter, upper-cased.
    pub initial: String,
    /// The circle's color.
    pub color: Color32,
}

/// One row.
#[derive(Clone, Debug)]
pub struct ContactRow<'a> {
    /// What the name cell shows: the name, or the address when there's none.
    pub name: &'a str,
    /// The address cell; empty for a row without one.
    pub email: &'a str,
    /// Already added: subdued, and the pointer can't pick it.
    pub disabled: bool,
    /// For a list with avatars: the avatar, or `None` for an empty cell.
    pub avatar: Option<RowAvatar>,
}

/// Where a row's cells sit, from the row's left: the avatar's left (lists with avatars), and each
/// text's left and width (the cell less its padding).
#[derive(Clone, Copy, Debug)]
pub struct ContactColumns {
    /// The 20px avatar's left, in a list with avatars.
    pub avatar: Option<f32>,
    /// The name's left and width.
    pub name: (f32, f32),
    /// The address's left and width.
    pub email: (f32, f32),
}

/// The list's metrics.
#[derive(Clone, Copy, Debug)]
pub struct ContactListStyle {
    /// The rows' text.
    pub text: TextStyle,
    /// A row's height.
    pub row: f32,
    /// The list's `padding` over the first row and under the last.
    pub padding: f32,
    /// Where the cells sit.
    pub columns: ContactColumns,
    /// Keep Snow's colors in Carbon.
    pub force_snow: bool,
}

/// What the pointer did over the list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContactListResponse {
    /// The row under a moving pointer.
    pub hovered: Option<usize>,
    /// The row a button went down on (a press picks).
    pub pressed: Option<usize>,
    /// The row whose `Remove Suggestion` was clicked.
    pub removed: Option<usize>,
}

/// A contact list of `rows` for `query`, with `highlight` filled.
pub struct ContactSelectorList<'a> {
    rows: &'a [ContactRow<'a>],
    query: &'a str,
    highlight: Option<usize>,
    style: ContactListStyle,
    removable: &'a [bool],
}

impl<'a> ContactSelectorList<'a> {
    /// The list; `highlight` is the row filled, if any.
    pub fn new(
        rows: &'a [ContactRow<'a>],
        query: &'a str,
        highlight: Option<usize>,
        style: ContactListStyle,
    ) -> Self {
        Self {
            rows,
            query,
            highlight,
            style,
            removable: &[],
        }
    }

    /// Which rows offer `Remove Suggestion` while highlighted and hovered (not the thread's own
    /// participants).
    pub fn removable(mut self, removable: &'a [bool]) -> Self {
        self.removable = removable;
        self
    }

    /// The box's height for the rows.
    pub fn height(&self) -> f32 {
        self.rows.len() as f32 * self.style.row + 2.0 * self.style.padding
    }

    /// Draws the list as a box of `width` at `at`, over everything in `order`.
    pub fn show(
        &self,
        ctx: &egui::Context,
        id: Id,
        at: Pos2,
        width: f32,
        order: egui::Order,
    ) -> ContactListResponse {
        let rect = Rect::from_min_size(at, vec2(width, self.height()));
        let colors = ContactListColors::new(crate::palette(ctx), self.style.force_snow);
        let (hover, pressed, moving) = ctx.input(|input| {
            (
                input.pointer.hover_pos(),
                input.pointer.primary_pressed(),
                input.pointer.is_moving(),
            )
        });
        let mut response = ContactListResponse::default();
        egui::Area::new(id)
            .order(order)
            .fixed_pos(rect.min)
            .constrain(false)
            .show(ctx, |area| {
                let painter = area.painter().clone();
                paint_contact_list_box(&painter, rect, colors.background);
                let _ = area.allocate_rect(rect, Sense::click());
                let text = &self.style.text;
                let columns = self.style.columns;
                for (index, contact) in self.rows.iter().enumerate() {
                    let row = Rect::from_min_size(
                        pos2(
                            rect.left(),
                            rect.top() + self.style.padding + index as f32 * self.style.row,
                        ),
                        vec2(width, self.style.row),
                    );
                    if self.highlight == Some(index) {
                        painter.rect_filled(pixel_snap(row), 0.0, colors.selected);
                    }
                    if let (Some(left), Some(avatar)) = (columns.avatar, &contact.avatar) {
                        let circle = Rect::from_center_size(
                            pos2(row.left() + left + 10.0, row.center().y),
                            vec2(20.0, 20.0),
                        );
                        // The initial: 12px, `.355em` down, in black.
                        let initial = TextStyle::new(text.font, 12.0, 16.0);
                        crate::paint_initial_avatar(
                            &painter,
                            circle,
                            avatar.color,
                            &avatar.initial,
                            &initial,
                            Color32::BLACK,
                            0.355 * 12.0,
                        );
                    }
                    // `padding-top: 4px` over the 28px line.
                    let line = row.top() + 4.0;
                    let (name, email) = if contact.disabled {
                        (
                            (colors.disabled, colors.disabled),
                            (colors.disabled, colors.disabled),
                        )
                    } else {
                        (colors.name, colors.email)
                    };
                    paint_matched(
                        &painter,
                        pos2(row.left() + columns.name.0, line),
                        contact.name,
                        self.query,
                        text,
                        name.0,
                        name.1,
                        columns.name.1,
                    );
                    if !contact.email.is_empty() {
                        paint_matched(
                            &painter,
                            pos2(row.left() + columns.email.0, line),
                            contact.email,
                            self.query,
                            text,
                            email.0,
                            email.1,
                            columns.email.1,
                        );
                    }
                    let row_hovered = hover.is_some_and(|at| row.contains(at));
                    let removing = row_hovered
                        && self.highlight == Some(index)
                        && self.removable.get(index).copied().unwrap_or(false);
                    let mut over_remove = false;
                    if removing {
                        let cell = Rect::from_min_size(
                            pos2(row.right() - REMOVE_WIDTH, row.top()),
                            vec2(REMOVE_WIDTH, REMOVE_HEIGHT),
                        );
                        let button =
                            area.interact(cell, id.with(("remove", index)), Sense::click());
                        over_remove = button.hovered();
                        crate::paint_fade_in(&painter, cell, colors.selected);
                        // The cross: 30% ink, 50% while hovered, over 150ms.
                        let hovered = crate::motion::transition(
                            ctx,
                            id.with(("remove-hover", index)),
                            over_remove,
                            0.15,
                            crate::motion::CubicBezier::EASE,
                        );
                        let alpha = 0.3 + 0.2 * hovered;
                        let ink =
                            if self.style.force_snow || crate::palette(ctx).theme == Theme::Snow {
                                Color32::from_black_alpha((alpha * 255.0).round() as u8)
                            } else {
                                Color32::from_white_alpha((alpha * 255.0).round() as u8)
                            };
                        let icon = Rect::from_min_size(
                            pos2(
                                cell.left() + REMOVE_INSET,
                                cell.center().y - REMOVE_ICON / 2.0,
                            ),
                            vec2(REMOVE_ICON, REMOVE_ICON),
                        );
                        crate::paint_icon_on(&painter, "legacy/close", icon, ink);
                        // Right of the button, past the tooltip's 8px padding.
                        KeyTooltip {
                            text: "Remove Suggestion",
                            keys: &[],
                        }
                        .show_at(area, &button, Placement::Right(8.0));
                        if button.clicked() {
                            response.removed = Some(index);
                        }
                    }
                    if row_hovered {
                        if moving {
                            response.hovered = Some(index);
                        }
                        if pressed && !contact.disabled && !over_remove {
                            response.pressed = Some(index);
                        }
                    }
                }
            });
        response
    }
}
