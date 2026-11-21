//! Dark tooltips that pair a line of text with keycaps: the hover tooltip naming a control's
//! shortcut (`Undo` `Z`) and the bottom-centre hint tooltip (`Next time, hit` `Z`
//! `to undo instantly!`).
//!
//! The box: 4px corners, padding `6px 7px 3px`, Exo Sans 12px/20px bold,
//! `box-shadow: 0 2px 4px rgba(0,0,0,.1), 0 2px 12px rgba(0,0,0,.05)`; dark (`#45494e`, text
//! `#ebeef2`) in Snow and light (`#f5f5f5`, text `#18191b`) in Carbon. Tooltips fade in over 200ms,
//! hint tooltips over 500ms. A hover tooltip shows once hover intent has the pointer settled on its
//! anchor (polled every 100ms), at once while the keyboard is on the anchor, and goes 150ms after
//! the pointer leaves the anchor, unless the pointer is on the tooltip.

use egui::{
    Area, Color32, Context, CornerRadius, Id, Order, Painter, Pos2, Rangef, Rect, Response, Sense,
    Shape, Ui, Vec2, text::LayoutJob,
};

use crate::{
    BoxShadow, Family, Font, HoverIntent, KeycapColors, Keycaps, TextStyle, keycap::KEYCAP_GAP,
    motion, paint_job, palette, pixel_snap,
};

const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 20.0);
const PADDING_TOP: f32 = 6.0;
const PADDING_X: f32 = 7.0;
const PADDING_BOTTOM: f32 = 3.0;
/// A hint's close icon: 12×13 at `margin: 4px 0 0 12.5px; top: -1px`.
const CLOSE_GAP: f32 = 12.5;
const CLOSE_SIZE: Vec2 = Vec2::new(12.0, 13.0);
/// A note line's gap above it.
const NOTE_MARGIN: f32 = 6.0;
/// A status line's gap above it; its text is at half opacity.
const STATUS_MARGIN: f32 = 4.0;
/// How long a tooltip stays once the pointer has left its anchor.
const HIDE_AFTER: f64 = 0.15;
/// Hover intent's poll on the split tabs, and on the rest.
const SLOW_INTERVAL: f64 = 0.3;
const INTERVAL: f64 = 0.1;
/// Padding on both sides of the box, toward the anchor and away: part of the tooltip, which stays
/// while the pointer is on it.
const PLACEMENT_PADDING: f32 = 8.0;

/// One piece of a tooltip's line: text, or a run of keycaps.
#[derive(Clone, Copy, Debug)]
pub enum TooltipPart<'a> {
    /// Words.
    Text(&'a str),
    /// Keycaps, one per token (see [`crate::keycap_label`]).
    Keys(&'a [&'a str]),
    /// Words between keycaps, at 85% (` · `).
    Separator(&'a str),
    /// Room, in px (10px between a shortcut's text and its keys).
    Gap(f32),
}

/// A tooltip's line, laid out.
struct Line {
    pieces: Vec<Piece>,
    width: f32,
}

enum Piece {
    Text {
        job: LayoutJob,
        left: f32,
        opacity: f32,
    },
    Keys {
        keycaps: Keycaps,
        left: f32,
    },
}

impl Line {
    /// Lays out `parts` with `text_gap` before every text and the keycaps' own 4px gap before
    /// every cap, except that a line can't start with a gap before keycaps.
    fn new(ctx: &Context, parts: &[TooltipPart<'_>], text_gap: f32, after_text: f32) -> Self {
        let color = text_color(ctx);
        let mut x = 0.0;
        let mut pieces = Vec::with_capacity(parts.len());
        for (i, part) in parts.iter().enumerate() {
            match part {
                TooltipPart::Text(text) | TooltipPart::Separator(text) => {
                    let job = LayoutJob::single_section((*text).to_owned(), TEXT.format(color));
                    let width = ctx
                        .fonts_mut(|fonts| fonts.layout_job(job.clone()))
                        .size()
                        .x;
                    x += text_gap;
                    let opacity = if matches!(part, TooltipPart::Separator(_)) {
                        0.85
                    } else {
                        1.0
                    };
                    pieces.push(Piece::Text {
                        job,
                        left: x,
                        opacity,
                    });
                    x += width + after_text;
                }
                TooltipPart::Gap(gap) => x += gap,
                TooltipPart::Keys(tokens) => {
                    let keycaps = Keycaps::new(ctx, tokens, keycap_colors(ctx));
                    let left = if i == 0 { x - KEYCAP_GAP } else { x };
                    x = left + keycaps.width();
                    pieces.push(Piece::Keys { keycaps, left });
                }
            }
        }
        Self { pieces, width: x }
    }

    fn paint(&self, painter: &Painter, left_top: Pos2) {
        for piece in &self.pieces {
            match piece {
                Piece::Text { job, left, opacity } => {
                    let mut painter = painter.clone();
                    painter.multiply_opacity(*opacity);
                    paint_job(&painter, left_top + Vec2::X * *left, job.clone(), &TEXT);
                }
                // A cap's top is its line's top.
                Piece::Keys { keycaps, left } => {
                    keycaps.paint(painter, left_top.x + left, left_top.y)
                }
            }
        }
    }
}

/// A tooltip's text: `#ebeef2` on Snow's dark card, `#18191b` on Carbon's light one.
pub fn text_color(ctx: &Context) -> Color32 {
    match crate::theme(ctx) {
        crate::Theme::Snow => Color32::from_rgb(0xeb, 0xee, 0xf2),
        crate::Theme::Carbon => Color32::from_rgb(0x18, 0x19, 0x1b),
    }
}

/// A tooltip's keycap: the base cap, its label the tooltip's text in Snow and
/// `hsla(0, 0%, 100%, .9)` in Carbon.
fn keycap_colors(ctx: &Context) -> KeycapColors {
    let base = KeycapColors::base(palette(ctx));
    let text = match crate::theme(ctx) {
        crate::Theme::Snow => text_color(ctx),
        crate::Theme::Carbon => Color32::from_white_alpha(230),
    };
    // A sequence's `then` takes the tooltip's text.
    KeycapColors {
        text,
        divider: text_color(ctx),
        ..base
    }
}

/// Paints a tooltip's box at `rect`, for tooltips with contents of their own (the Accounts dialog's
/// color picker).
pub fn paint_tooltip_box(painter: &Painter, rect: Rect) {
    paint_box(painter, rect, true);
}

/// Paints the box of a tooltip compose's editor anchors to its text, which each dropdown fills its
/// own colour (the emoji and contact pickers, autocorrect's pill): 4px corners and a tooltip's two
/// shadows.
pub fn paint_editor_tooltip(painter: &Painter, rect: Rect, fill: Color32) {
    let rect = pixel_snap(rect);
    let radius = CornerRadius::same(4);
    for shadow in [
        BoxShadow::new([0.0, 2.0], 12.0, 0.0, Color32::from_black_alpha(13)),
        BoxShadow::new([0.0, 2.0], 4.0, 0.0, Color32::from_black_alpha(26)),
    ] {
        shadow.paint(painter, rect, radius);
    }
    painter.rect_filled(rect, radius, fill);
}

/// Paints the tooltip box, with its two shadows when `shadow`.
fn paint_box(painter: &Painter, rect: Rect, shadow: bool) {
    let rect = pixel_snap(rect);
    let radius = CornerRadius::same(4);
    if shadow {
        BoxShadow::new([0.0, 2.0], 4.0, 0.0, Color32::from_black_alpha(26))
            .paint(painter, rect, radius);
        BoxShadow::new([0.0, 2.0], 12.0, 0.0, Color32::from_black_alpha(13))
            .paint(painter, rect, radius);
    }
    painter.rect_filled(
        rect,
        radius,
        palette(painter.ctx()).tooltip_background_default,
    );
}

/// A hover tooltip naming a control and its shortcut: the text, 10px, then the keycaps; with no
/// keys, the text alone (`Folders`).
pub struct KeyTooltip<'a> {
    /// What the control does (`Undo`).
    pub text: &'a str,
    /// Its shortcut's tokens (`["z"]`), or none.
    pub keys: &'a [&'a str],
}

