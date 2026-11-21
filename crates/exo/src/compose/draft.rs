//! A draft open in compose: its record, its editor, the form's state, and when it counts as
//! empty.

use exo_editor::Editor;
use exo_store::{Address, Alias, DraftAction, DraftRecord, StoredDraft};

use super::{assistant, auto_bcc, chips, factory, intro, recipients, signature};

/// A field or button of the form, which can have the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Field {
    /// A snippet's name (`Enter description`).
    SnippetName,
    /// The collapsed `Draft to …` header.
    CollapsedHeader,
    To,
    Cc,
    Bcc,
    From,
    Subject,
    Body,
    /// The `…` under the body.
    QuotingStart,
    /// The action pane's Send.
    Send,
    /// The action pane's Send later.
    SendLater,
    /// The action pane's Remind button.
    RemindMe,
    /// The paperclip.
    Attach,
    /// The trash can.
    Discard,
    /// The header's caret that shows Cc, Bcc and From.
    ExpandRecipients,
    /// The header's pop out (or pop in) button.
    PopAction,
    /// The pop-out's close button.
    PopClose,
}

impl Field {
    /// The recipient list it is, if it's one.
    pub fn list(self) -> Option<List> {
        match self {
            Self::To => Some(List::To),
            Self::Cc => Some(List::Cc),
            Self::Bcc => Some(List::Bcc),
            _ => None,
        }
    }
}

/// A recipient list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum List {
    To,
    Cc,
    Bcc,
}

impl List {
    pub const ALL: [Self; 3] = [Self::To, Self::Cc, Self::Bcc];

    pub fn field(self) -> Field {
        match self {
            Self::To => Field::To,
            Self::Cc => Field::Cc,
            Self::Bcc => Field::Bcc,
        }
    }

    /// The label.
    pub fn label(self) -> &'static str {
        match self {
            Self::To => "To",
            Self::Cc => "Cc",
            Self::Bcc => "Bcc",
        }
    }
}

/// The form's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct State {
    /// To (and the rest) shown rather than `Draft to …`.
    pub header_expanded: bool,
    /// Cc, Bcc and From shown.
    pub cc_bcc_shown: bool,
    /// The field that has, or gets, the keyboard.
    pub field: Field,
}

impl State {
    /// The state a draft opens in: a reply with recipients collapsed at its body, a subject-line
    /// edit at its subject, anything else at To; Cc and Bcc shown when expanded and not empty.
    pub fn default_for(record: &DraftRecord) -> Self {
        let has_recipients =
            !record.to.is_empty() || !record.cc.is_empty() || !record.bcc.is_empty();
        let reply = matches!(record.action, DraftAction::Reply | DraftAction::ReplyAll);
        let (header_expanded, field) = match record.action {
            DraftAction::EditSubjectLineReply => (true, Field::Subject),
            DraftAction::Forward => (true, Field::To),
            _ if reply && has_recipients => (false, Field::Body),
            _ => (true, Field::To),
        };
        let cc_bcc_shown = header_expanded && (!record.cc.is_empty() || !record.bcc.is_empty());
        Self {
            header_expanded,
            cc_bcc_shown,
            field,
        }
    }

    /// Shows everything and moves to `field`.
    pub fn expand(&mut self, field: Field) {
        self.header_expanded = true;
        self.cc_bcc_shown = true;
        self.field = field;
    }
}

/// What a recipient field is doing: the text being typed, which chips are selected, and the
/// suggestions for the text.
#[derive(Clone, Debug, Default)]
pub(crate) struct Input {
    pub text: String,
    /// Selected chips' indices, sorted; empty when the caret is in the input.
    pub selected: Vec<usize>,
    /// The chip a shift-selection extends from.
    pub anchor: Option<usize>,
    /// The text `suggestions` are for.
    pub query: String,
    /// Suggested contacts for the text, best first.
    pub suggestions: Vec<Address>,
    /// Which of them are the thread's own participants, which can't be removed.
    pub from_thread: Vec<bool>,
    /// The suggestion just removed, which ⌘Z brings back until
    /// something's typed.
    pub just_excluded: Option<Address>,
    /// The highlighted suggestion.
    pub highlight: usize,
    /// Escape closed the suggestions; typing opens them again.
    pub closed: bool,
    /// The suggestion list's width since it opened: it only grows while it shows.
    pub list_width: f32,
    /// The chip being edited where it stands rather than at the end: the input stands in for it,
    /// with its text.
    pub editing: Option<usize>,
    /// The input's text is to be selected whole as it next gets the keyboard.
    pub select_text: bool,
    /// Where the caret was in the text as last drawn, in characters, when nothing in it was
    /// selected: at either end, ← or → finishes the edit.
    pub caret: Option<usize>,
}

