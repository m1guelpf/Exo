//! The triage pickers and the label flows around them: Move (`V`), Label (`L`), taking threads
//! out of the list on screen (`Y`, `[`, `]`), removing every label (`⇧Y`), and creating a label
//! from either picker after a confirmation. Choices are remembered in `labels_last_used`, which
//! both pickers list first.

use std::cmp::Ordering;

use exo_commands::{Command, Commands, Hint, Region, SubPalette, Toast, ToastKind, command_score};
use exo_store::{Label, ListId, Mutation};
use jiff::Timestamp;

use crate::{
    Mail,
    account::Event,
    commands,
    mailbox::{ThreadSummary, compare_label_names},
    senders::SenderJob,
};

/// Names the Move and Label pickers never offer to create.
const RESERVED: [&str; 16] = [
    "chat",
    "chatter",
    "chats",
    "done",
    "draft",
    "drafts",
    "important",
    "inbox",
    "other",
    "reminders",
    "sent",
    "snoozed",
    "star",
    "starred",
    "todo",
    "unread",
];
/// Move's built-in destinations after the recently used ones, last first: Spam, Trash, Done,
/// Important, Other.
const MOVE_ORDER: [&str; 5] = ["SH_OTHER", "SH_IMPORTANT", "SH_ARCHIVED", "TRASH", "SPAM"];

/// What a picker or its keys asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Open the Move picker (`V`).
    OpenMove,
    /// Open the Label picker (`L`).
    OpenLabel,
    /// Move the actionable threads to this destination.
    Move(String),
    /// Open `Move to Other` (true) or `Move to Important` (`MoveToImportantOrOther`).
    OpenImportantOrOther(bool),
    /// `Move conversation to Other` (true) or `… to Important`.
    MoveConversation(bool),
    /// Add this label to the actionable threads.
    AddLabel(String),
    /// Remove this label from the actionable threads.
    RemoveLabel(String),
    /// `Create label “{name}”`: ask, then create it and move these threads to it or label them.
    Create {
        /// The label's name.
        name: String,
        /// The threads the picker was opened for.
        threads: Vec<String>,
        /// Move them there (from this list) rather than label them.
        moving: Option<Option<String>>,
    },
    /// `Y` (0), `[` (1, then the next conversation) and `]` (-1, the previous): take the
    /// threads out of the list on screen.
    RemoveFromList(isize),
    /// `⇧Y`: remove every label and mark done.
    RemoveAll,
    /// `!`: the Mark Spam picker for one conversation with a sender to block, else report spam.
    MarkSpam,
    /// Block everything from this address or domain, after asking; `spam` when asked from the Mark
    /// Spam picker.
    Block {
        /// The address or domain.
        from: String,
        /// A domain, not an address.
        domain: bool,
        /// From the Mark Spam picker.
        spam: bool,
    },
    /// Move everything from this address or domain to Other (or Important), after asking
    /// (`MoveToImportantOrOther`'s "all" rows).
    MoveSender {
        /// The address or domain.
        from: String,
        /// A domain, not an address.
        domain: bool,
        /// Other rather than Important.
        other: bool,
    },
    /// An alert's action, by index.
    Alert(usize),
    /// ← / → / Tab in an alert.
    AlertSelect(isize),
}

/// The pickers' state between frames.
#[derive(Default)]
pub struct Pickers {
    alert: Option<Alert>,
    /// The label being created, and what happens once it exists.
    creating: Option<Creation>,
    /// The last block's undo token and what it blocked, which its undo unblocks.
    blocked: Option<(crate::account::UndoToken, String)>,
}

/// An alert the pickers show: a confirmation, or the offline alert.
struct Alert {
    title: String,
    text: String,
    /// Right to left; the first is selected to start with and is what Enter does.
    actions: Vec<&'static str>,
    selected: usize,
    /// Keycaps continuing the text's last line, and the text after them.
    keys: Option<Keys>,
    /// What the first action does; any other only closes the alert.
    then: Option<Then>,
}

/// Keycaps in an alert's text, after its last line.
#[derive(Clone, Debug)]
pub(crate) struct Keys {
    /// Their labels.
    pub caps: &'static [&'static str],
    /// The text after them.
    pub after: &'static str,
}

/// What an alert's first action does.
#[derive(Clone, Debug)]
enum Then {
    CreateLabel(Creation),
    Block {
        from: String,
        thread: String,
        spam: bool,
    },
    MoveSender {
        from: String,
        other: bool,
        thread: String,
    },
    Bulk(Box<crate::bulk::Plan>),
    /// Unsubscribe.
    Unsubscribe(Box<crate::unsubscribe::Plan>),
    /// Undo (or cancel) the bulk action with this token.
    Undo(crate::account::UndoToken),
    /// Sign this account out, though it has changes Gmail doesn't have yet.
    SignOut(String),
    /// An answer to an invite in conversation `thread` Calendar refused: `Open in Gmail`, and
    /// the second action `See Message`.
    RsvpFailed(String),
    /// Update split `id` (named `name`) to definition `query`; the second action (and Escape)
    /// names the new split again.
    UpdateSplit {
        id: String,
        name: String,
        query: String,
    },
    /// Open Split Inbox Settings.
    OpenSplitSettings,
}

#[derive(Clone, Debug)]
struct Creation {
    name: String,
    threads: Vec<String>,
    moving: Option<Option<String>>,
}

impl Pickers {
    /// Asks before running bulk action `plan`: the first of `actions` runs it.
    pub(crate) fn confirm_bulk(
        &mut self,
        title: &str,
        text: String,
        keys: Option<Keys>,
        actions: Vec<&'static str>,
        plan: crate::bulk::Plan,
    ) {
        self.alert = Some(Alert {
            title: title.into(),
            text,
            actions,
            selected: 0,
            keys,
            then: Some(Then::Bulk(Box::new(plan))),
        });
    }

