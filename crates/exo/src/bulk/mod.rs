//! Bulk actions: triage on 50 or more selected conversations, or on a select-all that reaches
//! conversations not loaded yet, asks first (`This might take a while…`) and then runs in the
//! background ([`Jobs`]) instead of row by row. Its rows say `Pending:` until it's done, the footer
//! counts it, and the toast's Undo puts it all back. Get Me To Zero is one too, with its own
//! dialog, and the Bulk Actions dialog lists those.

mod actions_dialog;
mod get_me_to_zero;
mod jobs;

use std::collections::HashMap;

use egui::{Context, Rect};
use exo_commands::{Command, Commands, Hint, Region, Toast};
use exo_store::{
    BulkTarget, Filter, LabelIndex, ListCutoff, ListId, Mutation, Scope, SearchQuery, ThreadId,
};

pub use jobs::{Jobs, Request, Status, Summary};

use crate::{Mail, account::UndoToken, commands, inbox::SelectionHint};

/// Selections this large go through a bulk action even when every row is loaded.
const THRESHOLD: usize = 50;
/// Get Me To Zero's name as a bulk action, which the Bulk Actions dialog lists.
pub(crate) const GET_ME_TO_ZERO: &str = "Get Me To Zero";

/// A bulk action waiting for the user's OK, with the toast it starts with.
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    request: Request,
    toast: String,
}

/// What the bulk dialogs and their commands ask for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open Get Me To Zero.
    OpenGetMeToZero,
    /// Open Bulk Actions.
    OpenBulkActions,
    /// Something in Get Me To Zero.
    GetMeToZero(get_me_to_zero::Event),
    /// Something in Bulk Actions.
    BulkActions(actions_dialog::Event),
}

/// The bulk dialog on screen, or fading out.
#[derive(Debug, Default)]
pub(crate) struct Dialogs {
    shown: Option<Shown>,
    /// It hasn't been closed.
    open: bool,
}

#[derive(Debug)]
enum Shown {
    GetMeToZero(get_me_to_zero::State),
    BulkActions(actions_dialog::State),
}

impl Dialogs {
    /// A bulk dialog has the keyboard.
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }
}

/// Registers the palette's `Get Me To Zero` and `Bulk Actions`, and while a bulk dialog is open,
/// its keys.
pub(crate) fn register(commands: &mut Commands<commands::Action>, dialogs: &Dialogs) {
    let act = commands::Action::Bulk;
    commands.add(
        Command::new(
            "GET_ME_TO_ZERO",
            GET_ME_TO_ZERO,
            act(Action::OpenGetMeToZero),
        )
        .aliases([
            "Bulk Archive",
            "Bulk Mark Done",
            "Inbox Zero",
            "Clean up my inbox",
        ])
        .default_score(1.0)
        .icon("data/done"),
    );
    commands.add(
        Command::new(
            "BULK_ACTIONS_DIALOG",
            "Bulk Actions",
            act(Action::OpenBulkActions),
        )
        .aliases(["Settings", GET_ME_TO_ZERO])
        .operative_word("bulk")
        .default_score(0.98)
        .icon("data/settings")
        .multiple(),
    );
    const DIALOG: &[Region] = &[Region::Dialog];
    let key = |id: &'static str, action: Action, keys: &[&'static str]| {
        Command::shortcut(id, act(action))
            .keys(keys.iter().copied())
            .regions(DIALOG)
    };
    match (&dialogs.shown, dialogs.open) {
        (Some(Shown::GetMeToZero(_)), true) => {
            use get_me_to_zero::Event;
            let gmtz = Action::GetMeToZero;
            commands.add(key("GMTZ_ESCAPE", gmtz(Event::Escape), &["esc"]));
            commands.add(key(
                "GMTZ_ENTER",
                gmtz(Event::Activate),
                &["enter", "space"],
            ));
            commands.add(key("GMTZ_NEXT", gmtz(Event::Move(1)), &["tab", "down"]));
            commands.add(key(
                "GMTZ_PREVIOUS",
                gmtz(Event::Move(-1)),
                &["shift+tab", "up"],
            ));
        }
        (Some(Shown::BulkActions(_)), true) => {
            use actions_dialog::Event;
            let rows = Action::BulkActions;
            commands.add(key("BULK_ACTIONS_ESCAPE", rows(Event::Close), &["esc"]));
            commands.add(key("BULK_ACTIONS_ENTER", rows(Event::Activate), &["enter"]));
            commands.add(key(
                "BULK_ACTIONS_DOWN",
                rows(Event::Move(1)),
                &["down", "j", "tab", "ctralt+j", "ctralt+n"],
            ));
            commands.add(key(
                "BULK_ACTIONS_UP",
                rows(Event::Move(-1)),
                &["up", "k", "shift+tab", "ctralt+k", "ctralt+p"],
            ));
        }
        _ => {}
    }
}