/// Where a hover tooltip's box goes, relative to its anchor.
#[derive(Clone, Copy, Debug)]
pub enum Placement {
    /// Centred over the anchor, its bottom edge this far below the anchor's top.
    Above(f32),
    /// Under the anchor: its top edge `gap` px below the anchor's bottom, its centre `shift` px
    /// right of the anchor's.
    Below {
        /// From the anchor's bottom to the box's top.
        gap: f32,
        /// From the anchor's centre to the box's.
        shift: f32,
    },
    /// Right of the anchor, `gap` px from its right edge, centred on it vertically.
    Right(f32),
    /// Left of the anchor, `gap` px from its left edge, centred on it vertically.
    Left(f32),
    /// Its left edge at this point, centred on it vertically, wherever the hovered area is: a
    /// tether to a box of its own, such as the conversation status's 0×0 one.
    Beside(Pos2),
    /// [`Self::Right`], unless the box would end past `until`: then as far left of the anchor, a
    /// tooltip that doesn't fit its scroll container flipping to the other side.
    RightOrLeft {
        /// Between the anchor and the box.
        gap: f32,
        /// The furthest right the box may end.
        until: f32,
    },
    /// Under the anchor, its left edge `inset` px right of the anchor's (left of it when
    /// negative) and its top edge `gap` px below the anchor's bottom.
    BelowFrom {
        /// From the anchor's bottom to the box's top.
        gap: f32,
        /// From the anchor's left edge to the box's.
        inset: f32,
    },
}

impl Placement {
    /// Whether the box is left or right of the anchor rather than above or below it.
    fn beside(self) -> bool {
        matches!(
            self,
            Self::Right(_) | Self::Left(_) | Self::Beside(_) | Self::RightOrLeft { .. }
        )
    }
}

impl KeyTooltip<'_> {
    /// Shows the tooltip while `anchor` is hovered, centred over it with its bottom edge `overlap`
    /// px below the anchor's top (10px for a toast's).
    pub fn show_above(&self, ui: &Ui, anchor: &Response, overlap: f32) {
        self.rows().show_above(ui, anchor, overlap, false);
    }

    /// Shows the tooltip while `anchor` is hovered, centred under it with its top edge `gap` px
    /// below the anchor's bottom.
    pub fn show_below(&self, ui: &Ui, anchor: &Response, gap: f32) {
        self.rows()
            .show_below(ui, anchor, gap, false, Rangef::EVERYTHING);
    }

    /// Shows the tooltip while `anchor` is hovered, at `placement`.
    pub fn show_at(&self, ui: &Ui, anchor: &Response, placement: Placement) {
        self.rows().show_at(ui, anchor, placement);
    }

    /// [`show_at`](Self::show_at), shown also while `focused` (the keyboard is on the anchor), with
    /// [`show_at_with_status`](Self::show_at_with_status)'s line under it when there's a `status`.
    pub fn show_at_focused(
        &self,
        ui: &Ui,
        anchor: &Response,
        placement: Placement,
        focused: bool,
        status: Option<&str>,
    ) {
        self.rows().show_with(
            ui,
            anchor,
            focused,
            placement,
            Rangef::EVERYTHING,
            true,
            status,
        );
    }

    /// [`show_at`](Self::show_at) without the box's shadow, as on the sign-in screen.
    pub fn show_flat_at(&self, ui: &Ui, anchor: &Response, placement: Placement) {
        self.rows()
            .show(ui, anchor, false, placement, Rangef::EVERYTHING, false);
    }

    /// [`show_above`](Self::show_above) with a second line 6px under the shortcut, such as Attach's
    /// `(or drag 'n' drop files)`.
    pub fn show_above_with_note(&self, ui: &Ui, anchor: &Response, overlap: f32, note: &str) {
        ShortcutTooltip {
            note: Some(note),
            ..self.rows()
        }
        .show_above(ui, anchor, overlap, false);
    }

    /// [`show_at`](Self::show_at) with a dimmer line under it, such as Remind me's
    /// `Returning: tomorrow if no reply`, moved sideways as needed to stay `within` (the scroll
    /// container).
    pub fn show_at_with_status(
        &self,
        ui: &Ui,
        anchor: &Response,
        placement: Placement,
        status: &str,
        within: Rangef,
    ) {
        self.rows()
            .show_with(ui, anchor, false, placement, within, true, Some(status));
    }

    fn rows(&self) -> ShortcutTooltip<'_> {
        ShortcutTooltip {
            rows: std::slice::from_ref(self),
            note: None,
        }
    }
}

/// A tooltip naming one or more commands (rows 6px apart): each row's text at the left and its
/// keycaps at the right, the box as wide as the widest row; then an optional note line. Compose's
/// controls show one while hovered or focused (`Send` and `Send + Mark Done`; `Cc`, `Bcc` and
/// `From` on the expand button).
pub struct ShortcutTooltip<'a> {
    /// The commands, one row each.
    pub rows: &'a [KeyTooltip<'a>],
    /// A line under them, such as Attach's `(or drag 'n' drop files)`.
    pub note: Option<&'a str>,
}

