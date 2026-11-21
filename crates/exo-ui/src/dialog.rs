//! The dialog shell every settings and feature dialog uses: a 550px card centered over a dimmed
//! backdrop, with its header at the top left and a close × at the top right. The overlay fades in
//! and out over 200ms (`opacity 200ms ease`).
//!
//! `padding: 20px 32px`, a 20px header row (16px/20px text nudged down 2px), 4px corners and
//! `box-shadow: 0 2px 0 rgba(0,0,0,.1)`; the × is 12×13 at 40% opacity, 24px from the top and 32px
//! from the right. A dialog whose close goes back a step shows the 13px back arrow there instead.
//!
//! Carbon's dialogs are light cards (`#f5f5f5`) that keep Snow's colors, so a dialog draws its
//! contents with Snow's palette in either theme ([`crate::with_theme`]).

use egui::{
    Area, Color32, CornerRadius, Id, Order, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2,
    vec2,
};

use crate::{BoxShadow, Family, Font, TextStyle, Theme, motion, pixel_snap, theme, with_theme};

/// A dialog's width.
pub const DIALOG_WIDTH: f32 = 550.0;
const PADDING_X: f32 = 32.0;
const PADDING_Y: f32 = 20.0;
/// The header row: the header's 20px line box.
const HEADER_HEIGHT: f32 = 20.0;
const HEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 16.0, 20.0);
/// The header text's nudge down (`position: relative; top: 2px`).
const HEADER_NUDGE: f32 = 2.0;
/// The card's corners.
const RADIUS: u8 = 4;
const SHADOW: BoxShadow = BoxShadow::new(
    [0.0, 2.0],
    0.0,
    0.0,
    Color32::from_rgba_premultiplied(0, 0, 0, 26),
);
/// The ×: 12×13, `opacity: .4`.
const CLOSE_SIZE: [f32; 2] = [12.0, 13.0];
/// The back arrow: 13px square, black at `opacity: .4`.
const BACK_SIZE: f32 = 13.0;
const BACK_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 102);
/// The close control's inset from the card's top right.
const CLOSE_INSET: [f32; 2] = [32.0, 24.0];
/// A filling dialog's `min-width: 510px`.
const FILL_MIN_WIDTH: f32 = 510.0;
/// The header's `rgba(0, 0, 0, .85)`.
const HEADER_INK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 217);
/// The backdrop's `rgba(0,0,0,.5)`.
const BACKDROP: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 128);
/// The overlay's fade (`transition: opacity 200ms ease`).
const FADE: f32 = 0.2;

/// A dialog: its header, and how tall the body under it is.
#[must_use]
pub struct Dialog<'a> {
    id: Id,
    header: &'a str,
    header_style: TextStyle,
    header_ink: Color32,
    body_height: f32,
    top: bool,
    back: bool,
    fill: Option<(f32, egui::Vec2)>,
}

/// What a frame of a dialog came to.
pub struct DialogResponse<R> {
    /// What the body returned; `None` once the dialog has faded out, when its owner can drop it.
    pub inner: Option<R>,
    /// The backdrop or the × was clicked.
    pub close: bool,
}

impl<'a> Dialog<'a> {
    /// A dialog titled `header` whose body is `body_height` px tall, its margins included.
    pub fn new(
        id: impl std::hash::Hash + std::fmt::Debug,
        header: &'a str,
        body_height: f32,
    ) -> Self {
        Self {
            id: Id::new(("exo_ui::dialog", id)),
            header,
            header_style: HEADER,
            header_ink: HEADER_INK,
            body_height,
            top: false,
            back: false,
            fill: None,
        }
    }

    /// Fills the area less `margin` on each side, at most `max_width` (and at least 510px) wide,
    /// with no header or padding: the body is the whole card. Its back arrow sits 36px down and
    /// 44px in, in an 8px-padded box (the split library's dialog).
    pub fn fill(self, max_width: f32, margin: egui::Vec2) -> Self {
        Self {
            fill: Some((max_width, margin)),
            back: true,
            ..self
        }
    }