impl Mail {
    /// Takes triage of `ids` with `mutation` when it's a bulk action: the list's selection with 50
    /// rows or more, or reaching rows not loaded. It asks first, or says why it can't be done where
    /// the list can't be acted on in bulk; where it can't but every row is loaded, it's left to act
    /// row by row. Returns whether it took it.
    pub(crate) fn bulk_instead(&mut self, ids: &[String], mutation: &Mutation) -> bool {
        let Some(name) = name(mutation) else {
            return false;
        };
        if self.thread.is_some() || !self.inbox.is_selecting() || self.inbox.selection() != ids {
            return false;
        }
        let exact = self.inbox.is_exact();
        if exact && ids.len() < THRESHOLD {
            return false;
        }
        if let Some(text) = self.bulk_refusal() {
            if !exact {
                self.pickers.inform("Sorry!", text);
            }
            return !exact;
        }
        let searched = self.search.as_ref().and_then(|search| search.query());
        let scope = searched.map_or_else(
            || Scope::List(self.list.clone()),
            |query| Scope::Search(query.to_owned()),
        );
        // A search's filters are tokens of its query.
        let filter = self.filter.filter(|_| self.search.is_none());
        let target = BulkTarget {
            list: self
                .inbox
                .cutoff()
                .filter(|_| !exact)
                .map(|before| ListCutoff {
                    scope,
                    filter,
                    keep: Vec::new(),
                    before,
                }),
            include: ids.iter().map(|id| ThreadId::new(id.clone())).collect(),
            exclude: self
                .inbox
                .unselected()
                .into_iter()
                .map(ThreadId::new)
                .collect(),
        };
        let toast = self.bulk_toast(mutation);
        let request = Request {
            name,
            query: None,
            mutation: mutation.clone(),
            target,
            threads: ids.iter().cloned().collect(),
        };
        let what = if ids.len() >= THRESHOLD {
            "many conversations"
        } else {
            "conversations that are still loading"
        };
        let text = format!(
            "You are about to act on {what}, which may take some time. You can track progress in the bottom right of the screen."
        );
        self.pickers.confirm_bulk(
            "This might take a while…",
            text,
            None,
            vec!["OK", "Cancel"],
            Plan { request, toast },
        );
        true
    }

    /// Why the list or search on screen can't be acted on in bulk.
    fn bulk_refusal(&self) -> Option<String> {
        if let Some(query) = self.search.as_ref().and_then(|search| search.query()) {
            let labels = LabelIndex::new(
                [],
                self.snapshot
                    .labels
                    .iter()
                    .map(|label| (label.id.as_str(), label.name.as_str())),
            );
            return SearchQuery::parse(query).bulk_refusal(&labels);
        }
        Some(match self.filter {
            Some(Filter::Important) => "We can't complete this action on conversations filtered by important.\n\nPlease remove the filter and try again.",
            Some(Filter::NoReply) => "We can't complete this action on conversations filtered by no reply.\n\nPlease remove the filter and try again.",
            _ if self.list == ListId::Drafts => "We can't complete this action in the Drafts folder.\n\nPlease unselect these conversations and try again.",
            _ if self.list == ListId::Scheduled => "We can't complete this action in the Scheduled folder.\n\nPlease unselect these conversations and try again.",
            _ => return None,
        }.into())
    }