impl ShortcutTooltip<'_> {
    /// Shows the tooltip while `anchor` is hovered (or `focused`), centred over it with its bottom
    /// edge `overlap` px below the anchor's top.
    pub fn show_above(&self, ui: &Ui, anchor: &Response, overlap: f32, focused: bool) {
        self.show(
            ui,
            anchor,
            focused,
            Placement::Above(overlap),
            Rangef::EVERYTHING,
            true,
        );
    }

    /// Shows the tooltip while `anchor` is hovered (or `focused`), centred under it with its box's
    /// top `gap` px below the anchor's bottom, moved sideways as needed to stay `within` (the
    /// scroll container).
    pub fn show_below(&self, ui: &Ui, anchor: &Response, gap: f32, focused: bool, within: Rangef) {
        self.show(
            ui,
            anchor,
            focused,
            Placement::Below { gap, shift: 0.0 },
            within,
            true,
        );
    }

    /// Shows the tooltip while `anchor` is hovered, at `placement`.
    pub fn show_at(&self, ui: &Ui, anchor: &Response, placement: Placement) {
        self.show(ui, anchor, false, placement, Rangef::EVERYTHING, true);
    }

    /// Shows it as the footer's calendar button does (`padding: 7px 7px 5px`, 20px rows 4px apart),
    /// while `anchor` is hovered: centred over it with its bottom edge `overlap` px below the
    /// anchor's top, moved left as needed to stay `within`.
    pub fn show_calendar_above(&self, ui: &Ui, anchor: &Response, overlap: f32, within: Rangef) {
        self.show_spaced(
            ui,
            anchor,
            false,
            Placement::Above(overlap),
            within,
            true,
            None,
            &Spacing::CALENDAR,
        );
    }

    fn show(
        &self,
        ui: &Ui,
        anchor: &Response,
        focused: bool,
        placement: Placement,
        within: Rangef,
        shadow: bool,
    ) {
        self.show_with(ui, anchor, focused, placement, within, shadow, None);
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "`show` and the status line only Remind me has"
    )]
    fn show_with(
        &self,
        ui: &Ui,
        anchor: &Response,
        focused: bool,
        placement: Placement,
        within: Rangef,
        shadow: bool,
        status: Option<&str>,
    ) {
        self.show_spaced(
            ui,
            anchor,
            focused,
            placement,
            within,
            shadow,
            status,
            &Spacing::DEFAULT,
        );
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "`show_with` and the calendar's spacing"
    )]
    fn show_spaced(
        &self,
        ui: &Ui,
        anchor: &Response,
        focused: bool,
        placement: Placement,
        within: Rangef,
        shadow: bool,
        status: Option<&str>,
        spacing: &Spacing,
    ) {
        let Some(shown) = hover(ui, anchor, focused, INTERVAL) else {
            return;
        };
        let mut painter = ui.ctx().layer_painter(egui::LayerId::new(
            Order::Tooltip,
            anchor.id.with("exo_ui::tooltip"),
        ));
        painter.multiply_opacity(shown.opacity);
        let laid = self.lay_out(ui.ctx(), status, spacing);
        let size = laid.size;
        let around = anchor.rect;
        let centred = |x: f32| (x - size.x / 2.0).min(within.max - size.x).max(within.min);
        let min = match placement {
            Placement::Above(overlap) => {
                Pos2::new(centred(around.center().x), around.top() + overlap - size.y)
            }
            Placement::Below { gap, shift } => {
                Pos2::new(centred(around.center().x + shift), around.bottom() + gap)
            }
            Placement::Right(gap) => {
                Pos2::new(around.right() + gap, around.center().y - size.y / 2.0)
            }
            Placement::Left(gap) => Pos2::new(
                around.left() - gap - size.x,
                around.center().y - size.y / 2.0,
            ),
            Placement::Beside(at) => Pos2::new(at.x, at.y - size.y / 2.0),
            Placement::RightOrLeft { gap, until } => {
                let right = around.right() + gap;
                let x = if right + size.x > until {
                    around.left() - gap - size.x
                } else {
                    right
                };
                Pos2::new(x, around.center().y - size.y / 2.0)
            }
            Placement::BelowFrom { gap, inset } => {
                Pos2::new(around.left() + inset, around.bottom() + gap)
            }
        };
        let rect = Rect::from_min_size(min, size);
        shown.painted(ui, rect, placement.beside());
        laid.paint(&painter, rect, shadow, spacing);
    }

    /// The box's size, for a caller that places and shows it itself ([`Self::paint`]).
    pub fn size(&self, ctx: &Context) -> Vec2 {
        self.lay_out(ctx, None, &Spacing::DEFAULT).size
    }

    /// Paints it with its box's top left at `min`, on `painter`'s layer, for a caller that shows
    /// it for as long as it likes: compose's `Previous Day` / `Next Day` under a date in the body.
    pub fn paint(&self, painter: &Painter, min: Pos2) {
        let laid = self.lay_out(painter.ctx(), None, &Spacing::DEFAULT);
        laid.paint(
            painter,
            Rect::from_min_size(min, laid.size),
            true,
            &Spacing::DEFAULT,
        );
    }

    fn lay_out(&self, ctx: &Context, status: Option<&str>, spacing: &Spacing) -> LaidRows {
        let caps = keycap_colors(ctx);
        let rows: Vec<(Line, Keycaps)> = self
            .rows
            .iter()
            .map(|row| {
                (
                    Line::new(ctx, &[TooltipPart::Text(row.text)], 0.0, 0.0),
                    Keycaps::new(ctx, row.keys, caps),
                )
            })
            .collect();
        let note = self
            .note
            .map(|note| Line::new(ctx, &[TooltipPart::Text(note)], 0.0, 0.0));
        let status = status.map(|status| Line::new(ctx, &[TooltipPart::Text(status)], 0.0, 0.0));
        // A row is a flex row whose keycap line box grows it to 21px; the text keeps 10px clear
        // of the keys. Text alone is a 20px line.
        let width = rows
            .iter()
            .map(|(text, keys)| {
                if keys.width() > 0.0 {
                    text.width + 10.0 + keys.width()
                } else {
                    text.width
                }
            })
            .fold(note.as_ref().map_or(0.0, |note| note.width), f32::max)
            .max(status.as_ref().map_or(0.0, |status| status.width));
        let rows_height = rows
            .iter()
            .map(|(_, keys)| spacing.row_height(keys))
            .sum::<f32>()
            + rows.len().saturating_sub(1) as f32 * spacing.gap;
        let mut size = Vec2::new(
            width + 2.0 * PADDING_X,
            spacing.top + rows_height + spacing.bottom,
        );
        if note.is_some() {
            size.y += NOTE_MARGIN + TEXT.line_height;
        }
        if status.is_some() {
            size.y += STATUS_MARGIN + TEXT.line_height;
        }
        LaidRows {
            rows,
            note,
            status,
            width,
            size,
        }
    }
}

/// A [`ShortcutTooltip`] laid out.
struct LaidRows {
    rows: Vec<(Line, Keycaps)>,
    note: Option<Line>,
    status: Option<Line>,
    /// The widest row's.
    width: f32,
    /// The box's.
    size: Vec2,
}

impl LaidRows {
    fn paint(&self, painter: &Painter, rect: Rect, shadow: bool, spacing: &Spacing) {
        paint_box(painter, rect, shadow);
        // The lines go down with the box onto whole pixels, as Chromium puts them (see
        // `TextTooltip::show_left`).
        let mut row = Pos2::new(rect.min.x + PADDING_X, pixel_snap(rect).min.y + spacing.top);
        for (text, keys) in &self.rows {
            text.paint(painter, row);
            keys.paint(painter, row.x + self.width - keys.width(), row.y);
            row.y += spacing.row_height(keys) + spacing.gap;
        }
        if let Some(note) = &self.note {
            note.paint(painter, row);
        }
        if let Some(status) = &self.status {
            let mut dim = painter.clone();
            dim.multiply_opacity(0.5);
            status.paint(&dim, row - Vec2::Y * (NOTE_MARGIN - STATUS_MARGIN));
        }
    }
}

/// A shortcut row's height: a flex row grown to 21px by its keycaps' line box.
const ROW_HEIGHT: f32 = 21.0;

/// How a [`ShortcutTooltip`]'s box spaces its rows.
struct Spacing {
    /// The box's top and bottom padding.
    top: f32,
    bottom: f32,
    /// A row with keys, and the room between rows.
    row: f32,
    gap: f32,
}

impl Spacing {
    const DEFAULT: Self = Self {
        top: PADDING_TOP,
        bottom: PADDING_BOTTOM,
        row: ROW_HEIGHT,
        gap: NOTE_MARGIN,
    };

    /// A row with keys is a [`Self::row`] tall; text alone is a 20px line.
    fn row_height(&self, keys: &Keycaps) -> f32 {
        if keys.width() > 0.0 {
            self.row
        } else {
            TEXT.line_height
        }
    }
    /// The calendar button's. Its keycap labels sit 1px higher, which leaves them where the other
    /// tooltips' are, 2px into their caps (measured).
    const CALENDAR: Self = Self {
        top: 7.0,
        bottom: 5.0,
        row: 20.0,
        gap: 4.0,
    };
}

/// A split tab's tooltip: what the tab is over the key that goes there. The description is normal
/// weight in Classic, wraps at 224px (`text-wrap: pretty`) and stops at three lines; the box is as
/// wide as it or the key's row, whichever is wider.
pub struct DescribedTooltip<'a> {
    /// What the tab shows, if anything says.
    pub description: Option<&'a str>,
    /// The key's row.
    pub row: KeyTooltip<'a>,
}

