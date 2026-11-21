//! Compose: new messages, replies, reply all and forwards, drafts kept in Gmail, and sending
//! with undo send.
//!
//! A draft open in compose is an [`Open`]: its record, an `exo-editor` for the body, and the
//! form's state. It shows inline under the message it answers ([`crate::thread`] lays it out
//! through [`Inline`]) or, for a new message, in a pane of its own
//! ([`crate::Mail::show_compose_pane`]). Edits are saved within a second
//! ([`crate::Mail::compose_tick`]) to the store, which mirrors them to a Gmail draft. Sending
//! holds the draft for 20 s: the toast's Undo takes it back; hovering it buys 30 s more;
//! Instant Send sends at once.
//!
//! Send later picks a time Send then schedules the draft for ([`schedule`]), and Remind me a
//! reminder the message's thread gets at Send, which Undo takes back ([`reminder`]). Left out,
//! as for an account without them: AI, sharing and every other server feature.

mod assistant;
pub(crate) mod auto_bcc;
mod availability;
mod checks;
mod chips;
pub(crate) mod contacts;
pub(crate) mod discard;
mod draft;
mod factory;
mod failed;
mod files;
mod form;
mod images;
mod intro;
mod mailto;
mod outgoing;
mod picker;
mod plain;
mod popout;
pub(crate) mod recipients;
mod reminder;
mod schedule;
mod sent;
mod signature;
mod snippet_editor;
pub(crate) mod snippets;
mod spelling;
mod subject;
mod unsubscribe;

use exo_commands::{Command, Commands, Hint, Region, Toast, ToastKind};
use exo_store::{
    Address, Alias, DraftAction, DraftFailure, MessageDetail, Mutation, SentDraft, ThreadDetail,
};
use jiff::{SignedDuration, Timestamp};

pub(crate) use draft::{Field, Open};
pub(crate) use factory::{Fields, uuid_v4};
pub(crate) use form::Surface;
use form::from_menu;
pub(crate) use mailto::fields as mailto_fields;
pub(crate) use reminder::team_domains;
pub(crate) use snippet_editor::Leaving;

use crate::{account::Event, commands};

/// How long a send is held for undo.
const HOLD: SignedDuration = SignedDuration::from_secs(20);
/// Hovering the toast holds the send this much longer…
const HOVER_HOLD: SignedDuration = SignedDuration::from_secs(30);
/// …asked for at most this often.
const HOVER_EVERY: f64 = 5.0;
/// Edits are saved at most this often.
const SAVE_EVERY: f64 = 1.0;

/// What compose's commands, clicks and toasts ask for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// `C`, `Compose`: a new message.
    New,
    /// `Note to self`.
    NoteToSelf,
    /// `R`, `A`, `F`, Enter: reply, reply all or forward to the focused (or highlighted)
    /// conversation's message.
    Reply(DraftAction),
    /// `⌘⇧O/C/B/F/S` with no draft focused: a Reply All with that field focused.
    ReplyAllWith(Field),
    /// `⌘⇧O/C/B/F/S` in a draft, or a click on a field: focus it.
    Focus(Field),
    /// A click on a reply's subject, Enter on it, or the palette's `Change Subject`: the reply
    /// becomes a new message, popped out. A new message's
    /// subject is only focused.
    EditSubject,
    /// The palette's `Reply All, Change Subject` with no draft open: a Reply All of the focused
    /// message as a new message.
    ReplyAllEditSubject,
    /// A click on From's address, or Enter on it: its list opens or closes.
    FromToggle,
    /// ↓ / ↑ (and `j`/`n`, `k`/`p`) in the From list.
    FromMove(isize),
    /// Enter or Tab in the From list (the highlighted alias), or a click on one.
    FromChoose(Option<usize>),
    /// Escape in the From list: it closes.
    FromClose,
    /// A formatting key in the body.
    Format(exo_editor::FormatCommand),
    /// ⌥⇧` in the body with Backtick as Escape on: a backtick.
    TypeBacktick,
    /// The body's caret came onto a detected date saying this: its day in the right pane.
    DateActive(String),
    /// ⌘⇧– (-1) or ⌘⇧= (1) with the caret on a date: it says a day earlier or later.
    ShiftDate(i64),
    /// ↓ / ↑ in the `;` picker.
    SnippetMove(isize),
    /// Enter or Tab in the `;` picker (the highlighted snippet), or a press on a row.
    SnippetPick(Option<usize>),
    /// Escape in the `;` picker: it closes, the text stays.
    SnippetClose,
    /// The footer's `{ }`: `Use Snippet Inline`'s `;` in the body.
    SnippetInline,
    /// ⌘⇧A, the footer's calendar: Share Availability's menu at the caret, or closed.
    AvailabilityMenu(bool),
    /// Enter, Tab or a click on the menu's `Insert free times`.
    AvailabilityPick,
    /// A press outside the menu: it closes, and what was typed into it goes.
    AvailabilityDismiss,
    /// The `You can now hit ⌘ shift A…` tooltip's close icon: it doesn't show again.
    DismissAvailabilityTip,
    /// `⌘Enter` / `⌘⇧Enter`.
    Send { mark_done: bool },
    /// A click on Send (⌘⇧ held: and mark done), which also hints at `⌘Enter`.
    ClickSend { mark_done: bool },
    /// `⌘⇧,`, the trash icon.
    Discard,
    /// Escape.
    Escape,
    /// Tab / ⇧Tab between fields.
    Tab(isize),
    /// Enter, `,` or `;` in a recipient field: the typed text becomes a chip.
    Commit,
    /// Backspace in an empty recipient field: select the last chip, or remove the selected.
    Backspace,
    /// ⌘⇧P: pop the focused draft out, or back in.
    Pop,
    /// A click on the header's pop button: [`Action::Pop`], which also hints at ⌘⇧P.
    ClickPop,
    /// Escape: the pop-out closes, its draft kept.
    ClosePopout,
    /// A click on the pop-out's cross: [`Action::ClosePopout`], which also hints at Escape.
    ClickClosePopout,
    /// ⇧C: a new message, popped out.
    NewPoppedOut,
    /// ⇧R, ⇧A (⇧Enter), ⇧F: a reply, reply all or forward, popped out.
    ReplyPoppedOut(DraftAction),
    /// ⌘D: the keyboard between the pop-out and the rest of the app.
    CycleFocus,
    /// `Include Original Attachment(s)`: a reply takes the files of the message it answers.
    IncludeOriginals,
    /// Attach (⌘⇧U, the paperclip): the file picker.
    Attach,
    /// `Insert Image`: the picker for images, one of which goes in the body.
    InsertImage,
    /// The cross on an attachment card.
    RemoveAttachment(String),
    /// A click on an attachment card: Quick Look (`preview`), or a copy in Downloads.
    OpenAttachment { uuid: String, preview: bool },
    /// ↑ / ↓ in the suggestions.
    MoveSuggestion(isize),
    /// A suggestion picked with the pointer.
    PickSuggestion(usize),
    /// `Remove Suggestion` on a suggestion.
    RemoveSuggestion(usize),
    /// Dragged chips `chips` of `from` dropped into `into` at `at`.
    DropChips {
        from: draft::List,
        chips: Vec<usize>,
        into: draft::List,
        at: usize,
    },
    /// Undo for a removed suggestion: this address is suggested again.
    UndoRemoveSuggestion(String),
    /// Escape with suggestions showing: they close until the text changes.
    CloseSuggestions,
    /// ← / → in a recipient field with nothing typed: move the chip selection.
    MoveSelection(isize),
    /// The `…`: reveal the quote and signature.
    RevealQuote,
    /// A click on the `…`: it reveals, and a hint says the keys.
    ClickRevealQuote,
    /// Backspace on the `…`: drop the quote and signature.
    DeleteQuote,
    /// A click in the body's formatting toolbar or link popover, for its hint.
    Toolbar(exo_editor::ToolbarAction),
    /// Autocorrect's pill learned a word.
    Learned(String),
    /// The link popover's URL clicked: open it (a `mailto:` in a new draft).
    OpenLink(String),
    /// The toast's Undo on a send.
    UndoSend(u64),
    /// The toast's Instant Send.
    InstantSend(u64),
    /// The toast's Undo on a discard.
    UndoDiscard(u64),
    /// `⌘⇧,` in the list: discard the actionable conversations' drafts.
    DiscardListed,
    /// The toast's Undo on several drafts discarded from the list.
    UndoDiscards(u64),
    /// The toast's Undo on a discarded snippet.
    UndoDiscardSnippet(u64),
    /// An alert's action.
    Alert(usize),
    /// ← / → / Tab in an alert.
    AlertSelect(isize),
    /// Escape or the back gutter in the new-message pane without the keyboard: back to where it
    /// was opened from.
    LeavePane,
    /// A click on a failed or waiting send's status, by its message: `RecoverDraft`.
    Recover(String),
    /// `RecoverDraft`'s `Edit Message` on a failed draft.
    EditFailed(String),
    /// `RecoverDraft`'s `Discard` on a failed draft.
    DiscardFailed(String),
    /// `RecoverDraft`'s `Edit Message` on a send waiting to go: it's taken back.
    EditSending(String),
    /// `RecoverDraft`'s `Discard` on a send waiting to go.
    DiscardSending(String),
    /// ⌘⇧L, the palette's `Send Later`, the action pane's Send later: the picker.
    SendLater,
    /// ⌘⇧H, the palette's `Remind Me`, the action pane's Remind button: Remind me's picker for
    /// the draft.
    RemindMe,
    /// A Send later row for `draft`: its time (`None`: Unschedule, `now`), `if no reply`, and
    /// the row to remember as `last used`.
    Schedule {
        draft: String,
        at: Option<Timestamp>,
        abort_on_reply: bool,
        remember: Option<exo_store::PreviousSchedule>,
    },
    /// `RecoverDraft`'s `Reschedule` or `Edit Message` on a scheduled send: it's taken back to
    /// edit, its time kept.
    EditScheduled(String),
    /// `RecoverDraft`'s `Discard` on a scheduled send.
    DiscardScheduled(String),
    /// The `Reminder set` toast's Undo on a reminder the follow-up hint set for a message just
    /// sent as this draft.
    UnremindSent(String),
    /// ⌘⇧I in a draft: Instant Intro's next step.
    InstantIntro,
    /// ⌘⇧I on a conversation: its draft's next Instant Intro step, or a Reply All taking the first.
    InstantIntroReply,
    /// A key on the recipient list's chips.
    Chip(chips::Key),
    /// A key or press on the `:` emoji or `+`/`@`/`-` contact dropdown.
    Assistant(assistant::Key),
    /// `Use Emoji`: `:hello` in the body, opening the picker.
    UseEmoji,
    /// `Remove External Recipients` and the rest: everyone of that group off the draft.
    RemoveRecipients(assistant::Person),
    /// ⌘/ in a draft: popped out (unless it is), and search opened empty.
    PopAndSearch,
    /// `Compose to {email}`: a new message to the contact pane's person.
    ComposeTo(Address),
    /// `Exclude from Auto Bcc`: its picker, for this sender.
    OpenAutoBccExclusion(String),
    /// An address or domain picked there.
    ExcludeFromAutoBcc(String),
}

/// A send being held for undo, and its toast.
struct Sending {
    tag: u64,
    /// The draft as it was sent, to reopen on undo.
    open: Open,
    /// Where it was: inline in this thread, or the new-message pane.
    surface: Surface,
    /// The mark done that went with it, to undo with it.
    done: Option<crate::account::UndoToken>,
    /// When the hold ends, as last asked for.
    until: Timestamp,
    /// Gmail has the message: the toast reads `Message sent.`.
    acked: bool,
    undoing: bool,
    /// When the hover last held it longer.
    held_at: f64,
    /// Scheduled for its Send later time: no Instant Send, no hover hold.
    scheduled: bool,
}

impl Sending {
    /// The toast once it's sent.
    fn sent_text(&self) -> &'static str {
        if self.scheduled {
            return "Message scheduled.";
        }
        match (self.open.record.action, self.done.is_some()) {
            (DraftAction::Forward, true) => "Forwarded and Marked Done.",
            (DraftAction::Forward, false) => "Message forwarded.",
            (DraftAction::Reply | DraftAction::ReplyAll, true) => {
                "Reply sent and conversation Marked Done."
            }
            (DraftAction::Reply | DraftAction::ReplyAll, false) => "Reply sent.",
            _ => "Message sent.",
        }
    }

    /// The toast when it waits to go online.
    fn offline_text(&self) -> &'static str {
        if self.scheduled {
            return "Message will schedule when online.";
        }
        match self.open.record.action {
            DraftAction::Forward => "Message will forward when online.",
            DraftAction::Reply | DraftAction::ReplyAll => "Reply will send when online.",
            _ => "Message will send when online.",
        }
    }
}

/// A discarded draft a toast can bring back.
struct Discarded {
    tag: u64,
    open: Open,
    surface: Surface,
}

/// An alert compose is showing.
pub(crate) struct Alert {
    title: Option<&'static str>,
    text: String,
    actions: Vec<&'static str>,
    selected: usize,
    /// The action Escape takes: the last, but for `Save Snippet?`'s Cancel.
    escape: usize,
    /// The draft it's about.
    draft: String,
    then: AlertThen,
    /// Its text has the follow-up hint's `H` keycap.
    key: bool,
}

impl Alert {
    fn new(
        title: Option<&'static str>,
        text: impl Into<String>,
        actions: Vec<&'static str>,
        draft: impl Into<String>,
        then: AlertThen,
    ) -> Self {
        Self {
            title,
            text: text.into(),
            escape: actions.len() - 1,
            actions,
            selected: 0,
            draft: draft.into(),
            then,
            key: false,
        }
    }
}

/// What an alert's first action does.
enum AlertThen {
    /// "Send anyway" with these checks waved through, or "Cancel".
    SendAnyway(checks::Bypass, bool),
    /// "OK", focusing From when it's a From problem.
    Ok { focus_from: bool },
    /// "See Message": the draft is reopened.
    SeeMessage,
    /// `Save Snippet?`: Save, Cancel, or Delete / Don't save.
    SaveSnippet,
    /// `Discard changes?` on a snippet: Discard or Cancel.
    DiscardSnippet,
    /// `Snippets`, when there are none to use: Create Snippet or Cancel.
    CreateSnippet,
    /// The follow-up hint after a send: `Set reminder` or `Maybe later`.
    FollowUp,
}

/// A reply asked for before its thread was loaded (`R` from the list).
struct PendingReply {
    thread: String,
    action: DraftAction,
    field: Option<Field>,
    /// Popped out once made (⇧R, ⇧A, ⇧F).
    pop_out: bool,
    /// What its body starts with (Share Availability's Insert).
    body: Option<String>,
    /// The text selected in the message it answers, as HTML, which it quotes.
    quote: Option<String>,
}

