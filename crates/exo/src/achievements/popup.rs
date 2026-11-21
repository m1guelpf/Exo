//! The popup a level reached raises: a card in the toasts' stack with a title and emoji, a line or
//! two, and `View Achievements`. It goes after 8 s unless hovered, fading over 2 s; a click opens
//! the matching view.

use egui::{
    Area, Color32, Context, CornerRadius, CursorIcon, Order, Painter, Pos2, Rect, Sense, Vec2,
    pos2, vec2,
};
use exo_ui::{BoxShadow, Family, Font, TextStyle, motion, paint_icon};

use super::{Kind, Reached, View};

/// The title: 14px on a 24px line, `top: 1px`.
const TITLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 14.0, 24.0);
/// The body: 12px/16px, `top: 1px`.
const BODY: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 16.0);
/// The body's italics.
const BODY_ITALIC: TextStyle = TextStyle::new(Font::new(Family::Sans, 400).italic(), 12.0, 16.0);
/// The action: 12px/16px bold, `top: 1px`.
const ACTION: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 16.0);
/// The emoji `img`, drawn with the system's emoji at this size to fill its 16px box.
const EMOJI: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 13.0, 16.0);

/// 8 s on screen.
const SHOWN_FOR: f64 = 8.0;
/// The fade: opacity over 2 s `ease-in`, and the removal 2 s after it starts.
const FADE: f64 = 2.0;
/// The gap between cards.
const GAP: f32 = 12.0;

/// A popup on screen.
pub struct Popup {
    reached: Reached,
    body: Vec<String>,
    /// When it starts fading, unless hovered first.
    hide_at: Option<f64>,
    /// When it started fading.
    fading_since: Option<f64>,
    serial: u64,
}

/// The popups, oldest first (the last 3).
#[derive(Default)]
pub struct Popups {
    shown: Vec<Popup>,
    serial: u64,
}

impl Popups {
    /// Raises the popup for `reached` at `now` (`time`, seconds), its body picked from its pool.
    pub fn push(&mut self, reached: Reached, now: f64) {
        // Any fair pick from the pool does.
        let body = super::body(reached, |len| (now.to_bits() as usize) % len);
        self.serial += 1;
        self.shown.push(Popup {
            reached,
            body,
            hide_at: Some(now + SHOWN_FOR),
            fading_since: None,
            serial: self.serial,
        });
        let excess = self.shown.len().saturating_sub(3);
        self.shown.drain(..excess);
    }

    /// Any popup is up (for the dim layer behind them at Inbox Zero).
    pub fn any(&self) -> bool {
        !self.shown.is_empty()
    }

    /// Their titles, oldest first. For tests.
    pub fn titles(&self) -> Vec<String> {
        self.shown
            .iter()
            .map(|popup| super::title(popup.reached))
            .collect()
    }

    /// Paints the popups at the stack's bottom left in `area` (the page below the tab strip),
    /// their bottom `bottom` px up, over whatever the stack has under them; `photo`, over Inbox
    /// Zero's. Returns the dialog's view a click on one opens.
    pub fn show(
        &mut self,
        ctx: &Context,
        area: Rect,
        bottom: f32,
        photo: bool,
        now: f64,
    ) -> Option<View> {
        self.shown
            .retain(|popup| popup.fading_since.is_none_or(|since| now - since < FADE));
        let mut y = area.bottom() - bottom;
        let mut clicked = None;
        let mut closed = None;
        for popup in self.shown.iter_mut().rev() {
            let height = popup.height(ctx);
            let top = y - height;
            y = top - GAP;
            match popup.show(ctx, area, top, photo, now) {
                Some(Action::Open) => {
                    clicked = Some(popup.reached.kind.view());
                    closed = Some(popup.serial);
                }
                Some(Action::Close) => closed = Some(popup.serial),
                None => {}
            }
        }
        if let Some(serial) = closed {
            self.shown.retain(|popup| popup.serial != serial);
        }
        if let Some(at) = self
            .shown
            .iter()
            .filter_map(|popup| popup.hide_at)
            .reduce(f64::min)
        {
            ctx.request_repaint_after_secs((at - now).max(0.0) as f32);
        }
        if self.shown.iter().any(|popup| popup.fading_since.is_some()) {
            ctx.request_repaint();
        }
        clicked
    }
}

enum Action {
    Open,
    Close,
}

impl Popup {
    /// The card's width: 350px, 375px from a million.
    fn width(&self) -> f32 {
        if self.reached.completed >= 1_000_000 {
            375.0
        } else {
            350.0
        }
    }