impl DescribedTooltip<'_> {
    /// `max-width: 224px`, `margin-bottom: 8px`, `-webkit-line-clamp: 3`.
    const WIDTH: f32 = 224.0;
    const GAP: f32 = 8.0;
    const LINES: usize = 3;
    const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);

    /// Shows it while `anchor` is hovered, hover intent polling every 300ms, or `focused`, centred
    /// under it with its box's top `gap` px below the anchor's bottom.
    pub fn show_below(&self, ui: &Ui, anchor: &Response, gap: f32, focused: bool) {
        let Some(shown) = hover(ui, anchor, focused, SLOW_INTERVAL) else {
            return;
        };
        let ctx = ui.ctx();
        let mut painter = ctx.layer_painter(egui::LayerId::new(
            Order::Tooltip,
            anchor.id.with("exo_ui::tooltip"),
        ));
        painter.multiply_opacity(shown.opacity);
        let lines = self
            .description
            .map(|text| Self::lines(ctx, text))
            .unwrap_or_default();
        let described = lines
            .iter()
            .map(|line| Self::TEXT.width(ctx, line))
            .fold(0.0, f32::max);
        // A description that wraps is a block as wide as its `max-width`.
        let described = if lines.len() > 1 {
            Self::WIDTH
        } else {
            described
        };
        let text = Line::new(ctx, &[TooltipPart::Text(self.row.text)], 0.0, 0.0);
        let keys = Keycaps::new(ctx, self.row.keys, keycap_colors(ctx));
        let width = described.max(text.width + 10.0 + keys.width());
        let description_height = if lines.is_empty() {
            0.0
        } else {
            lines.len() as f32 * Self::TEXT.line_height + Self::GAP
        };
        let size = Vec2::new(
            width + 2.0 * PADDING_X,
            PADDING_TOP + description_height + ROW_HEIGHT + PADDING_BOTTOM,
        );
        let rect = Rect::from_min_size(
            Pos2::new(
                anchor.rect.center().x - size.x / 2.0,
                anchor.rect.bottom() + gap,
            ),
            size,
        );
        shown.painted(ui, rect, false);
        paint_box(&painter, rect, true);
        let mut top = rect.top() + PADDING_TOP;
        for line in &lines {
            Self::TEXT.paint_line(
                &painter,
                Pos2::new(rect.left() + PADDING_X, top),
                line,
                text_color(ctx),
                None,
            );
            top += Self::TEXT.line_height;
        }
        if !lines.is_empty() {
            top += Self::GAP;
        }
        text.paint(&painter, Pos2::new(rect.left() + PADDING_X, top));
        keys.paint(
            &painter,
            rect.left() + PADDING_X + width - keys.width(),
            top,
        );
    }

    /// The description's lines: wrapped, and a fourth line's worth cut to an ellipsis at the
    /// end of the third.
    fn lines(ctx: &Context, text: &str) -> Vec<String> {
        let mut lines = Self::TEXT.wrap_pretty(ctx, text, Self::WIDTH);
        if lines.len() > Self::LINES {
            let rest = lines.split_off(Self::LINES - 1).join(" ");
            let mut cut = rest.clone();
            while !cut.is_empty() && Self::TEXT.width(ctx, &format!("{cut}…")) > Self::WIDTH {
                cut.pop();
            }
            lines.push(format!("{}…", cut.trim_end()));
        }
        lines
    }
}

/// A tooltip of wrapping text, open for as long as it's shown, such as a failed draft's reason
/// under its header: `\n` breaks a line, lines wrap at `max_width`, and an optional `Learn more`
/// link follows on a line of its own, underlined and in the text's color.
pub struct TextTooltip<'a> {
    /// The text.
    pub text: &'a str,
    /// The widest the text gets before it wraps (the content's `max-width`).
    pub max_width: f32,
    /// Show the `Learn more` line.
    pub learn_more: bool,
}

impl TextTooltip<'_> {
    /// Paints it with `painter` (its layer and clip) centred under `anchor`, its box's top `gap`
    /// px below the anchor's bottom, moved sideways as needed to stay `within`. Returns whether
    /// `Learn more` was clicked.
    pub fn show_open(
        &self,
        ui: &Ui,
        painter: &Painter,
        id: Id,
        anchor: Rect,
        gap: f32,
        within: Rangef,
    ) -> bool {
        self.paint_below(ui, painter, id, anchor, gap, within).1
    }

    /// [`show_open`](Self::show_open) while `anchor` is hovered, on the module's hover timing.
    pub fn show_below(&self, ui: &Ui, anchor: &Response, gap: f32, within: Rangef) {
        let Some(shown) = hover(ui, anchor, false, INTERVAL) else {
            return;
        };
        let mut painter = ui.ctx().layer_painter(egui::LayerId::new(
            Order::Tooltip,
            anchor.id.with("exo_ui::tooltip"),
        ));
        painter.multiply_opacity(shown.opacity);
        let (rect, _) = self.paint_below(
            ui,
            &painter,
            anchor.id.with("exo_ui::tooltip"),
            anchor.rect,
            gap,
            within,
        );
        shown.painted(ui, rect, false);
    }

    /// Paints it under `anchor` and returns its box and whether `Learn more` was clicked.
    fn paint_below(
        &self,
        ui: &Ui,
        painter: &Painter,
        id: Id,
        anchor: Rect,
        gap: f32,
        within: Rangef,
    ) -> (Rect, bool) {
        let ctx = ui.ctx();
        let color = text_color(ctx);
        let mut job = LayoutJob::single_section(self.text.to_owned(), TEXT.format(color));
        job.wrap = egui::text::TextWrapping::wrap_at_width(self.max_width);
        let galley = crate::text::layout(ctx, job);
        let link = self.learn_more.then(|| {
            let mut job = LayoutJob::default();
            job.append(
                "Learn more",
                0.0,
                egui::TextFormat {
                    underline: egui::Stroke::new(1.0, color),
                    ..TEXT.format(color)
                },
            );
            job.append(".", 0.0, TEXT.format(color));
            let width = crate::text::layout(
                ctx,
                LayoutJob::single_section("Learn more".to_owned(), TEXT.format(color)),
            )
            .size()
            .x;
            (job, width)
        });
        let rows = galley.rows.len().max(1) as f32 + f32::from(u8::from(link.is_some()));
        let link_width = link.as_ref().map_or(0.0, |(job, _)| {
            crate::text::layout(ctx, job.clone()).size().x
        });
        // Shrink-to-fit, as the inline block's width is: text that wraps fills `max_width`.
        let unwrapped = crate::text::layout(
            ctx,
            LayoutJob::single_section(self.text.to_owned(), TEXT.format(color)),
        )
        .size()
        .x;
        let width = unwrapped.min(self.max_width).max(link_width);
        let size = Vec2::new(
            width + 2.0 * PADDING_X,
            PADDING_TOP + rows * TEXT.line_height + PADDING_BOTTOM,
        );
        let left = (anchor.center().x - size.x / 2.0)
            .min(within.max - size.x)
            .max(within.min);
        let rect = Rect::from_min_size(Pos2::new(left, anchor.bottom() + gap), size);
        paint_box(painter, rect, true);
        let text_left_top = rect.min + Vec2::new(PADDING_X, PADDING_TOP);
        crate::text::paint_text(painter, text_left_top, galley, &TEXT);
        let Some((job, learn_width)) = link else {
            return (rect, false);
        };
        let line_top = text_left_top + Vec2::Y * (rows - 1.0) * TEXT.line_height;
        paint_job(painter, line_top, job, &TEXT);
        let target = Rect::from_min_size(line_top, Vec2::new(learn_width, TEXT.line_height));
        let response = ui
            .interact(target, id.with("learn_more"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        (rect, response.clicked())
    }

    /// Shows the text (no `Learn more`) while `anchor` is hovered, left of it: the box's right
    /// edge `gap` px before the anchor's left and centred on it vertically, as the settings menu's
    /// toggles show theirs.
    pub fn show_left(&self, ui: &Ui, anchor: &Response, gap: f32) {
        let Some(shown) = hover(ui, anchor, false, INTERVAL) else {
            return;
        };
        let ctx = ui.ctx();
        let mut painter = ctx.layer_painter(egui::LayerId::new(
            Order::Tooltip,
            anchor.id.with("exo_ui::tooltip"),
        ));
        painter.multiply_opacity(shown.opacity);
        let color = text_color(ctx);
        let mut job = LayoutJob::single_section(self.text.to_owned(), TEXT.format(color));
        job.wrap = egui::text::TextWrapping::wrap_at_width(self.max_width);
        let galley = crate::text::layout(ctx, job);
        // Shrink-to-fit: text that wraps fills `max_width`.
        let unwrapped = crate::text::layout(
            ctx,
            LayoutJob::single_section(self.text.to_owned(), TEXT.format(color)),
        )
        .size()
        .x;
        let rows = galley.rows.len().max(1) as f32;
        let size = Vec2::new(
            unwrapped.min(self.max_width) + 2.0 * PADDING_X,
            PADDING_TOP + rows * TEXT.line_height + PADDING_BOTTOM,
        );
        let rect = Rect::from_min_size(
            Pos2::new(
                anchor.rect.left() - gap - size.x,
                anchor.rect.center().y - size.y / 2.0,
            ),
            size,
        );
        shown.painted(ui, rect, true);
        paint_box(&painter, rect, true);
        // Chromium puts the box on whole pixels, and the text's lines with it; across, the text
        // keeps its fraction (measured: a box at y 812.5).
        let text = Pos2::new(rect.min.x + PADDING_X, pixel_snap(rect).min.y + PADDING_TOP);
        crate::text::paint_text(&painter, text, galley, &TEXT);
    }
}

/// A tooltip of paragraphs whose lines mix text and keycaps, shown beside its anchor as
/// [`TextTooltip::show_left`] shows text: the settings menu's `Backtick as Escape`. Its lines don't
/// wrap; the box fits the widest.
pub struct LinesTooltip<'a> {
    /// Its paragraphs (`p`, `padding-bottom: 8px` but the last's), each lines (`br`) of text and
    /// keycaps. Text keeps its spaces; caps are 4px apart (`margin-left: 4px`).
    pub paragraphs: &'a [&'a [&'a [TooltipPart<'a>]]],
}

