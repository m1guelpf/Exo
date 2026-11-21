//! What the pointer and a selection in the bodies bring up: the `Quote` `enter` · `R` · `F` hint
//! under selected text, and hover intent, which shows a `mailto:` link's contact once the pointer
//! settles on it and a detected date's day with its `Create Event` tooltip.

use egui::{Context, Pos2, Rect, pos2};
use exo_ui::{LineTooltip, TooltipPart};

use super::{Action, Layout, ThreadView};
use crate::{
    body::{Bodies, Hit},
    format::names::Person,
};

/// The hint under selected text.
const QUOTE: &[TooltipPart<'static>] = &[
    TooltipPart::Text("Quote"),
    TooltipPart::Gap(10.0),
    TooltipPart::Keys(&["enter"]),
    TooltipPart::Separator(" · "),
    TooltipPart::Keys(&["R"]),
    TooltipPart::Separator(" · "),
    TooltipPart::Keys(&["F"]),
];
/// The hint's top: 31px under the selection's bottom, relative to the table, in the scroll
/// container, whose content starts 16px above the table.
const QUOTE_DROP: f32 = 31.0 - 16.0;
/// Where `Create Event`'s box starts below the date's box as Blitz has it (its line box): the
/// tooltip sits 15px under the date's bottom from a layer 16px above the table, and its box shows
/// 8px into it, so 7px under the date's content area, which ends 4px above the line box (a date
/// bottom at 249 puts the box's top at 256).
const DATE_DROP: f32 = 3.0;
const CREATE_EVENT: &[TooltipPart<'static>] = &[TooltipPart::Text("Create Event")];
/// `TOOLTIP_HOVER`: how long `Create Event` stays once the pointer leaves the date and it.
const TOOLTIP_HOVER: f64 = 0.15;

/// A detected date's `Create Event` tooltip: the message, the date's text and box (in its body),
/// and since when the pointer has been off both.
#[derive(Debug)]
pub(super) struct DateTip {
    id: String,
    text: String,
    bounds: Rect,
    left_at: Option<f64>,
}

impl ThreadView {
    /// The pointer over `href`, a `mailto:` link in a body, this frame; hover intent shows its
    /// first address's contact.
    pub(super) fn hover_mailto(&mut self, ctx: &Context, href: Option<String>) {
        if let Some(href) = self.mailto_intent.update(ctx, href) {
            // Its first address.
            let address = crate::compose::mailto_fields(href).and_then(|fields| {
                fields
                    .to
                    .into_iter()
                    .find(|address| !address.email.is_empty())
            });
            self.contact =
                address.map(|address| Person::new(address.name.unwrap_or_default(), address.email));
        }
    }

    /// The day a detected date in message `id` means, read against the day it was sent; `None`
    /// for one that names no day.
    pub(super) fn date_on(&self, id: &str, text: &str) -> Option<jiff::civil::Date> {
        let sent = self
            .message(id)?
            .date
            .to_zoned(self.now.time_zone().clone());
        super::dates::resolve(text, &sent)
    }

    /// Hover intent on a detected date (`interval: 100, sensitivity: 7`) shows its day and puts
    /// `Create Event` under it, until the pointer has been off the date and the tooltip for
    /// `TOOLTIP_HOVER`. `messages` is the scroll container on screen.
    pub(super) fn hover_date(
        &mut self,
        ui: &egui::Ui,
        layout: &Layout,
        messages: Rect,
        bodies: &Bodies,
        actions: &mut Vec<Action>,
    ) {
        let over = self
            .hovered_body
            .clone()
            .and_then(|id| match bodies.hovered(&id) {
                Some(Hit::Date { text, bounds }) => Some((
                    id,
                    text.clone(),
                    Rect::from_min_size(
                        pos2(bounds[0], bounds[1]),
                        egui::vec2(bounds[2], bounds[3]),
                    ),
                )),
                _ => None,
            });
        let target = over
            .as_ref()
            .filter(|(id, text, _)| self.date_on(id, text).is_some())
            .map(|(id, text, _)| (id.clone(), text.clone()));
        if let Some((id, text)) = self.date_intent.update(ui.ctx(), target).cloned()
            && let (Some(date), Some((.., bounds))) = (self.date_on(&id, &text), over.as_ref())
        {
            self.date_tip = Some(DateTip {
                id,
                text,
                bounds: *bounds,
                left_at: None,
            });
            actions.push(Action::ShowDay(date));
        }
        let Some(body) = self
            .date_tip
            .as_ref()
            .and_then(|tip| self.body_origin(&tip.id))
        else {
            self.date_tip = None;
            return;
        };
        let Some(tip) = &mut self.date_tip else {
            return;
        };
        let bounds = tip
            .bounds
            .translate(body.to_vec2() - egui::vec2(bodies.scroll_x(&tip.id), 0.0));
        let tooltip = LineTooltip {
            parts: CREATE_EVENT,
        };
        let size = tooltip.size(ui.ctx());
        let left = (bounds.center().x - size.x / 2.0)
            .trunc()
            .min(layout.card.right() - size.x)
            .max(layout.card.left());
        let rect = Rect::from_min_size(pos2(left, bounds.bottom() + DATE_DROP), size);
        let pointer = ui.ctx().pointer_hover_pos();
        let on_date = over
            .as_ref()
            .is_some_and(|(id, text, _)| *id == tip.id && *text == tip.text);
        let now = ui.input(|input| input.time);
        if on_date || pointer.is_some_and(|at| rect.contains(at)) {
            tip.left_at = None;
        } else if now - *tip.left_at.get_or_insert(now) > TOOLTIP_HOVER {
            self.date_tip = None;
            return;
        } else {
            ui.ctx().request_repaint_after_secs(TOOLTIP_HOVER as f32);
        }
        let painter = ui
            .painter()
            .with_clip_rect(messages.intersect(ui.clip_rect()));
        tooltip.paint(&painter, rect.min);
    }

    /// The hint under selected text in the message whose body is at `body` on screen, in the
    /// scroll container `messages`: centred on the selection, within the table's margins.
    pub(super) fn selection_hint(
        &self,
        ui: &egui::Ui,
        layout: &Layout,
        messages: Rect,
        body: Pos2,
        bodies: &Bodies,
        id: &str,
    ) {
        let Some((selected, bounds)) = bodies.selection_bounds() else {
            return;
        };
        if selected != id {
            return;
        }
        let bounds = bounds.translate(body.to_vec2() - egui::vec2(bodies.scroll_x(id), 0.0));
        let tooltip = LineTooltip { parts: QUOTE };
        let width = tooltip.size(ui.ctx()).x;
        let left = (bounds.center().x - width / 2.0)
            .trunc()
            .min(layout.card.right() - width)
            .max(layout.card.left());
        let painter = ui
            .painter()
            .with_clip_rect(messages.intersect(ui.clip_rect()));
        tooltip.paint(&painter, pos2(left, bounds.bottom() + QUOTE_DROP));
    }
}
