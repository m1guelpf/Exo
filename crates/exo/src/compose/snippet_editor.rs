//! A snippet open in compose: the regular new-message form with a name, its
//! header folded until asked for, `Done` instead of Send. It's never a Gmail draft: Done saves
//! it to the settings (no autosave), and leaving with unsaved changes asks first.

use exo_commands::{Hint, Toast};
use exo_store::Snippet;
use jiff::Timestamp;

use super::{Action, Alert, AlertThen, Field, Fields, Open, draft::SnippetEdit, factory};
use crate::commands;

/// Where leaving a snippet with unsaved changes was going, once `Save Snippet?` lets it.
pub(crate) type Leaving = Box<dyn FnOnce(&mut crate::Mail)>;

/// How many times saving a snippet shows the `To use a Snippet, hit ⌘ ;` hint.
const DONE_HINTS: u32 = 3;

impl crate::Mail {
    /// A new private snippet named `name`, in the pane with the name focused
    /// (`New Private Snippet`).
    pub(crate) fn create_snippet(&mut self, name: String) {
        let mut record =
            factory::compose(&self.compose_account(), Fields::default(), Timestamp::now());
        // `"use default"`: the From of the draft it's used in.
        record.from = None;
        let mut open = Open::new(factory::new_id(), None, record);
        open.snippet = Some(SnippetEdit { name, saved: None });
        open.state = super::draft::State {
            header_expanded: false,
            cc_bcc_shown: false,
            field: Field::SnippetName,
        };
        self.open_snippet_pane(open);
    }

    /// Opens saved snippet `id` for editing (`Edit Private Snippet`): its header shows when it
    /// has recipients or a subject.
    pub(crate) fn edit_snippet(&mut self, id: &str) {
        let Some(snippet) = self
            .account
            .settings()
            .snippets
            .iter()
            .find(|snippet| snippet.id == id)
            .cloned()
        else {
            return;
        };
        let mut record =
            factory::compose(&self.compose_account(), Fields::default(), Timestamp::now());
        record.from.clone_from(&snippet.from);
        record.to.clone_from(&snippet.to);
        record.cc.clone_from(&snippet.cc);
        record.bcc.clone_from(&snippet.bcc);
        record.subject.clone_from(&snippet.subject);
        record.body.clone_from(&snippet.body);
        record.attachments.clone_from(&snippet.attachments);
        let mut open = Open::new(snippet.id.clone(), None, record);
        let shown = !(snippet.to.is_empty() && snippet.cc.is_empty() && snippet.bcc.is_empty())
            || !snippet.subject.is_empty();
        open.state = super::draft::State {
            header_expanded: shown,
            cc_bcc_shown: shown,
            field: Field::SnippetName,
        };
        // What reopening it reads back is what's unchanged.
        let saved = Snippet {
            body: open.editor.html(),
            ..snippet.clone()
        };
        open.snippet = Some(SnippetEdit {
            name: snippet.name.clone(),
            saved: Some(saved),
        });
        self.open_snippet_pane(open);
    }

    fn open_snippet_pane(&mut self, open: Open) {
        if self.snippet_unsaved() {
            self.ask_leaving_snippet(Box::new(move |mail| mail.open_snippet_pane(open)));
            return;
        }
        self.leave_pane();
        self.close_thread();
        self.close_snippet_view();
        self.snippets.folder.open();
        self.compose.focus = Some(open.id.clone());
        self.compose.pane = Some((open.id.clone(), None));
        self.compose.drafts.push(open);
    }

