//! The Show New Messages bar: replies that arrived while the thread was open wait behind it, 12px
//! under the messages, or pinned to the bottom of the column when that's out of view. A click,
//! Enter on it, or ⇧N shows them.

use egui::{Color32, CornerRadius, Rect, Sense, Ui, pos2, vec2};
use exo_ui::{BoxShadow, Family, Font, KeyTooltip, Placement, TextStyle, motion};

use super::Layout;

/// The bar's label in Classic: bold, upper-cased, 14px/20px, `#54acdc`.
const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 14.0, 20.0);
const LABEL: &str = "Show New Messages";
const INK: Color32 = Color32::from_rgb(0x54, 0xac, 0xdc);
/// `padding: 8px 12px 4px` around the line.
const PADDING_X: f32 = 12.0;
const PADDING_TOP: f32 = 8.0;
const HEIGHT: f32 = 8.0 + 20.0 + 4.0;
/// The tethered element: the bar less its `margin-top: -1px`. The bar sits `top: 3px` down in
/// it, from 1px above its top.
const TETHERED: f32 = HEIGHT - 1.0;
const DROP: f32 = 3.0 - 1.0;
const OFFSET: f32 = 12.0;
/// `transition: opacity 96ms ease` as it shows; its shadow's `.4s` (hover) and `.2s` (focus)
/// `cubic-bezier(0.25, 0.46, 0.45, 0.94)`.
const FADE: f32 = 0.096;
const SHADOW_CURVE: motion::CubicBezier = motion::CubicBezier::new(0.25, 0.46, 0.45, 0.94);

/// Paints the bar under the messages, whose end is at `end` on screen, in the column
/// `messages`; `focused` while it has the keyboard.
/// Returns whether it was clicked.
pub(super) fn show(ui: &Ui, layout: &Layout, messages: Rect, end: f32, focused: bool) -> bool {
    let ctx = ui.ctx();
    let palette = exo_ui::palette(ctx);
    let id = ui.id().with("show-new-messages");
    let text = LABEL.to_uppercase();
    let width = TEXT
        .single_line(ui.painter(), &text, Color32::PLACEHOLDER, None)
        .size()
        .x
        + 2.0 * PADDING_X;
    // Pinned to the scroll container's bottom when it would fall below it.
    let tethered = end + OFFSET;
    let pinned = tethered + TETHERED > messages.bottom();
    let tethered = if pinned {
        messages.bottom() - TETHERED
    } else {
        tethered
    };
    let bar = Rect::from_min_size(
        pos2(layout.card.center().x - width / 2.0, tethered + DROP),
        vec2(width, HEIGHT),
    );
    let response = ui
        .interact(bar, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);

    let mut painter = ui.painter().clone();
    painter.multiply_opacity(motion::transition(
        ctx,
        id.with("shown"),
        true,
        FADE,
        motion::CubicBezier::EASE,
    ));
    // Pinned, the bar squares its bottom corners.
    let radius = if pinned {
        CornerRadius {
            nw: 2,
            ne: 2,
            sw: 0,
            se: 0,
        }
    } else {
        CornerRadius::same(2)
    };
    let rect = exo_ui::pixel_snap(bar);
    let hover = motion::transition(ctx, id.with("hover"), response.hovered(), 0.4, SHADOW_CURVE);
    let focus = motion::transition(ctx, id.with("focus"), focused, 0.2, SHADOW_CURVE);
    let black =
        |alpha: f32, weight: f32| Color32::from_black_alpha((alpha * weight * 255.0).round() as u8);
    let glow = |weight: f32| {
        Color32::from_rgba_unmultiplied(174, 177, 221, (0.3 * weight * 255.0).round() as u8)
    };
    let rest = (1.0 - hover) * (1.0 - focus);
    for shadow in [
        // At rest `0 0 8px 0 rgba(0,0,0,.03), 0 4px 12px 0 rgba(0,0,0,.1)`; hovered
        // `0 0 12px 0 rgba(0,0,0,.1), 0 8px 14px 0 rgba(0,0,0,.1)`; with the keyboard
        // `0 0 12px 0 rgba(174,177,221,.3), 0 8px 14px 0 rgba(174,177,221,.3)`.
        BoxShadow::new([0.0, 0.0], 8.0, 0.0, black(0.03, rest)),
        BoxShadow::new([0.0, 4.0], 12.0, 0.0, black(0.1, rest)),
        BoxShadow::new([0.0, 0.0], 12.0, 0.0, black(0.1, hover * (1.0 - focus))),
        BoxShadow::new([0.0, 8.0], 14.0, 0.0, black(0.1, hover * (1.0 - focus))),
        BoxShadow::new([0.0, 0.0], 12.0, 0.0, glow(focus)),
        BoxShadow::new([0.0, 8.0], 14.0, 0.0, glow(focus)),
    ] {
        shadow.paint(&painter, rect, radius);
    }
    // `#fff`, `#464b53` in Carbon, hovered or not.
    let background = match palette.theme {
        exo_ui::Theme::Snow => Color32::WHITE,
        exo_ui::Theme::Carbon => Color32::from_rgb(0x46, 0x4b, 0x53),
    };
    painter.rect_filled(rect, radius, background);
    TEXT.paint_line(
        &painter,
        pos2(bar.left() + PADDING_X, bar.top() + PADDING_TOP),
        &text,
        INK,
        None,
    );
    // Over the bar, its bottom on the bar's box, shown while it has the keyboard too.
    KeyTooltip {
        text: LABEL,
        keys: &["shift", "N"],
    }
    .show_at_focused(ui, &response, Placement::Above(-DROP - 1.0), focused, None);
    response.clicked()
}
