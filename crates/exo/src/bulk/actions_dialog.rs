//! The Bulk Actions dialog: this session's Get Me To Zero actions, each with its query, how many
//! conversations it changed, and Cancel or Undo while it can be.

use egui::{Color32, Context, CornerRadius, Rect, Sense, Ui, pos2};
use exo_ui::{Dialog, DialogResponse, Family, Font, TextStyle, palette};

use super::{Status, Summary};

/// The subheader: 12px/20px, 24px down (and `top: 3px`).
const TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
const SUBHEADER_TOP: f32 = 27.0;
/// A table row: 12px/20px semibold in 32px rows padded `8px 32px 4px`, the table 63px down and at
/// least 120px tall, the footer 8px under it.
const ROW_TEXT: TextStyle = TextStyle::new(Font::new(Family::Sans, 600), 12.0, 20.0);
const TABLE_TOP: f32 = 63.0;
const ROW: f32 = 32.0;
const ROW_PADDING_TOP: f32 = 8.0;
const TABLE_MIN: f32 = 120.0;
const TABLE_MAX: f32 = 210.0;
const TABLE_GAP: f32 = 8.0;
/// The focused row's fill, 16px into the dialog's padding.
const HIGHLIGHT_INSET: f32 = 16.0;
/// The footer: `padding-top: 20px` above a 20px line.
const FOOTER_PADDING: f32 = 20.0;
const FOOTER_HEIGHT: f32 = 41.0;
/// The columns' share of the row (`flex: 0 0 20%`, `10%` after a 4px margin, `17%`); the query
/// takes the rest.
const NAME: f32 = 0.20;
const COUNT: f32 = 0.10;
const COUNT_MARGIN: f32 = 4.0;
const ACTION: f32 = 0.17;
/// The spinner: 13px wide in Classic, `margin: 0 8px 0 0`, `top: 3px`.
const SPINNER: f32 = 13.0;
const SPINNER_GAP: f32 = 8.0;
const SPINNER_TOP: f32 = 3.0;

/// Which row has the keyboard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct State {
    focus: usize,
}

/// Something done in the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// A row was clicked (or Enter on it): its Cancel or Undo.
    Row(usize),
    /// ↓/J/Tab (1) or ↑/K/⇧Tab (-1).
    Move(isize),
    /// Enter.
    Activate,
    /// Close it.
    Close,
}

impl State {
    /// Applies `event` over `rows` rows; returns what's left for the app.
    pub(crate) fn handle(&mut self, event: Event, rows: usize) -> Option<Event> {
        match event {
            Event::Move(delta) if rows > 0 => {
                self.focus = (self.focus as isize + delta).rem_euclid(rows as isize) as usize
            }
            Event::Move(_) => {}
            Event::Activate => return (self.focus < rows).then_some(Event::Row(self.focus)),
            Event::Row(row) => {
                self.focus = row;
                return Some(event);
            }
            Event::Close => return Some(event),
        }
        None
    }
}

/// What a row's action column says, and whether the row can be clicked.
pub(crate) fn action(row: &Summary) -> (&'static str, bool) {
    match row.status {
        Status::Pending => ("Cancel", true),
        Status::Running => ("Undo", true),
        Status::Done if row.done == 0 => ("", false),
        Status::Done => ("Undo", true),
        Status::Undoing => ("Undoing", false),
        Status::Undone => ("Undone", false),
        Status::Failed => ("", false),
    }
}

/// How many conversations a row has changed, as the count column shows it: so far while running,
/// all while done, how far the undo is while undoing. A spinner goes before it while it runs or
/// undoes.
fn count(row: &Summary) -> String {
    let number = crate::format::thousands;
    match row.status {
        Status::Running | Status::Done => number(row.done),
        Status::Undoing => format!("{} of {}", number(row.done), number(row.total)),
        Status::Undone => number(row.total),
        Status::Pending | Status::Failed => String::new(),
    }
}

/// Draws the dialog over `area` while `open`, listing `rows`.
pub(crate) fn show(
    ctx: &Context,
    area: Rect,
    open: bool,
    state: &State,
    rows: &[Summary],
) -> DialogResponse<Option<Event>> {
    let table = if rows.is_empty() {
        0.0
    } else {
        (rows.len() as f32 * ROW).clamp(TABLE_MIN, TABLE_MAX) + TABLE_GAP
    };
    let height = TABLE_TOP + table + FOOTER_HEIGHT - 3.0;
    let dialog = Dialog::new("bulk-actions", "Bulk Actions", height);
    // With a table, the header is `rgba(0, 0, 0, .95)`.
    let dialog = if rows.is_empty() {
        dialog
    } else {
        dialog.header_ink(Color32::from_black_alpha(242))
    };
    dialog.show(ctx, area, open, |ui, body| {
        draw(ui, body, state, rows, table)
    })
}