    /// Sits 16% of the window's height from the top of the window rather than centered, at
    /// most 30% of the window's height shorter than the area (the split inbox dialogs'
    /// `margin-top: 16vh; margin-bottom: auto; max-height: calc(100% - 30vh)`).
    pub fn top(self) -> Self {
        Self { top: true, ..self }
    }

    /// Its header in `ink` rather than `rgba(0, 0, 0, .85)`, where a screen's rule says so
    /// (Bulk Actions' with its table, `.95`).
    pub fn header_ink(self, ink: Color32) -> Self {
        Self {
            header_ink: ink,
            ..self
        }
    }

    /// Its header in `style` and `ink`, for a dialog drawn in another style's theme (Out of
    /// Office Reply's, whose header is in [`Family::SansAlt`]).
    pub fn header_style(self, style: TextStyle, ink: Color32) -> Self {
        Self {
            header_style: style,
            header_ink: ink,
            ..self
        }
    }

    /// Its close control goes back a step: a back arrow instead of the ×.
    pub fn back(self) -> Self {
        Self { back: true, ..self }
    }

    /// Draws the dialog over `area` (the window below the tab strip) while `open`, and while it
    /// fades out after. `body` paints into the body's rect, the content width below the header.
    pub fn show<R>(
        mut self,
        ctx: &egui::Context,
        area: Rect,
        open: bool,
        body: impl FnOnce(&mut Ui, Rect) -> R,
    ) -> DialogResponse<R> {
        self.id = crate::id(ctx, self.id);
        let opacity = motion::transition(
            ctx,
            self.id.with("fade"),
            open,
            FADE,
            motion::CubicBezier::EASE,
        );
        if !open && opacity <= 0.0 {
            return DialogResponse {
                inner: None,
                close: false,
            };
        }
        Area::new(self.id)
            .order(Order::Foreground)
            .fixed_pos(area.min)
            .show(ctx, |ui| {
                ui.set_opacity(opacity);
                let (inner, close) = self.draw(ui, area, body);
                DialogResponse {
                    inner: Some(inner),
                    close,
                }
            })
            .inner
    }

    fn draw<R>(&self, ui: &mut Ui, area: Rect, body: impl FnOnce(&mut Ui, Rect) -> R) -> (R, bool) {
        let ctx = ui.ctx().clone();
        // The card: `#fff`, `#f5f5f5` in Carbon.
        let card_fill = match theme(&ctx) {
            Theme::Snow => Color32::WHITE,
            Theme::Carbon => Color32::from_rgb(0xf5, 0xf5, 0xf5),
        };
        with_theme(&ctx, Theme::Snow, || {
            self.draw_card(ui, area, card_fill, body)
        })
    }

