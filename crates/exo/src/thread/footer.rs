//! The hint under a message: under the only message of a conversation that introduces you to
//! someone, `Is this an intro? Hit ⌘ shift I to instantly move {first name} to BCC!`; under a new
//! message started in a conversation, `Referring to the last conversation? Hit ⌘ shift P to pop
//! out this draft!`. Its keys are inline blue caps.

use std::sync::LazyLock;

use egui::{Ui, pos2};
use exo_store::MessageDetail;
use exo_ui::{Family, Font, KeycapColors, Keycaps, TextStyle};
use regex::Regex;

use super::Layout;
use crate::format::names;

/// The hint: 12px bold on a 24px line, `top: 1px`, `padding-bottom: 28px`.
const HINT: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 24.0);
pub(super) const HEIGHT: f32 = 24.0 + 28.0;
/// `margin-top: -16px` under the card's 28px margin; -12px under a draft's.
pub(super) const MARGIN_TOP: f32 = -16.0;
pub(super) const MARGIN_TOP_UNDER_DRAFT: f32 = -12.0;
/// The caps' `margin-right: 4px` after them, their 4px before each, and their `top: 3px` on the
/// line (the labels back up 3px).
const KEYS_GAP: f32 = 4.0;
const KEYS_DROP: f32 = 3.0;

/// The first name the intro hint moves to Bcc, for `message`: to you, not from you,
/// with two people or more on it besides its sender, and no invitation; and its subject says
/// it's an intro (`<>`, `->`, `meet `, `intro`) or names both of two.
pub(super) fn intro(message: &MessageDetail) -> Option<String> {
    static SAYS_INTRO: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)<[ \-–—]*>|[-–—]+\s*>|\bmeet |\bintro").expect("valid regex")
    });
    let from = message.from.as_ref().filter(|from| !from.is_me)?;
    let recipients: Vec<_> = message
        .to
        .iter()
        .chain(&message.cc)
        .chain(&message.bcc)
        .collect();
    let others: Vec<_> = recipients
        .iter()
        .filter(|contact| !contact.email.eq_ignore_ascii_case(&from.email))
        .collect();
    let invite = super::invite::attachment(message).is_some();
    if !recipients.iter().any(|contact| contact.is_me) || others.len() < 2 || invite {
        return None;
    }
    let names_both = others.len() == 2
        && others.iter().all(|contact| {
            contact
                .name
                .as_deref()
                .is_some_and(|name| !name.is_empty() && message.subject.contains(name))
        });
    if !SAYS_INTRO.is_match(&message.subject) && !names_both {
        return None;
    }
    // From the name, else the address's local part.
    let name = from
        .name
        .clone()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| from.email.split('@').next().unwrap_or_default().to_owned());
    Some(names::first_name(&name, &from.email))
}

/// Paints the intro hint for `first_name`, its line box's top at `top`.
pub(super) fn paint_intro(ui: &Ui, layout: &Layout, top: f32, first_name: &str) {
    paint(
        ui,
        layout,
        top,
        "Is this an intro? Hit",
        &["⌘", "shift", "I"],
        &format!("to instantly move {first_name} to BCC!"),
    );
}

/// Paints the pop-out hint, its line box's top at `top`.
pub(super) fn paint_pop_out(ui: &Ui, layout: &Layout, top: f32) {
    paint(
        ui,
        layout,
        top,
        "Referring to the last conversation? Hit",
        &["⌘", "shift", "P"],
        "to pop out this draft!",
    );
}

/// A hint's line: `lead`, the `keys`, then `rest`, cut at the column's end.
fn paint(ui: &Ui, layout: &Layout, top: f32, lead: &str, keys: &[&str], rest: &str) {
    let ctx = ui.ctx();
    let palette = exo_ui::palette(ctx);
    let painter = ui.painter();
    // `color: rgba(0,0,0,.35)`, white in Carbon.
    let ink = palette.ink(0.35, 0.35);
    let line = pos2(layout.inner.left(), top + 1.0);
    let lead = HINT.paint_line(painter, line, lead, ink, None);
    let keys = Keycaps::new(ctx, keys, KeycapColors::shortcuts_pane(palette));
    keys.paint(painter, line.x + lead, line.y + KEYS_DROP);
    let after = line.x + lead + keys.width() + KEYS_GAP;
    HINT.paint_line(
        painter,
        pos2(after, line.y),
        rest,
        ink,
        Some((layout.inner.right() - after).max(0.0)),
    );
}
