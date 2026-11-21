//! The header's tooltips that stay while the pointer is on them: a participant's, `{name}
//! <{address}>` with a copy button, its address composing to them; and the subject's copy
//! tooltip. Both are dark boxes 8px under their target, kept inside the message column.

use egui::{Color32, Order, Pos2, Rect, Sense, Ui, pos2, vec2};
use exo_store::Contact;
use exo_ui::{Family, Font, HoverIntent, TextStyle, motion};

use crate::format::names::Person;

/// The tooltips' text: 12px/20px bold, `padding: 6px 7px 3px`.
const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 20.0);
const PADDING_X: f32 = 7.0;
const PADDING_TOP: f32 = 6.0;
const PADDING_BOTTOM: f32 = 3.0;
/// The gap between a tooltip and its target.
const GAP: f32 = 8.0;
/// A participant's tooltip polls its hover intent every 200ms; the subject's every 100ms.
const CONTACT_INTERVAL: f64 = 0.2;
/// How long a tooltip stays once the pointer has left its target, or longer while the pointer is
/// on the tooltip.
const HIDE_AFTER: f64 = 0.15;
const COPY: &str = "legacy/DEPRECATED_clipboard";

/// What a click in a tooltip asks for.
pub(super) enum Clicked {
    /// A new message to the participant, popped out with ⇧.
    ComposeTo(Person, bool),
    /// This went on the clipboard.
    Copied(String),
}

/// A tooltip up, and what it's for.
#[derive(Clone, Debug)]
pub(super) struct Open {
    /// Its target's box.
    target: Rect,
    /// When it showed, and when the pointer left its target.
    since: f64,
    left: Option<f64>,
    what: Showing,
}

#[derive(Clone, Debug, PartialEq)]
enum Showing {
    Contact(Contact),
    Subject(String),
}

/// The tooltip up, if any, between frames, and the pointer settling on a target.
pub(super) struct Tooltips {
    open: Option<Open>,
    contact_intent: HoverIntent<Contact>,
    subject_intent: HoverIntent<String>,
}

impl Default for Tooltips {
    fn default() -> Self {
        Self {
            open: None,
            contact_intent: HoverIntent::new(CONTACT_INTERVAL),
            subject_intent: HoverIntent::default(),
        }
    }
}