    /// Asks before undoing (or cancelling) the bulk action `token`: the first of `actions` does.
    pub(crate) fn confirm_undo(
        &mut self,
        title: &str,
        text: String,
        actions: Vec<&'static str>,
        token: crate::account::UndoToken,
    ) {
        self.alert = Some(Alert {
            title: title.into(),
            text,
            actions,
            selected: 0,
            keys: None,
            then: Some(Then::Undo(token)),
        });
    }

    /// Asks before signing `email` out when it has unsent changes, saying `text` about them.
    pub(crate) fn confirm_sign_out(&mut self, email: String, text: String) {
        let actions = crate::accounts::CONFIRM_ACTIONS.to_vec();
        self.alert = Some(Alert {
            title: crate::accounts::CONFIRM_TITLE.into(),
            text,
            actions,
            selected: 0,
            keys: None,
            then: Some(Then::SignOut(email)),
        });
    }

    /// Asks before unsubscribing as `plan` says (`Confirm` / `Cancel`).
    pub(crate) fn confirm_unsubscribe(
        &mut self,
        title: &str,
        text: String,
        plan: crate::unsubscribe::Plan,
    ) {
        self.alert = Some(Alert {
            title: title.into(),
            text,
            actions: vec!["Confirm", "Cancel"],
            selected: 0,
            keys: None,
            then: Some(Then::Unsubscribe(Box::new(plan))),
        });
    }

    /// Offers to block `from` for `thread`, which has nothing to unsubscribe from.
    pub(crate) fn confirm_block_instead(&mut self, from: String, thread: String) {
        let text = format!(
            "There isn't an unsubscribe link here.\n\nWould you like to block instead? This will trash all messages, including any future messages, from {}.",
            exo_ui::wbrise(&from)
        );
        self.alert = Some(Alert {
            title: "Block instead?".into(),
            text,
            actions: vec!["Block", "Cancel"],
            selected: 0,
            keys: None,
            then: Some(Then::Block {
                from,
                thread,
                spam: false,
            }),
        });
    }

    /// `Update existing Split Inbox?`: a split named as `existing` is being made with definition
    /// `query`. Yes updates it; No (or Escape) goes back to naming.
    pub(crate) fn ask_update_split(&mut self, existing: &exo_store::SplitInbox, query: &str) {
        let text = format!(
            "\"{}\" is already a Split Inbox. Would you like to update it?",
            existing.name()
        );
        let then = Then::UpdateSplit {
            id: existing.id.clone(),
            name: existing.name().to_owned(),
            query: query.to_owned(),
        };
        self.alert = Some(Alert {
            title: "Update existing Split Inbox?".into(),
            text,
            actions: vec!["Yes", "No"],
            selected: 0,
            keys: None,
            then: Some(then),
        });
    }

    /// The alert for a split whose definition is too long: `Open settings` unless Split Inbox
    /// Settings is open (`settings_open`), and `Ok`.
    pub(crate) fn split_too_long(&mut self, settings_open: bool) {
        let actions = if settings_open {
            vec!["Ok"]
        } else {
            vec!["Open settings", "Ok"]
        };
        let then = (!settings_open).then_some(Then::OpenSplitSettings);
        self.alert = Some(Alert {
            title: "Sorry!".into(),
            text: crate::settings::SPLIT_TOO_LONG.into(),
            actions,
            selected: 0,
            keys: None,
            then,
        });
    }

    /// An invite card's answer Calendar refused: try in Gmail, or go to the conversation.
    pub(crate) fn rsvp_failed(&mut self, thread: String) {
        let text = "We couldn't RSVP due to an error in Gmail. Please try opening this message in Gmail to save your RSVP.".to_owned();
        self.alert = Some(Alert {
            title: "Sorry!".into(),
            text,
            actions: vec!["Open in Gmail", "See Message"],
            selected: 0,
            keys: None,
            then: Some(Then::RsvpFailed(thread)),
        });
    }

    /// Shows an alert that only informs, with its OK.
    pub(crate) fn inform(&mut self, title: &str, text: String) {
        self.alert = Some(Alert {
            title: title.into(),
            text,
            actions: vec!["OK"],
            selected: 0,
            keys: None,
            then: None,
        });
    }

    /// An alert holds the keyboard.
    pub fn region(&self) -> Option<Region> {
        self.alert.as_ref().map(|_| Region::Alert)
    }

    /// Registers the alert's keys, while one shows (`Alert`'s Enter, Escape and arrows).
    pub fn register(&self, commands: &mut Commands<commands::Action>) {
        const ALERT: &[Region] = &[Region::Alert];
        let Some(alert) = &self.alert else { return };
        let act = |action: Action| commands::Action::Pick(action);
        commands.add(
            Command::shortcut("ALERT_ENTER", act(Action::Alert(alert.selected)))
                .key("enter")
                .regions(ALERT),
        );
        // Escape only closes it: an index past the actions, as the last of most is Cancel and two
        // of `RsvpFailed`'s act.
        commands.add(
            Command::shortcut("ALERT_ESC", act(Action::Alert(alert.actions.len())))
                .key("esc")
                .regions(ALERT),
        );
        commands.add(
            Command::shortcut("ALERT_LEFT", act(Action::AlertSelect(1)))
                .keys(["left", "shift+tab"])
                .regions(ALERT),
        );
        commands.add(
            Command::shortcut("ALERT_RIGHT", act(Action::AlertSelect(-1)))
                .keys(["right", "tab"])
                .regions(ALERT),
        );
    }
}