    /// Runs `plan`, once the user said OK: the selection clears, and the toast's Undo puts it back.
    pub(crate) fn start_bulk(&mut self, plan: Plan) {
        // No Get Me To Zero opens by itself after an interrupted first run.
        if plan.request.name == GET_ME_TO_ZERO {
            self.account
                .update_device_settings(|device| device.gmtz_started = true);
        }
        let token = self.account.undo_token();
        self.account.bulk(token, plan.request);
        self.inbox.clear_selection();
        self.undoable = None;
        self.commands
            .toast(Toast::new(plan.toast).undo(commands::Action::Undo(token)));
    }

    /// The toast's Undo ran on `token`: Get Me To Zero says so (`Undoing Get Me To Zero.`).
    pub(crate) fn bulk_undone(&mut self, token: UndoToken) {
        if self
            .account
            .bulk_actions()
            .iter()
            .any(|action| action.token == token && action.name == GET_ME_TO_ZERO)
        {
            self.commands.toast(Toast::new("Undoing Get Me To Zero."));
        }
    }

    /// Carries out a bulk dialog's `action`.
    pub(crate) fn bulk_run(&mut self, action: Action) {
        match action {
            Action::OpenGetMeToZero => {
                self.commands.close_palette();
                self.bulk_dialogs = Dialogs {
                    shown: Some(Shown::GetMeToZero(get_me_to_zero::State::default())),
                    open: true,
                };
            }
            Action::OpenBulkActions => {
                self.commands.close_palette();
                self.bulk_dialogs = Dialogs {
                    shown: Some(Shown::BulkActions(actions_dialog::State::default())),
                    open: true,
                };
            }
            Action::GetMeToZero(event) => {
                let Some(Shown::GetMeToZero(state)) = &mut self.bulk_dialogs.shown else {
                    return;
                };
                match state.handle(event) {
                    Some(get_me_to_zero::Event::Confirm) => {
                        let state = state.clone();
                        self.bulk_dialogs.open = false;
                        self.confirm_get_me_to_zero(&state);
                    }
                    // `Clean up your inbox anytime by hitting ⌘ K → Get Me To Zero!`
                    Some(get_me_to_zero::Event::DoLater) => {
                        self.bulk_dialogs.open = false;
                        self.commands.hint(
                            Hint::new()
                                .text("Clean up your inbox anytime by hitting")
                                .keys(["⌘", "K"])
                                .text("→ Get Me To Zero!"),
                        );
                    }
                    Some(get_me_to_zero::Event::Close) => self.bulk_dialogs.open = false,
                    _ => {}
                }
            }
            Action::BulkActions(event) => {
                let rows = self.get_me_to_zero_actions();
                let Some(Shown::BulkActions(state)) = &mut self.bulk_dialogs.shown else {
                    return;
                };
                match state.handle(event, rows.len()) {
                    Some(actions_dialog::Event::Row(row)) => {
                        let Some(row) = rows.get(row) else { return };
                        // Cancel a waiting one, Undo one that ran.
                        match actions_dialog::action(row) {
                            ("Cancel", true) => self.pickers.confirm_undo(
                                "Cancel action",
                                format!("Would you like to cancel this {} action?", row.name),
                                vec!["Cancel", "Keep"],
                                row.token,
                            ),
                            (_, true) => self.pickers.confirm_undo(
                                "Undo action",
                                format!("Would you like to undo this {} action?", row.name),
                                vec!["Undo", "Cancel"],
                                row.token,
                            ),
                            _ => {}
                        }
                    }
                    Some(actions_dialog::Event::Close) => self.bulk_dialogs.open = false,
                    _ => {}
                }
            }
        }
    }

