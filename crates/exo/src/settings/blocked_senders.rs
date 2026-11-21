//! Blocked Senders: the addresses and domains the account blocks, each with an Unblock behind an
//! `Unblock` alert. Unblocking takes it off the list and deletes the Gmail filter its block made;
//! what the filter trashed stays in Trash.

use egui::{Color32, Context, CornerRadius, Rect, Sense, Ui, pos2};
use exo_ui::{Alert, Dialog, Family, Font, KeycapColors, Keycaps, TextStyle, palette};

/// The subheader: 12px/20px, 24px down (and `top: 3px`).
const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
const SUBHEADER_TOP: f32 = 27.0;
/// A sender and its Unblock: 12px/20px bold, 7px into a 32px row.
const SENDER: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 20.0);
const ROW: f32 = 32.0;
const ROW_TEXT: f32 = 7.0;
/// The list's top: 20px under a one-line subheader, 88px under a two-line one.
const LIST_GAP: f32 = 21.0;
/// The focused row's fill, 16px into the dialog's padding, 4px corners.
const HIGHLIGHT_INSET: f32 = 16.0;
/// The footer's line, 22px under the list (or the subheader); the body ends 18px below its top.
const FOOTER_GAP: f32 = 22.0;
const EMPTY_FOOTER_GAP: f32 = 19.0;
const FOOTER_BELOW: f32 = 18.0;
/// `Unblock`: `#000` at `opacity: .5` on the focused row (`:focus`), hidden on the others.
const UNBLOCK_FOCUSED: f32 = 0.5;

/// The focused row's fill: `hsl(220, 45%, 97%)`, `#eaeaea` in Carbon; Bulk
/// Actions' rows take it too.
pub(crate) fn highlight(ctx: &Context) -> Color32 {
    match exo_ui::app_theme(ctx) {
        exo_ui::Theme::Snow => exo_ui::Theme::Snow.palette().fill_hover_default,
        exo_ui::Theme::Carbon => Color32::from_rgb(0xea, 0xea, 0xea),
    }
}

/// A dialog's footer line with `⌘ K` in it, the caps' tops at `top`: `rgba(0, 0, 0, .6)`, the
/// caps 19px and in the app's theme (dark in Carbon), which drops the text 1px. The first cap sits
/// `first_gap` after `lead` (a cap's own 4px, Bulk Actions' 2px), and `rest` `after_keys` after the
/// last (Blocked Senders' keeps a 1px right margin, the Images dialog's none). (Blocked Senders'
/// footer is a flex row, which trims its lead's trailing space.)
pub(crate) fn paint_footer(
    ui: &Ui,
    left: f32,
    top: f32,
    lead: &str,
    first_gap: f32,
    after_keys: f32,
    rest: &str,
) {
    let painter = ui.painter();
    let ink = Color32::from_black_alpha(153);
    let mut x = left + TEXT.paint_line(painter, pos2(left, top + 1.0), lead, ink, None);
    let keys = Keycaps::new(
        ui.ctx(),
        &["⌘", "K"],
        KeycapColors::shortcuts_pane(exo_ui::app_theme(ui.ctx()).palette()),
    )
    .height(19.0);
    x += first_gap - exo_ui::KEYCAP_GAP;
    keys.paint(painter, x, top);
    x += keys.width() + after_keys;
    TEXT.paint_line(painter, pos2(x, top + 1.0), rest, ink, None);
}

/// The dialog's row focus and its alert.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    focus: usize,
    /// `Unblock` is being asked about this sender, with this action selected.
    alert: Option<(String, usize)>,
}

/// Something done in the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A row was clicked: ask about unblocking it.
    Row(usize),
    /// ↓/J/Tab (1) or ↑/K/⇧Tab (-1).
    Move(isize),
    /// Enter: the focused row.
    Activate,
    /// The alert's action, by index (`Unblock` first).
    AlertAction(usize),
    /// ← / → in the alert.
    AlertCycle(isize),
    /// Escape or a click outside the alert.
    AlertDismiss,
}

impl State {
    /// An alert is up: it has the keyboard.
    pub fn alert_open(&self) -> bool {
        self.alert.is_some()
    }

    /// Back to the first row, as the dialog opens.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Applies `event` over `senders`; returns the sender to unblock once the alert says so.
    pub fn handle(&mut self, event: Event, senders: &[&str]) -> Option<String> {
        match event {
            Event::Move(delta) if !senders.is_empty() => {
                self.focus =
                    (self.focus as isize + delta).rem_euclid(senders.len() as isize) as usize
            }
            Event::Move(_) => {}
            Event::Activate => return self.handle(Event::Row(self.focus), senders),
            Event::Row(row) => {
                if let Some(sender) = senders.get(row) {
                    self.focus = row;
                    self.alert = Some(((*sender).to_owned(), 0));
                }
            }
            Event::AlertAction(index) => {
                let (sender, _) = self.alert.take()?;
                return (index == 0).then_some(sender);
            }
            Event::AlertCycle(step) => {
                if let Some((_, selected)) = &mut self.alert {
                    *selected = selected.saturating_add_signed(step).min(1);
                }
            }
            Event::AlertDismiss => self.alert = None,
        }
        None
    }

    /// Keeps the focus on a row after one goes.
    pub fn clamp(&mut self, rows: usize) {
        self.focus = self.focus.min(rows.saturating_sub(1));
    }
}

/// The blocked senders the dialog lists: the setting's, each once whatever its case, in order.
pub fn senders(blocked: &[String]) -> Vec<&str> {
    let mut seen = std::collections::HashSet::new();
    blocked
        .iter()
        .filter(|sender| seen.insert(sender.to_lowercase()))
        .map(String::as_str)
        .collect()
}