    /// Done / ⌘Enter on a snippet: the size and recipient checks, then the snippet is saved and
    /// the folder shows again, with the `⌘ ;` hint the first three times.
    pub(super) fn save_snippet(&mut self) {
        let aliases = self.snapshot.aliases.clone();
        let Some(id) = self
            .compose
            .focus
            .clone()
            .or_else(|| self.compose.pane().map(str::to_owned))
        else {
            return;
        };
        let Some(open) = self.compose.get_mut(&id).filter(|open| open.is_snippet()) else {
            return;
        };
        let html = super::outgoing::html(open, &aliases);
        if let Some(problem) = super::checks::check(
            open,
            &html,
            true,
            super::checks::Bypass::default(),
            jiff::Timestamp::now(),
        ) {
            self.compose.alert = Some(Alert::new(
                problem.title,
                problem.text,
                vec!["OK"],
                id,
                AlertThen::Ok { focus_from: false },
            ));
            return;
        }
        let Some(snippet) = snippet_of(open) else {
            return;
        };
        let at = Timestamp::now().to_string();
        let mut hint = false;
        self.account.update_settings(|settings| {
            let analytics = settings
                .snippets
                .iter()
                .find(|kept| kept.id == snippet.id)
                .map(|kept| kept.analytics.clone())
                .unwrap_or_default();
            let saved = Snippet {
                date: at.clone(),
                analytics,
                ..snippet
            };
            match settings
                .snippets
                .iter_mut()
                .find(|kept| kept.id == saved.id)
            {
                Some(kept) => *kept = saved,
                None => settings.snippets.push(saved),
            }
            settings.snippets_last_used.insert(id.clone(), at);
            if settings.use_snippet_hint_count < DONE_HINTS {
                settings.use_snippet_hint_count += 1;
                hint = true;
            }
        });
        self.close_snippet();
        if hint {
            self.commands.hint(
                Hint::new()
                    .text("To use a Snippet, hit")
                    .keys(["⌘", ";"])
                    .text("at any time!"),
            );
        }
    }

    /// `Create Snippet from Draft`: a new snippet with the focused draft's recipients, subject,
    /// body and files.
    pub(crate) fn snippet_from_draft(&mut self) {
        let Some(open) = self.compose.focused_mut().filter(|open| !open.is_snippet()) else {
            return;
        };
        open.sync_body();
        let record = open.record.clone();
        let snippet = Snippet {
            to: record.to,
            cc: record.cc,
            bcc: record.bcc,
            subject: record.subject,
            body: record.body,
            attachments: record.attachments,
            ..Snippet::default()
        };
        self.new_snippet_from(snippet);
    }

    /// `Create Snippet from Message`: a new snippet with the focused message's recipients,
    /// subject and unquoted body.
    pub(crate) fn snippet_from_message(&mut self) {
        let focused = self
            .thread
            .as_ref()
            .and_then(crate::thread::ThreadView::focused)
            .map(str::to_owned);
        let Some((message, html)) = self
            .copyable()
            .into_iter()
            .find(|(message, _)| Some(&message.id) == focused.as_ref())
        else {
            return;
        };
        let fields = Fields::of_message(message, html);
        let snippet = Snippet {
            to: fields.to,
            cc: fields.cc,
            bcc: fields.bcc,
            subject: fields.subject,
            body: fields.body,
            ..Snippet::default()
        };
        self.new_snippet_from(snippet);
    }

    /// A new, unsaved snippet holding `snippet`'s fields.
    fn new_snippet_from(&mut self, snippet: Snippet) {
        let mut record =
            factory::compose(&self.compose_account(), Fields::default(), Timestamp::now());
        record.from = None;
        record.to = snippet.to;
        record.cc = snippet.cc;
        record.bcc = snippet.bcc;
        record.subject = snippet.subject;
        record.body = snippet.body;
        record.attachments = snippet.attachments;
        let mut open = Open::new(factory::new_id(), None, record);
        let shown = open.recipients().next().is_some() || !open.record.subject.is_empty();
        open.state = super::draft::State {
            header_expanded: shown,
            cc_bcc_shown: shown,
            field: Field::SnippetName,
        };
        open.snippet = Some(SnippetEdit {
            name: String::new(),
            saved: None,
        });
        self.open_snippet_pane(open);
    }