/// The name of a label or folder id: the built-in ones' own, else the user's label's.
pub(crate) fn label_name<'a>(id: &str, labels: &'a [Label]) -> &'a str {
    match id {
        "INBOX" => "Inbox",
        "SH_IMPORTANT" => "Important",
        "SH_OTHER" => "Other",
        "SH_ARCHIVED" => "Done",
        "STARRED" => "Starred",
        "SPAM" => "Spam",
        "TRASH" => "Trash",
        id => labels
            .iter()
            .find(|label| label.id == id)
            .map_or("", |label| label.name.as_str()),
    }
}

/// A list's label as Move and `Y` see it.
fn list_label(list: &ListId) -> String {
    match list {
        ListId::Inbox => "INBOX".into(),
        ListId::Important => "SH_IMPORTANT".into(),
        ListId::Other => "SH_OTHER".into(),
        ListId::Done => "SH_ARCHIVED".into(),
        ListId::Starred => "STARRED".into(),
        ListId::Sent => "SENT".into(),
        ListId::Drafts => "DRAFT".into(),
        ListId::Spam => "SPAM".into(),
        ListId::Trash => "TRASH".into(),
        ListId::Muted => "SH_MUTED".into(),
        ListId::Scheduled => "SH_SCHEDULED".into(),
        ListId::Reminders => "SH_SNOOZED".into(),
        ListId::All => "SH_ALL".into(),
        ListId::Label(id) | ListId::Split(id) => id.clone(),
    }
}

/// When `id` was last chosen in a picker.
fn last_used(id: &str, recent: &std::collections::BTreeMap<String, String>) -> Option<Timestamp> {
    recent.get(id).and_then(|at| at.parse().ok())
}

/// The pickers' order: the more recently used first, then (Move only) `MOVE_ORDER`, then by name.
fn compare(
    a: &(String, String),
    b: &(String, String),
    recent: &std::collections::BTreeMap<String, String>,
    built_in: &[&str],
) -> Ordering {
    match (last_used(&a.0, recent), last_used(&b.0, recent)) {
        (Some(a), Some(b)) => b.cmp(&a),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => {
            let rank = |id: &str| built_in.iter().position(|candidate| *candidate == id);
            match (rank(&a.0), rank(&b.0)) {
                (Some(a), Some(b)) => b.cmp(&a),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => compare_label_names(&a.1, &b.1),
            }
        }
    }
}

/// Whether the footer offers to create `query`: something is typed, it isn't reserved, and no label
/// in the picker has that name.
fn creatable<'a>(query: &str, names: impl IntoIterator<Item = &'a str>) -> bool {
    let lower = query.to_lowercase();
    !query.is_empty()
        && !RESERVED.contains(&lower.as_str())
        && !names.into_iter().any(|name| name.to_lowercase() == lower)
}

/// The Move picker for `threads`, moving them out of `current` (the list on screen's label; none in
/// search): every place a thread can go, but Starred (and Important and Other with the unified
/// inbox, or the one on screen); `current` ticked, keys on Spam, Trash and Done.
pub fn move_palette(
    threads: Vec<String>,
    current: Option<String>,
    labels: &[Label],
    unified: bool,
    recent: std::collections::BTreeMap<String, String>,
) -> SubPalette<commands::Action> {
    let built_in = [
        "SH_IMPORTANT",
        "SH_OTHER",
        "INBOX",
        "TRASH",
        "SPAM",
        "SH_ARCHIVED",
    ]
    .map(|id| (id.to_owned(), label_name(id, labels).to_owned()));
    let destinations: Vec<(String, String)> = built_in
        .into_iter()
        .chain(
            crate::lists::listed_labels(labels).map(|label| (label.id.clone(), label.name.clone())),
        )
        .collect();
    let names: Vec<String> = destinations.iter().map(|(_, name)| name.clone()).collect();
    let from = current.clone();
    let footer_threads = threads.clone();
    SubPalette::ordered("MOVE", "Move to …", "data/move", move |query| {
        let query = query.trim();
        let mut matches: Vec<(f64, &(String, String))> = destinations
            .iter()
            .filter_map(|destination| {
                let score = if query.is_empty() {
                    1.0
                } else {
                    command_score(&destination.1, query)
                };
                (score > 0.0).then_some((score, destination))
            })
            .collect();
        matches.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then_with(|| compare(a.1, b.1, &recent, &MOVE_ORDER))
        });
        matches
            .into_iter()
            .filter(|(_, (id, _))| {
                !(matches!(id.as_str(), "SH_IMPORTANT" | "SH_OTHER")
                    && (unified || current.as_ref() == Some(id)))
            })
            .map(|(_, (id, name))| {
                let row = Command::new(
                    id.clone(),
                    name.clone(),
                    commands::Action::Pick(Action::Move(id.clone())),
                );
                match id.as_str() {
                    "TRASH" => row.key_code(["#"]),
                    "SPAM" => row.key_code(["!"]),
                    "SH_ARCHIVED" => row.key_code(["E"]),
                    _ if current.as_ref() == Some(id) => row.in_use(),
                    _ => row,
                }
            })
            .collect()
    })
    .footer(move |query| {
        let query = query.trim();
        creatable(query, names.iter().map(String::as_str))
            .then(|| create_row(query, footer_threads.clone(), Some(from.clone())))
    })
}