impl Tooltips {
    /// Follows the pointer over this frame's targets: the participants' names (`contacts`) and
    /// the subject. Shows a target's tooltip once hover intent has the pointer settled on it,
    /// keeps it until the pointer has been off the target for 150ms and isn't on the tooltip, and
    /// paints it within `within`.
    pub(super) fn show(
        &mut self,
        ui: &Ui,
        contacts: &[(Rect, Contact)],
        subject: Option<(Rect, &str)>,
        within: egui::Rangef,
    ) -> Option<Clicked> {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|input| input.time);
        let pointer = ctx.pointer_hover_pos();
        // Only what's on top under the pointer counts as hovered (not a target under the
        // palette), but the open tooltip is a layer of its own.
        let on_top = pointer.filter(|at| ctx.layer_id_at(*at) == Some(ui.layer_id()));
        let over = |rect: &Rect| on_top.is_some_and(|at| rect.contains(at));
        let hovered = contacts
            .iter()
            .find(|(rect, _)| over(rect))
            .map(|(rect, contact)| (*rect, Showing::Contact(contact.clone())))
            .or_else(|| {
                subject
                    .filter(|(rect, _)| over(rect))
                    .map(|(rect, text)| (rect, Showing::Subject(text.to_owned())))
            });
        let on_tooltip = self.open.as_ref().is_some_and(|open| {
            let rect = self.rect(ui, open, within);
            // The 8px between the target and the box is the tooltip's padding.
            pointer.is_some_and(|at| rect.expand2(vec2(0.0, GAP)).contains(at))
        });
        let (contact, subject) = match &hovered {
            Some((_, Showing::Contact(contact))) => (Some(contact.clone()), None),
            Some((_, Showing::Subject(subject))) => (None, Some(subject.clone())),
            None => (None, None),
        };
        let settled = match (
            self.contact_intent.update(&ctx, contact),
            self.subject_intent.update(&ctx, subject),
        ) {
            (Some(contact), _) => Some(Showing::Contact(contact.clone())),
            (_, Some(subject)) => Some(Showing::Subject(subject.clone())),
            _ => None,
        };
        if let (Some(what), Some((target, _))) = (settled, &hovered)
            && self.open.as_ref().is_none_or(|open| open.what != what)
        {
            self.open = Some(Open {
                target: *target,
                since: now,
                left: None,
                what,
            });
        }
        let open = self.open.as_mut()?;
        match &hovered {
            Some((target, what)) if *what == open.what => {
                (open.target, open.left) = (*target, None)
            }
            _ => {
                let left = *open.left.get_or_insert(now);
                if !on_tooltip && now - left >= HIDE_AFTER {
                    self.open = None;
                    return None;
                }
                if !on_tooltip {
                    ctx.request_repaint_after_secs((HIDE_AFTER - (now - left)) as f32);
                }
            }
        }
        let open = open.clone();
        let rect = self.rect(ui, &open, within);
        let fade = ((now - open.since) / 0.2).min(1.0) as f32;
        if fade < 1.0 {
            ctx.request_repaint();
        }
        let mut clicked = None;
        egui::Area::new(exo_ui::id(&ctx, "thread-header-tooltip"))
            .order(Order::Tooltip)
            .fixed_pos(rect.min)
            .constrain(false)
            .show(&ctx, |area| {
                let mut painter = area.painter().clone();
                painter.multiply_opacity(motion::CubicBezier::EASE.at(fade));
                exo_ui::paint_tooltip_box(&painter, rect);
                let line = pos2(rect.left() + PADDING_X, rect.top() + PADDING_TOP);
                let whole = area.interact(
                    rect,
                    exo_ui::id(area.ctx(), "thread-header-tooltip-box"),
                    Sense::click(),
                );
                clicked = match &open.what {
                    Showing::Contact(contact) => contact_content(area, &painter, line, contact),
                    Showing::Subject(subject) => subject_content(area, &painter, line, subject),
                };
                // A click anywhere else on it closes it.
                if clicked.is_some() || whole.clicked() {
                    self.open = None;
                }
            });
        clicked
    }

    /// Where the open tooltip's box goes: centred under its target, inside `within`.
    fn rect(&self, ui: &Ui, open: &Open, within: egui::Rangef) -> Rect {
        let width = match &open.what {
            Showing::Contact(contact) => contact_width(ui, contact),
            Showing::Subject(subject) => subject_width(ui, subject),
        };
        let size = vec2(
            width + 2.0 * PADDING_X,
            PADDING_TOP
                + TEXT.line_height
                + PADDING_BOTTOM
                + f32::from(u8::from(matches!(open.what, Showing::Subject(_)))),
        );
        let left = (open.target.center().x - size.x / 2.0)
            .min(within.max - size.x)
            .max(within.min);
        Rect::from_min_size(pos2(left, open.target.bottom() + GAP), size)
    }
}

/// The tooltip's text color: `#ebeef2` on Snow's dark card, `#18191b` on Carbon's light one.
fn ink(ui: &Ui) -> Color32 {
    match exo_ui::theme(ui.ctx()) {
        exo_ui::Theme::Snow => Color32::from_rgb(0xeb, 0xee, 0xf2),
        exo_ui::Theme::Carbon => Color32::from_rgb(0x18, 0x19, 0x1b),
    }
}

/// The copy button: `padding: 8px 4px 8px 7px`, the 12px icon box less its
/// `margin-right: -3px`; the glyph 1px under the line's top (`top: 2px; margin-top: -2px`, on
/// the baseline).
const CONTACT_COPY: f32 = 7.0 + 12.0 - 3.0 + 4.0;

/// The line's width as [`contact_content`] paints it, piece by piece (Chromium shapes each text
/// node on its own).
fn contact_width(ui: &Ui, contact: &Contact) -> f32 {
    let pieces = match contact.name.as_deref().filter(|name| !name.is_empty()) {
        Some(name) => vec![name, " <", &contact.email, ">"],
        None => vec![contact.email.as_str()],
    };
    pieces
        .into_iter()
        .map(|piece| TEXT.width(ui.ctx(), piece))
        .sum::<f32>()
        + CONTACT_COPY
}

/// `{name} <{address}>`, or the address alone, as the copy button copies it.
fn contact_text(contact: &Contact) -> String {
    match contact.name.as_deref().filter(|name| !name.is_empty()) {
        Some(name) => format!("{name} <{}>", contact.email),
        None => contact.email.clone(),
    }
}