    /// ⌘; with no snippets: `Snippets` offers to create one.
    pub(crate) fn ask_create_snippet(&mut self) {
        let draft = self.compose.focus.clone().unwrap_or_default();
        // Two `<br>`s, a blank line, not a paragraph.
        let text = "Instantly insert phrases, paragraphs, or whole emails.\n\nType less, do more!";
        self.compose.alert = Some(Alert::new(
            Some("Snippets"),
            text,
            vec!["Create Snippet", "Cancel"],
            draft,
            AlertThen::CreateSnippet,
        ));
    }

    /// `snippet` into the focused draft.
    pub(crate) fn compose_apply_snippet(&mut self, snippet: &Snippet, at_start: bool, now: f64) {
        let aliases = self.snapshot.aliases.clone();
        let Some((open, book)) = self.compose.focused_with_book() else {
            return;
        };
        super::snippets::apply(open, snippet, &aliases, book, at_start, now);
    }

    /// `Use Snippet Inline`: the body gets the keyboard, its caret at
    /// the end unless it had it, and a `;` (after a no-break space when the caret follows a
    /// word) opens the picker there.
    pub(crate) fn snippet_inline(&mut self, now: f64) {
        let Some(open) = self.compose.focused_mut() else {
            return;
        };
        if open.state.field != Field::Body {
            open.state.field = Field::Body;
            let end = open.editor.document().end();
            open.editor.set_selection(exo_editor::Selection::caret(end));
        }
        let caret = open.editor.selection().focus;
        let before = open
            .editor
            .document()
            .blocks
            .get(caret.block)
            .map(|block| block.text().chars().take(caret.offset).last());
        let after_word = before.flatten().is_some_and(|ch| !ch.is_whitespace());
        open.editor
            .type_text(if after_word { "\u{a0};" } else { ";" });
        open.touch(now);
    }

    /// Keeps the `;` picker's rows for the focused draft.
    pub(super) fn inline_snippet_rows(&mut self) {
        let settings = self.account.settings();
        let Some(open) = self
            .compose
            .focus
            .clone()
            .and_then(|id| self.compose.get_mut(&id))
        else {
            return;
        };
        let Some(query) = open
            .snippet_trigger
            .as_ref()
            .map(|trigger| trigger.query.clone())
        else {
            open.snippet_rows.clear();
            open.snippet_hint = false;
            return;
        };
        // A snippet being edited isn't offered in itself.
        let offered: Vec<Snippet> = settings
            .snippets
            .iter()
            .filter(|snippet| snippet.id != open.id)
            .cloned()
            .collect();
        let rows: Vec<super::draft::SnippetRow> =
            super::snippets::rank(&offered, &settings.snippets_last_used, &query)
                .into_iter()
                .map(|snippet| super::draft::SnippetRow {
                    id: snippet.id.clone(),
                    name: super::snippets::name(snippet).to_owned(),
                    preview: super::snippets::preview(snippet),
                })
                .collect();
        if rows != open.snippet_rows {
            open.snippet_pick = 0;
            open.snippet_rows = rows;
        }
        open.snippet_hint = query.is_empty() && settings.snippets.is_empty();
    }

    /// Enter, Tab or a press in the `;` picker: the typed `;query` goes and the snippet goes in
    /// its place.
    pub(super) fn pick_inline_snippet(&mut self, index: Option<usize>, now: f64) {
        let Some(open) = self.compose.focused_mut() else {
            return;
        };
        let Some(row) = open
            .snippet_rows
            .get(index.unwrap_or(open.snippet_pick))
            .cloned()
        else {
            return;
        };
        if let Some(trigger) = open.snippet_trigger.take() {
            if open.state.field == Field::Subject {
                super::subject::replace(open, &trigger, "");
            } else {
                open.editor.replace(trigger.start, trigger.end, "");
            }
        }
        open.snippet_rows.clear();
        let Some(snippet) = self
            .account
            .settings()
            .snippets
            .iter()
            .find(|snippet| snippet.id == row.id)
            .cloned()
        else {
            return;
        };
        self.compose_apply_snippet(&snippet, false, now);
        self.account.update_settings(|settings| {
            settings
                .snippets_last_used
                .insert(snippet.id.clone(), Timestamp::now().to_string());
        });
    }