/// Every draft compose has open, which one has the keyboard, and the sends being held.
#[derive(Default)]
pub(crate) struct Compose {
    drafts: Vec<Open>,
    /// The draft with the keyboard.
    focus: Option<String>,
    /// The new-message pane's draft, and the thread it was opened from.
    pane: Option<(String, Option<String>)>,
    /// The popped-out draft.
    popped: Option<String>,
    /// How many files of the message the focused reply answers it doesn't have.
    missing_originals: usize,
    sends: Vec<Sending>,
    discarded: Option<Discarded>,
    /// Several drafts the list discarded, which a toast can bring back.
    discards: Option<(u64, Vec<Open>)>,
    alert: Option<Alert>,
    pending: Option<PendingReply>,
    /// The thread whose drafts were last opened.
    loaded: Option<String>,
    /// A press landed in the focused draft's form this frame.
    pressed_inside: bool,
    /// The address book as last read, indexed.
    book: Option<(
        std::sync::Arc<Vec<exo_store::ScoredContact>>,
        contacts::Index,
    )>,
    /// The address book has been asked for (again).
    book_asked: bool,
    /// The `excluded_contacts` the book was indexed without.
    book_excluded: std::collections::BTreeMap<String, String>,
    /// The open thread's participants, indexed.
    thread_people: Option<(String, contacts::Index)>,
    /// Replies being sent, shown in their thread.
    in_thread: sent::ShownSends,
    /// A discarded snippet the toast's Undo brings back.
    discarded_snippet: Option<(u64, exo_store::Snippet)>,
    /// The focused draft's recipients as of the last frame, to tell one being added.
    recipients_seen: Option<(String, Vec<String>)>,
    /// The recipient the focused draft got last, for the contact pane to show.
    just_added: Option<Address>,
    /// The follow-up hint holding its send while it's open.
    follow_up: Option<reminder::FollowUp>,
    /// The threads the drafts sent this session went into, the latest last (a few).
    sent_threads: Vec<(String, String)>,
    /// The account's `Send + Mark Done` (`sendAndDone`), as of this frame.
    send_and_done: bool,
    /// The account's `Backtick as Escape` (`enableBacktickAsEscape`), as of this frame.
    backtick_escape: bool,
    /// The thread an Instant Intro's Reply All is waiting for, to take its first step.
    intro_pending: Option<String>,
    /// The drafts' images loading.
    images: images::Loads,
    /// The pointer rests over the contact pane (or the pop-out, once it slid for it): the
    /// pop-out slides out of the pane's way.
    still_over_right_pane: bool,
    /// The pop-out's card as last drawn: the wheel over it is its own.
    popout_card: Option<egui::Rect>,
    /// The command palette has the keyboard this frame: no draft takes it back.
    pub(crate) palette_open: bool,
    /// Share Availability's `You can now hit ⌘ shift A…` may show on a reply this frame.
    availability_tip: bool,
    next_tag: u64,
}

impl Compose {
    fn tag(&mut self) -> u64 {
        self.next_tag += 1;
        self.next_tag
    }

    /// Gives `view` the thread's latest copy, with the replies being sent in it.
    pub fn set_thread_detail(
        &mut self,
        view: &mut crate::thread::ThreadView,
        detail: std::sync::Arc<ThreadDetail>,
    ) {
        self.in_thread.set_detail(view, detail);
    }

    /// The region compose holds the keyboard in, if it does.
    pub fn region(&self) -> Option<Region> {
        if self.alert.is_some() {
            return Some(Region::Alert);
        }
        let Some(open) = self.focused() else {
            // The new-message pane is a thread pane.
            return self.pane.is_some().then_some(Region::ThreadMessages);
        };
        Some(if open.state.field.list().is_some() {
            Region::RecipientList
        } else {
            Region::ComposeForm
        })
    }

    /// The draft with the keyboard.
    pub fn focused(&self) -> Option<&Open> {
        let id = self.focus.as_deref()?;
        self.drafts.iter().find(|open| open.id == id)
    }

    /// The focused draft, and the address book beside it.
    fn focused_with_book(&mut self) -> Option<(&mut Open, Option<&contacts::Index>)> {
        let id = self.focus.clone()?;
        let book = self.book.as_ref().map(|(_, index)| index);
        let open = self.drafts.iter_mut().find(|open| open.id == id)?;
        Some((open, book))
    }

    fn focused_mut(&mut self) -> Option<&mut Open> {
        let id = self.focus.clone()?;
        self.drafts.iter_mut().find(|open| open.id == id)
    }

    fn get_mut(&mut self, id: &str) -> Option<&mut Open> {
        self.drafts.iter_mut().find(|open| open.id == id)
    }

    /// Who called off the scheduled sends of the drafts open, by a reply.
    pub fn unscheduled_by(&self) -> Vec<String> {
        self.drafts
            .iter()
            .filter_map(|open| open.unscheduled_by.clone())
            .collect()
    }

    /// The focused draft's To addresses and subject.
    pub fn focus_summary(&self) -> Option<(Vec<String>, String)> {
        let open = self.focused()?;
        Some((
            open.record
                .to
                .iter()
                .map(|address| address.email.clone())
                .collect(),
            open.record.subject.clone(),
        ))
    }

    /// How many drafts are open in `thread`.
    pub fn open_in(&self, thread: &str) -> usize {
        self.drafts
            .iter()
            .filter(|open| open.thread.as_deref() == Some(thread))
            .count()
    }

    /// The address book indexed for suggestions, once read ([`crate::Mail::index_address_book`]).
    pub fn book(&self) -> Option<&contacts::Index> {
        self.book.as_ref().map(|(_, index)| index)
    }

    /// The new-message pane's draft id, when it's showing.
    pub fn pane(&self) -> Option<&str> {
        self.pane.as_ref().map(|(id, _)| id.as_str())
    }

    /// The new message's last recipient, To through Bcc, while its pane shows (`Some(None)`
    /// with none yet); while chips are selected, the one at the selection's edge.
    pub fn pane_recipient(&self) -> Option<Option<&exo_store::Address>> {
        let (id, _) = self.pane.as_ref()?;
        let open = self.drafts.iter().find(|open| &open.id == id)?;
        if let Some(selected) = chips::edge(open) {
            return Some(Some(selected));
        }
        Some(
            open.record
                .to
                .iter()
                .chain(&open.record.cc)
                .chain(&open.record.bcc)
                .last(),
        )
    }

    /// Whom the contact pane is to show for the focused draft, when it isn't the new-message
    /// pane's (which [`Self::pane_recipient`] has): the chip at the selection's edge, else the
    /// recipient just added, taken.
    pub fn shown_recipient(&mut self) -> Option<Address> {
        let added = self.just_added.take();
        let open = self
            .focused()
            .filter(|open| self.pane() != Some(open.id.as_str()))?;
        chips::edge(open).cloned().or(added)
    }

    /// The popped-out draft's id.
    pub fn popped(&self) -> Option<&str> {
        self.popped.as_deref()
    }

    /// The drafts of `thread`, each under the message it answers (the last one when that isn't
    /// shown): for [`crate::thread::ThreadView`] to make room for them.
    pub fn inline<'a>(
        &'a mut self,
        thread: &'a ThreadDetail,
        aliases: &'a [Alias],
        me: &'a str,
        now: &'a jiff::Zoned,
        page: f32,
    ) -> Inline<'a> {
        let pane = self.pane.as_ref().map(|(id, _)| id.as_str());
        let thread_people = self
            .thread_people
            .as_ref()
            .filter(|(id, _)| *id == thread.id.as_str())
            .map(|(_, index)| index);
        Inline {
            drafts: &mut self.drafts,
            focus: &mut self.focus,
            pressed_inside: &mut self.pressed_inside,
            pane,
            popped: self.popped.as_deref(),
            alert: self.alert.is_some() || self.palette_open,
            thread,
            aliases,
            me,
            book: self.book.as_ref().map(|(_, index)| index),
            thread_people,
            now,
            send_and_done: self.send_and_done,
            availability_tip: self.availability_tip,
            max_height: page - form::REPLY_MAX_BELOW_PAGE,
        }
    }

    /// Drops every inline draft of `thread` (it was closed), saving or silently discarding each
    /// as [`clean_up`] does.
    fn close_thread_drafts(
        &mut self,
        thread: &str,
        account: &crate::account::Account,
        aliases: &[Alias],
        now: &jiff::Zoned,
    ) {
        let pane = self.pane().map(str::to_owned);
        let popped = self.popped.clone();
        let (closing, kept): (Vec<Open>, Vec<Open>) = std::mem::take(&mut self.drafts)
            .into_iter()
            .partition(|open| {
                open.thread.as_deref() == Some(thread)
                    && pane.as_deref() != Some(open.id.as_str())
                    && popped.as_deref() != Some(open.id.as_str())
            });
        self.drafts = kept;
        for mut open in closing {
            clean_up(&mut open, account, aliases, now);
        }
        if self
            .focus
            .as_ref()
            .is_some_and(|id| !self.drafts.iter().any(|open| &open.id == id))
        {
            self.focus = None;
        }
        self.loaded = None;
    }

    /// Registers compose's commands for this frame.
    pub fn register(
        &self,
        commands: &mut Commands<commands::Action>,
        aliases: &[Alias],
        team: &[String],
    ) {
        if self.pane.is_some() && self.focus.is_none() && self.alert.is_none() {
            // The new-message pane without the keyboard: Escape leaves it.
            commands.add(
                Command::shortcut("BACK", commands::Action::Compose(Action::LeavePane))
                    .keys(["esc", "backspace"])
                    .regions(&[Region::ThreadMessages]),
            );
            commands.add(
                Command::shortcut(
                    "FOCUS_DRAFT",
                    commands::Action::Compose(Action::Focus(Field::Body)),
                )
                .keys(["enter", "tab"])
                .regions(&[Region::ThreadMessages]),
            );
        }
        if let Some(alert) = &self.alert {
            const ALERT: &[Region] = &[Region::Alert];
            let act = |action: Action| commands::Action::Compose(action);
            commands.add(
                Command::shortcut("ALERT_ENTER", act(Action::Alert(alert.selected)))
                    .key("enter")
                    .regions(ALERT),
            );
            commands.add(
                Command::shortcut("ALERT_ESC", act(Action::Alert(alert.escape)))
                    .key("esc")
                    .regions(ALERT),
            );
            commands.add(
                Command::shortcut("ALERT_LEFT", act(Action::AlertSelect(-1)))
                    .keys(["left", "shift+tab"])
                    .regions(ALERT),
            );
            commands.add(
                Command::shortcut("ALERT_RIGHT", act(Action::AlertSelect(1)))
                    .keys(["right", "tab"])
                    .regions(ALERT),
            );
            // `h` in the follow-up hint is `Set reminder`.
            if matches!(alert.then, AlertThen::FollowUp) {
                commands.add(
                    Command::shortcut("FOLLOW_UP_H", act(Action::Alert(0)))
                        .key("h")
                        .regions(ALERT),
                );
            }
            return;
        }
        if self.popped.is_some() {
            // ⌘D cycles focus between the app and the pop-out, wherever either has the keyboard.
            commands.add(
                Command::shortcut(
                    "CYCLE_APP_FOCUS",
                    commands::Action::Compose(Action::CycleFocus),
                )
                .key("mod+d")
                .regions(&[
                    Region::ThreadList,
                    Region::ThreadMessages,
                    Region::ComposeForm,
                    Region::RecipientList,
                ]),
            );
        }
        if let Some(open) = self.focused() {
            let in_pane = self.pane() == Some(open.id.as_str());
            form::register(
                commands,
                open,
                aliases,
                self.popped.as_deref() == Some(open.id.as_str()),
                self.send_and_done,
                in_pane,
                self.backtick_escape,
            );
            if !open.is_snippet() {
                intro::register(commands);
            }
            assistant::register_use_emoji(commands);
            assistant::register_removals(commands, open, team);
            if self.missing_originals > 0 {
                let (title, alias) = if self.missing_originals == 1 {
                    ("Include Original Attachment", "Keep Attachment")
                } else {
                    ("Include Original Attachments", "Keep Attachments")
                };
                commands.add(
                    Command::new(
                        "INCLUDE_ORIGINAL_ATTACHMENTS",
                        title,
                        commands::Action::Compose(Action::IncludeOriginals),
                    )
                    .aliases([alias])
                    .default_score(1.79)
                    .regions(&[Region::ComposeForm, Region::RecipientList]),
                );
            }
        }
    }
}

/// Where compose's shortcuts start a draft from.
pub(crate) struct Shortcuts {
    /// A thread is open (rather than the list).
    pub in_thread: bool,
    /// The focused message is collapsed (Enter expands it rather than replying).
    pub focus_collapsed: bool,
    /// There's a conversation to reply to (a highlighted or open one, not a draft-only one).
    pub can_reply: bool,
    /// Several conversations are selected (the list's reply keys don't apply).
    pub selecting: bool,
}