/// The Label picker for `threads`: Starred and the user's labels. Those on every thread are ticked
/// and listed first, and choosing one removes it; then the recently used; then the rest by name.
pub fn label_palette(
    threads: &[&ThreadSummary],
    labels: &[Label],
    recent: std::collections::BTreeMap<String, String>,
) -> SubPalette<commands::Action> {
    let candidates: Vec<(String, String)> = std::iter::once((
        "STARRED".to_owned(),
        label_name("STARRED", labels).to_owned(),
    ))
    .chain(crate::lists::listed_labels(labels).map(|label| (label.id.clone(), label.name.clone())))
    .collect();
    let common: Vec<String> = candidates
        .iter()
        .filter(|(id, _)| !threads.is_empty() && threads.iter().all(|thread| thread.has_label(id)))
        .map(|(id, _)| id.clone())
        .collect();
    let prompt = if common.is_empty() {
        "Add label"
    } else {
        "Add or Remove label"
    };
    let ids: Vec<String> = threads.iter().map(|thread| thread.id.clone()).collect();
    let names: Vec<String> = candidates.iter().map(|(_, name)| name.clone()).collect();
    SubPalette::ordered("LABEL", prompt, "data/label", move |query| {
        let query = query.trim();
        let mut matches: Vec<(f64, &(String, String))> = candidates
            .iter()
            .filter_map(|label| {
                let on_all = common.contains(&label.0);
                let score = match query {
                    "" if on_all => 3.0,
                    "" if last_used(&label.0, &recent).is_some() => 2.0,
                    "" => 1.0,
                    query => match command_score(&label.1, query) {
                        score if score > 0.0 && on_all => score + 3.0,
                        score => score,
                    },
                };
                (score > 0.0).then_some((score, label))
            })
            .collect();
        matches.sort_by(|a, b| {
            b.0.total_cmp(&a.0)
                .then_with(|| compare(a.1, b.1, &recent, &[]))
        });
        matches
            .into_iter()
            .map(|(_, (id, name))| {
                if common.contains(id) {
                    Command::new(
                        id.clone(),
                        name.clone(),
                        commands::Action::Pick(Action::RemoveLabel(id.clone())),
                    )
                    .in_use()
                } else {
                    Command::new(
                        id.clone(),
                        name.clone(),
                        commands::Action::Pick(Action::AddLabel(id.clone())),
                    )
                }
            })
            .collect()
    })
    .footer(move |query| {
        let query = query.trim();
        creatable(query, names.iter().map(String::as_str))
            .then(|| create_row(query, ids.clone(), None))
    })
}

/// Someone a conversation's actions can block or move.
pub struct Sender {
    /// Their address.
    pub email: String,
    /// Their address's domain.
    pub domain: Option<String>,
}

/// The Mark Spam picker for `sender`: no input; report spam, or report and block the sender, or
/// their domain unless it's a webmail one or one of `own`.
fn spam_palette(sender: &Sender, own: &[String]) -> SubPalette<commands::Action> {
    let block = |from: &str, domain: bool| {
        let action = commands::Action::Pick(Action::Block {
            from: from.to_owned(),
            domain,
            spam: true,
        });
        Command::new(
            format!("BLOCK_{from}"),
            format!("Mark Spam, and block {from}"),
            action,
        )
        .icon("data/unsubscribe")
    };
    let mut rows = vec![
        Command::new(
            "MARK_SPAM",
            "Mark Spam",
            commands::Action::Apply(Mutation::Spam),
        )
        .icon("data/spam"),
        block(&sender.email, false),
    ];
    if let Some(domain) = sender
        .domain
        .as_ref()
        .filter(|domain| !crate::format::names::is_popular_domain(domain) && !own.contains(domain))
    {
        rows.push(block(domain, true));
    }
    SubPalette::new("MARK_SPAM", "Mark Spam", "data/spam", rows).hide_input()
}

/// `Create label “{name}”`.
fn create_row(
    name: &str,
    threads: Vec<String>,
    moving: Option<Option<String>>,
) -> Command<commands::Action> {
    let action = Action::Create {
        name: name.to_owned(),
        threads,
        moving,
    };
    Command::new(
        "CREATE_LABEL",
        format!("Create label “{name}”"),
        commands::Action::Pick(action),
    )
}

/// What `Y` takes off threads in `list`: the list's own label, or for Exo's own views and Sent,
/// the inbox (nothing for threads already done). Nothing in Drafts or Reminders.
pub(crate) fn removable_label(list: &ListId) -> Option<String> {
    match list {
        ListId::Drafts | ListId::Reminders => None,
        ListId::Starred | ListId::Spam | ListId::Trash | ListId::Label(_) => Some(list_label(list)),
        ListId::Inbox
        | ListId::Important
        | ListId::Other
        | ListId::Done
        | ListId::Sent
        | ListId::Muted
        | ListId::Scheduled
        | ListId::All
        | ListId::Split(_) => Some("INBOX".into()),
    }
}

/// The toast after a move.
pub fn move_toast(from: Option<&str>, to: &str, labels: &[Label]) -> String {
    move_text(from, to, labels, false)
}

/// [`move_toast`], or in the present tense for a bulk move (`present`).
pub(crate) fn move_text(from: Option<&str>, to: &str, labels: &[Label], present: bool) -> String {
    // The inbox, its views and its splits.
    let inbox_like = |id: &str| {
        matches!(id, "INBOX" | "SH_INBOX" | "SH_IMPORTANT" | "SH_OTHER")
            || id.starts_with(exo_store::SPLIT_ID_PREFIX)
    };
    let marking_done = from.is_some_and(|from| inbox_like(from) || to == "SH_ARCHIVED");
    let moving_to_inbox = from.is_some_and(|from| inbox_like(to) || from == "SH_ARCHIVED");
    // Out of Reminders the reminder goes too.
    let (moved, moving) = if from == Some("SH_SNOOZED") {
        ("Reminder removed and moved", "Reminder removed and moving")
    } else {
        ("Moved", "Moving")
    };
    match (to == "SPAM", marking_done && !moving_to_inbox, present) {
        (true, _, false) => "Marked as Spam.".into(),
        (true, _, true) => "Marking as Spam.".into(),
        (false, true, false) => "Moved and Marked Done.".into(),
        (false, true, true) => "Moving and Marking Done.".into(),
        (false, false, false) => format!("{moved} to {}.", label_name(to, labels)),
        (false, false, true) => format!("{moving} to {}.", label_name(to, labels)),
    }
}

