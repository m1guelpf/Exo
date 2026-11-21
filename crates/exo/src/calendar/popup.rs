//! Where the week view's popups (the event details, the event form) go against their target, and
//! their card.

use egui::{Color32, CornerRadius, Painter, Rect, pos2, vec2};
use exo_ui::{BoxShadow, Theme};

/// A popup's shadow: `0 -1px 16px rgba(0,0,0,.11), 0 3px 16px .12, 0 12px 24px .15`, in
/// Carbon `.24`, `.26`, `.3`.
const SHADOWS: [(f32, f32, [f32; 2]); 3] = [
    (-1.0, 16.0, [0.11, 0.24]),
    (3.0, 16.0, [0.12, 0.26]),
    (12.0, 24.0, [0.15, 0.3]),
];
/// Classic's `border-radius: 4px`.
const RADIUS: u8 = 4;

/// Paints a popup's card over `rect` (snapped): its shadows, Classic's corners and its fill,
/// white, Carbon `#2f3237`.
pub fn paint_card(painter: &Painter, rect: Rect) {
    let theme = exo_ui::theme(painter.ctx());
    let carbon = usize::from(theme == Theme::Carbon);
    for (y, blur, alpha) in SHADOWS {
        BoxShadow::new(
            [0.0, y],
            blur,
            0.0,
            Color32::from_black_alpha((alpha[carbon] * 255.0).round() as u8),
        )
        .paint(painter, rect, CornerRadius::same(RADIUS));
    }
    painter.rect_filled(rect, CornerRadius::same(RADIUS), fill(theme));
}

/// A popup's fill: white, Carbon `#2f3237`.
pub fn fill(theme: Theme) -> Color32 {
    match theme {
        Theme::Snow => Color32::WHITE,
        Theme::Carbon => Color32::from_rgb(0x2f, 0x32, 0x37),
    }
}

/// The window inset 18px; 16px beside the target and 21px above its top, or 8px above or below
/// it; never shorter than 350px when squeezed.
const INSET: f32 = 18.0;
const SIDE_GAP: f32 = 16.0;
const SIDE_RAISE: f32 = 21.0;
const VERTICAL_GAP: f32 = 8.0;
const MIN_HEIGHT: f32 = 350.0;
/// The week view's sidebar, which the right side has to clear.
const SIDEBAR: f32 = 288.0;
/// The right pane's popup goes 50px further left.
const PANE_SHIFT: f32 = 50.0;

/// Which side of its target a popup went, kept while it's open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Right,
    Left,
    Top,
    Bottom,
}

/// A popup's placement, sticky to the side it first took.
#[derive(Clone, Copy, Debug, Default)]
pub struct Placement {
    side: Option<Side>,
}

impl Placement {
    /// A `width` × `height` popup right of `target` while that leaves it in the week's left three
    /// sevenths, else left, above, below, or squeezed above or below; kept inside `window` inset
    /// 18px. `pane` shifts it for the right pane.
    pub fn place(
        &mut self,
        window: Rect,
        target: Rect,
        width: f32,
        height: f32,
        pane: bool,
    ) -> Rect {
        let window = window.shrink(INSET);
        let rect = |side: Side, squeeze: bool| -> Rect {
            match side {
                Side::Right => Rect::from_min_size(
                    pos2(target.right() + SIDE_GAP, target.top() - SIDE_RAISE),
                    vec2(width, height),
                ),
                Side::Left => Rect::from_min_size(
                    pos2(target.left() - width - SIDE_GAP, target.top() - SIDE_RAISE),
                    vec2(width, height),
                ),
                Side::Top => {
                    let mut rect = Rect::from_min_size(
                        pos2(
                            target.center().x - width / 2.0,
                            target.top() - height - VERTICAL_GAP,
                        ),
                        vec2(width, height),
                    );
                    if squeeze && rect.top() < window.top() {
                        rect = Rect::from_min_size(
                            pos2(rect.left(), window.top()),
                            vec2(
                                width,
                                MIN_HEIGHT.max(target.top() - window.top() - VERTICAL_GAP),
                            ),
                        );
                    }
                    rect
                }
                Side::Bottom => {
                    let mut rect = Rect::from_min_size(
                        pos2(
                            target.center().x - width / 2.0,
                            target.bottom() + VERTICAL_GAP,
                        ),
                        vec2(width, height),
                    );
                    if squeeze && rect.bottom() > window.bottom() {
                        rect.set_height(MIN_HEIGHT.max(window.bottom() - rect.top()));
                    }
                    rect
                }
            }
        };
        let side = self.side.unwrap_or_else(|| {
            let right = rect(Side::Right, false);
            if right.left() < (window.width() - SIDEBAR) * 3.0 / 7.0 + 120.0
                && right.right() < window.right()
            {
                Side::Right
            } else if rect(Side::Left, false).left() > window.left() {
                Side::Left
            } else if rect(Side::Top, false).top() > window.top() {
                Side::Top
            } else if rect(Side::Bottom, false).bottom() < window.bottom() {
                Side::Bottom
            } else if target.top() - window.top() > window.bottom() - target.bottom() {
                Side::Top
            } else {
                Side::Bottom
            }
        });
        let squeeze = self.side.is_some() || !matches!(side, Side::Right | Side::Left);
        self.side = Some(side);
        let wanted = rect(side, squeeze);
        let x = wanted
            .left()
            .clamp(window.left(), (window.right() - width).max(window.left()));
        let y = if wanted.top() < window.top() {
            window.top()
        } else if wanted.bottom() > window.bottom() {
            window.top().max(window.bottom() - wanted.height())
        } else {
            wanted.top()
        };
        let shift = if pane { PANE_SHIFT } else { 0.0 };
        Rect::from_min_size(
            pos2(x - shift, y),
            vec2(width, wanted.height().min(window.height()).floor()),
        )
    }
}