impl Input {
    /// Whether the suggestions show.
    pub fn suggesting(&self) -> bool {
        !self.closed && !self.suggestions.is_empty()
    }
}

/// A draft compose has open.
pub(crate) struct Open {
    /// Exo's draft id.
    pub id: String,
    /// Its Gmail thread, once it has one.
    pub thread: Option<String>,
    /// A draft written elsewhere that Exo hasn't saved yet: its Gmail message.
    pub adopt: Option<String>,
    /// Everything but the body, which is the editor's.
    pub record: DraftRecord,
    pub editor: Editor,
    pub state: State,
    pub inputs: [Input; 3],
    /// When it first changed since the last save.
    pub changed: Option<f64>,
    /// When it was last saved.
    pub saved_at: f64,
    /// The record as the store has it, once it does: what it was last saved as, or opened from.
    /// A save that wouldn't change it isn't made: a draft is saved only when a field is set to a
    /// different value.
    pub saved: Option<DraftRecord>,
    /// The editor's last height, for the layout above it.
    pub body_height: f32,
    /// The From list is open, with this row highlighted.
    pub from_menu: Option<usize>,
    /// The subject's whole text is selected when it next gets the keyboard.
    pub select_subject: bool,
    /// The subject's last Unicode replacement, which Backspace right after it undoes.
    pub subject_replacement: Option<SubjectReplacement>,
    /// The snippets used in it, whose sends it counts once it's sent.
    pub applied_snippets: Vec<String>,
    /// The snippet this edits, when it's one: never a Gmail draft.
    pub snippet: Option<SnippetEdit>,
    /// The `;` being typed in the body, as the editor last reported it.
    pub snippet_trigger: Option<exo_editor::Trigger>,
    /// The snippets it finds, best first.
    pub snippet_rows: Vec<SnippetRow>,
    /// There are no snippets at all: `To create a Snippet, hit ⌘ K → "Snippets"` shows instead.
    pub snippet_hint: bool,
    /// The highlighted row of the `;` picker.
    pub snippet_pick: usize,
    /// Share Availability's menu, open at the caret.
    pub availability_menu: Option<super::availability::Menu>,
    /// Who replied and so called off its scheduled send, as the Send
    /// later button names them: `Unscheduled: {name} replied`.
    pub unscheduled_by: Option<String>,
    /// Automatic Reminders' view of it.
    pub automatic: super::reminder::Automatic,
    /// `includeSignatureOnReplies`, as the account's settings last said
    /// ([`super::signature::includes_content`]).
    pub signature_on_replies: bool,
    /// Auto Bcc's bookkeeping.
    pub auto_bcc: auto_bcc::State,
    /// Where Instant Intro's cycle is.
    pub intro: intro::Cycle,
    /// The recipients' undo and redo.
    pub history: chips::History,
    /// The `:` emoji and `+`/`@`/`-` contact dropdowns.
    pub assistant: assistant::State,
    /// A right-click in the body, whose menu shows next frame.
    pub context_menu: Option<exo_editor::ContextMenu>,
    /// The detected date the body's caret was last on, while it had the keyboard.
    pub date: Option<exo_editor::ActiveDate>,
    /// The subject's spelling: Chromium's, autocorrect or not.
    pub subject_spelling: exo_editor::FieldSpelling,
    /// The subject's `:` and `;` pickers.
    pub subject_assistant: super::subject::State,
    /// A snippet's name's spelling.
    pub name_spelling: exo_editor::FieldSpelling,
    /// The address book the body's names were last taken from (its `Arc`'s address, `None`
    /// before it's read), which autocorrect leaves alone; `None` until they're taken.
    pub names_from: Option<Option<usize>>,
    /// Where the body's caret was last drawn.
    pub caret: egui::Rect,
    /// The body's scroll while the form is at its max height.
    pub body_scroll: super::form::BodyScroll,
    /// The header's scroll while its recipients pass its max height.
    pub header_scroll: super::form::BodyScroll,
    /// Chips being dragged, while the button is down.
    pub chip_drag: Option<chips::Drag>,
    /// How its files stand with Gmail, as the store last said, and whether Gmail is in reach:
    /// its cards' upload bars and tooltips.
    pub upload: exo_store::Upload,
    pub online: bool,
}

/// A row of the `;` picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnippetRow {
    pub id: String,
    pub name: String,
    pub preview: String,
}

/// A snippet open in compose: its name, and the snippet as last saved (`None` until it's
/// first saved), to tell unsaved changes from it.
#[derive(Clone, Debug)]
pub(crate) struct SnippetEdit {
    pub name: String,
    pub saved: Option<exo_store::Snippet>,
}