/// A participant's tooltip line from `line`: the name, ` <address>` at 70%,
/// then the copy button at 40%, 60% hovered.
fn contact_content(
    ui: &Ui,
    painter: &egui::Painter,
    line: Pos2,
    contact: &Contact,
) -> Option<Clicked> {
    let ink = ink(ui);
    let name = contact.name.as_deref().filter(|name| !name.is_empty());
    let mut x = line.x;
    if let Some(name) = name {
        x += TEXT.paint_line(painter, line, name, ink, None);
    }
    let mut dim = painter.clone();
    dim.multiply_opacity(0.7);
    let open = if name.is_some() { " <" } else { "" };
    x += TEXT.paint_line(&dim, pos2(x, line.y), open, ink, None);
    let email = TEXT.paint_line(&dim, pos2(x, line.y), &contact.email, ink, None);
    let address = Rect::from_min_size(pos2(x, line.y), vec2(email, TEXT.line_height));
    x += email;
    if name.is_some() {
        x += TEXT.paint_line(&dim, pos2(x, line.y), ">", ink, None);
    }
    let button = Rect::from_min_size(pos2(x, line.y - 2.0), vec2(CONTACT_COPY, 24.0));
    let copy = ui
        .interact(
            button,
            exo_ui::id(ui.ctx(), "thread-contact-copy"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let shown = if copy.hovered() { 0.6 } else { 0.4 };
    // `fill: #fff` (`#000` in Carbon) on the button's opacity.
    let fill = if exo_ui::theme(ui.ctx()) == exo_ui::Theme::Snow {
        Color32::WHITE
    } else {
        Color32::BLACK
    };
    let glyph = Rect::from_min_size(pos2(x + 7.0, line.y + 1.0), vec2(11.0, 14.0));
    exo_ui::paint_icon_on(painter, COPY, glyph, fill.gamma_multiply(shown));
    let compose = ui
        .interact(
            address,
            exo_ui::id(ui.ctx(), "thread-contact-compose"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if copy.clicked() {
        return Some(Clicked::Copied(contact_text(contact)));
    }
    if compose.clicked() {
        let pop_out = ui.input(|input| input.modifiers.shift_only());
        return Some(Clicked::ComposeTo(
            Person::new(
                contact.name.clone().unwrap_or_default(),
                contact.email.clone(),
            ),
            pop_out,
        ));
    }
    None
}

/// The subject's copy tooltip: its text at most 460px wide, then the icon 8px after it.
const SUBJECT_MAX: f32 = 460.0;
const SUBJECT_ICON_GAP: f32 = 8.0;
const SUBJECT_ICON: f32 = 11.0;

fn subject_width(ui: &Ui, subject: &str) -> f32 {
    TEXT.single_line(ui.painter(), subject, Color32::PLACEHOLDER, None)
        .size()
        .x
        .min(SUBJECT_MAX)
        + SUBJECT_ICON_GAP
        + SUBJECT_ICON
}

/// The subject's copy tooltip from `line`: the subject in 70% white (black in Carbon), `top:
/// -1px`, cut at 460px, underlined while hovered; the copy icon at 66% after it, `top: 1px`.
/// A click on either copies it.
fn subject_content(ui: &Ui, painter: &egui::Painter, line: Pos2, subject: &str) -> Option<Clicked> {
    let carbon = exo_ui::theme(ui.ctx()) == exo_ui::Theme::Carbon;
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
    let at = line - vec2(0.0, 1.0);
    let width = TEXT
        .single_line(painter, subject, Color32::PLACEHOLDER, None)
        .size()
        .x
        .min(SUBJECT_MAX);
    let text = Rect::from_min_size(at, vec2(width, TEXT.line_height));
    let hovered = ui.rect_contains_pointer(text);
    TEXT.paint_line(painter, at, subject, text_ink, Some(SUBJECT_MAX));
    if hovered {
        // Hovered, the text is underlined.
        let underline = Rect::from_min_size(
            pos2(at.x, at.y + TEXT.baseline(ui.ctx()) + 1.0),
            vec2(width, 1.0),
        );
        painter.rect_filled(exo_ui::pixel_snap(underline), 0.0, text_ink);
    }
    let icon = Rect::from_min_size(
        pos2(at.x + width + SUBJECT_ICON_GAP, line.y + 1.0),
        vec2(SUBJECT_ICON, 14.0),
    );
    exo_ui::paint_icon_on(painter, COPY, icon, icon_ink.gamma_multiply(0.66));
    let hit = Rect::from_min_max(text.min, pos2(icon.right(), text.bottom()));
    let copy = ui
        .interact(
            hit,
            exo_ui::id(ui.ctx(), "thread-subject-copy"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    copy.clicked().then(|| Clicked::Copied(subject.to_owned()))
}
