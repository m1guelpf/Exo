//! The copy tooltip: a dark tooltip box 8px under its target, centred on it and kept in the window,
//! on the hover tooltips' timing: it shows once hover intent has the pointer settled on the target,
//! and goes 150ms after the pointer has left the target, unless it's on the tooltip. Its text is at
//! 70% (`top: -1px`, cut at 460px, underlined while hovered) and the copy icon 8px after it at 66%
//! (100% hovered). The text does what its caller says (copy, or compose to the address); the icon
//! copies.

use egui::{Color32, Id, Order, Rect, Sense, Ui, Vec2, pos2, vec2};

use crate::{Family, Font, HoverIntent, TextStyle, motion};

/// A tooltip's text: 12px/20px bold, `padding: 6px 7px 3px`; the icon's row a pixel taller.
const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 20.0);
const PADDING_X: f32 = 7.0;
const PADDING_TOP: f32 = 6.0;
const HEIGHT: f32 = 30.0;
/// Between the target and the box.
const GAP: f32 = 8.0;
/// The text's `max-width`; the icon's `margin-left`.
const MAX_TEXT: f32 = 460.0;
const ICON_GAP: f32 = 8.0;
const ICON: Vec2 = Vec2::new(11.0, 14.0);
const COPY: &str = "legacy/DEPRECATED_clipboard";
/// How long it stays once the pointer is off its target; its 200ms fade in.
const HIDE_AFTER: f64 = 0.15;
const FADE: f64 = 0.2;

/// A copy tooltip's target: where it is, what it shows and what its icon copies.
#[derive(Clone, Debug, PartialEq)]
pub struct CopyTip {
    pub target: Rect,
    pub shown: String,
    pub copied: String,
}

/// What was clicked in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyClick {
    /// The text (its caller's action, or copy without one).
    Text,
    /// The copy icon.
    Icon,
}

/// The tooltip between frames: hover intent on the targets, and the one up, since when, and
/// since when the pointer has been off its target.
#[derive(Clone, Debug, Default)]
pub struct CopyTooltip {
    intent: HoverIntent<(String, String)>,
    open: Option<(CopyTip, f64, Option<f64>)>,
}

impl CopyTooltip {
    /// Whether a tooltip is up.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Follows the pointer: `hovered` is the target under it this frame, if any. Shows the
    /// tooltip once the pointer has settled on a target, keeps it 150ms after the pointer has
    /// left the target and while it's on the tooltip (or its 8px towards the target), and paints
    /// it. Returns a click on it, with its target.
    pub fn show(
        &mut self,
        ui: &Ui,
        id: Id,
        hovered: Option<CopyTip>,
    ) -> Option<(CopyClick, CopyTip)> {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|input| input.time);
        let pointer = ctx.pointer_hover_pos();
        let settled = self
            .intent
            .update(
                &ctx,
                hovered
                    .as_ref()
                    .map(|tip| (tip.shown.clone(), tip.copied.clone())),
            )
            .is_some();
        match (hovered, &mut self.open) {
            (Some(tip), _) if settled => self.open = Some((tip, now, None)),
            (Some(tip), Some((open, _, left)))
                if open.shown == tip.shown && open.copied == tip.copied =>
            {
                open.target = tip.target;
                *left = None;
            }
            (_, Some((open, _, left))) => {
                let rect = rect(ui, open);
                let on_tooltip =
                    pointer.is_some_and(|at| rect.expand2(vec2(0.0, GAP)).contains(at));
                let left = *left.get_or_insert(now);
                if !on_tooltip && now - left >= HIDE_AFTER {
                    self.open = None;
                } else if !on_tooltip {
                    ctx.request_repaint_after_secs((HIDE_AFTER - (now - left)) as f32);
                }
            }
            (_, None) => {}
        }
        let (tip, since, _) = self.open.clone()?;
        let fade = ((now - since) / FADE).min(1.0) as f32;
        if fade < 1.0 {
            ctx.request_repaint();
        }
        let rect = rect(ui, &tip);
        let mut clicked = None;
        egui::Area::new(id)
            .order(Order::Tooltip)
            .fixed_pos(rect.min)
            .constrain(false)
            .show(&ctx, |area| {
                let mut painter = area.painter().clone();
                painter.multiply_opacity(motion::CubicBezier::EASE.at(fade));
                crate::paint_tooltip_box(&painter, rect);
                let carbon = crate::theme(&ctx) == crate::Theme::Carbon;
                // The text: white at 70% (Carbon's black); the icon's fill at 60%.
                let (text_ink, icon_ink) = if carbon {
                    (
                        Color32::from_black_alpha(179),
                        Color32::from_black_alpha(153),
                    )
                } else {
                    (
                        Color32::from_white_alpha(179),
                        Color32::from_white_alpha(153),
                    )
                };
                let at = pos2(rect.left() + PADDING_X, rect.top() + PADDING_TOP - 1.0);
                let width = text_width(area, &tip.shown);
                let text = Rect::from_min_size(at, vec2(width, TEXT.line_height));
                let text_hit = area
                    .interact(text, id.with("text"), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                TEXT.paint_line(&painter, at, &tip.shown, text_ink, Some(MAX_TEXT));
                if text_hit.hovered() {
                    let underline = Rect::from_min_size(
                        pos2(at.x, at.y + TEXT.baseline(&ctx) + 1.0),
                        vec2(width, 1.0),
                    );
                    painter.rect_filled(crate::pixel_snap(underline), 0.0, text_ink);
                }
                // The icon: `top: 1px` on the line.
                let icon = Rect::from_min_size(
                    pos2(text.right() + ICON_GAP, rect.top() + PADDING_TOP + 1.0),
                    ICON,
                );
                let icon_hit = area
                    .interact(icon, id.with("icon"), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                let opacity = motion::transition(
                    &ctx,
                    id.with("icon-hover"),
                    icon_hit.hovered(),
                    0.15,
                    motion::CubicBezier::EASE,
                );
                crate::paint_icon_on(
                    &painter,
                    COPY,
                    icon,
                    icon_ink.gamma_multiply(0.66 + 0.34 * opacity),
                );
                clicked = if text_hit.clicked() {
                    Some(CopyClick::Text)
                } else {
                    icon_hit.clicked().then_some(CopyClick::Icon)
                };
            });
        if clicked.is_some() {
            self.open = None;
        }
        clicked.map(|click| (click, tip))
    }
}

fn text_width(ui: &Ui, text: &str) -> f32 {
    TEXT.width(ui.ctx(), text).min(MAX_TEXT)
}

/// The box: centred 8px under its target, inside the window.
fn rect(ui: &Ui, tip: &CopyTip) -> Rect {
    let width = text_width(ui, &tip.shown) + ICON_GAP + ICON.x + 2.0 * PADDING_X;
    let window = ui.ctx().content_rect();
    let left = (tip.target.center().x - width / 2.0)
        .min(window.right() - width)
        .max(window.left());
    Rect::from_min_size(pos2(left, tip.target.bottom() + GAP), vec2(width, HEIGHT))
}