impl LinesTooltip<'_> {
    /// Between paragraphs.
    const PARAGRAPH_GAP: f32 = 8.0;

    /// Shows it while `anchor` is hovered, left of it: the box's right edge `gap` px before the
    /// anchor's left and centred on it vertically, as [`TextTooltip::show_left`].
    pub fn show_left(&self, ui: &Ui, anchor: &Response, gap: f32) {
        let Some(shown) = hover(ui, anchor, false, INTERVAL) else {
            return;
        };
        let ctx = ui.ctx();
        let mut painter = ctx.layer_painter(egui::LayerId::new(
            Order::Tooltip,
            anchor.id.with("exo_ui::tooltip"),
        ));
        painter.multiply_opacity(shown.opacity);
        let paragraphs: Vec<Vec<Line>> = self
            .paragraphs
            .iter()
            .map(|lines| {
                lines
                    .iter()
                    .map(|parts| Line::new(ctx, parts, 0.0, 0.0))
                    .collect()
            })
            .collect();
        let height = line_height;
        let width = paragraphs
            .iter()
            .flatten()
            .map(|line| line.width)
            .fold(0.0, f32::max);
        let content = paragraphs
            .iter()
            .map(|lines| lines.iter().map(height).sum::<f32>())
            .sum::<f32>()
            + paragraphs.len().saturating_sub(1) as f32 * Self::PARAGRAPH_GAP;
        let size = Vec2::new(
            width + 2.0 * PADDING_X,
            PADDING_TOP + content + PADDING_BOTTOM,
        );
        let rect = Rect::from_min_size(
            Pos2::new(
                anchor.rect.left() - gap - size.x,
                anchor.rect.center().y - size.y / 2.0,
            ),
            size,
        );
        shown.painted(ui, rect, true);
        paint_box(&painter, rect, true);
        // As [`TextTooltip::show_left`]'s: the lines on the box's whole pixels down, their own
        // fraction across.
        let mut top = pixel_snap(rect).min.y + PADDING_TOP;
        for lines in &paragraphs {
            for line in lines {
                line.paint(&painter, Pos2::new(rect.min.x + PADDING_X, top));
                top += height(line);
            }
            top += Self::PARAGRAPH_GAP;
        }
    }
}

/// A tooltip of one line of text and keycaps that its caller places and shows for as long as
/// it likes, such as the `Quote` `enter` · `R` · `F` under selected text.
pub struct LineTooltip<'a> {
    /// The line, nothing between its parts but what they say (a cap's own 4px before it).
    pub parts: &'a [TooltipPart<'a>],
}

impl LineTooltip<'_> {
    /// The box's size: a line with keycaps is a shortcut row's 21px, text alone 20px.
    pub fn size(&self, ctx: &Context) -> Vec2 {
        let line = Line::new(ctx, self.parts, 0.0, 0.0);
        Vec2::new(
            line.width + 2.0 * PADDING_X,
            PADDING_TOP + line_height(&line) + PADDING_BOTTOM,
        )
    }

    /// Paints it with its box's top left at `min`, on `painter`'s layer.
    pub fn paint(&self, painter: &Painter, min: Pos2) {
        let line = Line::new(painter.ctx(), self.parts, 0.0, 0.0);
        let size = Vec2::new(
            line.width + 2.0 * PADDING_X,
            PADDING_TOP + line_height(&line) + PADDING_BOTTOM,
        );
        paint_box(painter, Rect::from_min_size(min, size), true);
        line.paint(painter, min + Vec2::new(PADDING_X, PADDING_TOP));
    }
}

/// A line's height: 21px with keycaps (a shortcut's flex row), else a 20px text line.
fn line_height(line: &Line) -> f32 {
    if line
        .pieces
        .iter()
        .any(|piece| matches!(piece, Piece::Keys { .. }))
    {
        ROW_HEIGHT
    } else {
        TEXT.line_height
    }
}

/// A hover tooltip between frames.
#[derive(Clone)]
struct Hover {
    intent: HoverIntent<()>,
    /// Since when it shows, if it does.
    shown: Option<f64>,
    /// When the pointer left the anchor while it showed.
    left: Option<f64>,
    /// The tooltip with its padding, as last painted: the pointer on it keeps it.
    over: Rect,
    /// Whether the keyboard was on the anchor: losing it hides the tooltip at once.
    focused: bool,
    /// The pass that last drew the anchor: an anchor that went unseen starts over.
    pass: u64,
}

/// A hover tooltip that shows this frame, at `opacity`.
struct Shown {
    id: Id,
    opacity: f32,
}

impl Shown {
    /// Remembers that the tooltip's box is at `rect`, its padding on its left and right when it's
    /// `beside` its anchor, else above and below it.
    fn painted(&self, ui: &Ui, rect: Rect, beside: bool) {
        let padding = if beside {
            Vec2::new(PLACEMENT_PADDING, 0.0)
        } else {
            Vec2::new(0.0, PLACEMENT_PADDING)
        };
        ui.data_mut(|data| {
            if let Some(mut hover) = data.get_temp::<Hover>(self.id) {
                hover.over = rect.expand2(padding);
                data.insert_temp(self.id, hover);
            }
        });
    }
}