impl Mail {
    /// Carries out a picker's `action`.
    pub(crate) fn pick(&mut self, action: Action, now: f64) {
        match action {
            Action::OpenMove => {
                let threads = crate::actionable(self.thread.as_ref(), &self.inbox)
                    .iter()
                    .map(|thread| thread.id.clone())
                    .collect();
                let settings = self.account.settings();
                let palette = move_palette(
                    threads,
                    self.current_label(),
                    &self.snapshot.labels,
                    settings.unified_inbox,
                    settings.labels_last_used.clone(),
                );
                self.commands.open_sub_palette(palette);
            }
            Action::OpenLabel => {
                let threads = crate::actionable(self.thread.as_ref(), &self.inbox);
                let palette = label_palette(
                    &threads,
                    &self.snapshot.labels,
                    self.account.settings().labels_last_used.clone(),
                );
                self.commands.open_sub_palette(palette);
            }
            // One conversation going to Important or Other first asks whether it's just this one.
            Action::Move(to)
                if matches!(to.as_str(), "SH_IMPORTANT" | "SH_OTHER")
                    && self.has_actionable_sender() =>
            {
                self.pick(Action::OpenImportantOrOther(to == "SH_OTHER"), now)
            }
            Action::Move(to) => {
                let ids = self.actionable_ids();
                self.move_threads(&ids, to, now);
            }
            Action::MoveConversation(other) => {
                let ids = self.actionable_ids();
                self.move_threads(
                    &ids,
                    if other { "SH_OTHER" } else { "SH_IMPORTANT" }.to_owned(),
                    now,
                );
            }
            Action::OpenImportantOrOther(other) => {
                let name = if other { "Other" } else { "Important" };
                let to = if other { "SH_OTHER" } else { "SH_IMPORTANT" };
                let mut rows = vec![Command::new(
                    to,
                    format!("Move conversation to {name}"),
                    commands::Action::Pick(Action::MoveConversation(other)),
                )];
                // Not in Spam or Trash; the sender's row unless it's you, the domain's unless
                // it's a webmail one.
                if !matches!(self.list, ListId::Spam | ListId::Trash)
                    && let Some(sender) = self.actionable_sender()
                {
                    let row = |from: &str, domain: bool| {
                        let action = Action::MoveSender {
                            from: from.to_owned(),
                            domain,
                            other,
                        };
                        Command::new(
                            format!("{to}_{from}"),
                            format!("Move all conversations from {from} to {name}"),
                            commands::Action::Pick(action),
                        )
                    };
                    rows.push(row(&sender.email, false));
                    if let Some(domain) = sender
                        .domain
                        .filter(|domain| !crate::format::names::is_popular_domain(domain))
                    {
                        rows.push(row(&domain, true));
                    }
                }
                self.commands.open_sub_palette(
                    SubPalette::new(
                        "MoveToImportantOrOther",
                        format!("Move to {name}"),
                        "data/move",
                        rows,
                    )
                    .hide_input(),
                );
            }
            Action::AddLabel(id) => {
                self.remember_label(&id);
                let text = format!("Added to label {}.", label_name(&id, &self.snapshot.labels));
                let ids = self.actionable_ids();
                self.triage_threads(&ids, Mutation::AddLabel(id), Some(text), 0, now);
            }
            Action::RemoveLabel(id) => {
                let text = if id == "INBOX" {
                    "Marked as Done.".into()
                } else {
                    format!(
                        "Removed from label {}.",
                        label_name(&id, &self.snapshot.labels)
                    )
                };
                let ids = self.actionable_ids();
                self.triage_threads(&ids, Mutation::RemoveLabel(id), Some(text), 0, now);
            }
            Action::Create {
                name,
                threads,
                moving,
            } => {
                self.commands.close_palette();
                self.pickers.alert = Some(Alert {
                    title: "Create Label".into(),
                    text: format!("Would you like to create the label \"{name}\"?"),
                    actions: vec!["Create", "Cancel"],
                    selected: 0,
                    keys: None,
                    then: Some(Then::CreateLabel(Creation {
                        name,
                        threads,
                        moving,
                    })),
                });
            }
            Action::MarkSpam => {
                let threads = crate::actionable(self.thread.as_ref(), &self.inbox);
                let reportable = matches!(threads[..], [thread] if !thread.has_label("SPAM"))
                    && self.list != ListId::Spam;
                match self.actionable_sender().filter(|_| reportable) {
                    Some(sender) => self
                        .commands
                        .open_sub_palette(spam_palette(&sender, &self.own_domains())),
                    None => self.triage(Mutation::Spam, now),
                }
            }
            Action::Block { from, domain, spam } => {
                let Some(thread) = self.actionable_ids().into_iter().next() else {
                    return;
                };
                self.commands.close_palette();
                self.pickers.alert = Some(Alert {
                    title: if domain {
                        "Block domain"
                    } else {
                        "Block sender"
                    }
                    .into(),
                    text: format!(
                        "Trash all messages, including any future messages, from {}?",
                        exo_ui::wbrise(&from)
                    ),
                    actions: vec!["Block", "Cancel"],
                    selected: 0,
                    keys: None,
                    then: Some(Then::Block { from, thread, spam }),
                });
            }
            Action::MoveSender {
                from,
                domain,
                other,
            } => {
                let Some(thread) = self.actionable_ids().into_iter().next() else {
                    return;
                };
                self.commands.close_palette();
                let (what, name) = (
                    if domain { "domain" } else { "sender" },
                    if other { "Other" } else { "Important" },
                );
                self.pickers.alert = Some(Alert {
                    title: format!("Move {what} to {name}"),
                    text: format!("Move all messages, including any future messages, from {from}?"),
                    actions: vec!["Move", "Cancel"],
                    selected: 0,
                    keys: None,
                    then: Some(Then::MoveSender {
                        from,
                        other,
                        thread,
                    }),
                });
            }
            Action::RemoveFromList(direction) => self.remove_from_list(direction, now),
            Action::RemoveAll => self.remove_all_labels(now),
            Action::Alert(index) => {
                let Some(alert) = self.pickers.alert.take() else {
                    return;
                };
                match (index, alert.then) {
                    (0, Some(Then::CreateLabel(creation))) => {
                        self.account.create_label(&creation.name);
                        self.pickers.creating = Some(creation);
                    }
                    (0, Some(Then::Block { from, thread, spam })) => {
                        self.block(from, thread, spam, now)
                    }
                    (
                        0,
                        Some(Then::MoveSender {
                            from,
                            other,
                            thread,
                        }),
                    ) => self.move_sender(from, other, thread, now),
                    (0, Some(Then::Bulk(plan))) => self.start_bulk(*plan),
                    (0, Some(Then::Unsubscribe(plan))) => self.unsubscribe_confirmed(*plan),
                    (0, Some(Then::Undo(token))) => self.account.undo(token),
                    (0, Some(Then::SignOut(email))) => {
                        self.request = Some(crate::accounts::Request::SignOut(email))
                    }
                    (0, Some(Then::RsvpFailed(thread))) => self.open_url(
                        &crate::google_apps::App::Message(thread)
                            .url(self.account.email(), &crate::compose::uuid_v4()),
                    ),
                    (1, Some(Then::RsvpFailed(thread))) => self.open_thread(thread),
                    (0, Some(Then::UpdateSplit { id, query, .. })) => {
                        self.update_existing_split(&id, &query, now)
                    }
                    (_, Some(Then::UpdateSplit { name, query, .. })) => {
                        self.rename_split(name, query)
                    }
                    (0, Some(Then::OpenSplitSettings)) => self.run(
                        commands::Action::Settings(crate::settings::Action::Open(
                            crate::settings::Dialog::SplitInbox,
                        )),
                        now,
                    ),
                    _ => {}
                }
            }
            Action::AlertSelect(delta) => {
                if let Some(alert) = &mut self.pickers.alert {
                    alert.selected = alert
                        .selected
                        .saturating_add_signed(delta)
                        .min(alert.actions.len() - 1);
                }
            }
        }
    }