/// Draws the dialog over `area` while `open`, listing `senders`, and its alert.
pub fn show(
    ctx: &Context,
    area: Rect,
    open: bool,
    state: &State,
    senders: &[&str],
) -> exo_ui::DialogResponse<Option<Event>> {
    let subheader = subheader(senders);
    let lines = TEXT.wrap(ctx, subheader, exo_ui::DIALOG_WIDTH - 64.0).len();
    let list_top = SUBHEADER_TOP + lines as f32 * TEXT.line_height + LIST_GAP;
    // With no list, the footer sits 19px under the subheader.
    let footer = if senders.is_empty() {
        list_top - LIST_GAP + EMPTY_FOOTER_GAP
    } else {
        list_top + senders.len() as f32 * ROW + FOOTER_GAP
    };
    let mut response = Dialog::new("blocked-senders", "Blocked Senders", footer + FOOTER_BELOW)
        .show(ctx, area, open, |ui, body| {
            draw(ui, body, state, senders, list_top, footer)
        });
    if let (Some(event), Some((sender, selected))) = (&mut response.inner, &state.alert) {
        let text = format!("Remove the block on {sender}?");
        let alert = Alert {
            title: "Unblock",
            text: &text,
            actions: &["Unblock", "Cancel"],
            selected: *selected,
        };
        let clicked = egui::Area::new(exo_ui::id(ctx, "blocked-senders-alert"))
            .order(egui::Order::Tooltip)
            .fixed_pos(area.min)
            .show(ctx, |ui| {
                // The alert's backdrop, over the dialog's.
                let backdrop = ui.allocate_rect(area, Sense::click());
                ui.painter()
                    .rect_filled(area, 0.0, Color32::from_black_alpha(128));
                match alert.show(ui, area) {
                    Some(index) => Some(Event::AlertAction(index)),
                    None => backdrop.clicked().then_some(Event::AlertDismiss),
                }
            });
        *event = clicked.inner.or(event.take());
        response.close = false;
    }
    response
}

/// `There are no blocked senders.`, or what blocking does.
fn subheader(senders: &[&str]) -> &'static str {
    if senders.is_empty() {
        "There are no blocked senders."
    } else {
        "All mail from the senders below will be moved to Trash. Messages in Trash for more than 30 days will be deleted automatically."
    }
}

fn draw(
    ui: &mut Ui,
    body: Rect,
    state: &State,
    senders: &[&str],
    list_top: f32,
    footer: f32,
) -> Option<Event> {
    let tokens = palette(ui.ctx());
    let painter = ui.painter().clone();
    for (i, line) in TEXT
        .wrap(ui.ctx(), subheader(senders), body.width())
        .iter()
        .enumerate()
    {
        // The subheader: `rgba(0, 0, 0, .85)`.
        TEXT.paint_line(
            &painter,
            pos2(
                body.left(),
                body.top() + SUBHEADER_TOP + i as f32 * TEXT.line_height,
            ),
            line,
            tokens.ink(0.85, 0.85),
            None,
        );
    }

    let mut event = None;
    for (i, sender) in senders.iter().enumerate() {
        let top = body.top() + list_top + i as f32 * ROW;
        let rect = Rect::from_min_max(
            pos2(body.left() - HIGHLIGHT_INSET, top),
            pos2(body.right() + HIGHLIGHT_INSET, top + ROW),
        );
        let response = ui
            .interact(
                rect,
                exo_ui::id(ui.ctx(), ("blocked-sender", i)),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        // Hovering a row moves the focus to it.
        if response.hovered() && state.focus != i && state.alert.is_none() {
            event = Some(Event::Move(i as isize - state.focus as isize));
        }
        if response.clicked() {
            event = Some(Event::Row(i));
        }
        let focused = state.focus == i;
        if focused {
            painter.rect_filled(rect, CornerRadius::same(4), highlight(ui.ctx()));
        }
        // The sender: `rgba(0, 0, 0, .85)`.
        SENDER.paint_line(
            &painter,
            pos2(body.left(), top + ROW_TEXT),
            sender,
            tokens.ink(0.85, 0.85),
            Some(body.width() - 60.0),
        );
        if focused {
            let width = SENDER.width(ui.ctx(), "Unblock");
            SENDER.paint_line(
                &painter,
                pos2(body.right() - width, top + ROW_TEXT),
                "Unblock",
                tokens.ink(UNBLOCK_FOCUSED, UNBLOCK_FOCUSED),
                None,
            );
        }
    }

    // `To block a sender, open a message from them, hit ⌘ K → Block.`
    paint_footer(
        ui,
        body.left(),
        body.top() + footer,
        "To block a sender, open a message from them, hit",
        exo_ui::KEYCAP_GAP,
        1.0,
        " → Block.",
    );
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn senders_show_once_whatever_their_case() {
        let blocked = [
            "spam@x.com".to_owned(),
            "Zed@y.com".to_owned(),
            "SPAM@x.com".to_owned(),
        ];
        assert_eq!(senders(&blocked), ["spam@x.com", "Zed@y.com"]);
    }

    #[test]
    fn unblocking_asks_first() {
        let senders = ["a@x.com", "b@y.com"];
        let mut state = State::default();
        state.handle(Event::Move(-1), &senders);
        assert_eq!(
            state.handle(Event::Activate, &senders),
            None,
            "the alert first"
        );
        assert!(state.alert_open());
        assert_eq!(
            state.handle(Event::AlertAction(1), &senders),
            None,
            "Cancel"
        );
        state.handle(Event::Row(0), &senders);
        assert_eq!(
            state.handle(Event::AlertAction(0), &senders).as_deref(),
            Some("a@x.com")
        );
    }
}
