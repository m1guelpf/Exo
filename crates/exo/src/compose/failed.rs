//! Failed drafts: a send that failed stays in its thread as a message whose header says why,
//! and lifts its row to the top of the Inbox, until the user deals with it. A click on that
//! header opens `RecoverDraft`: `Send failed`, no input, `Edit Message` and `Discard`. So does a
//! click on `Send: when online`, a reply being sent while Gmail can't be reached, and on a
//! scheduled send's `Scheduled: {date}`, with the prompt `Send message`, the scheduled one with
//! `Reschedule` first.

use exo_commands::{Command, SubPalette, Toast};
use exo_store::{DraftAction, StoredDraft};

use super::{Action, Discarded, Open, Surface};
use crate::{Mail, commands};

impl Mail {
    /// The failed draft whose Gmail copy is message `message`.
    fn failed_draft(&self, message: &str) -> Option<&StoredDraft> {
        self.snapshot
            .drafts
            .iter()
            .find(|draft| draft.failure.is_some() && draft.message_id.as_deref() == Some(message))
    }

    /// The scheduled send whose Gmail copy is message `message`.
    fn scheduled_draft(&self, message: &str) -> Option<&StoredDraft> {
        self.snapshot.drafts.iter().find(|draft| {
            draft.send_at.is_some()
                && draft.failure.is_none()
                && draft.record.send_later.is_some()
                && draft.message_id.as_deref() == Some(message)
        })
    }

    /// A click on the status in message `message`'s header: `RecoverDraft` for its draft, failed,
    /// scheduled or being sent.
    pub(super) fn recover(&mut self, message: &str) {
        let mut rows = Vec::new();
        let (prompt, edit, discard) = if let Some(draft) = self.failed_draft(message) {
            (
                "Send failed",
                Action::EditFailed(draft.id.clone()),
                Action::DiscardFailed(draft.id.clone()),
            )
        } else if let Some(draft) = self.scheduled_draft(message) {
            // `Reschedule` and `Edit Message` both take it back to edit.
            rows.push(Command::new(
                "RESCHEDULE",
                "Reschedule",
                commands::Action::Compose(Action::EditScheduled(draft.id.clone())),
            ));
            (
                "Send message",
                Action::EditScheduled(draft.id.clone()),
                Action::DiscardScheduled(draft.id.clone()),
            )
        } else if let Some(draft) = self.compose.in_thread.draft_of(message) {
            (
                "Send message",
                Action::EditSending(draft.to_owned()),
                Action::DiscardSending(draft.to_owned()),
            )
        } else {
            return;
        };
        rows.extend([
            Command::new(
                "EDIT_MESSAGE",
                "Edit Message",
                commands::Action::Compose(edit),
            ),
            Command::new("DISCARD", "Discard", commands::Action::Compose(discard)),
        ]);
        self.commands.open_sub_palette(
            SubPalette::new("RecoverDraft", prompt, "data/send-message", rows).hide_input(),
        );
    }

    /// `Discard` on a send waiting to go, which cancels its send job: it goes, with the toast
    /// whose Undo brings it back as a draft.
    pub(super) fn discard_sending(&mut self, id: &str) {
        let Some(at) = self
            .compose
            .sends
            .iter()
            .position(|sending| sending.open.id == id)
        else {
            return;
        };
        let sending = self.compose.sends.remove(at);
        self.compose.in_thread.forget(id);
        self.account.discard_draft(id);
        let tag = self.compose.tag();
        self.compose.discarded = Some(Discarded {
            tag,
            open: sending.open,
            surface: sending.surface,
        });
        self.commands.toast(
            Toast::new("Draft discarded.")
                .undo(commands::Action::Compose(Action::UndoDiscard(tag)))
                .tag(tag),
        );
    }

    /// `Edit Message`: the failed send is called off, and the draft opens to edit where it was
    /// written.
    pub(super) fn edit_failed(&mut self, id: &str) {
        let Some(stored) = self
            .snapshot
            .drafts
            .iter()
            .find(|draft| draft.id == id)
            .cloned()
        else {
            return;
        };
        self.compose.sends.retain(|sending| sending.open.id != id);
        self.account.recover_draft(id);
        self.reopen(Open::stored(&stored), surface(&stored));
    }

    /// `Edit Message` (or `Reschedule`) on a scheduled send: the send is called off, and the
    /// draft opens to edit where it was written, its time still set.
    pub(super) fn edit_scheduled(&mut self, id: &str) {
        let Some(stored) = self
            .snapshot
            .drafts
            .iter()
            .find(|draft| draft.id == id)
            .cloned()
        else {
            return;
        };
        self.compose.sends.retain(|sending| sending.open.id != id);
        self.account.cancel_send(id);
        // The send job goes, and with it what an `if no reply` would have watched for.
        let mut open = Open::stored(&stored);
        if let Some(later) = &mut open.record.send_later {
            later.current_message_ids.clear();
        }
        self.reopen(open, surface(&stored));
    }

    /// `Discard`, with its toast, on a failed or scheduled send: Undo brings it back as a draft
    /// to edit. A new message's conversation was only the draft, so it leaves the screen.
    pub(super) fn discard_failed(&mut self, id: &str) {
        let Some(stored) = self
            .snapshot
            .drafts
            .iter()
            .find(|draft| draft.id == id)
            .cloned()
        else {
            return;
        };
        self.compose.sends.retain(|sending| sending.open.id != id);
        self.account.discard_draft(id);
        let on_screen = self.thread.as_ref().is_some_and(|view| {
            stored
                .thread
                .as_ref()
                .is_some_and(|thread| thread.as_str() == view.id())
        });
        if on_screen && stored.record.action == DraftAction::Compose {
            self.close_thread();
        }
        let tag = self.compose.tag();
        self.compose.discarded = Some(Discarded {
            tag,
            open: Open::stored(&stored),
            surface: surface(&stored),
        });
        self.commands.toast(
            Toast::new("Draft discarded.")
                .undo(commands::Action::Compose(Action::UndoDiscard(tag)))
                .tag(tag),
        );
    }

    /// `See Message` on a send's failure alert: the failed draft in its thread. Returns whether
    /// there was one; a send Gmail had no copy of is a draft again, for compose to reopen.
    pub(super) fn see_failed(&mut self, id: &str) -> bool {
        let Some((thread, message)) = self
            .snapshot
            .drafts
            .iter()
            .find(|draft| draft.id == id && draft.failure.is_some())
            .and_then(|draft| Some((draft.thread.clone()?, draft.message_id.clone()?)))
        else {
            return false;
        };
        if self
            .thread
            .as_ref()
            .is_none_or(|view| view.id() != thread.as_str())
        {
            self.open_thread(thread.as_str().to_owned());
        }
        if let Some(view) = &mut self.thread {
            view.reveal_draft(&message);
        }
        true
    }
}

/// Where a draft opens again: a new message in its pane, a reply under its message.
fn surface(draft: &StoredDraft) -> Surface {
    if draft.record.action == DraftAction::Compose {
        Surface::Pane
    } else {
        Surface::Inline
    }
}
