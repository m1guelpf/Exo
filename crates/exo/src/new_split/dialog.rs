//! The New Split dialog: a name, and optionally words the subject must contain, for a split of a
//! mailing list's or a service's mail (`New Split Inbox: {list}`).
//!
//! Its metrics are the other dialogs'.

use egui::{Context, Rect, Ui, pos2, vec2};
use exo_store::{SplitInbox, clean_whitespace, name_error};
use exo_ui::{
    ActionKind, DIALOG_ACTION_HEIGHT, DIALOG_WIDTH, Dialog, DialogAction, DialogInput,
    DialogResponse, Family, Font, TextStyle, palette,
};

use crate::Mail;

/// The subheader: 12px/20px, `padding-top: 24px`, `margin-bottom: 16px`, `rgba(0, 0, 0, .85)`.
const SUBHEADER: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 12.0, 20.0);
/// An input: 12px/20px bold.
const INPUT: TextStyle = TextStyle::new(Font::new(Family::Sans, 700), 12.0, 20.0);
/// The dialog text's `position: relative; top: 3px`.
const NUDGE: f32 = 3.0;
const CONTENT_WIDTH: f32 = DIALOG_WIDTH - 64.0;
/// A subheader and its field: `24 + 20 + 16` px, then the 20px input.
const FIELD: f32 = 80.0;
/// The buttons: `margin-top: 44px`, 28px tall; the card ends 20px under them, 4px short of their
/// `margin-bottom: 7px` and the dialog's padding.
const ACTIONS_TOP: f32 = 44.0;
const BODY_HEIGHT: f32 = 2.0 * FIELD + ACTIONS_TOP + DIALOG_ACTION_HEIGHT + 3.0;

/// Something done in the dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// `Create Split`, or Enter anywhere in it.
    Save,
    /// `Cancel`, Escape, the × or the backdrop.
    Close,
}

/// The dialog's fields.
#[derive(Debug)]
pub(crate) struct State {
    /// The split's definition: the list's address, or the service's query.
    query: String,
    name: String,
    subject: String,
    /// The name was refused since it last changed.
    name_error: bool,
    /// Still open; it fades out after.
    open: bool,
    /// The name field takes the keyboard on the next frame.
    focus: bool,
}

impl State {
    /// The dialog for a split defined by `query`.
    pub(crate) fn new(query: String) -> Self {
        Self {
            query,
            name: String::new(),
            subject: String::new(),
            name_error: false,
            open: true,
            focus: true,
        }
    }

    /// The split to create: its name and definition, with ` subject:"…"` for a subject (its first
    /// `"` dropped); `None` when the name is empty, a built-in split's or taken, which marks it.
    fn save(&mut self, splits: &[SplitInbox]) -> Option<(String, String)> {
        let name = clean_whitespace(&self.name);
        if name_error(&name, splits).is_some() {
            self.name_error = true;
            self.focus = true;
            return None;
        }
        let subject = clean_whitespace(&self.subject);
        let query = if subject.is_empty() {
            self.query.clone()
        } else {
            format!(
                "{} subject:\"{}\"",
                self.query,
                subject.replacen('"', "", 1)
            )
        };
        Some((name, query))
    }

    /// Draws the dialog over `area`; returns what was done in it.
    fn show(&mut self, ctx: &Context, area: Rect) -> DialogResponse<Option<Event>> {
        let header = format!("New Split: {}", self.query);
        let open = self.open;
        Dialog::new("create-split-inbox", &header, BODY_HEIGHT)
            .show(ctx, area, open, |ui, body| self.draw(ui, body))
    }