/// A tooltip its caller draws, on the same timing as the module's own: a frame [`hover_tooltip`]
/// shows it in.
pub struct HoverTooltip(Shown);

impl HoverTooltip {
    /// How visible it is, fading in over 200ms.
    pub fn opacity(&self) -> f32 {
        self.0.opacity
    }

    /// Says where its box was painted, above or below its anchor: the pointer on it (or on the 8px
    /// about it) keeps it.
    pub fn painted(&self, ui: &Ui, rect: Rect) {
        self.0.painted(ui, rect, false);
    }
}

/// Whether a tooltip of the caller's own on `anchor` shows this frame: once hover intent has
/// the pointer settled on it, until 150ms after the pointer has left it and the tooltip.
pub fn hover_tooltip(ui: &Ui, anchor: &Response) -> Option<HoverTooltip> {
    hover(ui, anchor, false, INTERVAL).map(HoverTooltip)
}

/// Whether a hover tooltip on `anchor` shows this frame, and how visibly: from when hover
/// intent, polling every `interval`, has the pointer settled on the anchor, or at once while
/// it's `focused`, fading in over 200ms; until the pointer has been off the anchor for 150ms and
/// isn't on the tooltip, or at once when the anchor loses the keyboard.
fn hover(ui: &Ui, anchor: &Response, focused: bool, interval: f64) -> Option<Shown> {
    let ctx = ui.ctx();
    let id = anchor.id.with("exo_ui::tooltip_hover");
    let (now, pointer) = ui.input(|input| (input.time, input.pointer.latest_pos()));
    let pass = ctx.cumulative_pass_nr();
    let mut hover = ui
        .data(|data| data.get_temp::<Hover>(id))
        .filter(|hover| hover.pass + 1 >= pass)
        .unwrap_or_else(|| Hover {
            intent: HoverIntent::new(interval),
            shown: None,
            left: None,
            over: Rect::NOTHING,
            focused: false,
            pass,
        });
    let on_anchor = anchor.hovered();
    if hover.intent.update(ctx, on_anchor.then_some(())).is_some() || focused {
        hover.shown.get_or_insert(now);
    }
    if on_anchor || focused {
        hover.left = None;
    } else if hover.focused {
        hover.shown = None;
    } else if hover.shown.is_some() {
        let left = *hover.left.get_or_insert(now);
        let on_tooltip = pointer.is_some_and(|at| hover.over.contains(at));
        if !on_tooltip && now - left >= HIDE_AFTER {
            hover.shown = None;
        } else if !on_tooltip {
            ctx.request_repaint_after_secs((HIDE_AFTER - (now - left)) as f32);
        }
    }
    if hover.shown.is_none() {
        hover.left = None;
    }
    (hover.focused, hover.pass) = (focused, pass);
    let since = hover.shown;
    ui.data_mut(|data| data.insert_temp(id, hover));
    let fade = ((now - since?) / 0.2).min(1.0) as f32;
    if fade < 1.0 {
        ctx.request_repaint();
    }
    Some(Shown {
        id,
        opacity: motion::CubicBezier::EASE.at(fade),
    })
}

/// A tooltip open until its close icon dismisses it: a tooltip box `width` wide, padded
/// `8px 9px 8px 12px`, its title (12px bold on a 16px line, `top: 1px`) and close icon
/// (`DEPRECATED_closeV2`, 10px, white at 30% and 60% hovered, Carbon's black) over content that
/// wraps 4px under them. The content is drawn as Share Availability's, the one Exo shows: 12px bold
/// on 20px lines `top: 3px`, its keycaps 1px lower still and 4px clear of the text after them. It
/// fades in over 350ms, the first 43% of them unseen.
pub struct DismissableTooltip<'a> {
    pub title: &'a str,
    pub content: &'a [TooltipPart<'a>],
    /// The box's width, padding included.
    pub width: f32,
}

/// What a click on a [`DismissableTooltip`] hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DismissableClick {
    /// The close icon.
    Close,
    /// The rest of it.
    Body,
}

