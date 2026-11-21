//! The hide-empty-splits nudge: at Inbox Zero in a split that shows while empty, a card offering to
//! hide empty splits. It's the same dark card in both themes.

use egui::{Color32, CornerRadius, CursorIcon, Rect, Sense, Ui, pos2, vec2};
use exo_ui::{BoxShadow, Family, Font, TextStyle, motion::CubicBezier};

/// The nudge's id in `nudges` (`HIDE_EMPTY_SPLIT`).
pub const HIDE_EMPTY_SPLIT: &str = "hideEmptySplit";

/// What the card was asked to do; either way it's done with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NudgeAction {
    /// Its close button.
    Close,
    /// `Try it out`.
    Try,
}

/// The title and the button: 12px/24px semibold, `top: 1px`.
const TITLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 12.0, 24.0);
/// The body: 12px/20px, `top: 3px`.
const BODY: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
/// `max-width: 262px`, which the copy fills.
const WIDTH: f32 = 262.0;
/// The header's and body's side padding.
const PADDING: f32 = 12.0;
/// The header's 12px top padding and the title's line.
const HEADER: f32 = 12.0 + 24.0;
/// The footer's 8px padding around the button's line.
const FOOTER: f32 = 8.0 + 24.0 + 8.0;
/// The card: `#45494e` with `#ebeef2` text, the footer `#2d2f34` and the button `#54acdc`.
const CARD: Color32 = Color32::from_rgb(0x45, 0x49, 0x4e);
const FOOTER_FILL: Color32 = Color32::from_rgb(0x2d, 0x2f, 0x34);
const TEXT: Color32 = Color32::from_rgb(0xeb, 0xee, 0xf2);
const BUTTON: Color32 = Color32::from_rgb(0x54, 0xac, 0xdc);
/// `box-shadow: 0 2px 4px 0 rgba(0,0,0,.1), 0 2px 12px 0 rgba(0,0,0,.05)`.
const SHADOWS: [BoxShadow; 2] = [
    BoxShadow::new([0.0, 2.0], 4.0, 0.0, Color32::from_black_alpha(26)),
    BoxShadow::new([0.0, 2.0], 12.0, 0.0, Color32::from_black_alpha(13)),
];
const PARAGRAPHS: [&str; 2] = [
    "You can now hide empty Split Inboxes. These Split Inboxes will show again as soon as new messages arrive.",
    "Stay focused and in flow!",
];

/// When the card started showing, and the last frame it showed.
#[derive(Clone, Copy)]
struct Shown {
    since: f64,
    frame: u64,
}

/// Draws the card at the list's top left (`left: 62px; top: 0`): each time it appears, it rises
/// 20px into place as it fades in over 250ms (`ease`).
pub(super) fn show(ui: &Ui, list: Rect) -> Option<NudgeAction> {
    let ctx = ui.ctx();
    let id = ui.id().with("hide-split-nudge");
    let (now, frame) = (ctx.input(|input| input.time), ctx.cumulative_frame_nr());
    let since = ctx.data_mut(|data| {
        let since = data
            .get_temp::<Shown>(id)
            .filter(|shown| shown.frame + 1 >= frame)
            .map_or(now, |shown| shown.since);
        data.insert_temp(id, Shown { since, frame });
        since
    });
    let shown = CubicBezier::EASE.at((((now - since) / 0.25) as f32).clamp(0.0, 1.0));
    if shown < 1.0 {
        ctx.request_repaint();
    }

    let lines = PARAGRAPHS.map(|paragraph| BODY.wrap(ctx, paragraph, WIDTH - 2.0 * PADDING));
    // The body's padding, `8px 12px 12px`, and 12px between paragraphs.
    let body = 8.0
        + lines
            .iter()
            .map(|lines| lines.len() as f32 * BODY.line_height)
            .sum::<f32>()
        + 12.0
        + 12.0;
    let card = Rect::from_min_size(
        pos2(list.left() + 62.0, list.top() + 20.0 * (1.0 - shown)),
        vec2(WIDTH, HEADER + body + FOOTER),
    );
    let mut painter = ui.painter().clone();
    painter.multiply_opacity(shown);
    // Classic's `border-radius: 4px`.
    let radius = CornerRadius::same(4);
    for shadow in SHADOWS {
        shadow.paint(&painter, card, radius);
    }
    painter.rect_filled(card, radius, CARD);
    let footer = Rect::from_min_max(pos2(card.left(), card.bottom() - FOOTER), card.max);
    painter.rect_filled(
        footer,
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: 4,
            se: 4,
        },
        FOOTER_FILL,
    );

    // The card takes clicks over the list (registered first, so its buttons win).
    ui.interact(card, id.with("card"), Sense::click());

    let left = card.left() + PADDING;
    TITLE.paint_line(
        &painter,
        pos2(left, card.top() + 12.0 + 1.0),
        "Hide empty Split Inboxes",
        TEXT,
        None,
    );
    let mut top = card.top() + HEADER + 8.0 + 3.0;
    for (index, paragraph) in lines.iter().enumerate() {
        if index > 0 {
            top += 12.0;
        }
        for line in paragraph {
            BODY.paint_line(&painter, pos2(left, top), line, TEXT, None);
            top += BODY.line_height;
        }
    }

    // The close button: a 12px cross 12px in from the top right, white at 35%, 60% hovered (over
    // 150ms).
    let cross = Rect::from_min_size(
        pos2(card.right() - 12.0 - 12.0, card.top() + 12.0),
        vec2(12.0, 12.0),
    );
    let close = ui
        .interact(
            Rect::from_min_size(cross.min, vec2(12.0, 15.0)),
            id.with("close"),
            Sense::click(),
        )
        .on_hover_cursor(CursorIcon::PointingHand);
    let opacity = 0.35
        + 0.25
            * exo_ui::motion::transition(
                ctx,
                id.with("close-hover"),
                close.hovered(),
                0.15,
                CubicBezier::EASE,
            );
    exo_ui::paint_icon_on(
        &painter,
        "legacy/close",
        cross,
        Color32::WHITE.gamma_multiply(opacity),
    );

    // The button's label is uppercase, right-aligned in the footer.
    let label = "Try it out".to_uppercase();
    let width = TITLE.width(ctx, &label);
    let cta = Rect::from_min_size(
        pos2(footer.right() - PADDING - width, footer.top() + 8.0),
        vec2(width, 24.0),
    );
    TITLE.paint_line(&painter, cta.min + vec2(0.0, 1.0), &label, BUTTON, None);
    let cta = ui
        .interact(cta, id.with("cta"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);

    if close.clicked() {
        Some(NudgeAction::Close)
    } else if cta.clicked() {
        Some(NudgeAction::Try)
    } else {
        None
    }
}