    fn draw(&mut self, ui: &mut Ui, body: Rect) -> Option<Event> {
        let ctx = ui.ctx().clone();
        let ink = palette(&ctx).ink(0.85, 0.85);
        let left = body.left();
        let field = |ui: &mut Ui, top: f32, label: &str, text: &mut String, error: bool| {
            SUBHEADER.paint_line(
                ui.painter(),
                pos2(left, top + 24.0 + NUDGE),
                label,
                ink,
                None,
            );
            let line =
                Rect::from_min_size(pos2(left, top + 60.0 + NUDGE), vec2(CONTENT_WIDTH, 20.0));
            let before = text.clone();
            let response = DialogInput::new(INPUT, "").error(error).show(
                ui,
                exo_ui::id(ui.ctx(), ("create-split-field", label)),
                text,
                line,
            );
            (response, *text != before)
        };
        let (name, renamed) = field(
            ui,
            body.top(),
            "Split name:",
            &mut self.name,
            self.name_error,
        );
        if renamed {
            self.name_error = false;
        }
        if std::mem::take(&mut self.focus) {
            name.request_focus();
        }
        let _ = field(
            ui,
            body.top() + FIELD,
            "(Optional) subject contains:",
            &mut self.subject,
            false,
        );

        // `Cancel` (8px left of the other) and `Create Split`, right-aligned: the one with the
        // keyboard is filled, `Create Split` while a field has it.
        let top = body.top() + 2.0 * FIELD + ACTIONS_TOP + NUDGE;
        let cancel_id = exo_ui::id(&ctx, ("create-split-action", "cancel"));
        let cancel_focused = ctx.memory(|memory| memory.has_focus(cancel_id));
        let mut right = body.right() + 13.0;
        let mut event = None;
        for (id, label, selected, clicked) in [
            ("create", "Create Split", !cancel_focused, Event::Save),
            ("cancel", "Cancel", cancel_focused, Event::Close),
        ] {
            // The selected button fills and glows.
            let action = DialogAction::new(
                label,
                if selected {
                    ActionKind::Primary
                } else {
                    ActionKind::Plain
                },
            )
            .focused(selected);
            let width = action.width(&ctx);
            let rect =
                Rect::from_min_size(pos2(right - width, top), vec2(width, DIALOG_ACTION_HEIGHT));
            let response = action.show(
                ui,
                if id == "cancel" {
                    cancel_id
                } else {
                    exo_ui::id(&ctx, ("create-split-action", id))
                },
                rect,
            );
            if response.clicked() {
                event = Some(clicked);
            }
            right = rect.left() - 8.0;
        }
        event
    }
}

impl Mail {
    /// Carries out `event` in the New Split dialog.
    pub(crate) fn new_split_dialog(&mut self, event: Event, now: f64) {
        let Some(state) = self.new_split.dialog.as_mut().filter(|state| state.open) else {
            return;
        };
        match event {
            Event::Close => state.open = false,
            Event::Save => {
                let splits: Vec<SplitInbox> = self.account.settings().splits().cloned().collect();
                if let Some((name, query)) = state.save(&splits) {
                    state.open = false;
                    self.create_split(&name, &query, now);
                }
            }
        }
    }

    /// The definition the New Split dialog is open for. For tests.
    pub fn new_split_dialog_query(&self) -> Option<&str> {
        self.new_split
            .dialog
            .as_ref()
            .filter(|state| state.open)
            .map(|state| state.query.as_str())
    }

    /// The New Split dialog has the keyboard.
    pub(crate) fn new_split_dialog_open(&self) -> bool {
        self.new_split
            .dialog
            .as_ref()
            .is_some_and(|state| state.open)
    }

    /// Draws the New Split dialog, if one shows, over `area`.
    pub(crate) fn show_new_split_dialog(&mut self, ctx: &Context, area: Rect) {
        let Some(state) = self.new_split.dialog.as_mut() else {
            return;
        };
        let response = state.show(ctx, area);
        let Some(event) = response.inner else {
            self.new_split.dialog = None;
            return;
        };
        let now = ctx.input(|input| input.time);
        if response.close {
            self.new_split_dialog(Event::Close, now);
        } else if let Some(event) = event {
            self.new_split_dialog(event, now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subject_narrows_the_definition_and_a_taken_name_is_refused() {
        let mut state = State::new("rust-users@lists.example".into());
        state.name = " Rust  users ".into();
        state.subject = "\"weekly\" digest".into();
        assert_eq!(
            state.save(&[]),
            Some((
                "Rust users".into(),
                "rust-users@lists.example subject:\"weekly\" digest\"".into()
            ))
        );
        state.name = "Inbox".into();
        assert_eq!(state.save(&[]), None);
        assert!(state.name_error);
    }
}