    /// One conversation is actionable and has a sender (you included): everything but a new
    /// message being written.
    fn has_actionable_sender(&self) -> bool {
        matches!(crate::actionable(self.thread.as_ref(), &self.inbox)[..], [thread] if !thread.draft_only)
    }

    /// The label of the list actions take threads out of: none while searching.
    fn current_label(&self) -> Option<String> {
        self.search.is_none().then(|| list_label(&self.list))
    }

    /// The one actionable conversation's sender, not you: whom the contact pane shows for it. Only
    /// a plain address, since blocking and moving put it in a Gmail filter and search
    /// (`senders::is_plain_address`); for others `!` reports spam alone.
    pub(crate) fn actionable_sender(&self) -> Option<Sender> {
        // One loaded row of a select-all that reaches further isn't one conversation.
        if self.thread.is_none() && !self.inbox.is_exact() {
            return None;
        }
        let [thread] = crate::actionable(self.thread.as_ref(), &self.inbox)[..] else {
            return None;
        };
        let email = thread.contact.as_ref()?.email.clone();
        if !crate::senders::is_plain_address(&email) {
            return None;
        }
        let domain = email
            .rsplit_once('@')
            .map(|(_, domain)| domain.to_lowercase());
        Some(Sender { email, domain })
    }

    /// The domains of your addresses, which are never offered for blocking.
    pub(crate) fn own_domains(&self) -> Vec<String> {
        std::iter::once(self.account.email())
            .chain(
                self.snapshot
                    .aliases
                    .iter()
                    .map(|alias| alias.email.as_str()),
            )
            .filter_map(|email| email.rsplit_once('@'))
            .map(|(_, domain)| domain.to_lowercase())
            .collect()
    }

    /// Blocks `from`: it joins the block list, `thread` goes to Trash at once, and a Gmail filter
    /// and a relabel trash everything from them, now and later.
    fn block(&mut self, from: String, thread: String, spam: bool, now: f64) {
        self.account
            .update_settings(|settings| settings.blocked_senders.push(from.clone()));
        let text = if spam {
            format!("Marked as spam, and blocked {from}.")
        } else {
            format!("Blocked {from}.")
        };
        if let Some(token) = self.triage_threads(
            std::slice::from_ref(&thread),
            Mutation::Trash,
            Some(text),
            0,
            now,
        ) {
            self.account.sender_job(
                SenderJob::Block {
                    from: from.clone(),
                    thread,
                },
                token,
            );
            self.pickers.blocked = Some((token, from));
        }
    }

    /// Moves `thread` to Other (or Important) and, through a Gmail filter and a relabel,
    /// everything from `from` in the inbox and to come.
    fn move_sender(&mut self, from: String, other: bool, thread: String, now: f64) {
        let to = if other { "SH_OTHER" } else { "SH_IMPORTANT" };
        let text = format!(
            "Moving to {}: {from}.",
            if other { "Other" } else { "Important" }
        );
        let split = self.search.is_none() && matches!(self.list, ListId::Split(_));
        let mutation = Mutation::Move {
            from: self.current_label(),
            to: to.to_owned(),
        };
        if let Some(token) =
            self.triage_threads(std::slice::from_ref(&thread), mutation, Some(text), 0, now)
        {
            self.account.sender_job(
                SenderJob::Importance {
                    from,
                    other,
                    thread,
                },
                token,
            );
        }
        if split {
            self.commands.hide_toast();
            let name = if other { "Other" } else { "Important" };
            self.commands.hint(Hint::new().text(format!(
                "This conversation cannot move as it matches both this Split Inbox and {name}."
            )));
        }
    }