/// A Unicode replacement just made in the subject.
#[derive(Clone, Debug)]
pub(crate) struct SubjectReplacement {
    /// Where it left the caret.
    pub caret: usize,
    /// The subject as typed, before it.
    pub typed: String,
    /// The caret in that text.
    pub typed_caret: usize,
}

impl Open {
    /// A draft just made by the factory, not saved yet.
    pub fn new(id: String, thread: Option<String>, record: DraftRecord) -> Self {
        let editor = if record.body.is_empty() {
            Editor::new()
        } else {
            Editor::from_html(&record.body)
        };
        let state = State::default_for(&record);
        Self {
            id,
            thread,
            adopt: None,
            record,
            editor,
            state,
            inputs: Default::default(),
            changed: None,
            saved_at: f64::NEG_INFINITY,
            saved: None,
            body_height: 0.0,
            from_menu: None,
            select_subject: false,
            subject_replacement: None,
            applied_snippets: Vec::new(),
            snippet: None,
            snippet_trigger: None,
            snippet_rows: Vec::new(),
            snippet_hint: false,
            snippet_pick: 0,
            availability_menu: None,
            unscheduled_by: None,
            automatic: super::reminder::Automatic::default(),
            signature_on_replies: true,
            auto_bcc: auto_bcc::State::default(),
            intro: intro::Cycle::default(),
            history: chips::History::default(),
            assistant: assistant::State::default(),
            context_menu: None,
            date: None,
            subject_spelling: exo_editor::FieldSpelling::default(),
            subject_assistant: super::subject::State::default(),
            name_spelling: exo_editor::FieldSpelling::default(),
            names_from: None,
            caret: egui::Rect::NOTHING,
            body_scroll: super::form::BodyScroll::default(),
            header_scroll: super::form::BodyScroll::default(),
            chip_drag: None,
            upload: exo_store::Upload::default(),
            online: true,
        }
    }

    /// A saved draft, opened again.
    pub fn stored(draft: &StoredDraft) -> Self {
        let mut open = Self::new(
            draft.id.clone(),
            draft
                .thread
                .as_ref()
                .map(|thread| thread.as_str().to_owned()),
            draft.record.clone(),
        );
        open.mark_saved();
        open
    }

    /// The store has it.
    pub fn is_stored(&self) -> bool {
        self.saved.is_some()
    }

    /// Takes its record, the editor's body included, as what the store has.
    pub fn mark_saved(&mut self) {
        self.sync_body();
        self.saved = Some(self.record.clone());
    }

    /// Whether it differs from what the store has, taking the editor's body into
    /// the record first.
    pub fn is_dirty(&mut self) -> bool {
        self.sync_body();
        self.saved.as_ref() != Some(&self.record)
    }

    /// A recipient list's chips.
    pub fn list(&self, list: List) -> &Vec<Address> {
        match list {
            List::To => &self.record.to,
            List::Cc => &self.record.cc,
            List::Bcc => &self.record.bcc,
        }
    }

    pub fn list_mut(&mut self, list: List) -> &mut Vec<Address> {
        match list {
            List::To => &mut self.record.to,
            List::Cc => &mut self.record.cc,
            List::Bcc => &mut self.record.bcc,
        }
    }

    pub fn input(&self, list: List) -> &Input {
        &self.inputs[list as usize]
    }

    pub fn input_mut(&mut self, list: List) -> &mut Input {
        &mut self.inputs[list as usize]
    }

    /// Notes a change, to be saved within the second.
    pub fn touch(&mut self, now: f64) {
        self.changed.get_or_insert(now);
    }

    /// Whether the field with the keyboard takes typing (the body, the subject, a recipient
    /// list, a snippet's name), where `;` is text.
    pub fn takes_text(&self) -> bool {
        match self.state.field {
            Field::To | Field::Cc | Field::Bcc | Field::Body | Field::SnippetName => true,
            Field::Subject => self.is_new_email(),
            _ => false,
        }
    }

    /// Whether this is a snippet being edited.
    pub fn is_snippet(&self) -> bool {
        self.snippet.is_some()
    }

    /// Whether this is a new message rather than a reply or forward.
    pub fn is_new_email(&self) -> bool {
        matches!(
            self.record.action,
            DraftAction::Compose | DraftAction::EditSubjectLineReply
        )
    }

    /// Whether the body has nothing in it.
    pub fn is_body_empty(&self) -> bool {
        self.editor.is_empty()
    }