    fn draw_card<R>(
        &self,
        ui: &mut Ui,
        area: Rect,
        card_fill: Color32,
        body: impl FnOnce(&mut Ui, Rect) -> R,
    ) -> (R, bool) {
        let backdrop = ui.allocate_rect(area, Sense::click());
        ui.painter().rect_filled(area, 0.0, BACKDROP);

        // The card is centered, `max-height: calc(100% - 72px)`. The area reaches the window's
        // bottom, so its bottom is the window's height (`vh`).
        let natural = 2.0 * PADDING_Y + HEADER_HEIGHT + self.body_height;
        let card = if let Some((max_width, margin)) = self.fill {
            let width = (area.width() - 2.0 * margin.x)
                .min(max_width)
                .max(FILL_MIN_WIDTH);
            Rect::from_min_size(
                pos2(area.center().x - width / 2.0, area.top() + margin.y),
                vec2(width, area.height() - 2.0 * margin.y),
            )
        } else if self.top {
            let height = natural.min(area.height() - 0.3 * area.bottom());
            Rect::from_min_size(
                pos2(
                    area.center().x - DIALOG_WIDTH / 2.0,
                    area.top() + 0.16 * area.bottom(),
                ),
                vec2(DIALOG_WIDTH, height),
            )
        } else {
            Rect::from_center_size(
                area.center(),
                vec2(DIALOG_WIDTH, natural.min(area.height() - 72.0)),
            )
        };
        // Centering can land on a half pixel; Chromium paints the dialog's layer at the whole
        // pixel below it, text and all.
        let card = pixel_snap(card);
        // The card swallows clicks so they don't reach the backdrop.
        ui.allocate_rect(card, Sense::click());
        let painter = ui.painter().clone();
        SHADOW.paint(&painter, card, CornerRadius::same(RADIUS));
        painter.rect_filled(card, CornerRadius::same(RADIUS), card_fill);

        let content = if self.fill.is_some() {
            card
        } else {
            card.shrink2(vec2(PADDING_X, PADDING_Y))
        };
        let size = if self.back {
            vec2(BACK_SIZE, BACK_SIZE)
        } else {
            CLOSE_SIZE.into()
        };
        let (icon, hit) = if self.fill.is_some() {
            // `margin: 36px 44px 0 0` around a clickable box padded 8px.
            let icon = Rect::from_min_size(
                pos2(card.right() - 44.0 - 8.0 - size.x, card.top() + 36.0 + 8.0),
                size,
            );
            (icon, icon.expand(8.0))
        } else {
            let icon = Rect::from_min_size(
                pos2(
                    card.right() - CLOSE_INSET[0] - size.x,
                    card.top() + CLOSE_INSET[1],
                ),
                size,
            );
            // The clickable box pads the icon 30px, 12px below; the × has no hover style.
            (
                icon,
                Rect::from_min_max(icon.min - vec2(30.0, 30.0), icon.max + vec2(30.0, 12.0)),
            )
        };
        let close = ui
            .interact(hit, self.id.with("close"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);

        // A dialog cut short by its max height gets a shorter body, which can scroll.
        let body_rect = if self.fill.is_some() {
            card
        } else {
            Rect::from_min_max(
                pos2(content.left(), content.top() + HEADER_HEIGHT),
                pos2(
                    content.right(),
                    content.top()
                        + HEADER_HEIGHT
                        + self.body_height.min(content.height() - HEADER_HEIGHT),
                ),
            )
        };
        let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body_rect));
        // A window too short for the dialog cuts the body off at the card.
        body_ui.set_clip_rect(card.intersect(ui.clip_rect()));
        let inner = body(&mut body_ui, body_rect);

        // The header and the close control, over anything the body paints behind them.
        if self.fill.is_none() {
            self.header_style.paint_line(
                &painter,
                pos2(content.left(), content.top() + HEADER_NUDGE),
                self.header,
                self.header_ink,
                None,
            );
        }
        if self.back {
            // The back arrow, its viewBox fitted into the box.
            crate::paint_icon(ui, "legacy/DEPRECATED_backArrow", icon, BACK_INK);
        } else {
            paint_close(&painter, icon.min, Color32::BLACK.gamma_multiply(0.4));
        }
        (inner, close.clicked() || backdrop.clicked())
    }
}

/// The close ×: two 1px bars corner to corner of its 12×13 box, in `color` (black, or white under
/// the dark themes' `filter: invert(1)`, at some opacity).
pub fn paint_close(painter: &Painter, top_left: Pos2, color: Color32) {
    let bars = [
        [
            [0.697, 0.49],
            [12.01, 11.803],
            [11.303, 12.51],
            [-0.01, 1.197],
        ],
        [
            [-0.01, 11.803],
            [11.303, 0.49],
            [12.01, 1.197],
            [0.697, 12.51],
        ],
    ];
    for bar in bars {
        let points = bar.iter().map(|[x, y]| top_left + vec2(*x, *y)).collect();
        painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
    }
}