    /// An undo ran: a block it undoes comes off the block list.
    pub(crate) fn picker_undone(&mut self, token: crate::account::UndoToken) {
        if let Some((_, from)) = self
            .pickers
            .blocked
            .take_if(|(blocked, _)| *blocked == token)
        {
            self.account.update_settings(|settings| {
                settings.blocked_senders.retain(|blocked| *blocked != from)
            });
        }
    }

    /// The ids of the threads actions apply to.
    pub(crate) fn actionable_ids(&self) -> Vec<String> {
        crate::actionable(self.thread.as_ref(), &self.inbox)
            .iter()
            .map(|thread| thread.id.clone())
            .collect()
    }

    /// Records in `labels_last_used` that label `id` was just chosen, as an ISO time with
    /// milliseconds.
    fn remember_label(&mut self, id: &str) {
        let now = format!("{:.3}", Timestamp::now());
        self.account.update_settings(|settings| {
            settings.labels_last_used.insert(id.to_owned(), now);
        });
    }

    /// Moves `threads` from the list on screen to `to`. From a split inbox to the Inbox, Important
    /// or Other, the move happens but a hint says it can't take the thread out of the split.
    fn move_threads(&mut self, threads: &[String], to: String, now: f64) {
        self.remember_label(&to);
        let from = self.current_label();
        let text = move_toast(from.as_deref(), &to, &self.snapshot.labels);
        let blocked = match to.as_str() {
            "INBOX" => Some("Inbox"),
            "SH_IMPORTANT" => Some("Important"),
            "SH_OTHER" => Some("Other"),
            _ => None,
        }
        .filter(|_| self.search.is_none() && matches!(self.list, ListId::Split(_)));
        // A bulk move (`None` here) says nothing of splits.
        let moved = self.triage_threads(threads, Mutation::Move { from, to }, Some(text), 0, now);
        if let Some(name) = blocked.filter(|_| moved.is_some()) {
            self.commands.hide_toast();
            self.commands.hint(Hint::new().text(format!(
                "This conversation cannot move as it matches both this Split Inbox and {name}."
            )));
        }
    }

    /// `Y`, `[` and `]`: takes the list's label off the actionable threads; with a `direction`, the
    /// cursor moves that way after.
    fn remove_from_list(&mut self, direction: isize, now: f64) {
        // Search results have no label: they leave the inbox.
        let label = if self.search.is_some() {
            Some("INBOX".to_owned())
        } else {
            removable_label(&self.list)
        };
        let Some(label) = label else { return };
        let threads = crate::actionable(self.thread.as_ref(), &self.inbox);
        let affected = threads.iter().any(|thread| {
            if label == "INBOX" {
                !thread.done
            } else {
                thread.has_label(&label)
            }
        });
        if !affected {
            if direction != 0 {
                self.inbox.clear_selection();
                self.step(direction);
            }
            return;
        }
        let ids: Vec<String> = threads.iter().map(|thread| thread.id.clone()).collect();
        let text = match label.as_str() {
            "INBOX" => "Marked as Done.".to_owned(),
            label => match label_name(label, &self.snapshot.labels) {
                "" => "Removed from label.".to_owned(),
                name => format!("Removed from {name}."),
            },
        };
        self.triage_threads(
            &ids,
            Mutation::RemoveLabel(label),
            Some(text),
            direction,
            now,
        );
        // Search results stay listed, so the cursor just moves on.
        if self.search.is_some() && direction != 0 {
            self.step(direction);
        }
    }

    /// `⇧Y`: takes off every label the threads show (and Spam and Trash) and marks them done.
    fn remove_all_labels(&mut self, now: f64) {
        if self.refuse_unloaded("remove all labels") {
            return;
        }
        let mut labels: Vec<String> = crate::lists::user_labels(&self.snapshot.labels)
            .map(|label| label.id.clone())
            .collect();
        labels.extend(["TRASH", "SPAM", "INBOX"].map(str::to_owned));
        let threads = crate::actionable(self.thread.as_ref(), &self.inbox);
        let affected: Vec<&&ThreadSummary> = threads
            .iter()
            .filter(|thread| labels.iter().any(|label| thread.has_label(label)))
            .collect();
        if affected.is_empty() {
            return;
        }
        let text = if affected.iter().all(|thread| thread.has_label("INBOX")) {
            "Removed all labels and marked as Done."
        } else {
            "Removed all labels."
        };
        let ids: Vec<String> = threads.iter().map(|thread| thread.id.clone()).collect();
        self.triage_threads(
            &ids,
            Mutation::RemoveLabels(labels),
            Some(text.to_owned()),
            0,
            now,
        );
    }

    /// Moves the cursor one row in `direction`, or from a thread opens that conversation.
    pub(crate) fn step(&mut self, direction: isize) {
        if self.thread.is_some() {
            self.next_conversation(direction);
        } else {
            self.inbox.move_cursor(direction);
        }
    }

    /// Handles what the account says about a label being created.
    pub(crate) fn label_event(&mut self, event: Event, now: f64) {
        let Some(creation) = self.pickers.creating.take() else {
            return;
        };
        match event {
            Event::LabelCreated(label) => match creation.moving {
                Some(_) => self.move_threads(&creation.threads, label.id, now),
                None => {
                    self.remember_label(&label.id);
                    let text = format!("Added to label {}.", label.name);
                    self.triage_threads(
                        &creation.threads,
                        Mutation::AddLabel(label.id),
                        Some(text),
                        0,
                        now,
                    );
                }
            },
            Event::LabelNotCreated { offline: true } => {
                self.pickers.alert = Some(Alert {
                    title: "Sorry!".into(),
                    // The offline alert.
                    text: "We're working hard to make Exo fully function offline. In the meantime, please check your internet connection and try again.".into(),
                    actions: vec!["OK"],
                    selected: 0,
                    keys: None,
                    then: None,
                });
            }
            _ => self
                .commands
                .toast(Toast::new("Sorry! Failed to create label.").kind(ToastKind::Error)),
        }
    }