fn draw(ui: &mut Ui, body: Rect, state: &State, rows: &[Summary], table: f32) -> Option<Event> {
    let tokens = palette(ui.ctx());
    let painter = ui.painter().clone();
    let subheader = if rows.is_empty() {
        "There are no recent bulk actions."
    } else {
        "Here are your bulk actions from the last 7 days."
    };
    // The subheader and the table: `rgba(0, 0, 0, .85)`; the query and count `.7`; the action `.5`
    // (`.7` on the focused row, `.3` when it can't be pressed).
    let ink = tokens.ink(0.85, 0.85);
    TEXT.paint_line(
        &painter,
        pos2(body.left(), body.top() + SUBHEADER_TOP),
        subheader,
        ink,
        None,
    );

    let mut event = None;
    let width = body.width();
    for (i, row) in rows.iter().enumerate() {
        let top = body.top() + TABLE_TOP + i as f32 * ROW;
        let rect = Rect::from_min_max(
            pos2(body.left() - HIGHLIGHT_INSET, top),
            pos2(body.right() + HIGHLIGHT_INSET, top + ROW),
        );
        let (label, editable) = action(row);
        let response = ui.interact(
            rect,
            exo_ui::id(ui.ctx(), ("bulk-action-row", row.token)),
            Sense::click(),
        );
        if response.hovered() && state.focus != i {
            event = Some(Event::Move(i as isize - state.focus as isize));
        }
        if response.clicked() && editable {
            event = Some(Event::Row(i));
        }
        let focused = state.focus == i;
        if focused {
            painter.rect_filled(
                rect,
                CornerRadius::same(4),
                crate::settings::block_highlight(ui.ctx()),
            );
        }
        let text_top = top + ROW_PADDING_TOP;
        ROW_TEXT.paint_line(
            &painter,
            pos2(body.left(), text_top),
            row.name,
            ink,
            Some(NAME * width),
        );
        let query_left = body.left() + NAME * width;
        let count_left = body.right() - (ACTION + COUNT) * width;
        if let Some(query) = &row.query {
            TEXT.paint_line(
                &painter,
                pos2(query_left, text_top),
                query,
                tokens.ink(0.7, 0.7),
                Some(count_left - COUNT_MARGIN - query_left),
            );
        }
        let count = count(row);
        let count_right = count_left + COUNT * width;
        let count_left = count_right - ROW_TEXT.width(ui.ctx(), &count);
        ROW_TEXT.paint_line(
            &painter,
            pos2(count_left, text_top),
            &count,
            tokens.ink(0.7, 0.7),
            None,
        );
        if matches!(row.status, Status::Running | Status::Undoing) {
            // The row's spinner: 13px, 8px before the count, its bottom on the text's baseline and
            // then `top: 3px`; black at .3 in both themes, turning.
            let bottom = text_top + ROW_TEXT.baseline(ui.ctx()) + SPINNER_TOP;
            let spinner = Rect::from_min_max(
                pos2(count_left - SPINNER_GAP - SPINNER, bottom - SPINNER),
                pos2(count_left - SPINNER_GAP, bottom),
            );
            exo_ui::paint_spinner(
                &painter,
                spinner,
                ui.input(|input| input.time),
                Color32::from_black_alpha(77),
            );
        }
        let action_ink = match (editable, focused) {
            (false, _) => tokens.ink(0.3, 0.3),
            (true, true) => tokens.ink(0.7, 0.7),
            (true, false) => tokens.ink(0.5, 0.5),
        };
        ROW_TEXT.paint_line(
            &painter,
            pos2(body.right() - ROW_TEXT.width(ui.ctx(), label), text_top),
            label,
            action_ink,
            None,
        );
    }

    // `Would you like to clean up your inbox? Hit ⌘ K → Get Me To Zero.`
    // The footer's first key has 2px before it.
    crate::settings::paint_dialog_footer(
        ui,
        body.left(),
        body.top() + TABLE_TOP + table + FOOTER_PADDING,
        "Would you like to clean up your inbox? Hit ",
        2.0,
        1.0,
        " → Get Me To Zero.",
    );
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_wraps_and_enter_takes_the_focused_row() {
        let mut state = State::default();
        assert_eq!(state.handle(Event::Move(-1), 3), None);
        assert_eq!(state.handle(Event::Activate, 3), Some(Event::Row(2)));
        assert_eq!(state.handle(Event::Move(1), 3), None);
        assert_eq!(state.handle(Event::Activate, 3), Some(Event::Row(0)));
        assert_eq!(state.handle(Event::Activate, 0), None, "no rows");
    }
}