    /// Get Me To Zero's `Let's go!`: the dialog closes and an alert says it may take a while; its
    /// `Got it` (or Escape) starts it: every inbox conversation older than the age, but those kept,
    /// marked Done.
    fn confirm_get_me_to_zero(&mut self, state: &get_me_to_zero::State) {
        let before = get_me_to_zero::RANGES[state.range].before(&self.now);
        let keep: Vec<Filter> = [
            (state.keep_unread, Filter::Unread),
            (state.keep_starred, Filter::Starred),
        ]
        .into_iter()
        .filter_map(|(kept, filter)| kept.then_some(filter))
        .collect();
        // Gmail's inbox label, which every inbox view and split is drawn from (`in:inbox`).
        let inbox = ListId::Label("INBOX".into());
        let target = BulkTarget {
            // `before` is exclusive here: a message dated at the cutoff isn't older than it.
            list: Some(ListCutoff {
                scope: Scope::List(inbox),
                filter: None,
                keep,
                before: before - jiff::SignedDuration::from_millis(1),
            }),
            ..BulkTarget::default()
        };
        // The loaded rows it reaches, which show `Pending:` while it runs.
        let threads = self
            .inbox
            .threads()
            .iter()
            .filter(|thread| {
                !thread.done
                    && thread.received < before
                    && !(state.keep_unread && thread.unread)
                    && !(state.keep_starred && thread.starred)
            })
            .map(|thread| thread.id.clone())
            .collect();
        let query = state.display_query(&before.to_zoned(self.now.time_zone().clone()));
        let request = Request {
            name: GET_ME_TO_ZERO,
            query: Some(query),
            mutation: Mutation::MarkDone,
            target,
            threads,
        };
        // `You can track progress or undo this action with ⌘ K → Bulk Actions.`
        let text = format!(
            "Moving a lot of emails may take some time.{}You can track progress or undo this action \nwith",
            exo_ui::PARAGRAPH
        );
        let keys = crate::pickers::Keys {
            caps: &["⌘", "K"],
            after: "→ Bulk Actions.",
        };
        self.pickers.confirm_bulk(
            "You're on your way! 🎉",
            text,
            Some(keys),
            vec!["Got it"],
            Plan {
                request,
                toast: "Marking as Done.".into(),
            },
        );
    }

    /// The session's Get Me To Zero actions, newest first: the Bulk Actions dialog's rows.
    fn get_me_to_zero_actions(&self) -> Vec<Summary> {
        self.account
            .bulk_actions()
            .into_iter()
            .rev()
            .filter(|action| action.name == GET_ME_TO_ZERO)
            .collect()
    }

    /// Draws the bulk dialog, if one shows, over `area` (the window below the tab strip).
    pub(crate) fn show_bulk_dialog(&mut self, ctx: &Context, area: Rect) {
        let open = self.bulk_dialogs.open;
        let (inner, close) = match &self.bulk_dialogs.shown {
            None => return,
            Some(Shown::GetMeToZero(state)) => {
                let response = get_me_to_zero::show(ctx, area, open, state);
                (
                    response.inner.map(|event| event.map(Action::GetMeToZero)),
                    response.close,
                )
            }
            Some(Shown::BulkActions(state)) => {
                let rows = self.get_me_to_zero_actions();
                let response = actions_dialog::show(ctx, area, open, state, &rows);
                (
                    response.inner.map(|event| event.map(Action::BulkActions)),
                    response.close,
                )
            }
        };
        match inner {
            None => self.bulk_dialogs.shown = None,
            Some(action) => {
                if close {
                    self.bulk_dialogs.open = false;
                }
                if let Some(action) = action {
                    self.bulk_run(action);
                }
            }
        }
    }

    /// The toast for a bulk action: what it's doing.
    fn bulk_toast(&self, mutation: &Mutation) -> String {
        let label = |id: &str| crate::pickers::label_name(id, &self.snapshot.labels).to_owned();
        match mutation {
            Mutation::MarkDone => "Marking as Done.".into(),
            Mutation::MarkNotDone => "Marking not Done.".into(),
            Mutation::Star => "Starring.".into(),
            Mutation::Unstar => "Unstarring.".into(),
            Mutation::MarkRead => "Marking as Read.".into(),
            Mutation::MarkUnread => "Marking as Unread.".into(),
            Mutation::Trash => "Moving to Trash.".into(),
            Mutation::Untrash => "Moving to Inbox.".into(),
            Mutation::Spam => "Marking as Spam.".into(),
            Mutation::NotSpam => "Marking as Not Spam.".into(),
            Mutation::Mute => "Muting.".into(),
            Mutation::Unmute => "Unmuting.".into(),
            Mutation::AddLabel(id) => format!("Adding to label {}.", label(id)),
            // `Y` names the list it takes threads out of.
            Mutation::RemoveLabel(id) => {
                let name = if crate::pickers::removable_label(&self.list).as_ref() == Some(id) {
                    crate::lists::name(
                        &self.list,
                        &self.snapshot.labels,
                        self.account.settings().splits(),
                    )
                    .to_owned()
                } else {
                    label(id)
                };
                if name.is_empty() {
                    "Removing from label.".into()
                } else {
                    format!("Removing from label {name}.")
                }
            }
            Mutation::Move { from, to } => {
                crate::pickers::move_text(from.as_deref(), to, &self.snapshot.labels, true)
            }
            // Never bulk: these need loaded conversations.
            Mutation::RemoveLabels(_)
            | Mutation::Remind { .. }
            | Mutation::CancelReminder { .. }
            | Mutation::ReturnFromReminder => String::new(),
        }
    }