/// Registers the shortcuts that start drafts: `C` and the palette's `Compose` everywhere, `R`,
/// `A`, `F` on a conversation, Enter on an expanded message, and `⌘⇧O/C/B/F/S` in a thread.
pub(crate) fn register_shortcuts(commands: &mut Commands<commands::Action>, cx: &Shortcuts) {
    let act = commands::Action::Compose;
    commands.add(
        Command::new("COMPOSE", "Compose", act(Action::New))
            .key("c")
            .key_code(["C"])
            .aliases(["New Email", "New Draft", "Write Email", "Send Email"])
            .default_score(1.75)
            .scale(1.2)
            .regions(exo_commands::LIST_AND_THREAD),
    );
    commands.add(
        Command::new("NOTE_TO_SELF", "Note to self", act(Action::NoteToSelf))
            .operative_word("note")
            .regions(exo_commands::LIST_AND_THREAD),
    );
    commands.add(
        Command::new(
            "POP_OUT_COMPOSE",
            "Pop Out Compose",
            act(Action::NewPoppedOut),
        )
        .key("shift+c")
        .key_code(["shift", "C"])
        .aliases(["Pop-out", "Popout"])
        .regions(exo_commands::LIST_AND_THREAD),
    );
    if !cx.can_reply || cx.selecting {
        return;
    }
    const CONVERSATION: &[Region] = exo_commands::CONVERSATION;
    commands.add(
        Command::new(
            "REPLY_ALL",
            "Reply All",
            act(Action::Reply(DraftAction::ReplyAll)),
        )
        .key("a")
        .key_code(["enter"])
        .default_score(3.0)
        .regions(CONVERSATION),
    );
    commands.add(
        Command::new("REPLY", "Reply", act(Action::Reply(DraftAction::Reply)))
            .key("r")
            .key_code(["R"])
            .default_score(2.0)
            .regions(CONVERSATION),
    );
    commands.add(
        Command::new(
            "FORWARD",
            "Forward",
            act(Action::Reply(DraftAction::Forward)),
        )
        .key("f")
        .key_code(["F"])
        .default_score(1.9)
        .regions(CONVERSATION),
    );
    let popped =
        |id: &'static str, title: &'static str, action: DraftAction, aliases: [&'static str; 2]| {
            Command::new(id, title, act(Action::ReplyPoppedOut(action)))
                .aliases(aliases)
                .default_score(1.8)
                .regions(CONVERSATION)
        };
    commands.add(
        popped(
            "REPLY_POP_OUT",
            "Reply, Pop Out Draft",
            DraftAction::Reply,
            ["Reply and Pop-out", "Reply and Popout"],
        )
        .key("shift+r")
        .key_code(["shift", "R"]),
    );
    commands.add(
        popped(
            "REPLY_ALL_POP_OUT",
            "Reply All, Pop Out Draft",
            DraftAction::ReplyAll,
            ["Reply All and Pop-out", "Reply All and Popout"],
        )
        .keys(["shift+enter", "shift+a"])
        .key_code(["shift", "enter"]),
    );
    commands.add(
        popped(
            "FORWARD_POP_OUT",
            "Forward, Pop Out Draft",
            DraftAction::Forward,
            ["Forward and Pop-out", "Forward and Popout"],
        )
        .key("shift+f")
        .key_code(["shift", "F"]),
    );
    intro::register_on_conversation(commands, cx.in_thread);
    if !cx.in_thread {
        return;
    }
    const MESSAGES: &[Region] = &[Region::ThreadMessages];
    if !cx.focus_collapsed {
        commands.add(
            Command::shortcut("REPLY_ALL_ENTER", act(Action::Reply(DraftAction::ReplyAll)))
                .key("enter")
                .regions(MESSAGES),
        );
    }
    let with = |id: &'static str,
                title: &'static str,
                field: Field,
                key: &'static str,
                code: &'static str| {
        Command::new(id, title, act(Action::ReplyAllWith(field)))
            .key(key)
            .key_code(["⌘", "shift", code])
            .default_score(1.8)
            .regions(MESSAGES)
    };
    commands.add(with(
        "REPLY_ALL_CHANGE_TO",
        "Reply All, Change To",
        Field::To,
        "mod+shift+o",
        "O",
    ));
    commands.add(
        with(
            "REPLY_ALL_CHANGE_CC",
            "Reply All, Change CC",
            Field::Cc,
            "mod+shift+c",
            "C",
        )
        .aliases(["Reply All, Add CC"]),
    );
    commands.add(
        with(
            "REPLY_ALL_CHANGE_BCC",
            "Reply All, Change BCC",
            Field::Bcc,
            "mod+shift+b",
            "B",
        )
        .aliases(["Reply All, Add BCC"]),
    );
    commands.add(
        with(
            "REPLY_ALL_CHANGE_FROM",
            "Reply All, Change From",
            Field::From,
            "mod+shift+f",
            "F",
        )
        .aliases(["Change Alias", "Change Aliases"]),
    );
    // ⌘⇧S starts a Reply All on its subject; the palette's row makes it a new message.
    commands.add(
        Command::new(
            "REPLY_ALL_CHANGE_SUBJECT",
            "Reply All, Change Subject",
            act(Action::ReplyAllEditSubject),
        )
        .key("mod+shift+s")
        .key_code(["⌘", "shift", "S"])
        .display_only()
        .aliases(["Edit Subject"])
        .default_score(1.8)
        .regions(MESSAGES),
    );
    commands.add(
        Command::shortcut(
            "REPLY_ALL_CHANGE_SUBJECT_KEY",
            act(Action::ReplyAllWith(Field::Subject)),
        )
        .key("mod+shift+s")
        .regions(MESSAGES),
    );
}

/// `Compose to {email}` for the person the contact pane shows, after `Compose`.
pub(crate) fn register_compose_to(
    commands: &mut Commands<commands::Action>,
    shown: &crate::format::names::Person,
) {
    let to = Address {
        name: Some(shown.name.clone()).filter(|name| !name.is_empty()),
        email: shown.email.clone(),
    };
    commands.add(
        Command::new(
            "COMPOSE_TO_EMAIL",
            format!("Compose to {}", shown.email),
            commands::Action::Compose(Action::ComposeTo(to)),
        )
        .follow("COMPOSE")
        .operative_word("compose")
        .icon("data/compose")
        .regions(exo_commands::LIST_AND_THREAD),
    );
}

/// Leaves a draft: an unmodified draft is discarded silently; any other is saved, if it
/// changed.
fn clean_up(
    open: &mut Open,
    account: &crate::account::Account,
    aliases: &[Alias],
    now: &jiff::Zoned,
) {
    if open.is_unmodified() && open.adopt.is_none() {
        if open.is_stored() {
            account.discard_draft(&open.id);
        }
    } else {
        save_changes(open, account, aliases, now);
    }
}

/// Saves `open` if it differs from what the store has: a touch that changed nothing, such as
/// Tab past an empty recipient input, saves nothing.
fn save_changes(
    open: &mut Open,
    account: &crate::account::Account,
    aliases: &[Alias],
    local: &jiff::Zoned,
) {
    if open.is_dirty() {
        save(open, account, aliases, local, false);
    } else {
        open.changed = None;
    }
}

/// Saves `open` now, dated by the system's clock in `local`'s time zone; `sending` when it's
/// saved to be sent, which Gmail gets even unchanged since it dates the message. A snippet
/// never goes to Gmail: Done saves it to the settings.
fn save(
    open: &mut Open,
    account: &crate::account::Account,
    aliases: &[Alias],
    local: &jiff::Zoned,
    sending: bool,
) {
    if open.is_snippet() {
        return;
    }
    open.sync_body();
    let email = outgoing::message(
        open,
        aliases,
        None,
        &Timestamp::now().to_zoned(local.time_zone().clone()),
    );
    account.save_draft(
        exo_store::Draft {
            id: open.id.clone(),
            thread: open.thread.as_deref().map(exo_store::ThreadId::new),
            adopted: open.adopt.take(),
            record: open.record.clone(),
            // The account adds the attachments, writes it and says whether it has them all.
            raw: String::new(),
            complete: false,
            sending,
        },
        email,
    );
    open.changed = None;
    open.saved = Some(open.record.clone());
}

/// The drafts of one thread, as the thread view lays them out.
pub(crate) struct Inline<'a> {
    drafts: &'a mut Vec<Open>,
    focus: &'a mut Option<String>,
    pressed_inside: &'a mut bool,
    pane: Option<&'a str>,
    /// The popped-out draft, of this thread or another.
    popped: Option<&'a str>,
    /// An alert, or the command palette, holds the keyboard.
    alert: bool,
    thread: &'a ThreadDetail,
    aliases: &'a [Alias],
    me: &'a str,
    book: Option<&'a contacts::Index>,
    thread_people: Option<&'a contacts::Index>,
    now: &'a jiff::Zoned,
    send_and_done: bool,
    availability_tip: bool,
    /// The inline forms' maximum height.
    max_height: f32,
}

impl<'a> Inline<'a> {
    /// A draft is popped out: the thread makes room for it.
    pub fn popped_out(&self) -> bool {
        self.popped.is_some()
    }

    fn shown(&self) -> Vec<&MessageDetail> {
        self.thread
            .messages
            .iter()
            .filter(|message| !message.is_draft())
            .collect()
    }

    /// Whether the wheel at `pointer`, moving the content `delta` down, scrolls a form's body or
    /// header rather than the thread ([`form::BodyScroll::takes_wheel`]).
    pub fn takes_wheel(&self, pointer: egui::Pos2, delta: f32) -> bool {
        self.drafts.iter().any(|open| {
            open.thread.as_deref() == Some(self.thread.id.as_str())
                && [&open.body_scroll, &open.header_scroll]
                    .iter()
                    .any(|scroll| scroll.takes_wheel(pointer, delta))
        })
    }

    /// The messages that have a draft under them.
    pub fn messages(&self) -> Vec<String> {
        let shown = self.shown();
        self.drafts
            .iter()
            .filter(|open| {
                open.thread.as_deref() == Some(self.thread.id.as_str())
                    && self.pane != Some(open.id.as_str())
            })
            .filter_map(|open| anchor(open, &shown))
            .collect()
    }

    /// The form context for the draft at `at`, in a thread whose message column spans `column`,
    /// in the message card `card`; it borrows nothing of `self`.
    fn context(
        &self,
        at: usize,
        column: egui::Rangef,
        card: Option<egui::Rect>,
    ) -> form::Context<'a> {
        form::Context {
            aliases: self.aliases,
            me: self.me,
            surface: Surface::Inline,
            focused: !self.alert && self.focus.as_deref() == Some(self.drafts[at].id.as_str()),
            book: self.book,
            thread_people: self.thread_people,
            pin: column,
            drop: card,
            now: self.now,
            thread_reminder: set_reminder(self.thread.reminder.as_ref()),
            send_and_done: self.send_and_done,
            max_height: self.max_height,
            availability_tip: self.availability_tip,
        }
    }

    fn draft_under(&self, message: &str) -> Option<usize> {
        let shown = self.shown();
        let thread = self.thread.id.as_str();
        self.drafts.iter().position(|open| {
            open.thread.as_deref() == Some(thread)
                && self.pane != Some(open.id.as_str())
                && anchor(open, &shown).as_deref() == Some(message)
        })
    }

    /// Whether the draft under `message` shows its form there rather than popped out.
    pub fn has_form(&self, message: &str) -> bool {
        self.draft_under(message)
            .is_some_and(|at| self.popped != Some(self.drafts[at].id.as_str()))
    }

    /// The height of the draft under `message` (separator included) at `width`.
    pub fn height(&mut self, ui: &egui::Ui, message: &str, width: f32) -> f32 {
        let Some(at) = self.draft_under(message) else {
            return 0.0;
        };
        if self.popped == Some(self.drafts[at].id.as_str()) {
            return form::SEPARATOR + form::PLACEHOLDER;
        }
        let cx = self.context(at, egui::Rangef::EVERYTHING, None);
        form::SEPARATOR + form::height(ui, &mut self.drafts[at], width, &cx)
    }

    /// Draws the draft under `message` in `form` (its separator's top and its width) inside the
    /// message's `card`, in a message column spanning `column`. Returns what it asked for.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        message: &str,
        form: egui::Rect,
        card: egui::Rect,
        column: egui::Rangef,
    ) -> Vec<Action> {
        let Some(at) = self.draft_under(message) else {
            return Vec::new();
        };
        let (left, top, width) = (form.left(), form.top(), form.width());
        form::separator(ui, left, top, width);
        let cx = self.context(at, column, Some(card));
        if self.popped == Some(self.drafts[at].id.as_str()) {
            return form::placeholder_header(
                ui,
                &self.drafts[at],
                left,
                top + form::SEPARATOR,
                width,
                &cx,
            );
        }
        let rect = egui::Rect::from_min_size(
            egui::pos2(left, top + form::SEPARATOR),
            egui::vec2(width, 0.0),
        );
        let response = form::show(ui, &mut self.drafts[at], rect, &cx);
        if response.clicked_inside {
            *self.pressed_inside = true;
            *self.focus = Some(self.drafts[at].id.clone());
        }
        response.actions
    }
}

/// A thread's reminder as the Remind button shows it: when, and whether it's `regardless`, while
/// it's set.
fn set_reminder(reminder: Option<&exo_store::ReminderStatus>) -> Option<(Timestamp, bool)> {
    match reminder? {
        exo_store::ReminderStatus::Set { at, keep_on_reply } => Some((*at, *keep_on_reply)),
        _ => None,
    }
}

/// Whether `from` was on `message` as Bcc, so replying all would tell everyone. Addresses
/// compare without case, as mailboxes do.
fn bcc_reply(message: &MessageDetail, from: &str) -> bool {
    message
        .bcc
        .iter()
        .any(|bcc| bcc.email.eq_ignore_ascii_case(from))
}

/// The message a draft hangs under: the one it answers, else the last.
fn anchor(open: &Open, shown: &[&MessageDetail]) -> Option<String> {
    let answers = open
        .record
        .in_reply_to
        .as_deref()
        .filter(|id| shown.iter().any(|message| message.id == *id));
    answers
        .map(str::to_owned)
        .or_else(|| shown.last().map(|message| message.id.clone()))
}

/// The draft record of a draft written elsewhere (Gmail web), from its message
/// (`adopt`): its HTML is the body, quote and all, as a revealed quote is, and its files are
/// its own (the first save fetches them, since it replaces the message they're in).
fn adopted(message: &MessageDetail, thread: &ThreadDetail) -> Open {
    let address = |contact: &exo_store::Contact| Address {
        name: contact.name.clone(),
        email: contact.email.clone(),
    };
    let answers = message.in_reply_to.as_deref().and_then(|rfc| {
        thread
            .messages
            .iter()
            .find(|other| other.rfc822_id.as_deref() == Some(rfc))
    });
    let action = if message.subject.to_lowercase().starts_with("fwd:") {
        DraftAction::Forward
    } else if message.in_reply_to.is_some() {
        if message.cc.is_empty() {
            DraftAction::Reply
        } else {
            DraftAction::ReplyAll
        }
    } else {
        DraftAction::Compose
    };
    let body = match (&message.body.html, &message.body.text) {
        (Some(html), _) => html.clone(),
        (None, Some(text)) => exo_mail::text_to_html(text),
        (None, None) => String::new(),
    };
    let mut record = exo_store::DraftRecord {
        action,
        from: message.from.as_ref().map(address),
        to: message.to.iter().map(address).collect(),
        cc: message.cc.iter().map(address).collect(),
        bcc: message.bcc.iter().map(address).collect(),
        subject: message.subject.clone(),
        body,
        quoted_content: None,
        quoted_inlined: true,
        in_reply_to: answers.map(|answers| answers.id.clone()),
        in_reply_to_rfc822: message.in_reply_to.clone(),
        references: message.references.clone(),
        rfc822_id: message.rfc822_id.clone().unwrap_or_else(|| {
            factory::new_rfc822_id(
                &message
                    .from
                    .as_ref()
                    .map(|from| from.email.clone())
                    .unwrap_or_default(),
                Timestamp::now(),
            )
        }),
        fingerprint: String::new(),
        attachments: Vec::new(),
        send_later: None,
        reminder: None,
    };
    factory::copy_attachments(&mut record, message, true);
    record.fingerprint = factory::fingerprint(&record);
    let mut open = Open::new(
        factory::new_id(),
        Some(thread.id.as_str().to_owned()),
        record,
    );
    open.adopt = Some(message.id.clone());
    open.mark_saved();
    open
}

/// The quoted body of `message`: its render's `<body>` contents and attributes, sanitized;
/// the raw HTML sanitized when it hasn't been rendered.
fn quoted_body(message: &MessageDetail, facts: Option<&crate::body::Facts>) -> factory::QuotedBody {
    match facts {
        Some(facts) => factory::QuotedBody {
            inner_html: body_inner(&facts.html),
            class: facts.body_attributes.class.clone().unwrap_or_default(),
            style: facts.body_attributes.style.clone().unwrap_or_default(),
            id: facts.body_attributes.id.clone().unwrap_or_default(),
        },
        None => {
            let html = match (&message.body.html, &message.body.text) {
                (Some(html), _) => html.clone(),
                (None, Some(text)) => exo_mail::text_to_html(text),
                (None, None) => String::new(),
            };
            factory::QuotedBody {
                inner_html: exo_mail::sanitize_fragment(&html),
                ..factory::QuotedBody::default()
            }
        }
    }
}