    /// Discard on a snippet: unsaved changes ask first (`Discard changes?`).
    pub(super) fn ask_discard_snippet(&mut self) {
        let Some(open) = self.compose.focused() else {
            return;
        };
        if is_dirty(open) {
            let draft = open.id.clone();
            self.compose.alert = Some(Alert::new(
                Some("Discard changes?"),
                "Your changes will not be saved.",
                vec!["Discard", "Cancel"],
                draft,
                AlertThen::DiscardSnippet,
            ));
        } else {
            self.discard_snippet();
        }
    }

    /// Discarding a snippet: it's gone from the folder, with `Snippet discarded.` and its Undo.
    pub(super) fn discard_snippet(&mut self) {
        let Some(id) = self
            .compose
            .focus
            .clone()
            .or_else(|| self.compose.pane().map(str::to_owned))
        else {
            return;
        };
        self.close_snippet();
        self.discard_saved_snippet(&id);
    }

    /// Snippet `id` leaves the settings, with `Snippet discarded.` and its Undo (which a snippet
    /// never saved has nothing to bring back).
    pub(crate) fn discard_saved_snippet(&mut self, id: &str) {
        let Some(saved) = self
            .account
            .settings()
            .snippets
            .iter()
            .find(|snippet| snippet.id == id)
            .cloned()
        else {
            self.commands.toast(Toast::new("Snippet discarded."));
            return;
        };
        self.account
            .update_settings(|settings| settings.snippets.retain(|snippet| snippet.id != id));
        let tag = self.compose.tag();
        self.compose.discarded_snippet = Some((tag, saved));
        self.commands.toast(
            Toast::new("Snippet discarded.")
                .undo(commands::Action::Compose(Action::UndoDiscardSnippet(tag)))
                .tag(tag),
        );
    }

    /// The toast's Undo: the discarded snippet is back.
    pub(super) fn undo_discard_snippet(&mut self, tag: u64) {
        let Some((_, snippet)) = self
            .compose
            .discarded_snippet
            .take_if(|(kept, _)| *kept == tag)
        else {
            return;
        };
        self.account.update_settings(|settings| {
            if !settings.snippets.iter().any(|kept| kept.id == snippet.id) {
                settings.snippets.push(snippet);
            }
        });
    }

    /// Escape in a snippet: an empty new one goes; otherwise the form lets go of the keyboard.
    pub(super) fn escape_snippet(&mut self) {
        let Some(open) = self.compose.focused() else {
            return;
        };
        if open
            .snippet
            .as_ref()
            .is_some_and(|edit| edit.saved.is_none())
            && is_empty(open)
        {
            self.close_snippet();
        } else {
            self.compose.focus = None;
        }
    }

    /// Back from the snippet in the pane: it closes, once `Save Snippet?`
    /// has asked about unsaved changes. Returns whether a snippet was there.
    pub(super) fn leaving_snippet_asks(&mut self) -> bool {
        if self.pane_snippet().is_none() {
            return false;
        }
        if self.snippet_unsaved() {
            self.ask_leaving_snippet(Box::new(|_| {}));
        } else {
            self.close_snippet();
        }
        true
    }

    /// The snippet in the pane has changes Done hasn't saved.
    pub(crate) fn snippet_unsaved(&self) -> bool {
        self.pane_snippet().is_some_and(is_dirty)
    }

    /// The snippet open in the pane.
    pub(super) fn pane_snippet(&self) -> Option<&Open> {
        let id = self.compose.pane()?;
        self.compose
            .drafts
            .iter()
            .find(|open| open.id == id)
            .filter(|open| open.is_snippet())
    }