impl DismissableTooltip<'_> {
    const PADDING_TOP: f32 = 8.0;
    const PADDING_RIGHT: f32 = 9.0;
    const PADDING_BOTTOM: f32 = 8.0;
    const PADDING_LEFT: f32 = 12.0;
    const TITLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 16.0);
    const TITLE_TOP: f32 = 1.0;
    const KEY_PADDING_TOP: f32 = 1.0;
    const CONTENT_GAP: f32 = 4.0;
    const TEXT_TOP: f32 = 3.0;
    const KEYS_TOP: f32 = 1.0;
    const AFTER_KEYS: f32 = 4.0;
    const CLOSE: f32 = 10.0;
    /// The close icon's span sits 2px up (`margin-top: -2px`); its icon 3px under that.
    const CLOSE_TOP: f32 = -2.0;
    const CLOSE_ICON_TOP: f32 = 1.0;
    const FADE: f64 = 0.35;
    const UNSEEN: f64 = 0.43;

    /// Paints it at `placement` from `anchor` (above or below, centred, kept `within`), and
    /// reports a click on it. It fades in from the first frame it's shown in a row.
    pub fn show(
        &self,
        ui: &Ui,
        id: Id,
        anchor: Rect,
        placement: Placement,
        within: Rangef,
    ) -> Option<DismissableClick> {
        let ctx = ui.ctx();
        let (now, pass) = (ctx.input(|input| input.time), ctx.cumulative_pass_nr());
        let since = ctx.data_mut(|data| {
            let shown = data.get_temp_mut_or_insert_with(id, || (now, pass));
            if shown.1 + 1 < pass {
                shown.0 = now;
            }
            shown.1 = pass;
            shown.0
        });
        let t = (now - since) / Self::FADE;
        let opacity = if t < Self::UNSEEN {
            0.0
        } else {
            motion::CubicBezier::EASE
                .at(((t - Self::UNSEEN) / (1.0 - Self::UNSEEN)).min(1.0) as f32)
        };
        if t < 1.0 {
            ctx.request_repaint();
        }
        let inner = self.width - Self::PADDING_LEFT - Self::PADDING_RIGHT;
        let lines = Self::lines(ctx, self.content, inner);
        let content = lines.iter().map(line_height).sum::<f32>();
        let size = Vec2::new(
            self.width,
            Self::PADDING_TOP
                + Self::TITLE.line_height
                + Self::CONTENT_GAP
                + content
                + Self::PADDING_BOTTOM,
        );
        let centred = (anchor.center().x - size.x / 2.0)
            .min(within.max - size.x)
            .max(within.min);
        let top = match placement {
            Placement::Above(overlap) => anchor.top() + overlap - size.y,
            Placement::Below { gap, .. } | Placement::BelowFrom { gap, .. } => {
                anchor.bottom() + gap
            }
            Placement::Right(_)
            | Placement::Left(_)
            | Placement::Beside(_)
            | Placement::RightOrLeft { .. } => anchor.top(),
        };
        let rect = Rect::from_min_size(Pos2::new(centred, top), size);
        Area::new(id)
            .order(Order::Tooltip)
            .fixed_pos(rect.min)
            .constrain(false)
            .show(ctx, |ui| {
                ui.set_opacity(opacity);
                let body = ui.allocate_rect(rect, Sense::click());
                let painter = ui.painter();
                paint_box(painter, rect, true);
                let color = text_color(ctx);
                // Chromium puts the box on whole pixels, and its lines with it; across, they keep
                // their fraction (as `TextTooltip::show_left`'s).
                let content_left_top = Pos2::new(
                    rect.min.x + Self::PADDING_LEFT,
                    pixel_snap(rect).min.y + Self::PADDING_TOP,
                );
                Self::TITLE.paint_line(
                    painter,
                    content_left_top + Vec2::Y * Self::TITLE_TOP,
                    self.title,
                    color,
                    None,
                );
                let close = Rect::from_min_size(
                    Pos2::new(
                        rect.right() - Self::PADDING_RIGHT - Self::CLOSE,
                        content_left_top.y + Self::CLOSE_TOP,
                    ),
                    Vec2::splat(Self::CLOSE),
                );
                let hit = ui
                    .interact(close, id.with("close"), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                let (rest, hovered) = match crate::theme(ctx) {
                    crate::Theme::Snow => (
                        Color32::from_white_alpha(77),
                        Color32::from_white_alpha(153),
                    ),
                    crate::Theme::Carbon => (
                        Color32::from_black_alpha(77),
                        Color32::from_black_alpha(153),
                    ),
                };
                let icon = "legacy/DEPRECATED_closeV2";
                crate::paint_icon(
                    ui,
                    icon,
                    close.translate(Vec2::Y * (Self::CLOSE_ICON_TOP - Self::CLOSE_TOP)),
                    if hit.hovered() { hovered } else { rest },
                );
                let mut top = content_left_top.y + Self::TITLE.line_height + Self::CONTENT_GAP;
                for line in &lines {
                    // A cap is an inline block on its label's baseline, `padding-top: 1px` over a line
                    // of the text's own 20px: its line's baseline (and text) sits 1px lower.
                    let keys = line
                        .pieces
                        .iter()
                        .any(|piece| matches!(piece, Piece::Keys { .. }));
                    let text_top =
                        top + Self::TEXT_TOP + if keys { Self::KEY_PADDING_TOP } else { 0.0 };
                    for piece in &line.pieces {
                        match piece {
                            Piece::Text { job, left, .. } => {
                                paint_job(
                                    painter,
                                    Pos2::new(content_left_top.x + left, text_top),
                                    job.clone(),
                                    &TEXT,
                                );
                            }
                            Piece::Keys { keycaps, left } => keycaps.paint(
                                painter,
                                content_left_top.x + left,
                                top + Self::TEXT_TOP + Self::KEYS_TOP,
                            ),
                        }
                    }
                    top += line_height(line);
                }
                if hit.clicked() {
                    Some(DismissableClick::Close)
                } else {
                    body.clicked().then_some(DismissableClick::Body)
                }
            })
            .inner
    }

    /// The content broken into lines no wider than `width`, as Chromium breaks inline content:
    /// after a word's spaces, and on either side of a run of keycaps.
    fn lines(ctx: &Context, parts: &[TooltipPart<'_>], width: f32) -> Vec<Line> {
        let color = text_color(ctx);
        let mut lines = Vec::new();
        let mut line = Line {
            pieces: Vec::new(),
            width: 0.0,
        };
        // The words since the last keycaps, and where they start.
        let (mut run, mut run_left) = (String::new(), 0.0);
        let flush = |line: &mut Line, run: &mut String, left: f32| {
            if !run.is_empty() {
                line.width = left + TEXT.width(ctx, run);
                line.pieces.push(Piece::Text {
                    job: LayoutJob::single_section(std::mem::take(run), TEXT.format(color)),
                    left,
                    opacity: 1.0,
                });
            }
        };
        for part in parts {
            match part {
                TooltipPart::Text(text) | TooltipPart::Separator(text) => {
                    for word in text.split_inclusive(' ') {
                        let wider = format!("{run}{word}");
                        if !(line.pieces.is_empty() && run.is_empty())
                            && run_left + TEXT.width(ctx, wider.trim_end()) > width
                        {
                            flush(&mut line, &mut run, run_left);
                            lines.push(std::mem::replace(
                                &mut line,
                                Line {
                                    pieces: Vec::new(),
                                    width: 0.0,
                                },
                            ));
                            (run, run_left) = (word.to_owned(), 0.0);
                        } else {
                            run = wider;
                        }
                    }
                }
                TooltipPart::Keys(tokens) => {
                    flush(&mut line, &mut run, run_left);
                    let keycaps = Keycaps::new(ctx, tokens, keycap_colors(ctx));
                    if !line.pieces.is_empty() && line.width + keycaps.width() > width {
                        lines.push(std::mem::replace(
                            &mut line,
                            Line {
                                pieces: Vec::new(),
                                width: 0.0,
                            },
                        ));
                    }
                    let left = line.width;
                    line.width = left + keycaps.width();
                    run_left = line.width + Self::AFTER_KEYS;
                    line.pieces.push(Piece::Keys { keycaps, left });
                }
                TooltipPart::Gap(gap) => run_left += gap,
            }
        }
        flush(&mut line, &mut run, run_left);
        lines.push(line);
        lines
    }
}

/// Compose's tooltip of text around keycaps, open for as long as it's shown:
/// `Hit ⌘ enter to Schedule` while a draft with a Send later time has the keyboard. Its keys are
/// compose's: 3px apart and 3px after the last, 1px down, white at 30% (Carbon `#555b64`) under 90%
/// white labels.
pub struct ComposeKeysTooltip<'a> {
    /// Before the keys: `Hit`.
    pub before: &'a str,
    pub keys: &'a [&'a str],
    /// After them: ` to Schedule`.
    pub after: &'a str,
}

impl ComposeKeysTooltip<'_> {
    /// Paints the tooltip centred under `anchor`, its box's top `gap` px below the anchor's
    /// bottom and kept `within`, its keycaps at `keys_opacity` (compose's breathe).
    pub fn show_open(
        &self,
        ui: &Ui,
        id: Id,
        anchor: Rect,
        gap: f32,
        within: Rangef,
        keys_opacity: f32,
    ) {
        const KEY_GAP: f32 = 3.0;
        let ctx = ui.ctx();
        let painter = ctx.layer_painter(egui::LayerId::new(Order::Tooltip, id));
        let color = text_color(ctx);
        let text = |text: &str| {
            let job = LayoutJob::single_section(text.to_owned(), TEXT.format(color));
            let width = ctx
                .fonts_mut(|fonts| fonts.layout_job(job.clone()))
                .size()
                .x;
            (job, width)
        };
        let background = match crate::theme(ctx) {
            crate::Theme::Snow => Color32::from_white_alpha(77),
            crate::Theme::Carbon => Color32::from_rgb(0x55, 0x5b, 0x64),
        };
        let white = Color32::from_white_alpha(230);
        let keycaps = Keycaps::new(
            ctx,
            self.keys,
            KeycapColors {
                background,
                text: white,
                divider: white,
            },
        )
        .gap(KEY_GAP);
        let ((before, before_width), (after, after_width)) = (text(self.before), text(self.after));
        let width = before_width + keycaps.width() + KEY_GAP + after_width;
        let size = Vec2::new(
            width + 2.0 * PADDING_X,
            PADDING_TOP + ROW_HEIGHT + PADDING_BOTTOM,
        );
        let x = (anchor.center().x - size.x / 2.0)
            .min(within.max - size.x)
            .max(within.min);
        let rect = Rect::from_min_size(Pos2::new(x, anchor.bottom() + gap), size);
        paint_box(&painter, rect, true);
        let line = rect.min + Vec2::new(PADDING_X, PADDING_TOP);
        paint_job(&painter, line, before, &TEXT);
        let mut keys = painter.clone();
        keys.multiply_opacity(keys_opacity);
        keycaps.paint(&keys, line.x + before_width, line.y + 1.0);
        paint_job(
            &painter,
            line + Vec2::X * (before_width + keycaps.width() + KEY_GAP),
            after,
            &TEXT,
        );
    }
}