/// The contents of `html`'s `<body>`, serialized.
fn body_inner(html: &str) -> String {
    use html5ever::{parse_document, serialize, tendril::TendrilSink};
    use markup5ever_rcdom::{NodeData, RcDom, SerializableHandle};
    let dom = parse_document(RcDom::default(), Default::default()).one(html);
    let find_body = |node: &markup5ever_rcdom::Handle| -> Option<markup5ever_rcdom::Handle> {
        let mut stack = vec![node.clone()];
        while let Some(node) = stack.pop() {
            if let NodeData::Element { name, .. } = &node.data
                && &*name.local == "body"
            {
                return Some(node);
            }
            stack.extend(node.children.borrow().iter().rev().cloned());
        }
        None
    };
    let Some(body) = find_body(&dom.document) else {
        return String::new();
    };
    let mut out = Vec::new();
    let handle: SerializableHandle = body.into();
    let options = serialize::SerializeOpts {
        traversal_scope: serialize::TraversalScope::ChildrenOnly(None),
        ..Default::default()
    };
    if serialize::serialize(&mut out, &handle, options).is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl crate::Mail {
    /// The factory's view of the account.
    fn compose_account(&self) -> factory::Account<'_> {
        factory::Account {
            aliases: &self.snapshot.aliases,
            email: self.account.email(),
            tz: self.now.time_zone().clone(),
        }
    }

    /// Carries out a compose action.
    pub(crate) fn compose_run(&mut self, action: Action, now: f64) {
        match action {
            Action::New => self.compose_new(Fields::default(), None),
            Action::ComposeTo(address) => self.compose_new(
                Fields {
                    to: vec![address],
                    ..Fields::default()
                },
                None,
            ),
            Action::PopAndSearch => self.pop_out_and_search(now),
            Action::OpenAutoBccExclusion(email) => self.open_auto_bcc_exclusion(&email),
            Action::ExcludeFromAutoBcc(entry) => self.exclude_from_auto_bcc(entry),
            Action::NoteToSelf => {
                // Note to self: to the default alias named `Me`, subject selected.
                let from = self.compose_account().default_from();
                let fields = Fields {
                    to: vec![Address {
                        name: Some("Me".into()),
                        email: from.email.clone(),
                    }],
                    subject: "Note to self".into(),
                    ..Fields::default()
                };
                self.compose_new(fields, Some(Field::Subject));
            }
            Action::Reply(action) => self.compose_reply(action, None),
            // `Reply All, Change From` with one alias goes to the body.
            Action::ReplyAllWith(Field::From)
                if self
                    .snapshot
                    .aliases
                    .iter()
                    .filter(|alias| alias.active)
                    .count()
                    <= 1 =>
            {
                self.compose_reply(DraftAction::ReplyAll, Some(Field::Body))
            }
            Action::ReplyAllWith(field) => self.compose_reply(DraftAction::ReplyAll, Some(field)),
            Action::EditSubject => self.edit_reply_subject(now),
            Action::ReplyAllEditSubject => self.reply_all_as_new_message(),
            Action::FromToggle
            | Action::FromMove(_)
            | Action::FromChoose(_)
            | Action::FromClose => {
                let aliases = self.snapshot.aliases.clone();
                if let Some(open) = self.compose.focused_mut() {
                    from_menu(open, &aliases, &action, now);
                }
            }
            Action::SnippetMove(delta) => {
                if let Some(open) = self
                    .compose
                    .focused_mut()
                    .filter(|open| !open.snippet_rows.is_empty())
                {
                    open.snippet_pick = (open.snippet_pick as isize + delta)
                        .rem_euclid(open.snippet_rows.len() as isize)
                        as usize;
                }
            }
            Action::SnippetClose => {
                if let Some(open) = self.compose.focused_mut() {
                    if open.state.field == Field::Subject {
                        subject::dismiss(open);
                    } else {
                        open.editor.dismiss_trigger();
                    }
                    open.snippet_trigger = None;
                    open.snippet_rows.clear();
                }
            }
            Action::SnippetPick(index) => self.pick_inline_snippet(index, now),
            Action::SnippetInline => self.snippet_inline(now),
            Action::AvailabilityMenu(open) => self.compose_availability_menu(open),
            Action::AvailabilityDismiss => self.compose_availability_dismiss(),
            Action::DismissAvailabilityTip => self
                .account
                .update_settings(|settings| settings.has_shown_share_availability_tooltip = true),
            Action::AvailabilityPick => self.compose_availability_pick(),
            Action::Format(command) => {
                if let Some(open) = self.compose.focused_mut() {
                    command.run(&mut open.editor);
                    open.touch(now);
                }
            }
            Action::TypeBacktick => {
                if let Some(open) = self.compose.focused_mut() {
                    open.editor.type_text("`");
                    open.touch(now);
                }
            }
            Action::DateActive(text) => {
                if let Some(day) = crate::thread::dates::resolve(&text, &self.now) {
                    self.calendar_show_day(day);
                }
            }
            Action::ShiftDate(days) => {
                let today = self.now.clone();
                if let Some(open) = self.compose.focused_mut()
                    && let Some(date) = open.editor.active_date()
                    && let Some(text) = crate::thread::dates::shift(&date.text, days, &today)
                {
                    open.editor.shift_date(&date, &text);
                    open.touch(now);
                }
            }
            Action::LeavePane => {
                self.leave_pane_to_origin();
            }
            Action::Pop => self.compose_pop(),
            // A click pops the draft out or in, with the hint for its keys.
            Action::ClickPop => {
                self.compose_pop();
                let what = if self.compose.popped.is_some() {
                    "to pop out your draft!"
                } else {
                    "to pop in your draft!"
                };
                self.commands.hint(
                    Hint::new()
                        .text("Next time, hit")
                        .keys(["⌘", "shift", "P"])
                        .text(what),
                );
            }
            Action::ClosePopout => self.close_popout(),
            Action::ClickClosePopout => {
                self.close_popout();
                self.commands.hint(
                    Hint::new()
                        .text("Next time, hit")
                        .keys(["esc"])
                        .text("to instantly close your draft!"),
                );
            }
            Action::NewPoppedOut => self.compose_new_popped(Fields::default()),
            Action::ReplyPoppedOut(action) => self.compose_reply_popped(action),
            Action::CycleFocus => self.cycle_focus(),
            Action::IncludeOriginals => {
                if let Some(message) = self.focused_original().cloned()
                    && let Some(open) = self.compose.focused_mut()
                {
                    factory::copy_attachments(&mut open.record, &message, true);
                    open.touch(now);
                }
            }
            Action::Focus(field) => {
                if self.compose.focus.is_none()
                    && let Some((id, _)) = &self.compose.pane
                {
                    self.compose.focus = Some(id.clone());
                }
                let aliases = self.snapshot.aliases.clone();
                if let Some(open) = self.compose.focused_mut() {
                    let only_alias = aliases.iter().filter(|alias| alias.active).count() <= 1;
                    let field = if field == Field::From && only_alias {
                        Field::Body
                    } else {
                        field
                    };
                    let was = open.state.field;
                    match field {
                        Field::SnippetName
                        | Field::Body
                        | Field::QuotingStart
                        | Field::CollapsedHeader
                        | Field::Send
                        | Field::SendLater
                        | Field::RemindMe
                        | Field::Attach
                        | Field::Discard
                        | Field::ExpandRecipients
                        | Field::PopAction
                        | Field::PopClose => open.state.field = field,
                        other => open.state.expand(other),
                    }
                    // Focusing From opens its list; ⌘⇧F there moves it on.
                    if field == Field::From {
                        let cycle = if was == Field::From {
                            Action::FromMove(1)
                        } else {
                            Action::FromToggle
                        };
                        from_menu(open, &aliases, &cycle, now);
                    }
                }
            }
            Action::Send { mark_done } => self.compose_send(mark_done, checks::Bypass::default()),
            Action::ClickSend { mark_done } => {
                self.compose_send(mark_done, checks::Bypass::default());
                if self.compose.alert.is_none() {
                    self.commands.hint(send_hint());
                }
            }
            Action::Discard => self.compose_discard(),
            Action::Attach => {
                let paths = picker::pick_files(&[]);
                self.compose_attach(&paths, now);
            }
            // Insert Image picks from the image types; one picked goes in the body, more go on the
            // draft, as dropped files do.
            Action::InsertImage => {
                let paths = picker::pick_files(&[
                    "jpg", "jpeg", "jif", "jfif", "gif", "png", "bmp", "webp",
                ]);
                self.compose_add_files(&paths, now);
            }
            Action::OpenAttachment { uuid, preview } => self.open_draft_file(&uuid, preview),
            Action::RemoveAttachment(uuid) => {
                // Removing a file asks for no confirmation and has no undo.
                if let Some(open) = self.compose.focused_mut() {
                    open.record
                        .attachments
                        .retain(|attachment| attachment.uuid != uuid);
                    open.touch(now);
                }
            }
            Action::Escape => self.compose_escape(),
            Action::Tab(delta) => {
                let aliases = self.snapshot.aliases.clone();
                let popped = self.compose.popped.clone();
                if let Some((open, book)) = self.compose.focused_with_book() {
                    form::commit_input(open, now, book);
                    let surface = if popped.as_deref() == Some(open.id.as_str()) {
                        Surface::Popout
                    } else {
                        Surface::Inline
                    };
                    open.state.field = form::next_field(open, &aliases, surface, delta);
                    if open.state.field == Field::From {
                        from_menu(open, &aliases, &Action::FromToggle, now);
                    }
                }
            }
            Action::Commit => {
                if let Some((open, book)) = self.compose.focused_with_book() {
                    form::commit_input(open, now, book);
                }
            }
            Action::MoveSuggestion(delta) => {
                if let Some(input) = self
                    .compose
                    .focused_mut()
                    .and_then(|open| open.state.field.list().map(|list| open.input_mut(list)))
                {
                    let count = input.suggestions.len().max(1);
                    input.highlight =
                        (input.highlight as isize + delta).rem_euclid(count as isize) as usize;
                }
            }
            Action::PickSuggestion(index) => {
                if let Some((open, book)) = self.compose.focused_with_book()
                    && let Some(list) = open.state.field.list()
                {
                    open.input_mut(list).highlight = index;
                    form::commit_input(open, now, book);
                }
            }
            Action::RemoveSuggestion(index) => self.remove_suggestion(index),
            Action::DropChips {
                from,
                chips,
                into,
                at,
            } => {
                let Some(open) = self.compose.focused_mut() else {
                    return;
                };
                // Chips dropped into a reply's Bcc hint at ⌘⇧I, the recipients counted before the
                // drop.
                let hint = into == draft::List::Bcc
                    && open.record.in_reply_to.is_some()
                    && open.recipients().count() > 1;
                chips::drop(open, from, &chips, into, at, now);
                self.compose_run(Action::Focus(into.field()), now);
                if hint {
                    self.commands.hint(
                        Hint::new()
                            .text("Next time, hit")
                            .keys(["⌘", "shift", "I"])
                            .text("to instantly move contacts to BCC!"),
                    );
                }
            }
            Action::UndoRemoveSuggestion(email) => self.undo_remove_suggestion(&email),
            Action::CloseSuggestions => {
                if let Some(input) = self
                    .compose
                    .focused_mut()
                    .and_then(|open| open.state.field.list().map(|list| open.input_mut(list)))
                {
                    input.closed = true;
                }
            }
            Action::Backspace => {
                if let Some(open) = self.compose.focused_mut() {
                    form::backspace(open, now);
                }
            }
            Action::MoveSelection(delta) => {
                if let Some((open, book)) = self.compose.focused_with_book() {
                    form::move_selection(open, delta, book, now);
                }
            }
            Action::RevealQuote => {
                let aliases = self.snapshot.aliases.clone();
                if let Some(open) = self.compose.focused_mut() {
                    form::reveal_quote(open, &aliases, now);
                }
            }
            Action::Toolbar(action) => {
                if let Some((before, keys, after)) = action.hint() {
                    self.commands.hint(
                        Hint::new()
                            .text(before)
                            .keys(keys.iter().copied())
                            .text(after),
                    );
                }
            }
            Action::Learned(word) => self
                .commands
                .hint(Hint::new().text(format!("Learned \"{word}\""))),
            Action::OpenLink(url)
                if url
                    .get(..7)
                    .is_some_and(|scheme| scheme.eq_ignore_ascii_case("mailto:")) =>
            {
                self.compose_mailto(&url, false)
            }
            Action::OpenLink(url) => self.open_url(&url),
            Action::ClickRevealQuote => {
                self.compose_run(Action::RevealQuote, now);
                self.commands.hint(
                    Hint::new()
                        .text("Go faster next time: use")
                        .keys(["↓"])
                        .text("to focus")
                        .keys(["…"])
                        .text(", then hit")
                        .keys(["enter"])
                        .text("!"),
                );
            }
            Action::DeleteQuote => {
                if let Some(open) = self.compose.focused_mut() {
                    open.record.quoted_inlined = true;
                    open.state.field = Field::Body;
                    open.touch(now);
                }
            }
            Action::UndoSend(tag) => self.undo_send(tag),
            Action::InstantSend(tag) => {
                if let Some(sending) = self
                    .compose
                    .sends
                    .iter_mut()
                    .find(|sending| sending.tag == tag)
                {
                    sending.until = Timestamp::now();
                    self.account.send_draft(&sending.open.id, sending.until);
                    self.commands
                        .toast(Toast::new("Send accelerated!").tag(tag));
                }
            }
            Action::UndoDiscard(tag) => {
                if let Some(discarded) = self
                    .compose
                    .discarded
                    .take_if(|discarded| discarded.tag == tag)
                {
                    let Discarded {
                        mut open, surface, ..
                    } = discarded;
                    if open.is_stored() {
                        save(
                            &mut open,
                            &self.account,
                            &self.snapshot.aliases,
                            &self.now,
                            false,
                        );
                    }
                    self.reopen(open, surface);
                }
            }
            Action::DiscardListed => self.discard_listed(),
            Action::Recover(message) => self.recover(&message),
            Action::EditFailed(id) => self.edit_failed(&id),
            Action::DiscardFailed(id) => self.discard_failed(&id),
            Action::EditSending(id) => {
                if let Some(tag) = self
                    .compose
                    .sends
                    .iter()
                    .find(|sending| sending.open.id == id)
                    .map(|sending| sending.tag)
                {
                    self.undo_send(tag);
                }
            }
            Action::DiscardSending(id) => self.discard_sending(&id),
            Action::SendLater => self.open_send_later(),
            Action::RemindMe => self.open_draft_reminder(),
            Action::Schedule {
                draft,
                at,
                abort_on_reply,
                remember,
            } => self.schedule_picked(&draft, at, abort_on_reply, remember),
            Action::EditScheduled(id) => self.edit_scheduled(&id),
            Action::DiscardScheduled(id) => self.discard_failed(&id),
            Action::UnremindSent(id) => self.unremind_sent(&id),
            Action::InstantIntro => self.instant_intro(now),
            Action::InstantIntroReply => self.instant_intro_reply(now),
            Action::Assistant(key) => self.assistant_key(key, now),
            Action::UseEmoji => {
                if let Some(open) = self.compose.focused_mut() {
                    assistant::use_emoji(open, now);
                }
            }
            Action::RemoveRecipients(group) => {
                let team = reminder::team_domains(
                    self.account.settings(),
                    &self.snapshot.aliases,
                    self.account.email(),
                );
                if let Some(open) = self.compose.focused_mut() {
                    assistant::remove(open, &group, &team);
                    open.touch(now);
                }
            }
            Action::Chip(key) => {
                if let Some((open, book)) = self.compose.focused_with_book()
                    && let Some(text) = chips::run(open, key, book, now)
                {
                    self.ctx.copy_text(text);
                }
            }
            Action::UndoDiscards(tag) => self.undo_discards(tag),
            Action::UndoDiscardSnippet(tag) => self.undo_discard_snippet(tag),
            Action::Alert(index) => self.alert_action(index),
            Action::AlertSelect(delta) => {
                if let Some(alert) = &mut self.compose.alert {
                    alert.selected = alert
                        .selected
                        .saturating_add_signed(delta)
                        .min(alert.actions.len() - 1);
                }
            }
        }
    }

    /// A new message in the thread pane, from `fields`.
    pub(crate) fn compose_new(&mut self, fields: Fields, field: Option<Field>) {
        let record = factory::compose(&self.compose_account(), fields, Timestamp::now());
        self.open_new(Open::new(factory::new_id(), None, record), field);
    }

    /// Shows `open`, a new message, in the thread pane with the keyboard (on `field`).
    fn open_new(&mut self, mut open: Open, field: Option<Field>) {
        if self.snippet_unsaved() {
            self.ask_leaving_snippet(Box::new(move |mail| mail.open_new(open, field)));
            return;
        }
        self.with_auto_bcc(&mut open);
        // Leaving any other new message first.
        self.leave_pane();
        if let Some(field) = field {
            open.state.field = field;
        }
        let from = self.thread.as_ref().map(|thread| thread.id().to_owned());
        self.close_thread();
        self.compose.focus = Some(open.id.clone());
        self.compose.pane = Some((open.id.clone(), from));
        self.compose.drafts.push(open);
    }

    /// Change Subject: Gmail threads by subject, so a reply whose subject is to change
    /// becomes a new message: the reply is discarded and its copy, unthreaded, pops out with its
    /// subject selected. A new message's subject is only focused.
    fn edit_reply_subject(&mut self, now: f64) {
        let Some(id) = self.compose.focus.clone() else {
            return;
        };
        let Some(at) = self.compose.drafts.iter().position(|open| open.id == id) else {
            return;
        };
        if self.compose.drafts[at].is_new_email() {
            self.compose.drafts[at].state.expand(Field::Subject);
            return;
        }
        let mut reply = self.compose.drafts.remove(at);
        reply.sync_body();
        if reply.is_stored() {
            self.account.discard_draft(&reply.id);
        }
        self.compose.focus = None;
        if self.compose.popped.as_deref() == Some(id.as_str()) {
            self.compose.popped = None;
        }
        let mut record = reply.record;
        factory::unthread(&mut record);
        let mut open = Open::new(factory::new_id(), None, record);
        open.select_subject = true;
        open.touch(now);
        let new = open.id.clone();
        self.compose.drafts.push(open);
        self.pop_out(&new);
    }

    /// `Reply All, Change Subject` from the palette with no draft open: a Reply All of the
    /// focused message, as a new message with its subject selected.
    fn reply_all_as_new_message(&mut self) {
        let Some(detail) = self
            .thread
            .as_ref()
            .and_then(crate::thread::ThreadView::detail)
        else {
            return;
        };
        let focused = self
            .thread
            .as_ref()
            .and_then(crate::thread::ThreadView::focused);
        let Some(message) = detail
            .messages
            .iter()
            .filter(|message| !message.is_draft())
            .find(|message| Some(message.id.as_str()) == focused)
            .cloned()
        else {
            return;
        };
        let body = quoted_body(&message, self.bodies.facts(&message.id));
        let mut record = factory::reply(
            &self.compose_account(),
            DraftAction::ReplyAll,
            &message,
            message.has_label("SENT"),
            &body,
            Timestamp::now(),
        );
        factory::unthread(&mut record);
        let mut open = Open::new(factory::new_id(), None, record);
        open.select_subject = true;
        self.open_new(open, Some(Field::Subject));
    }

    /// A reply, reply all or forward of the open (or highlighted) conversation's focused
    /// message; an existing draft on that message is reused.
    fn compose_reply(&mut self, action: DraftAction, field: Option<Field>) {
        let thread = match &self.thread {
            Some(thread) => thread.id().to_owned(),
            None => {
                // From the list: open the highlighted conversation (marking it read) and reply
                // once it's loaded.
                let Some(id) = self.inbox.cursor().map(|thread| thread.id.clone()) else {
                    return;
                };
                self.open_thread(id.clone());
                id
            }
        };
        // A selection in the focused message is quoted, unless a field was asked for.
        let quote = field
            .is_none()
            .then(|| {
                self.thread
                    .as_ref()
                    .and_then(|view| self.bodies.selected_html(view.focused()?))
            })
            .flatten()
            .map(str::to_owned);
        if quote.is_some() {
            // The draft takes the keyboard, and with it the page's one selection.
            self.bodies.clear_selection();
        }
        self.compose.pending = Some(PendingReply {
            thread,
            action,
            field,
            pop_out: false,
            body: None,
            quote,
        });
        self.create_pending_reply();
    }

    /// Creates the reply asked for once its thread has loaded.
    fn create_pending_reply(&mut self) {
        let Some(pending) = &self.compose.pending else {
            return;
        };
        let Some(view) = &self.thread else {
            self.compose.pending = None;
            return;
        };
        if view.id() != pending.thread {
            self.compose.pending = None;
            return;
        }
        let Some(detail) = view.detail().cloned() else {
            return;
        };
        // The message answered: the focused one, else the first unread, else the last.
        let shown: Vec<&MessageDetail> = detail
            .messages
            .iter()
            .filter(|message| !message.is_draft())
            .collect();
        let message = view
            .focused()
            .and_then(|id| shown.iter().find(|message| message.id == id))
            .or_else(|| shown.iter().find(|message| message.is_unread()))
            .or_else(|| shown.last())
            .copied();
        // Its quote needs its body, rendered: the reply waits for it, and without Gmail to fetch
        // it from, doesn't start.
        if message.is_some_and(|message| self.snapshot.bodiless.contains(&message.id)) {
            if !self.snapshot.connection.is_online() {
                self.compose.pending = None;
            }
            return;
        }
        let pending = self.compose.pending.take().expect("checked above");
        let (action, field) = (pending.action, pending.field);
        let Some(message) = message.cloned() else {
            return;
        };
        let in_sent = message.has_label("SENT");
        let body = quoted_body(&message, self.bodies.facts(&message.id));
        let thread_id = detail.id.as_str().to_owned();
        let existing = self.compose.drafts.iter().position(|open| {
            open.thread.as_deref() == Some(thread_id.as_str())
                && open.record.in_reply_to.as_deref() == Some(message.id.as_str())
        });
        // Quoting into a draft with Reply All keeps the draft's own action.
        let keep_action = pending.quote.is_some() && action == DraftAction::ReplyAll;
        let id = match existing {
            Some(at) => {
                let account = factory::Account {
                    aliases: &self.snapshot.aliases,
                    email: self.account.email(),
                    tz: self.now.time_zone().clone(),
                };
                let open = &mut self.compose.drafts[at];
                if open.record.action != action && open.adopt.is_none() && !keep_action {
                    factory::retarget(&mut open.record, &account, action, &message, in_sent, &body);
                    open.state = draft::State::default_for(&open.record);
                    open.touch(0.0);
                }
                open.id.clone()
            }
            None => {
                let record = factory::reply(
                    &self.compose_account(),
                    action,
                    &message,
                    in_sent,
                    &body,
                    Timestamp::now(),
                );
                let mut open = Open::new(factory::new_id(), Some(thread_id), record);
                self.with_auto_bcc(&mut open);
                let id = open.id.clone();
                self.compose.drafts.push(open);
                id
            }
        };
        if let (Some(open), Some(field)) = (self.compose.get_mut(&id), field) {
            open.state.expand(field);
        }
        if let (Some(open), Some(body)) = (self.compose.get_mut(&id), &pending.body) {
            availability::insert_at_caret(open, body);
        }
        if let (Some(open), Some(quote)) = (self.compose.get_mut(&id), &pending.quote) {
            open.editor.add_quote(quote);
            open.touch(0.0);
        }
        self.compose.focus = Some(id.clone());
        if let Some(view) = &mut self.thread {
            view.reveal_draft(&message.id);
        }
        self.pop_out_created(&pending, &id);
        let from = self
            .compose
            .get_mut(&id)
            .and_then(|open| open.record.from.clone());
        if action == DraftAction::ReplyAll
            && from.is_some_and(|from| bcc_reply(&message, &from.email))
        {
            self.commands.hint(
                Hint::new()
                    .text("Careful, you were on BCC! Reply instead of Reply All?")
                    .keys(["esc", "R"]),
            );
        }
    }

    /// Opens the drafts of the open thread once it's loaded: Exo's own, and drafts written
    /// elsewhere (which Exo takes over on their first save). Then any reply waiting for it.
    pub(crate) fn compose_sync(&mut self) {
        self.compose.send_and_done = self.account.settings().send_and_done;
        self.compose.backtick_escape = self.account.settings().backtick_as_escape();
        self.compose.availability_tip = self.availability_tip_due();
        let Some(view) = &self.thread else { return };
        let Some(detail) = view.detail().cloned() else {
            return;
        };
        let thread = detail.id.as_str().to_owned();
        if self.compose.loaded.as_deref() != Some(thread.as_str()) {
            // A draft written elsewhere is taken up with its body: until that's in, the thread
            // loads again.
            let waiting = detail
                .messages
                .iter()
                .any(|message| message.is_draft() && self.snapshot.bodiless.contains(&message.id));
            if !waiting {
                self.compose.loaded = Some(thread.clone());
            }
            let sending: Vec<&str> = self
                .compose
                .sends
                .iter()
                .map(|sending| sending.open.id.as_str())
                .collect();
            // A failed draft is a message of the thread until it's edited.
            for stored in self.snapshot.drafts.iter().filter(|draft| {
                draft
                    .thread
                    .as_ref()
                    .is_some_and(|id| id.as_str() == thread)
                    && draft.send_at.is_none()
                    && draft.failure.is_none()
            }) {
                if !self.compose.drafts.iter().any(|open| open.id == stored.id)
                    && !sending.contains(&stored.id.as_str())
                {
                    self.compose.drafts.push(Open::stored(stored));
                }
            }
            let ours: Vec<&str> = self
                .snapshot
                .drafts
                .iter()
                .filter_map(|draft| draft.message_id.as_deref())
                .collect();
            let foreign: Vec<Open> = detail
                .messages
                .iter()
                .filter(|message| {
                    message.is_draft()
                        && !ours.contains(&message.id.as_str())
                        && !self.snapshot.bodiless.contains(&message.id)
                })
                .filter(|message| {
                    !self
                        .compose
                        .drafts
                        .iter()
                        .any(|open| open.adopt.as_deref() == Some(message.id.as_str()))
                })
                .map(|message| adopted(message, &detail))
                .collect();
            self.compose.drafts.extend(foreign);
        } else {
            // A reply that called off a scheduled send makes it a draft again, in its thread.
            let unscheduled: Vec<Open> = self
                .snapshot
                .drafts
                .iter()
                .filter(|draft| {
                    draft
                        .thread
                        .as_ref()
                        .is_some_and(|id| id.as_str() == thread)
                        && draft.send_at.is_none()
                        && draft.failure.is_none()
                })
                .filter(|draft| {
                    draft.record.send_later.as_ref().is_some_and(|later| {
                        later.abort_on_reply && !later.current_message_ids.is_empty()
                    })
                })
                .filter(|draft| {
                    !self.compose.drafts.iter().any(|open| open.id == draft.id)
                        && !self
                            .compose
                            .sends
                            .iter()
                            .any(|sending| sending.open.id == draft.id)
                })
                .map(Open::stored)
                .collect();
            self.compose.drafts.extend(unscheduled);
        }
        self.create_pending_reply();
    }

    /// Opens a draft-only thread: its draft in the new-message pane.
    pub(crate) fn open_draft_thread(&mut self, thread: &str) -> bool {
        let Some(stored) = self.snapshot.drafts.iter().find(|draft| {
            draft
                .thread
                .as_ref()
                .is_some_and(|id| id.as_str() == thread)
                && draft.send_at.is_none()
                && draft.failure.is_none()
        }) else {
            return false;
        };
        if self.compose.popped.as_deref() == Some(stored.id.as_str()) {
            self.pop_in();
            return true;
        }
        let open = Open::stored(stored);
        self.leave_pane();
        self.compose.focus = Some(open.id.clone());
        self.compose.pane = Some((open.id.clone(), None));
        self.compose.drafts.retain(|other| other.id != open.id);
        self.compose.drafts.push(open);
        true
    }

    /// Closes the new-message pane, saving or silently discarding its draft.
    pub(crate) fn leave_pane(&mut self) {
        let Some((id, _)) = self.compose.pane.take() else {
            return;
        };
        if let Some(at) = self.compose.drafts.iter().position(|open| open.id == id) {
            let mut open = self.compose.drafts.remove(at);
            clean_up(&mut open, &self.account, &self.snapshot.aliases, &self.now);
        }
        if self.compose.focus.as_deref() == Some(id.as_str()) {
            self.compose.focus = None;
        }
    }

    /// The thread is closing: its drafts are saved or dropped.
    pub(crate) fn compose_close_thread(&mut self, thread: &str) {
        self.compose
            .close_thread_drafts(thread, &self.account, &self.snapshot.aliases, &self.now);
        self.compose.in_thread.close(thread);
        self.compose.pending = None;
    }

    /// Escape in a draft: an unmodified draft goes; otherwise the form keeps it and the
    /// keyboard goes back to the thread.
    fn compose_escape(&mut self) {
        if self.compose.focused().is_some_and(Open::is_snippet) {
            self.escape_snippet();
            return;
        }
        let Some(id) = self.compose.focus.clone() else {
            return;
        };
        if self.compose.popped.as_deref() == Some(id.as_str()) {
            self.close_popout();
            return;
        }
        let Some(at) = self.compose.drafts.iter().position(|open| open.id == id) else {
            return;
        };
        let unmodified = {
            let open = &self.compose.drafts[at];
            open.is_unmodified() && open.adopt.is_none()
        };
        self.compose.focus = None;
        if unmodified {
            let open = self.compose.drafts.remove(at);
            if open.is_stored() {
                self.account.discard_draft(&open.id);
            }
            if self.compose.pane() == Some(id.as_str()) {
                self.close_pane_to_origin();
            }
        } else {
            let open = &mut self.compose.drafts[at];
            save_changes(open, &self.account, &self.snapshot.aliases, &self.now);
        }
    }

    /// Leaves the new-message pane for wherever it was opened from.
    fn close_pane_to_origin(&mut self) {
        let Some((id, from)) = self.compose.pane.take() else {
            return;
        };
        self.compose
            .drafts
            .retain(|open| open.id != id || open.changed.is_some());
        if let Some(thread) = from {
            self.open_thread(thread);
        }
    }

    /// Discard, with its toast: the draft goes (from Gmail too), and Undo brings it back where
    /// it was.
    fn compose_discard(&mut self) {
        if self.compose.focused().is_some_and(Open::is_snippet) {
            self.ask_discard_snippet();
            return;
        }
        let Some(id) = self.compose.focus.clone() else {
            return;
        };
        let Some(at) = self.compose.drafts.iter().position(|open| open.id == id) else {
            return;
        };
        let mut open = self.compose.drafts.remove(at);
        open.sync_body();
        self.compose.focus = None;
        if open.is_stored() {
            self.account.discard_draft(&open.id);
        }
        let surface = if self.compose.pane() == Some(id.as_str()) {
            self.close_pane_to_origin();
            Surface::Pane
        } else if self
            .compose
            .popped
            .take_if(|popped| *popped == id)
            .is_some()
        {
            Surface::Popout
        } else {
            Surface::Inline
        };
        let tag = self.compose.tag();
        self.compose.discarded = Some(Discarded { tag, open, surface });
        self.commands.toast(
            Toast::new("Draft discarded.")
                .undo(commands::Action::Compose(Action::UndoDiscard(tag)))
                .tag(tag),
        );
    }

    /// Puts a draft back where it was (after an undo), with the keyboard. Into the pane or a
    /// thread, it takes the place of a snippet being edited once `Save Snippet?` has asked, so
    /// the snippet's changes are never dropped unasked.
    fn reopen(&mut self, open: Open, surface: Surface) {
        if surface != Surface::Popout && self.snippet_unsaved() {
            self.ask_leaving_snippet(Box::new(move |mail| mail.reopen(open, surface)));
            return;
        }
        let id = open.id.clone();
        let thread = open.thread.clone();
        let anchor = open.record.in_reply_to.clone();
        self.compose.drafts.retain(|other| other.id != id);
        match surface {
            Surface::Popout => {
                self.compose.drafts.push(open);
                self.pop_out(&id);
            }
            Surface::Pane => {
                self.leave_pane();
                self.close_thread();
                self.compose.pane = Some((id.clone(), None));
                self.compose.drafts.push(open);
            }
            Surface::Inline => {
                if let Some(thread) = &thread
                    && self.thread.as_ref().is_none_or(|view| view.id() != thread)
                {
                    self.open_thread(thread.clone());
                }
                // Loading the thread opens its stored drafts; this one is ours already.
                self.compose.loaded = thread.clone();
                self.compose.drafts.push(open);
                if let (Some(view), Some(anchor)) = (&mut self.thread, anchor) {
                    view.reveal_draft(&anchor);
                }
            }
        }
        self.compose.focus = Some(id);
    }

    /// Send: the checks, then the send (held for undo) and its toast.
    fn compose_send(&mut self, mark_done: bool, bypass: checks::Bypass) {
        if self.compose.focused().is_some_and(Open::is_snippet) {
            self.save_snippet();
            return;
        }
        let aliases = self.snapshot.aliases.clone();
        let Some(id) = self.compose.focus.clone() else {
            return;
        };
        let Some(at) = self.compose.drafts.iter().position(|open| open.id == id) else {
            return;
        };
        let now = self.ctx.input(|input| input.time);
        {
            let book = self.compose.book.as_ref().map(|(_, index)| index);
            let open = &mut self.compose.drafts[at];
            form::commit_all(open, now, book);
            let html = outgoing::html(open, &aliases);
            let from_ok = open.record.from.as_ref().is_none_or(|from| {
                aliases.is_empty()
                    || aliases.iter().any(|alias| {
                        alias.active && recipients::same_address(&alias.email, &from.email)
                    })
            });
            if let Some(problem) = checks::check(open, &html, from_ok, bypass, self.now.timestamp())
            {
                let (actions, then) = match problem.kind {
                    checks::Kind::SendAnyway(bypass) => (
                        vec!["Send anyway", "Cancel"],
                        AlertThen::SendAnyway(bypass, mark_done),
                    ),
                    checks::Kind::Ok { focus_from } => (vec!["OK"], AlertThen::Ok { focus_from }),
                };
                self.compose.alert =
                    Some(Alert::new(problem.title, problem.text, actions, id, then));
                return;
            }
        }
        let mut open = self.compose.drafts.remove(at);
        let surface = if self.compose.pane() == Some(id.as_str()) {
            Surface::Pane
        } else if self
            .compose
            .popped
            .take_if(|popped| *popped == id)
            .is_some()
        {
            Surface::Popout
        } else {
            Surface::Inline
        };
        // A send a reply called off goes now: its time goes with the old job.
        if open.unscheduled_by.take().is_some() {
            open.record.send_later = None;
        }
        let scheduled = open.record.send_later.as_ref().map(|later| later.at);
        // Writing to a removed suggestion brings it back (its `excluded_contacts` entry goes).
        let sent_to: Vec<String> = open
            .recipients()
            .map(|address| address.email.clone())
            .collect();
        self.account.update_settings(|settings| {
            settings
                .excluded_contacts
                .retain(|email, _| !sent_to.iter().any(|sent| sent.eq_ignore_ascii_case(email)))
        });
        save(&mut open, &self.account, &aliases, &self.now, true);
        let until = match scheduled {
            Some(at) => {
                // Held for its time; its thread shows Gmail's copy as scheduled.
                self.account.schedule_draft(&open.id);
                at
            }
            None => {
                let until = Timestamp::now().checked_add(HOLD).unwrap_or(Timestamp::MAX);
                self.account.send_draft(&open.id, until);
                until
            }
        };
        self.compose.focus = None;
        // A reply shows in its thread as sent at once, focused at the end.
        if !open.is_new_email()
            && scheduled.is_none()
            && let Some(shown) = self.compose.in_thread.show(
                &open,
                &outgoing::html(&open, &aliases),
                &aliases,
                self.now.timestamp().checked_add(HOLD).unwrap_or(until),
            )
            && let Some(view) = self
                .thread
                .as_mut()
                .filter(|view| open.thread.as_deref() == Some(view.id()))
        {
            view.reveal_sent(&shown);
        }
        // Outside the new-message pane, a send marks its conversation done when asked
        // (`Send & Mark Done`, ⌘⇧) or with `Send + Mark Done` on; one that's only this draft has
        // nothing to mark.
        let mark_done =
            surface != Surface::Pane && (mark_done || self.account.settings().send_and_done);
        let thread = open
            .thread
            .clone()
            .filter(|_| mark_done && !open.is_new_email());
        let done = thread.map(|thread| self.mark_done_with_send(&thread, now));
        let tag = self.compose.tag();
        let reminded = open.record.reminder.is_some();
        let sending = Sending {
            tag,
            open,
            surface,
            done,
            until,
            acked: false,
            undoing: false,
            held_at: f64::NEG_INFINITY,
            scheduled: scheduled.is_some(),
        };
        let text = if sending.scheduled {
            "Scheduling…"
        } else {
            "Sending…"
        };
        self.commands
            .toast(send_toast(text, tag, !sending.scheduled).sticky());
        self.compose.sends.push(sending);
        if surface == Surface::Pane {
            self.close_pane_to_origin();
        }
        self.follow_up_hint(&id, reminded, now);
    }

    /// Marks `thread` done as part of a send, without a toast of its own; the conversation
    /// leaves the list as `e` would take it, Auto Advance and all. Returns the undo token.
    fn mark_done_with_send(&mut self, thread: &str, now: f64) -> crate::account::UndoToken {
        let token = self.account.undo_token();
        self.account
            .apply(&[thread.to_owned()], Mutation::MarkDone, Some(token));
        let leaving = self
            .inbox
            .threads()
            .iter()
            .any(|row| row.id == thread && row.removed_by.contains(&Mutation::MarkDone));
        if leaving {
            self.dismiss_and_advance(&[thread.to_owned()], 0, now);
        }
        token
    }

    /// The toast's Undo: the send is taken back if it's still held.
    fn undo_send(&mut self, tag: u64) {
        let Some(sending) = self
            .compose
            .sends
            .iter_mut()
            .find(|sending| sending.tag == tag)
        else {
            return;
        };
        sending.undoing = true;
        self.account.undo_send(&sending.open.id);
        self.commands
            .toast(Toast::new("Undoing…").tag(tag).sticky());
    }

    /// An alert's action.
    fn alert_action(&mut self, index: usize) {
        let Some(alert) = self.compose.alert.take() else {
            return;
        };
        let first = index == 0;
        match alert.then {
            AlertThen::SendAnyway(bypass, mark_done) if first => {
                self.compose.focus = Some(alert.draft);
                self.compose_send(mark_done, bypass);
            }
            AlertThen::SendAnyway(..) => self.compose.focus = Some(alert.draft),
            AlertThen::Ok { focus_from } => {
                self.compose.focus = Some(alert.draft);
                if focus_from && let Some(open) = self.compose.focused_mut() {
                    open.state.expand(Field::From);
                }
            }
            AlertThen::SeeMessage => {
                if let Some(at) = self
                    .compose
                    .sends
                    .iter()
                    .position(|sending| sending.open.id == alert.draft)
                {
                    let sending = self.compose.sends.remove(at);
                    if !self.see_failed(&alert.draft) {
                        self.reopen(sending.open, sending.surface);
                    }
                }
            }
            AlertThen::SaveSnippet => self.answer_save_snippet(index, alert.draft),
            AlertThen::DiscardSnippet if first => self.discard_snippet(),
            AlertThen::DiscardSnippet => self.compose.focus = Some(alert.draft),
            AlertThen::CreateSnippet if first => self.create_snippet(String::new()),
            AlertThen::CreateSnippet => {}
            AlertThen::FollowUp if first => self.follow_up_set_reminder(alert.draft),
            AlertThen::FollowUp => self.compose.follow_up = None,
        }
    }

    /// Saves what changed, at most once a second per draft, and keeps held sends' toasts up
    /// to date (hovering one holds the send longer).
    pub(crate) fn compose_tick(&mut self, now: f64) {
        self.compose_address_book();
        // What typing does follows the settings.
        let settings = self.account.settings();
        let editor = exo_editor::Settings {
            hyphen_starts_list: settings.convert_hyphen_to_bullet_list,
            ..exo_editor::Settings::default()
        };
        let signature_on_replies = settings.include_signature_on_replies();
        let online = self.snapshot.connection.is_online();
        for open in &mut self.compose.drafts {
            // No date detection in a snippet's body; where its date tooltip keeps clear of.
            let popped_out = self.compose.popped.as_deref() == Some(open.id.as_str());
            let another_popped_out = self.compose.popped.is_some() && !popped_out;
            open.editor.set_settings(exo_editor::Settings {
                skip_dates: open.is_snippet(),
                popped_out,
                another_popped_out,
                ..editor
            });
            open.signature_on_replies = signature_on_replies;
            open.upload = self.account.upload(&open.id);
            open.online = online;
        }
        self.compose.missing_originals = match (self.focused_original(), self.compose.focused()) {
            (Some(message), Some(open)) if open.record.action != DraftAction::Forward => {
                factory::missing_originals(&open.record, message)
            }
            _ => 0,
        };
        self.include_attachments_hint();
        self.inline_snippet_rows();
        self.assistant_rows();
        self.postpone_for_follow_up(now);
        self.automatic_reminders(now);
        self.note_unscheduled();
        self.unschedule_passed();
        let book = self.compose.book.as_ref().map(|(_, index)| index);
        for open in &mut self.compose.drafts {
            let has_keyboard = self.compose.focus.as_deref() == Some(open.id.as_str());
            chips::blur(open, has_keyboard, book, now);
            chips::note(open);
        }
        self.auto_bcc_tick();
        for open in &mut self.compose.drafts {
            chips::settle(open);
        }
        self.start_pending_intro(now);
        if let Some(wait) = self.compose_save_changes(now, true) {
            self.ctx.request_repaint_after_secs(wait as f32);
        }
        let showing = self.commands.toast_tag();
        let hovered = self.commands.toast_hovered();
        // A scheduled send's toast is its only way back here; once it's gone, the send waits in
        // the store, and its thread shows it. One a reply called off is a draft again.
        let unscheduled: Vec<&str> = self
            .snapshot
            .drafts
            .iter()
            .filter(|draft| {
                draft.send_at.is_none()
                    && draft
                        .record
                        .send_later
                        .as_ref()
                        .is_some_and(|later| !later.current_message_ids.is_empty())
            })
            .map(|draft| draft.id.as_str())
            .collect();
        self.compose.sends.retain(|sending| {
            !(sending.scheduled
                && sending.acked
                && !sending.undoing
                && (showing != Some(sending.tag)
                    || unscheduled.contains(&sending.open.id.as_str())))
        });
        for sending in self
            .compose
            .sends
            .iter_mut()
            .filter(|sending| !sending.scheduled)
        {
            if Some(sending.tag) == showing
                && hovered
                && sending.acked
                && !sending.undoing
                && now - sending.held_at >= HOVER_EVERY
            {
                sending.held_at = now;
                let wanted = Timestamp::now()
                    .checked_add(HOVER_HOLD)
                    .unwrap_or(Timestamp::MAX);
                if wanted > sending.until {
                    sending.until = wanted;
                    self.account.send_draft(&sending.open.id, wanted);
                }
            }
        }
        if self.compose.sends.iter().any(|sending| {
            Some(sending.tag) == showing && hovered && sending.acked && !sending.scheduled
        }) {
            self.ctx.request_repaint_after_secs(HOVER_EVERY as f32);
        }
    }

    /// Saves the drafts with changes, at time `now`: once a second per draft when `throttled`,
    /// else every one at once. Returns how long until the next throttled save is due.
    fn compose_save_changes(&mut self, now: f64, throttled: bool) -> Option<f64> {
        let aliases = self.snapshot.aliases.clone();
        let mut wait = None::<f64>;
        for open in &mut self.compose.drafts {
            // A snippet is saved with Done only.
            let Some(since) = open.changed.filter(|_| !open.is_snippet()) else {
                continue;
            };
            if open.adopt.is_none() && !open.is_stored() && open.is_unmodified() {
                // Nothing worth a Gmail draft yet.
                open.changed = None;
                continue;
            }
            let due = since.max(open.saved_at) + SAVE_EVERY;
            if now >= due || !throttled {
                save_changes(open, &self.account, &aliases, &self.now);
                open.saved_at = now;
            } else {
                wait = Some(wait.map_or(due - now, |wait: f64| wait.min(due - now)));
            }
        }
        wait
    }

    /// Saves every open draft's latest changes at once, rather than within the second: quitting
    /// does, before the account sends what it holds, and so does leaving the account for
    /// another.
    pub fn save_drafts(&mut self) {
        let now = self.ctx.input(|input| input.time);
        self.compose_save_changes(now, false);
    }

    /// After the frame is drawn: a press anywhere but the focused draft's form takes the
    /// keyboard from it (the draft is saved and stays open).
    pub(crate) fn compose_blur_on_press(&mut self, ctx: &egui::Context, content: egui::Rect) {
        let pressed = ctx.input(|input| input.pointer.any_pressed());
        let inside = std::mem::take(&mut self.compose.pressed_inside);
        if !pressed || inside || self.compose.alert.is_some() {
            return;
        }
        let Some(id) = self.compose.focus.take() else {
            return;
        };
        // A press in the content pane takes the keyboard from the pop-out, with a hint at ⌘D.
        if self.compose.popped.as_deref() == Some(id.as_str())
            && ctx
                .input(|input| input.pointer.press_origin())
                .is_some_and(|at| content.contains(at))
        {
            self.commands.hint(
                Hint::new()
                    .text("Next time, hit")
                    .keys(["⌘", "D"])
                    .text("to instantly move from your draft!"),
            );
        }
        if let Some(open) = self.compose.get_mut(&id)
            && open.changed.is_some()
            && (open.is_stored() || !open.is_unmodified())
        {
            save_changes(open, &self.account, &self.snapshot.aliases, &self.now);
        }
        ctx.memory_mut(|memory| {
            if let Some(focused) = memory.focused() {
                memory.surrender_focus(focused);
            }
        });
    }

    /// The include-attachments hint: someone who wasn't on the message a reply answers was just
    /// added to it, and that message has files the reply hasn't: the hint offers `Include
    /// Original Attachment(s)`.
    fn include_attachments_hint(&mut self) {
        let Some(open) = self.compose.focused() else {
            self.compose.recipients_seen = None;
            return;
        };
        let recipients: Vec<Address> = open.recipients().cloned().collect();
        let now: Vec<String> = recipients
            .iter()
            .map(|address| address.email.to_lowercase())
            .collect();
        let reply = matches!(
            open.record.action,
            DraftAction::Reply | DraftAction::ReplyAll
        );
        let id = open.id.clone();
        let before = self
            .compose
            .recipients_seen
            .replace((id.clone(), now.clone()))
            .filter(|(seen, _)| *seen == id)
            .map(|(_, before)| before);
        // The contact pane shows whom the draft just got.
        if let Some(before) = &before
            && let Some(added) = recipients.into_iter().rfind(|address| {
                !address.email.is_empty() && !before.contains(&address.email.to_lowercase())
            })
        {
            self.compose.just_added = Some(added);
        }
        let Some(before) = before.filter(|_| reply && self.compose.missing_originals > 0) else {
            return;
        };
        let Some(original) = self.focused_original() else {
            return;
        };
        if factory::adds_newcomer(&now, &before, original) {
            // Singular or plural by the message's own file count.
            let files = original
                .attachments
                .iter()
                .filter(|attachment| !attachment.inline)
                .count();
            let (noun, command) = if files == 1 {
                ("attachment", "Attachment")
            } else {
                ("attachments", "Attachments")
            };
            self.commands.hint(
                Hint::new()
                    .text(format!("Want to include the original {noun}? Hit"))
                    .keys(["⌘", "K"])
                    .text(format!("→ Include Original {command}!")),
            );
        }
    }

    /// The message the focused draft answers, when its thread is open.
    fn focused_original(&self) -> Option<&MessageDetail> {
        let open = self.compose.focused()?;
        let answers = open.record.in_reply_to.as_deref()?;
        let detail = self.thread.as_ref()?.detail()?;
        detail.messages.iter().find(|message| message.id == answers)
    }

    /// Keeps the address book (asked for once compose opens) and the open thread's
    /// participants indexed for suggestions.
    fn compose_address_book(&mut self) {
        if self.compose.drafts.is_empty() {
            return;
        }
        self.index_address_book();
        let Some(detail) = self
            .thread
            .as_ref()
            .and_then(crate::thread::ThreadView::detail)
        else {
            return;
        };
        if self
            .compose
            .thread_people
            .as_ref()
            .is_some_and(|(id, _)| id == detail.id.as_str())
        {
            return;
        }
        let people: Vec<Address> = detail
            .messages
            .iter()
            .filter(|message| !message.is_draft())
            .flat_map(|message| message.from.iter().chain(&message.to).chain(&message.cc))
            .map(|contact| Address {
                name: contact.name.clone(),
                email: contact.email.clone(),
            })
            .collect();
        let empty = contacts::Index::default();
        let book = self
            .compose
            .book
            .as_ref()
            .map_or(&empty, |(_, index)| index);
        let index = contacts::participants(&people, book);
        self.compose.thread_people = Some((detail.id.as_str().to_owned(), index));
    }

    /// Keeps the address book indexed for suggestions (compose's, and the contact pickers'),
    /// asking for it the first time.
    pub(crate) fn index_address_book(&mut self) {
        let excluded = &self.account.settings().excluded_contacts;
        match &self.snapshot.address_book {
            None if !self.compose.book_asked => {
                self.compose.book_asked = true;
                self.account.load_address_book();
            }
            Some(book)
                if self
                    .compose
                    .book
                    .as_ref()
                    .is_none_or(|(indexed, _)| !std::sync::Arc::ptr_eq(indexed, book))
                    || self.compose.book_excluded != *excluded =>
            {
                self.compose.book =
                    Some((book.clone(), contacts::Index::book_without(book, excluded)));
                self.compose.book_excluded.clone_from(excluded);
            }
            _ => {}
        }
    }

    /// What the account says about drafts.
    pub(crate) fn compose_event(&mut self, event: Event) {
        let showing = self.commands.toast_tag();
        match event {
            Event::DraftSaved(id) => {
                if let Some(sending) = self
                    .compose
                    .sends
                    .iter_mut()
                    .find(|sending| sending.open.id == id && !sending.acked && !sending.undoing)
                {
                    sending.acked = true;
                    if showing == Some(sending.tag) {
                        let text = sending.sent_text();
                        self.commands
                            .toast(send_toast(text, sending.tag, !sending.scheduled));
                    }
                }
            }
            Event::DraftsOffline => {
                for sending in self.compose.sends.iter().filter(|sending| {
                    !sending.acked && !sending.undoing && showing == Some(sending.tag)
                }) {
                    self.commands.toast(
                        Toast::new(sending.offline_text())
                            .undo(commands::Action::Compose(Action::UndoSend(sending.tag)))
                            .tag(sending.tag),
                    );
                }
            }
            Event::DraftSent(SentDraft {
                id,
                thread,
                message,
            }) => {
                // Its recipients join the address book on send.
                self.account.load_address_book();
                self.compose
                    .sent_threads
                    .push((id.clone(), thread.as_str().to_owned()));
                let excess = self.compose.sent_threads.len().saturating_sub(4);
                self.compose.sent_threads.drain(..excess);
                self.compose.in_thread.sent(
                    &id,
                    &message,
                    self.thread.as_ref().map(crate::thread::ThreadView::id),
                );
                if let Some(at) = self
                    .compose
                    .sends
                    .iter()
                    .position(|sending| sending.open.id == id)
                {
                    let sending = self.compose.sends.remove(at);
                    // Each snippet used in it counts a send in its analytics.
                    self.snippets_sent(&sending.open.applied_snippets, &thread, &message);
                    if showing == Some(sending.tag) && !sending.undoing {
                        self.commands
                            .toast(Toast::new(sending.sent_text()).tag(sending.tag));
                    }
                }
            }
            Event::SendTakenBack { id, held } => {
                let Some(at) = self
                    .compose
                    .sends
                    .iter()
                    .position(|sending| sending.open.id == id)
                else {
                    return;
                };
                if held {
                    self.compose.in_thread.forget(&id);
                    let sending = self.compose.sends.remove(at);
                    if let Some(token) = sending.done {
                        self.account.undo(token);
                    }
                    let text = if sending.scheduled {
                        "Scheduling undone."
                    } else if sending.done.is_some() {
                        "Sending undone and conversation moved to Inbox."
                    } else {
                        "Sending undone."
                    };
                    self.commands.toast(Toast::new(text).tag(sending.tag));
                    self.reopen(sending.open, sending.surface);
                } else {
                    self.commands.hide_toast();
                    self.show_failure_alert(
                        "Undo Send failed due to poor internet connectivity…".into(),
                        &id,
                        false,
                    );
                }
            }
            Event::Incomplete(ids) => {
                for id in ids {
                    match self.compose.drafts.iter_mut().find(|open| open.id == id) {
                        Some(open) => save(
                            open,
                            &self.account,
                            &self.snapshot.aliases,
                            &self.now,
                            false,
                        ),
                        None => {
                            let Some(stored) =
                                self.snapshot.drafts.iter().find(|draft| draft.id == id)
                            else {
                                continue;
                            };
                            save(
                                &mut Open::stored(stored),
                                &self.account,
                                &self.snapshot.aliases,
                                &self.now,
                                false,
                            );
                        }
                    }
                }
            }
            Event::AttachmentsFailed { id, count } => {
                if self.compose.drafts.iter().any(|open| open.id == id) {
                    let text = if count > 1 {
                        "Sorry! Failed to save attachments. Please try reuploading."
                    } else {
                        "Sorry! Failed to save attachment. Please try reuploading."
                    };
                    self.commands.toast(Toast::new(text).kind(ToastKind::Error));
                }
            }
            Event::DraftFailed(DraftFailure::Save { id, .. }) => {
                if self.compose.drafts.iter().any(|open| open.id == id) {
                    self.commands
                        .toast(Toast::new("Sorry! Failed to save draft.").kind(ToastKind::Error));
                }
            }
            Event::DraftFailed(DraftFailure::Send { id, reason }) => {
                // Gmail's copy shows in the thread instead, as a failed draft.
                self.compose.in_thread.forget(&id);
                // Only a send of this session's has an observer to say so at once.
                let Some(at) = self
                    .compose
                    .sends
                    .iter()
                    .position(|sending| sending.open.id == id)
                else {
                    return;
                };
                match crate::format::failure::alert(reason) {
                    // Its See Message takes the send from there.
                    Some(text) => {
                        self.commands.hide_toast();
                        self.show_failure_alert(text.into(), &id, true);
                    }
                    // Nothing says so but the failed draft. Its toast, if still up, goes with
                    // the send it would hold or speed up.
                    None => {
                        let sending = self.compose.sends.remove(at);
                        if showing == Some(sending.tag) {
                            self.commands.hide_toast();
                        }
                    }
                }
            }
            Event::Failed { .. }
            | Event::LabelCreated(_)
            | Event::LabelNotCreated { .. }
            | Event::Unsent(_)
            | Event::Thread { .. } => {}
        }
    }

    fn show_failure_alert(&mut self, text: String, draft: &str, see_message: bool) {
        let (actions, then) = if see_message {
            (vec!["See Message"], AlertThen::SeeMessage)
        } else {
            (vec!["OK"], AlertThen::Ok { focus_from: false })
        };
        self.compose.alert = Some(Alert::new(Some("Sorry!"), text, actions, draft, then));
    }

    /// The focused draft's files' names, in order (for tests).
    pub fn compose_attachments(&self) -> Vec<String> {
        self.compose
            .focused()
            .map(|open| {
                open.record
                    .attachments
                    .iter()
                    .filter(|attachment| attachment.content_id.is_none())
                    .map(|attachment| attachment.name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The focused draft's files on their way to Gmail, by name (for tests).
    pub fn compose_uploading(&self) -> Vec<String> {
        self.compose
            .focused()
            .map(|open| {
                form::listed(open)
                    .filter(|attachment| form::uploading(open, attachment).is_some())
                    .map(|attachment| attachment.name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The text of the alert compose is showing, if any. For tests.
    pub fn compose_alert(&self) -> Option<&str> {
        self.compose.alert.as_ref().map(|alert| alert.text.as_str())
    }

    /// Draws the alert compose is showing, if any.
    pub(crate) fn show_compose_alert(&mut self, ui: &mut egui::Ui, page: egui::Rect) {
        let Some(alert) = &self.compose.alert else {
            return;
        };
        // The alert's overlay: the page dimmed to half, and nothing behind it clickable.
        ui.painter()
            .rect_filled(page, 0.0, egui::Color32::from_black_alpha(128));
        let _ = ui.interact(
            page,
            ui.id().with("compose-alert-overlay"),
            egui::Sense::click(),
        );
        let title = alert.title.unwrap_or_default();
        let shown = exo_ui::Alert {
            title,
            text: &alert.text,
            actions: &alert.actions,
            selected: alert.selected,
        };
        let clicked = if alert.key {
            shown.show_with_key(
                ui,
                page,
                reminder::HINT_BEFORE,
                reminder::HINT_KEY,
                reminder::HINT_AFTER,
            )
        } else {
            shown.show(ui, page)
        };
        if let Some(index) = clicked {
            self.alert_action(index);
        }
    }

    /// Leaves the new-message pane for where it was opened from, saving or dropping its draft.
    fn leave_pane_to_origin(&mut self) {
        if self.leaving_snippet_asks() {
            return;
        }
        let origin = self
            .compose
            .pane
            .as_ref()
            .and_then(|(_, from)| from.clone());
        self.leave_pane();
        if let Some(thread) = origin {
            self.open_thread(thread);
        }
    }

    /// Draws the new-message pane in `rect` (the content pane).
    pub(crate) fn show_compose_pane(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        rem: f32,
        window_width: f32,
        now: f64,
    ) {
        let Some(id) = self.compose.pane().map(str::to_owned) else {
            return;
        };
        let aliases = self.snapshot.aliases.clone();
        let me = self.account.email().to_owned();
        let focused = self.compose.focus.as_deref() == Some(id.as_str())
            && self.compose.alert.is_none()
            && !self.compose.palette_open;
        let Some(open) = self.compose.drafts.iter_mut().find(|open| open.id == id) else {
            self.compose.pane = None;
            return;
        };
        let book = self.compose.book.as_ref().map(|(_, index)| index);
        let popped_out = self.compose.popped.is_some();
        let now_zoned = &self.now;
        let send_and_done = self.compose.send_and_done;
        let mut cx = form::Context {
            aliases: &aliases,
            me: &me,
            surface: Surface::Pane,
            focused,
            book,
            thread_people: None,
            pin: egui::Rangef::EVERYTHING,
            drop: None,
            now: now_zoned,
            thread_reminder: None,
            send_and_done,
            max_height: rect.height() - form::MAX_BELOW_PAGE,
            availability_tip: self.compose.availability_tip,
        };
        let width = crate::thread::content_width(rect, rem, window_width, popped_out);
        let height = form::height(ui, open, width, &cx);
        // A snippet's pane is titled for it, its name above the card.
        let (title, above) = if open.is_snippet() {
            (snippet_editor::pane_title(open), 36.0)
        } else {
            ("New Message", 0.0)
        };
        let mut clicked = false;
        // The pop-out hint shows for a message started from a conversation, until its
        // recipients or anything else in it change.
        let pop_out_hint = self
            .compose
            .pane
            .as_ref()
            .is_some_and(|(_, from)| from.is_some())
            && !open.is_snippet()
            && open.is_unmodified();
        let pane = crate::thread::PaneCard {
            title,
            height,
            above,
            pop_out_hint,
            read_only: false,
        };
        let actions = crate::thread::new_message_pane(
            ui,
            rect,
            rem,
            window_width,
            popped_out,
            &pane,
            |ui, content, column, card| {
                cx.pin = column;
                cx.drop = Some(card);
                let response = form::show(ui, open, content, &cx);
                clicked = response.clicked_inside;
                response.actions
            },
        );
        if clicked {
            self.compose.pressed_inside = true;
            self.compose.focus = Some(id);
        }
        for action in actions {
            match action {
                crate::thread::Action::Back => self.leave_pane_to_origin(),
                // A snippet's arrows go to the next snippet, read-only.
                crate::thread::Action::Navigate(delta) if self.pane_snippet().is_some() => {
                    let step = move |mail: &mut crate::Mail| {
                        mail.close_snippet();
                        mail.snippet_step(delta);
                    };
                    if self.snippet_unsaved() {
                        self.ask_leaving_snippet(Box::new(step))
                    } else {
                        step(self)
                    }
                }
                crate::thread::Action::Navigate(delta) => {
                    self.leave_pane();
                    self.inbox.move_cursor(delta);
                    if let Some(next) = self.inbox.cursor().map(|thread| thread.id.clone()) {
                        self.open_thread(next);
                    }
                }
                crate::thread::Action::Compose(action) => self.compose_run(action, now),
                _ => {}
            }
        }
    }

    /// ⌘V with nothing editable focused makes a draft of the clipboard.
    pub(crate) fn compose_paste_events(&mut self, ctx: &egui::Context) {
        self.compose_file_events(ctx);
        if !matches!(
            self.commands.region(),
            Region::ThreadList | Region::ThreadMessages
        ) || self.compose.focus.is_some()
        {
            return;
        }
        let Some(text) = ctx.input(|input| {
            input.events.iter().find_map(|event| {
                if let egui::Event::Paste(text) = event {
                    Some(text.clone())
                } else {
                    None
                }
            })
        }) else {
            return;
        };
        let html = crate::clipboard::read_html(ctx);
        self.compose_paste(html, Some(text));
    }

    /// Files dropped on the window go to the focused draft (or the new-message pane's): one
    /// image into its body, anything else on it (Exo isn't told where a drop lands, so an image
    /// dropped on the footer still goes in the body). A paste in a body of files, or of an image
    /// (a screenshot), puts each in as dropped alone: images in the body, other files on the
    /// draft, their names not typed. HTML that's just images (an image copied from a page) pastes
    /// as its image; other HTML is the editor's.
    fn compose_file_events(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        if self.compose_paste_attachments(ctx, now) {
            return;
        }
        let dropped: Vec<std::path::PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_owned())
                .collect()
        });
        if !dropped.is_empty() {
            self.compose_add_files(&dropped, now);
        }
        let in_body = self
            .compose
            .focused()
            .is_some_and(|open| open.state.field == Field::Body);
        let pasting = in_body && ctx.input(|input| input.events.iter().any(is_paste));
        if !pasting {
            return;
        }
        // The clipboard's plain text takes its `:shortcode:`s as emoji.
        ctx.input_mut(|input| {
            for event in &mut input.events {
                if let egui::Event::Paste(text) = event {
                    *text = crate::emoji::replace_codes(text);
                }
            }
        });
        let files = crate::clipboard::read_files(ctx);
        let image = if files.is_empty() {
            crate::clipboard::read_image(ctx)
        } else {
            None
        };
        if files.is_empty() && image.is_none() {
            return;
        }
        if crate::clipboard::read_html(ctx).is_some_and(|html| !is_just_images(&html)) {
            return;
        }
        ctx.input_mut(|input| input.events.retain(|event| !is_paste(event)));
        for file in &files {
            self.compose_add_files(std::slice::from_ref(file), now);
        }
        if let (Some(png), Some(id)) = (image, self.compose.focus.clone()) {
            // Chromium names the clipboard's image `image.png`.
            self.compose_insert_image(&id, "image.png", "image/png", &png, now);
        }
    }

    /// Files added as if dropped: one image goes in the body, anything else on the draft.
    fn compose_add_files(&mut self, paths: &[std::path::PathBuf], now: f64) {
        match paths {
            [path] if files::is_inline_image(&path.to_string_lossy()) => {
                let Some(id) = self
                    .compose
                    .focus
                    .clone()
                    .or_else(|| self.compose.pane().map(str::to_owned))
                else {
                    return;
                };
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "untitled".into());
                match std::fs::read(path) {
                    Ok(bytes) => {
                        self.compose_insert_image(&id, &name, files::mime_type(&name), &bytes, now)
                    }
                    Err(error) => {
                        log::warn!("reading an image to insert: {error}");
                        self.compose_alert_notice(&id, files::unreadable(&name));
                    }
                }
            }
            _ => self.compose_attach(paths, now),
        }
    }

    /// Puts an image in draft `id`'s body at its caret, kept as an inline attachment.
    fn compose_insert_image(
        &mut self,
        id: &str,
        name: &str,
        mime_type: &str,
        bytes: &[u8],
        now: f64,
    ) {
        let aliases = self.snapshot.aliases.clone();
        let folder = self.account.draft_files().to_owned();
        let Some(open) = self.compose.get_mut(id) else {
            return;
        };
        let cid = factory::uuid_v4();
        match files::add_inline(
            name,
            mime_type,
            bytes,
            &cid,
            &open.record.attachments,
            &outgoing::html(open, &aliases),
            &folder,
        ) {
            Ok(attachment) => {
                open.record.attachments.push(attachment);
                open.editor.insert_image(&cid);
                open.touch(now);
            }
            Err(notice) => self.compose_alert_notice(id, notice),
        }
    }

    /// Shows `notice` over draft `id`: `Sorry!` unless it says otherwise, with OK.
    fn compose_alert_notice(&mut self, id: &str, notice: files::Notice) {
        let then = AlertThen::Ok { focus_from: false };
        self.compose.alert = Some(Alert::new(
            Some(notice.title.unwrap_or("Sorry!")),
            notice.text,
            vec!["OK"],
            id.to_owned(),
            then,
        ));
    }

    /// Whether the focused draft's remote images aren't shown yet.
    pub(crate) fn compose_can_show_images(&self) -> bool {
        self.compose
            .focus
            .as_deref()
            .is_some_and(|id| !self.compose.images.is_shown(id))
    }

    /// Show Images for the focused draft, for the session. Returns whether there was one.
    pub(crate) fn compose_show_images(&mut self) -> bool {
        let Some(id) = self.compose.focus.clone() else {
            return false;
        };
        self.compose.images.show(&id);
        if let Some(open) = self.compose.get_mut(&id) {
            open.editor.reload_remote_images();
        }
        true
    }

    /// Each frame: the drafts' images loading, and the `data:` images pasted into their bodies
    /// kept as inline attachments (or the paste taken back when they'd make the message too large
    /// for Gmail).
    pub(crate) fn compose_images(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);
        let aliases = self.snapshot.aliases.clone();
        let folder = self.account.draft_files().to_owned();
        let mut refused = None;
        for open in &mut self.compose.drafts {
            let uploads = open.editor.take_uploads();
            for upload in &uploads {
                let html = outgoing::html(open, &aliases);
                match files::add_inline(
                    &upload.cid,
                    &upload.mime_type,
                    &upload.bytes,
                    &upload.cid,
                    &open.record.attachments,
                    &html,
                    &folder,
                ) {
                    Ok(attachment) => open.record.attachments.push(attachment),
                    Err(notice) => {
                        open.editor.undo();
                        refused = Some((open.id.clone(), notice));
                        break;
                    }
                }
            }
            if !uploads.is_empty() {
                open.touch(now);
            }
        }
        if let Some((id, notice)) = refused {
            self.compose_alert_notice(&id, notice);
        }
        let loader = self.account.loader();
        let sources = images::Sources {
            loader: &loader,
            draft_files: &folder,
            setting: self
                .account
                .settings()
                .remote_content_blocking
                .unwrap_or_default(),
            ctx,
        };
        let Compose { drafts, images, .. } = &mut self.compose;
        images.update(drafts, &sources);
    }

    /// A click on a card of the focused draft: the file as kept beside the account's database,
    /// or for a forward's file not fetched yet, the original message's part, opens the way a
    /// message's attachment does.
    fn open_draft_file(&mut self, uuid: &str, preview: bool) {
        let Some(attachment) = self
            .compose
            .focused()
            .and_then(|open| {
                open.record
                    .attachments
                    .iter()
                    .find(|attachment| attachment.uuid == uuid)
            })
            .cloned()
        else {
            return;
        };
        self.open_draft_attachment(attachment, preview);
    }

    /// A draft's (or a snippet's) file opens, previewed unless asked not to: from the draft
    /// files, or Gmail's part when it isn't kept there yet.
    pub(crate) fn open_draft_attachment(
        &mut self,
        attachment: exo_store::DraftAttachment,
        preview: bool,
    ) {
        let kept = self.account.draft_files().join(&attachment.uuid);
        if kept.is_file() {
            self.attachment_files
                .open_file(kept, &attachment.uuid, attachment.name, preview);
            return;
        }
        if let exo_store::AttachmentSource::Gmail {
            message_id,
            attachment_id,
            part_id,
        } = attachment.source
        {
            let part = exo_store::Attachment {
                message_id,
                attachment_id,
                part_id,
                filename: attachment.name,
                mime_type: attachment.mime_type,
                size: u32::try_from(attachment.size).unwrap_or(u32::MAX),
                content_id: None,
                inline: false,
            };
            self.attachment_files
                .open(&self.account.loader(), part, preview);
        }
    }

    /// Adds files to the focused draft, or the new-message pane's, and says why any were turned
    /// away.
    fn compose_attach(&mut self, paths: &[std::path::PathBuf], now: f64) {
        let Some(id) = self
            .compose
            .focus
            .clone()
            .or_else(|| self.compose.pane().map(str::to_owned))
        else {
            return;
        };
        if paths.is_empty() {
            return;
        }
        let aliases = self.snapshot.aliases.clone();
        let folder = self.account.draft_files().to_owned();
        let Some(open) = self.compose.get_mut(&id) else {
            return;
        };
        let html = outgoing::html(open, &aliases);
        let (added, notice) = files::add(paths, &open.record.attachments, &html, &folder);
        if !added.is_empty() {
            open.record.attachments.extend(added);
            open.touch(now);
        }
        if let Some(notice) = notice {
            self.compose_alert_notice(&id, notice);
        }
    }

    /// A `mailto:` link was clicked: a new message to it.
    pub(crate) fn compose_mailto(&mut self, url: &str, pop_out: bool) {
        match mailto::fields(url) {
            // Shift-click pops it out.
            Some(fields) if pop_out => self.compose_new_popped(fields),
            Some(fields) => self.compose_new(fields, None),
            None => {}
        }
    }

    /// ⌘V outside a text field: a new draft from the clipboard, the copied message's recipients
    /// and subject too when it's the message Exo last copied.
    pub(crate) fn compose_paste(&mut self, html: Option<String>, text: Option<String>) {
        let copied = self
            .copied_message
            .clone()
            .filter(|(copied_html, _)| html.as_deref() == Some(copied_html.as_str()));
        let body = match (copied.as_ref(), &html, &text) {
            (Some((_, fields)), _, _) => fields.body.clone(),
            (None, Some(html), _) => exo_mail::sanitize_fragment(html),
            (None, None, Some(text)) if !text.trim().is_empty() => exo_mail::text_to_html(text),
            _ => return,
        };
        let fields = match copied {
            Some((_, fields)) => Fields { body, ..fields },
            None => Fields {
                body,
                ..Fields::default()
            },
        };
        self.compose_new(fields, None);
    }
}

/// A paste: egui's, which egui-winit makes of ⌘V only while the clipboard has text, or the ⌘V
/// key itself, which is all that reaches egui of a paste of nothing but an image or files (the
/// keyboard's, and Edit ▸ Paste's).
fn is_paste(event: &egui::Event) -> bool {
    match event {
        egui::Event::Paste(_) => true,
        egui::Event::Key {
            key: egui::Key::V,
            pressed: true,
            modifiers,
            ..
        } => modifiers.command && !modifiers.shift && !modifiers.alt && !modifiers.ctrl,
        _ => false,
    }
}

/// HTML with nothing in it but images (an image copied from a page).
fn is_just_images(html: &str) -> bool {
    if html.len() > 4096 {
        return false;
    }
    let document = exo_editor::html::parse(html);
    let runs = || document.blocks.iter().flat_map(|block| block.runs());
    let only_images = document
        .blocks
        .iter()
        .all(|block| block.is_inline() && block.path.is_empty())
        && runs().all(|run| match &run.piece {
            exo_editor::Piece::Image(_) => true,
            exo_editor::Piece::Text(text) => text.trim().is_empty(),
            exo_editor::Piece::Break => false,
        });
    only_images && runs().any(|run| matches!(run.piece, exo_editor::Piece::Image(_)))
}

/// The send toast: `Undo` and, revealed on hover, `Instant Send ⚡` (not for a scheduled send).
fn send_toast(text: &'static str, tag: u64, instant: bool) -> Toast<commands::Action> {
    let toast = Toast::new(text)
        .undo(commands::Action::Compose(Action::UndoSend(tag)))
        .tag(tag);
    if !instant {
        return toast;
    }
    toast
        .action(
            "Instant Send ⚡",
            Some("mod+shift+z"),
            commands::Action::Compose(Action::InstantSend(tag)),
        )
        .hidden()
        .tooltip("Send message right now")
        .keep_on_action()
}

/// The hint after Send is clicked.
fn send_hint() -> Hint {
    Hint::new()
        .text("Next time, hit")
        .keys(["⌘", "enter"])
        .text("to send even faster!")
}