    /// Leaving the snippet in the pane, with unsaved changes, for `then`: `Save Snippet?` asks,
    /// and `then` goes on after Save or Don't save (Delete for one never saved), not after Cancel.
    pub(crate) fn ask_leaving_snippet(&mut self, then: Leaving) {
        let Some(open) = self.pane_snippet() else {
            return;
        };
        let never_saved = open
            .snippet
            .as_ref()
            .is_some_and(|edit| edit.saved.is_none());
        let actions = vec![
            "Save",
            "Cancel",
            if never_saved { "Delete" } else { "Don't save" },
        ];
        let mut alert = Alert::new(
            Some("Save Snippet?"),
            "Would you like to save changes to this Snippet? \n You cannot undo this action.",
            actions,
            open.id.clone(),
            AlertThen::SaveSnippet,
        );
        alert.escape = 1;
        self.compose.alert = Some(alert);
        self.snippets.leaving = Some(then);
    }

    /// `Save Snippet?`'s answer (`index` of Save, Cancel, Don't save) about snippet `id`.
    pub(super) fn answer_save_snippet(&mut self, index: usize, id: String) {
        let then = self.snippets.leaving.take();
        match index {
            // The snippet, though a popped-out draft may have the keyboard.
            0 => {
                self.compose.focus = Some(id.clone());
                self.save_snippet();
            }
            1 => {
                self.compose.focus = Some(id);
                return;
            }
            _ => self.close_snippet(),
        }
        // Saving stops at a check's alert, the snippet still open.
        if self.compose.pane() != Some(id.as_str())
            && let Some(then) = then
        {
            then(self);
        }
    }

    /// The snippet in the pane closes, unsaved changes dropped; the folder shows.
    pub(crate) fn close_snippet(&mut self) {
        let Some((id, _)) = self.compose.pane.take_if(|(id, _)| {
            self.compose
                .drafts
                .iter()
                .any(|open| open.id == *id && open.is_snippet())
        }) else {
            return;
        };
        self.compose.drafts.retain(|open| open.id != id);
        if self.compose.focus.as_deref() == Some(id.as_str()) {
            self.compose.focus = None;
        }
    }
}

/// The snippet `open` holds, as Done saves it (without its date and analytics).
pub(super) fn snippet_of(open: &mut Open) -> Option<Snippet> {
    open.sync_body();
    let edit = open.snippet.as_ref()?;
    let record = &open.record;
    Some(Snippet {
        id: open.id.clone(),
        name: edit.name.clone(),
        subject: record.subject.clone(),
        body: record.body.clone(),
        to: record.to.clone(),
        cc: record.cc.clone(),
        bcc: record.bcc.clone(),
        from: record.from.clone(),
        attachments: record.attachments.clone(),
        date: String::new(),
        analytics: exo_store::SnippetAnalytics::default(),
    })
}

/// Nothing in it at all.
fn is_empty(open: &Open) -> bool {
    open.snippet
        .as_ref()
        .is_some_and(|edit| edit.name.trim().is_empty())
        && open.is_body_empty()
        && open.recipients().next().is_none()
        && open.record.subject.trim().is_empty()
        && open.record.attachments.is_empty()
}

/// It differs from what was last saved; a new one, once anything is in it.
fn is_dirty(open: &Open) -> bool {
    let Some(edit) = &open.snippet else {
        return false;
    };
    match &edit.saved {
        None => !is_empty(open),
        Some(saved) => {
            let record = &open.record;
            edit.name != saved.name
                || record.subject != saved.subject
                || open.editor.html() != saved.body
                || record.to != saved.to
                || record.cc != saved.cc
                || record.bcc != saved.bcc
                || record.from != saved.from
                || record.attachments != saved.attachments
        }
    }
}

/// The pane's title for a snippet.
pub(crate) fn pane_title(open: &Open) -> &'static str {
    if open
        .snippet
        .as_ref()
        .is_some_and(|edit| edit.saved.is_some())
    {
        "Edit Private Snippet"
    } else {
        "New Private Snippet"
    }
}