/// The hint tooltip: a single line of coaching with a close button, centred at the bottom of the
/// window.
pub struct HintTooltip<'a> {
    /// The line: `Next time, hit`, `["z"]`, `to undo instantly!`.
    pub parts: &'a [TooltipPart<'a>],
}

impl HintTooltip<'_> {
    /// Paints the hint centred in `area` (the window below the tab bar), `bottom` px above its
    /// bottom edge (18px normally), at `opacity`, and reports a click on it (which closes a hint).
    pub fn show(&self, ctx: &Context, id: Id, area: Rect, bottom: f32, opacity: f32) -> Response {
        let line = Line::new(ctx, self.parts, 4.0, 0.0);
        let size = Vec2::new(
            PADDING_X + line.width + CLOSE_GAP + CLOSE_SIZE.x + PADDING_X,
            PADDING_TOP + TEXT.line_height + PADDING_BOTTOM,
        );
        // The tooltip's 8px padding sits between the box and `bottom`.
        let rect = Rect::from_min_size(
            Pos2::new(
                area.center().x - size.x / 2.0,
                area.bottom() - bottom - 8.0 - size.y,
            ),
            size,
        );
        Area::new(id)
            .order(Order::Tooltip)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                let response = ui.allocate_rect(rect, Sense::click());
                let mut painter = ui.painter().clone();
                painter.multiply_opacity(opacity);
                paint_box(&painter, rect, true);
                let line_top = rect.min + Vec2::new(PADDING_X, PADDING_TOP);
                line.paint(&painter, line_top);
                let close = Rect::from_min_size(
                    Pos2::new(line_top.x + line.width + CLOSE_GAP, line_top.y + 4.0 - 1.0),
                    CLOSE_SIZE,
                );
                // The close button pads the icon by 20px and shows it at 40%, 60% hovered.
                let near = response
                    .hover_pos()
                    .is_some_and(|pos| close.expand(20.0).contains(pos));
                let opacity = if near { 0.6 } else { 0.4 };
                let icon = match crate::theme(ctx) {
                    crate::Theme::Snow => Color32::from_rgb(0xeb, 0xee, 0xf2),
                    crate::Theme::Carbon => Color32::from_black_alpha(230),
                };
                paint_close_mask(&painter, close, icon.gamma_multiply(opacity));
                response
            })
            .inner
    }
}

/// The hint's close icon: two diagonal bars from a 12×13 SVG, drawn as a mask at 85% size.
fn paint_close_mask(painter: &Painter, rect: Rect, color: Color32) {
    let scale = 0.85;
    let point = |x: f32, y: f32| rect.min + Vec2::new(x, y) * scale;
    for bar in [
        [
            (0.697, 0.49),
            (12.01, 11.803),
            (11.303, 12.51),
            (-0.01, 1.197),
        ],
        [
            (-0.01, 11.803),
            (11.303, 0.49),
            (12.01, 1.197),
            (0.697, 12.51),
        ],
    ] {
        let points = bar.iter().map(|&(x, y)| point(x, y)).collect();
        painter.add(Shape::convex_polygon(points, color, egui::Stroke::NONE));
    }
}

#[cfg(test)]
mod tests {
    use egui::{Event, RawInput, pos2};

    use super::*;

    const ANCHOR: Rect = Rect {
        min: pos2(100.0, 100.0),
        max: pos2(132.0, 132.0),
    };
    /// The tooltip's box, 8px under the anchor.
    const BOX: Rect = Rect {
        min: pos2(80.0, 140.0),
        max: pos2(152.0, 170.0),
    };
    const ON: Pos2 = pos2(116.0, 116.0);
    const OFF: Pos2 = pos2(10.0, 10.0);

    /// Runs a frame at `time` with the pointer at `at`, the anchor drawn (unless `!drawn`) and
    /// `focused`; returns whether its tooltip shows.
    fn frame(ctx: &Context, time: f64, at: Pos2, focused: bool, drawn: bool) -> bool {
        let input = RawInput {
            time: Some(time),
            events: vec![Event::PointerMoved(at)],
            ..Default::default()
        };
        let mut shows = false;
        ctx.run_ui(input, |ui| {
            if !drawn {
                return;
            }
            let anchor = ui.interact(ANCHOR, Id::new("anchor"), Sense::hover());
            if let Some(shown) = hover(ui, &anchor, focused, INTERVAL) {
                shown.painted(ui, BOX, false);
                shows = true;
            }
        })
        .drop_without_applying_deltas();
        shows
    }

    /// A context whose pointer came onto the anchor at 1s and rested there until it showed.
    fn shown() -> Context {
        let ctx = Context::default();
        frame(&ctx, 0.0, OFF, false, true);
        frame(&ctx, 1.0, ON, false, true);
        assert!(frame(&ctx, 1.1, ON, false, true));
        ctx
    }

    #[test]
    fn shows_once_the_pointer_settles_on_the_anchor() {
        let ctx = Context::default();
        frame(&ctx, 0.0, OFF, false, true);
        assert!(!frame(&ctx, 1.0, ON, false, true), "just arrived");
        assert!(!frame(&ctx, 1.06, ON, false, true), "before the first poll");
        assert!(frame(&ctx, 1.1, ON, false, true), "still at the first poll");
    }

    #[test]
    fn a_pointer_sweeping_across_never_settles() {
        let ctx = Context::default();
        frame(&ctx, 0.0, OFF, false, true);
        // 5px every 50ms: 10px between polls.
        for step in 0..6 {
            let at = pos2(101.0 + 5.0 * step as f32, 116.0);
            assert!(
                !frame(&ctx, 1.0 + 0.05 * f64::from(step), at, false, true),
                "step {step}"
            );
        }
        let rest = pos2(126.0, 116.0);
        assert!(
            !frame(&ctx, 1.3, rest, false, true),
            "moved since the last poll"
        );
        assert!(frame(&ctx, 1.4, rest, false, true), "rested a poll");
    }

    #[test]
    fn stays_150ms_after_the_pointer_leaves() {
        let ctx = shown();
        assert!(frame(&ctx, 2.0, OFF, false, true));
        assert!(frame(&ctx, 2.14, OFF, false, true));
        assert!(!frame(&ctx, 2.16, OFF, false, true));
        assert!(
            !frame(&ctx, 2.2, ON, false, true),
            "back on the anchor, it settles again first"
        );
    }

    #[test]
    fn stays_while_the_pointer_is_on_the_tooltip() {
        let ctx = shown();
        // The 8px between the anchor and the box is the tooltip's padding.
        assert!(frame(&ctx, 2.0, pos2(116.0, 135.0), false, true));
        assert!(
            frame(&ctx, 2.5, BOX.center(), false, true),
            "long after leaving the anchor"
        );
        assert!(
            !frame(&ctx, 2.6, OFF, false, true),
            "off it, the 150ms are long over"
        );
    }

    #[test]
    fn the_keyboard_shows_it_at_once_and_takes_it_away_at_once() {
        let ctx = Context::default();
        assert!(frame(&ctx, 0.0, OFF, true, true));
        assert!(!frame(&ctx, 0.05, OFF, false, true));
    }

    #[test]
    fn an_anchor_that_went_unseen_starts_over() {
        let ctx = shown();
        frame(&ctx, 2.0, ON, false, false);
        assert!(!frame(&ctx, 5.0, ON, false, true));
        assert!(
            !frame(&ctx, 5.05, ON, false, true),
            "the anchor is hovered from its second frame"
        );
        assert!(frame(&ctx, 5.15, ON, false, true));
    }
}