    /// Whether it's untouched: a new message with nothing in it, or a reply whose body is empty
    /// and whose recipients and files are as created, Auto Bcc's addresses aside. Such a draft is
    /// discarded silently when it closes.
    pub fn is_unmodified(&self) -> bool {
        if !self.is_body_empty() {
            return false;
        }
        let bcc_untouched = self
            .record
            .bcc
            .iter()
            .all(|address| self.auto_bcc.emails.contains(&address.email.to_lowercase()));
        if self.is_new_email() {
            self.record.to.is_empty()
                && self.record.cc.is_empty()
                && bcc_untouched
                && self.record.subject.trim().is_empty()
                && self.record.attachments.is_empty()
        } else {
            bcc_untouched && factory::fingerprint(&self.record) == self.record.fingerprint
        }
    }

    /// There's a quote or signature behind a `…`.
    pub fn has_hidden_quote(&self, aliases: &[Alias]) -> bool {
        !self.is_snippet()
            && !self.record.quoted_inlined
            && (self.record.quoted_content.is_some()
                || signature::of(self.record.from.as_ref(), aliases).is_some())
    }

    /// Every recipient, To, Cc then Bcc.
    pub fn recipients(&self) -> impl Iterator<Item = &Address> {
        self.record
            .to
            .iter()
            .chain(&self.record.cc)
            .chain(&self.record.bcc)
    }

    /// Whether it has a recipient, and all of them are valid.
    pub fn has_valid_recipients(&self) -> bool {
        self.recipients().next().is_some() && self.recipients().all(recipients::sendable)
    }

    /// Takes the editor's body into the record, as saving does.
    pub fn sync_body(&mut self) {
        self.record.body = self.editor.html();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exo_store::DraftRecord;

    fn record(action: DraftAction, to: &[&str]) -> DraftRecord {
        let mut record = DraftRecord {
            action,
            from: None,
            to: to
                .iter()
                .map(|email| Address {
                    name: None,
                    email: (*email).into(),
                })
                .collect(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: String::new(),
            body: String::new(),
            quoted_content: None,
            quoted_inlined: false,
            in_reply_to: None,
            in_reply_to_rfc822: None,
            references: Vec::new(),
            rfc822_id: "x@y".into(),
            fingerprint: String::new(),
            attachments: Vec::new(),
            send_later: None,
            reminder: None,
        };
        record.fingerprint = factory::fingerprint(&record);
        record
    }

    #[test]
    fn a_reply_opens_at_its_body_and_the_rest_at_to() {
        let reply = State::default_for(&record(DraftAction::Reply, &["a@x.com"]));
        assert_eq!((reply.header_expanded, reply.field), (false, Field::Body));
        let lonely = State::default_for(&record(DraftAction::Reply, &[]));
        assert_eq!((lonely.header_expanded, lonely.field), (true, Field::To));
        let forward = State::default_for(&record(DraftAction::Forward, &[]));
        assert_eq!((forward.header_expanded, forward.field), (true, Field::To));
        let new = State::default_for(&record(DraftAction::Compose, &[]));
        assert_eq!(
            (new.header_expanded, new.cc_bcc_shown, new.field),
            (true, false, Field::To)
        );
    }

    #[test]
    fn untouched_drafts_are_unmodified() {
        let mut reply = Open::new("d".into(), None, record(DraftAction::Reply, &["a@x.com"]));
        assert!(reply.is_unmodified());
        reply.record.to.push(Address {
            name: None,
            email: "b@x.com".into(),
        });
        assert!(!reply.is_unmodified(), "recipients changed");
        let mut new = Open::new("d".into(), None, record(DraftAction::Compose, &[]));
        assert!(new.is_unmodified());
        new.record.subject = "Hi".into();
        assert!(!new.is_unmodified());
        let mut typed = Open::new("d".into(), None, record(DraftAction::Compose, &[]));
        typed.editor.type_text("Hello");
        assert!(!typed.is_unmodified());
    }

    #[test]
    fn a_file_added_modifies_a_draft() {
        let file = exo_store::DraftAttachment {
            uuid: "f".into(),
            name: "notes.txt".into(),
            mime_type: "text/plain".into(),
            size: 5,
            content_id: None,
            source: exo_store::AttachmentSource::Local,
        };
        for action in [DraftAction::Reply, DraftAction::Compose] {
            let mut open = Open::new(
                "d".into(),
                None,
                record(
                    action,
                    &["a@x.com"][..usize::from(action == DraftAction::Reply)],
                ),
            );
            open.record.attachments.push(file.clone());
            assert!(!open.is_unmodified(), "{action:?} with a file");
        }
        // A forward made with its message's files is untouched until they change.
        let mut forward = record(DraftAction::Forward, &[]);
        forward.attachments.push(file);
        forward.fingerprint = factory::fingerprint(&forward);
        assert!(Open::new("d".into(), None, forward).is_unmodified());
    }
}