    /// The pickers' alert, its title and text, if one shows. For tests.
    pub fn alert(&self) -> Option<(&str, &str)> {
        self.pickers
            .alert
            .as_ref()
            .map(|alert| (alert.title.as_str(), alert.text.as_str()))
    }

    /// Draws the pickers' alert, if one shows, over `page`.
    pub(crate) fn show_picker_alert(&mut self, ui: &mut egui::Ui, page: egui::Rect) {
        let Some(alert) = &self.pickers.alert else {
            return;
        };
        // Over any dialog it was asked from (Bulk Actions' `Undo action`).
        let clicked = egui::Area::new(ui.id().with("picker-alert"))
            .order(egui::Order::Tooltip)
            .fixed_pos(page.min)
            .show(ui.ctx(), |ui| {
                // The overlay: the page dimmed to half, nothing behind it clickable.
                ui.painter()
                    .rect_filled(page, 0.0, egui::Color32::from_black_alpha(128));
                let _ = ui.allocate_rect(page, egui::Sense::click());
                let shown = exo_ui::Alert {
                    title: &alert.title,
                    text: &alert.text,
                    actions: &alert.actions,
                    selected: alert.selected,
                };
                match &alert.keys {
                    Some(keys) => {
                        shown.show_with_keys(ui, page, &alert.text, keys.caps, keys.after)
                    }
                    None => shown.show(ui, page),
                }
            });
        if let Some(index) = clicked.inner {
            let now = ui.input(|input| input.time);
            self.pick(Action::Alert(index), now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use exo_store::LabelKind;

    fn label(id: &str, name: &str) -> Label {
        Label {
            id: id.into(),
            name: name.into(),
            kind: LabelKind::User,
            text_color: None,
            background_color: None,
            hidden: false,
        }
    }

    fn titles(palette: &SubPalette<commands::Action>, query: &str) -> Vec<String> {
        palette.titles(query)
    }

    fn labels() -> Vec<Label> {
        vec![
            label("Label_1", "Projects"),
            label("Label_2", "Clients"),
            label("Label_3", "Receipts 10"),
            label("Label_4", "Receipts 9"),
            label("Label_9", "[Exo]/Muted"),
        ]
    }

    #[test]
    fn move_lists_spam_trash_done_then_the_rest_by_name() {
        let palette = move_palette(
            vec!["t".into()],
            Some("INBOX".into()),
            &labels(),
            true,
            Default::default(),
        );
        assert_eq!(
            titles(&palette, ""),
            [
                "Spam",
                "Trash",
                "Done",
                "Clients",
                "Inbox",
                "Projects",
                "Receipts 9",
                "Receipts 10"
            ]
        );
        let split = move_palette(
            vec!["t".into()],
            Some("SH_IMPORTANT".into()),
            &labels(),
            false,
            Default::default(),
        );
        assert_eq!(titles(&split, "")[..4], ["Spam", "Trash", "Done", "Other"]);
    }

    #[test]
    fn recently_used_labels_come_first() {
        let recent = [
            ("Label_1".to_owned(), "2026-11-16T10:00:00.000Z".to_owned()),
            ("Label_4".to_owned(), "2026-11-17T10:00:00.000Z".to_owned()),
        ]
        .into();
        let palette = move_palette(
            vec!["t".into()],
            Some("INBOX".into()),
            &labels(),
            true,
            recent,
        );
        assert_eq!(
            titles(&palette, "")[..3],
            ["Receipts 9", "Projects", "Spam"]
        );
    }

    #[test]
    fn create_is_offered_for_new_unreserved_names() {
        assert!(creatable("Travel", ["Projects", "Clients"]));
        assert!(!creatable("clients", ["Projects", "Clients"]));
        assert!(!creatable("Todo", ["Projects"]));
        assert!(!creatable("", ["Projects"]));
    }

    #[test]
    fn move_toasts_say_when_threads_leave_the_inbox() {
        let labels = labels();
        assert_eq!(
            move_toast(Some("INBOX"), "Label_1", &labels),
            "Moved and Marked Done."
        );
        assert_eq!(
            move_toast(Some("INBOX"), "TRASH", &labels),
            "Moved and Marked Done."
        );
        assert_eq!(
            move_toast(Some("SH_ARCHIVED"), "Label_1", &labels),
            "Moved to Projects."
        );
        assert_eq!(
            move_toast(Some("Label_2"), "SH_ARCHIVED", &labels),
            "Moved and Marked Done."
        );
        assert_eq!(
            move_toast(Some("SH_IMPORTANT"), "SH_OTHER", &labels),
            "Moved to Other."
        );
        assert_eq!(
            move_toast(Some("SH_SNOOZED"), "INBOX", &labels),
            "Reminder removed and moved to Inbox."
        );
        assert_eq!(
            move_text(Some("SH_SNOOZED"), "Label_1", &labels, true),
            "Reminder removed and moving to Projects."
        );
        assert_eq!(
            move_toast(Some("SH_SNOOZED"), "SH_ARCHIVED", &labels),
            "Moved and Marked Done.",
            "done comes first"
        );
        assert_eq!(
            move_toast(Some("Label_2"), "SPAM", &labels),
            "Marked as Spam."
        );
    }
}