    /// The status' content width: `padding-left: 1.75rem; padding-right: 16px`.
    fn content_width(&self, ctx: &Context) -> f32 {
        self.width() - 1.75 * exo_ui::rem(ctx.content_rect().width(), false) - 16.0
    }

    /// The body's lines as they wrap in the card (`white-space: wrap`).
    fn lines(&self, ctx: &Context) -> Vec<String> {
        let width = self.content_width(ctx);
        self.body
            .iter()
            .flat_map(|line| BODY.wrap(ctx, &line.replace('_', ""), width))
            .collect()
    }

    /// The title row (`padding: 12px 0 5px`), the body, then the action (`padding: 12px 0 11px`).
    fn height(&self, ctx: &Context) -> f32 {
        12.0 + TITLE.line_height
            + 5.0
            + self.lines(ctx).len() as f32 * BODY.line_height
            + 12.0
            + ACTION.line_height
            + 11.0
    }

    fn show(
        &mut self,
        ctx: &Context,
        area: Rect,
        top: f32,
        photo: bool,
        now: f64,
    ) -> Option<Action> {
        let rem = exo_ui::rem(area.width(), false);
        // `left: calc(3.5rem + 40px - 1.75rem + 6px)`, 44px for 3.5rem at ≤1152px.
        let inset = if area.width() <= 1152.0 {
            44.0
        } else {
            3.5 * rem
        };
        let card = Rect::from_min_size(
            pos2(area.left() + inset + 40.0 - 1.75 * rem + 6.0, top),
            vec2(self.width(), self.height(ctx)),
        );
        let id = exo_ui::id(ctx, ("achievement-popup", self.serial));
        Area::new(id)
            .order(Order::Foreground)
            .fixed_pos(card.min)
            .show(ctx, |ui| {
                let response = ui
                    .interact(card, id.with("card"), Sense::click())
                    .on_hover_cursor(CursorIcon::PointingHand);
                // Hovering keeps it, fading or not; leaving starts the 8 s again.
                let hovered = response.contains_pointer();
                if hovered {
                    self.hide_at = None;
                    self.fading_since = None;
                } else if self.fading_since.is_none() {
                    let hide_at = *self.hide_at.get_or_insert(now + SHOWN_FOR);
                    if now >= hide_at {
                        self.fading_since = Some(now);
                        self.hide_at = None;
                    }
                }
                let opacity = self.fading_since.map_or(1.0, |since| {
                    1.0 - motion::CubicBezier::EASE_IN.at(((now - since) / FADE) as f32)
                });
                let mut painter = ui.painter().clone();
                painter.multiply_opacity(opacity);
                let hover = motion::transition(
                    ctx,
                    id.with("hover"),
                    hovered,
                    0.4,
                    motion::CubicBezier::new(0.25, 0.46, 0.45, 0.94),
                );
                let close = self.paint(ui, &painter, card, photo, hover);
                if close {
                    Some(Action::Close)
                } else {
                    response.clicked().then_some(Action::Open)
                }
            })
            .inner
    }