    /// Shows the hint the selection asked for.
    pub(crate) fn selection_hint(&mut self) {
        let hint = match self.inbox.take_hint() {
            Some(SelectionHint::SelectAll) => Hint::new()
                .text("Want to select everything before this conversation? Hit")
                .keys(["⌘", "A"])
                .text("!"),
            Some(SelectionHint::ChangedYourMind) => Hint::new()
                .text("Changed your mind? Hit")
                .keys(["esc"])
                .text("to unselect all conversations."),
            None => return,
        };
        self.commands.hint(hint);
    }

    /// `⇧Y` on a select-all that reaches rows not loaded: removing every label needs each
    /// row's labels, so it's refused. Returns whether it was.
    pub(crate) fn refuse_unloaded(&mut self, what: &str) -> bool {
        if self.thread.is_some() || self.inbox.is_exact() {
            return false;
        }
        self.commands.hint(Hint::new().text(format!("You cannot {what} on conversations that have not loaded. Please clear your selection and try again.")));
        true
    }

    /// What each loaded row a bulk action waits or runs for shows instead of its date:
    /// `Pending: Mark Done`.
    pub(crate) fn pending_rows(&self, actions: &[Summary]) -> HashMap<String, String> {
        let mut rows = HashMap::new();
        for action in actions
            .iter()
            .filter(|action| matches!(action.status, Status::Pending | Status::Running))
        {
            for thread in action.threads.iter() {
                rows.entry(thread.clone())
                    .or_insert_with(|| format!("Pending: {}", action.name));
            }
        }
        rows
    }
}

/// The footer's status while a bulk action works: `Updating: 120 of 4,000`, `of many` until its
/// threads are all found.
pub(crate) fn status(actions: &[Summary]) -> Option<String> {
    let action = actions.iter().find(|action| action.active)?;
    let done = crate::format::thousands(action.done.max(1));
    Some(if action.total == 0 {
        format!("Updating: {done} of many")
    } else {
        format!(
            "Updating: {done} of {}",
            crate::format::thousands(action.total)
        )
    })
}

/// What a bulk action of `mutation` is called; `None` for one that only runs on loaded
/// conversations.
fn name(mutation: &Mutation) -> Option<&'static str> {
    Some(match mutation {
        Mutation::MarkDone => "Mark Done",
        Mutation::MarkNotDone => "Mark not Done",
        Mutation::Star => "Star",
        Mutation::Unstar => "Unstar",
        Mutation::MarkRead => "Mark Read",
        Mutation::MarkUnread => "Mark Unread",
        Mutation::Move { .. } => "Move to Label",
        Mutation::Mute => "Mute",
        Mutation::Unmute => "Unmute",
        Mutation::AddLabel(_) => "Add Label",
        Mutation::RemoveLabel(_) => "Remove Label",
        Mutation::Spam => "Mark Spam",
        Mutation::NotSpam => "Mark not Spam",
        Mutation::Trash => "Trash",
        Mutation::Untrash => "Remove from Trash",
        Mutation::RemoveLabels(_)
        | Mutation::Remind { .. }
        | Mutation::CancelReminder { .. }
        | Mutation::ReturnFromReminder => return None,
    })
}
