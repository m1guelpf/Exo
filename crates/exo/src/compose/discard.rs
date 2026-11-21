//! Discard Draft(s) from the list or a thread: `⌘⇧,` discards every draft of the conversations
//! actions apply to, with a toast; Undo brings them all back, and one draft reopens where it was
//! written.

use exo_commands::{Command, Commands, Region, Toast};
use exo_store::{DraftAction, StoredDraft};

use super::{Action, Discarded, Open, Surface, save};
use crate::{Mail, commands};

/// The list's and the thread's regions: in a draft, compose's own `Discard Draft` has the keys.
const REGIONS: &[Region] = &[
    Region::ThreadList,
    Region::ThreadMessagesHeader,
    Region::ThreadMessages,
    Region::ThreadMessageCalendarInvite,
    Region::ActionPane,
    Region::ReadOnlySnippet,
];

/// Registers `Discard Draft(s)` while the conversations actions apply to have `drafts` drafts in
/// `threads` of them (`DISCARD_DRAFT`); for a selection that reaches rows not loaded (`exact`
/// false) only its keys, which say why it can't.
pub(crate) fn register(commands: &mut Commands<commands::Action>, threads: usize, exact: bool) {
    if threads == 0 {
        return;
    }
    let action = commands::Action::Compose(Action::DiscardListed);
    let keys = ["mod+shift+,", "mod+shift+."];
    if !exact {
        commands.add(
            Command::shortcut("DISCARD_DRAFT", action)
                .keys(keys)
                .regions(REGIONS),
        );
        return;
    }
    let (title, aliases) = if threads > 1 {
        (
            "Discard Drafts",
            ["Delete Drafts", "Trash Drafts", "Abandon Drafts"],
        )
    } else {
        (
            "Discard Draft",
            ["Delete Draft", "Trash Draft", "Abandon Draft"],
        )
    };
    commands.add(
        Command::new("DISCARD_DRAFT", title, action)
            .keys(keys)
            .key_code(["⌘", "shift", ","])
            .aliases(aliases)
            .default_score(6.1)
            .scale(1.1)
            .icon("data/discard")
            .multiple()
            .regions(REGIONS),
    );
}

impl Mail {
    /// Exo's drafts in the conversations actions apply to, sends being held aside, and how many
    /// conversations have one.
    pub(crate) fn actionable_drafts(&self) -> (Vec<StoredDraft>, usize) {
        let ids: Vec<String> = crate::actionable(self.thread.as_ref(), &self.inbox)
            .iter()
            .map(|thread| thread.id.clone())
            .collect();
        let drafts: Vec<StoredDraft> = self
            .snapshot
            .drafts
            .iter()
            .filter(|draft| {
                draft.send_at.is_none()
                    && draft
                        .thread
                        .as_ref()
                        .is_some_and(|thread| ids.iter().any(|id| id == thread.as_str()))
            })
            .cloned()
            .collect();
        let threads = drafts
            .iter()
            .filter_map(|draft| draft.thread.as_ref())
            .collect::<std::collections::HashSet<_>>()
            .len();
        (drafts, threads)
    }

    /// `⌘⇧,` in the list or a thread: discards the actionable conversations' drafts, or, for a selection
    /// that reaches rows not loaded, says it can't.
    pub(crate) fn discard_listed(&mut self) {
        if self.refuse_unloaded("discard drafts") {
            return;
        }
        let (drafts, _) = self.actionable_drafts();
        if drafts.is_empty() {
            return;
        }
        let mut discarded: Vec<Open> = Vec::new();
        for draft in &drafts {
            // One open in compose closes there.
            let open = match self
                .compose
                .drafts
                .iter()
                .position(|open| open.id == draft.id)
            {
                Some(at) => {
                    let mut open = self.compose.drafts.remove(at);
                    open.sync_body();
                    open
                }
                None => Open::stored(draft),
            };
            if self.compose.focus.as_deref() == Some(draft.id.as_str()) {
                self.compose.focus = None;
            }
            if self.compose.popped.as_deref() == Some(draft.id.as_str()) {
                self.compose.popped = None;
            }
            self.account.discard_draft(&draft.id);
            discarded.push(open);
        }
        let tag = self.compose.tag();
        let toast = match discarded.len() {
            1 => {
                let open = discarded.remove(0);
                // Undo reopens it where drafts open: a new message in its pane, a reply under its
                // message.
                let surface = if open.record.action == DraftAction::Compose {
                    Surface::Pane
                } else {
                    Surface::Inline
                };
                self.compose.discarded = Some(Discarded { tag, open, surface });
                Toast::new("Draft discarded.")
                    .undo(commands::Action::Compose(Action::UndoDiscard(tag)))
            }
            count => {
                self.compose.discards = Some((tag, discarded));
                Toast::new(format!("{count} drafts discarded."))
                    .undo(commands::Action::Compose(Action::UndoDiscards(tag)))
            }
        };
        self.commands.toast(toast.tag(tag));
    }

    /// The toast's Undo on several discarded drafts: each is saved again.
    pub(crate) fn undo_discards(&mut self, tag: u64) {
        let Some((_, drafts)) = self
            .compose
            .discards
            .take_if(|(discarded, _)| *discarded == tag)
        else {
            return;
        };
        let aliases = self.snapshot.aliases.clone();
        for mut open in drafts {
            save(&mut open, &self.account, &aliases, &self.now, false);
        }
    }
}