    /// Paints the card; returns whether its × was clicked.
    fn paint(&self, ui: &egui::Ui, painter: &Painter, card: Rect, photo: bool, hover: f32) -> bool {
        let ctx = ui.ctx();
        let palette = exo_ui::palette(ctx);
        let card = exo_ui::pixel_snap(card);
        // `0 6px 30px 2px rgba(0,0,0,.2)` and the `inset 3px 0 0` bar,
        // `0 8px 30px 2px rgba(0,0,0,.1), 0 10px 30px 2px rgba(0,0,0,.3)` hovered (Classic's for
        // the send, Mark Done and Get Me To Zero cards; the default for Inbox Zero's).
        let black = |alpha: f32| Color32::from_black_alpha((alpha * 255.0).round() as u8);
        BoxShadow::new([0.0, 6.0], 30.0, 2.0, black(0.2 * (1.0 - hover))).paint(
            painter,
            card,
            CornerRadius::ZERO,
        );
        BoxShadow::new([0.0, 8.0], 30.0, 2.0, black(0.1 * hover)).paint(
            painter,
            card,
            CornerRadius::ZERO,
        );
        BoxShadow::new([0.0, 10.0], 30.0, 2.0, black(0.3 * hover)).paint(
            painter,
            card,
            CornerRadius::ZERO,
        );
        // `#fff`, `#464b53` in Carbon; over the photo `hsla(0,0%,100%,.2)` (and a 12.5px blur of
        // it, which Exo leaves out).
        let background = match (photo, palette.theme) {
            (true, _) => Color32::from_white_alpha(51),
            (false, exo_ui::Theme::Snow) => Color32::WHITE,
            (false, exo_ui::Theme::Carbon) => Color32::from_rgb(0x46, 0x4b, 0x53),
        };
        painter.rect_filled(card, 0.0, background);
        let accent = match self.reached.kind {
            Kind::Send => Color32::from_rgb(0xac, 0x88, 0xe6),
            Kind::MarkDone | Kind::Gmtz => palette.border_success_default,
            Kind::InboxZero | Kind::InboxZeroDays => palette.link_border_primary,
        };
        painter.rect_filled(
            Rect::from_min_size(card.min, vec2(3.0, card.height())),
            0.0,
            accent,
        );

        // `rgba(0,0,0,.9)` / `.6` (white in Carbon); over the photo both `hsla(0,0%,100%,.75)`.
        let (title_color, body_color, action_color) = if photo {
            (
                Color32::from_white_alpha(191),
                Color32::from_white_alpha(191),
                Color32::from_white_alpha(89),
            )
        } else {
            (
                palette.ink(0.9, 0.9),
                palette.ink(0.6, 0.6),
                Color32::from_rgb(0x36, 0xa0, 0xda),
            )
        };
        let rem = exo_ui::rem(ctx.content_rect().width(), false);
        let left = card.left() + 1.75 * rem;
        let row = card.top() + 12.0;
        let title = super::title(self.reached);
        // `max-width: 500px`, ellipsized.
        let title_width = TITLE.paint_line(
            painter,
            pos2(left, row + 1.0),
            &title,
            title_color,
            Some(500.0),
        );
        // The emoji (`margin-left: 8px; margin-top: 3px`, 16px; Inbox Zero's 13px wide).
        let emoji = match self.reached.kind {
            Kind::InboxZero | Kind::InboxZeroDays => "🔥",
            Kind::MarkDone | Kind::Gmtz => "✅",
            Kind::Send => "💌",
        };
        EMOJI.paint_line(
            painter,
            pos2(left + title_width + 8.0, row + 3.0),
            emoji,
            Color32::WHITE,
            None,
        );
        // The ×: 10px, 10px in from the status' right padding, on the title row's middle.
        let icon = Rect::from_min_size(
            pos2(card.right() - 16.0 - 10.0, row + 7.0),
            Vec2::splat(10.0),
        );
        let close = ui.interact(
            icon.expand(10.0),
            ui.id().with(("achievement-close", self.serial)),
            Sense::click(),
        );
        // `opacity: .3`, `.5` hovered; over the photo, white at .5 either way.
        let close_color = match (photo, close.hovered()) {
            (true, _) => Color32::from_white_alpha(128),
            (false, hovered) => palette.ink(
                if hovered { 0.5 } else { 0.3 },
                if hovered { 0.5 } else { 0.3 },
            ),
        };
        paint_icon(
            ui,
            "legacy/close",
            icon,
            close_color.gamma_multiply(painter.opacity()),
        );

        let mut y = row + TITLE.line_height + 5.0;
        for line in self.lines(ctx) {
            paint_with_italics(painter, pos2(left, y + 1.0), &line, &self.body, body_color);
            y += BODY.line_height;
        }
        ACTION.paint_line(
            painter,
            pos2(left, y + 12.0 + 1.0),
            "View Achievements",
            action_color,
            None,
        );
        close.clicked()
    }
}

/// Paints `line`, italicizing the words the body wraps in underscores.
fn paint_with_italics(painter: &Painter, at: Pos2, line: &str, body: &[String], color: Color32) {
    let italic: Vec<&str> = body
        .iter()
        .flat_map(|line| line.split('_').skip(1).step_by(2))
        .collect();
    let Some(word) = italic
        .iter()
        .find(|word| !word.is_empty() && line.contains(*word))
    else {
        BODY.paint_line(painter, at, line, color, None);
        return;
    };
    let (before, after) = line.split_once(word).unwrap_or((line, ""));
    let mut x = at.x;
    x += BODY.paint_line(painter, pos2(x, at.y), before, color, None);
    x += BODY_ITALIC.paint_line(painter, pos2(x, at.y), word, color, None);
    BODY.paint_line(painter, pos2(x, at.y), after, color, None);
}
