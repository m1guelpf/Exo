//! The lists Exo shows (its views over Gmail labels), what they're called, and how `g` sequences
//! and the palette's `go to` rows reach them.

use exo_store::{Label, LabelKind, ListId, SplitInbox};

use crate::mailbox::{EXO_LABEL_PREFIX, FOREIGN_STATE_LABEL_PREFIX};

/// The name a view's header and the window title show; `splits` are the account's.
pub fn name<'a>(
    list: &ListId,
    labels: &'a [Label],
    splits: impl IntoIterator<Item = &'a SplitInbox>,
) -> &'a str {
    match list {
        ListId::Inbox => "Inbox",
        ListId::Important => "Important",
        ListId::Other => "Other",
        ListId::Done => "Done",
        ListId::Starred => "Starred",
        ListId::Sent => "Sent",
        ListId::Drafts => "Drafts",
        ListId::Spam => "Spam",
        ListId::Trash => "Trash",
        ListId::Muted => "Muted",
        ListId::Scheduled => "Scheduled",
        ListId::Reminders => "Reminders",
        ListId::All => "All Mail",
        ListId::Label(id) => labels
            .iter()
            .find(|label| label.id == *id)
            .map_or("", |label| label.name.as_str()),
        ListId::Split(id) => splits
            .into_iter()
            .find(|split| split.id == *id)
            .map_or("", SplitInbox::name),
    }
}

/// The inbox views: the tab bar with counts and Add Split, and Inbox Zero when empty.
pub fn is_inbox(list: &ListId) -> bool {
    crate::splits::is_inbox_view(list)
}

/// A system view's `go to` row and `g` sequence.
pub struct GoTo {
    /// The view.
    pub list: ListId,
    /// The row's id (the label's slug).
    pub id: &'static str,
    /// The view's name (`go to {name}`).
    pub name: &'static str,
    /// The word that boosts the row when typed.
    pub operative_word: &'static str,
    /// The sequences that go there.
    pub keys: &'static [&'static str],
    /// The keycaps the row shows.
    pub key_code: &'static [&'static str],
    /// The row this one sorts after when both match (`go to trash` after `Trash`).
    pub follow: Option<&'static str>,
}

const fn go_to(
    list: ListId,
    id: &'static str,
    name: &'static str,
    keys: &'static [&'static str],
    key_code: &'static [&'static str],
    follow: Option<&'static str>,
) -> GoTo {
    GoTo {
        list,
        id,
        name,
        operative_word: name,
        keys,
        key_code,
        follow,
    }
}

/// The system views before the user's labels, in the palette's order: Important and Other
/// unless the inbox is unified, All Mail spliced in after them, then Gmail's labels as
/// `labels.list` orders them. Snippets don't exist in Exo yet. `g i` and `g o` aren't bound
/// here: they go home and to Other whatever the rows are.
pub fn system_go_tos(unified: bool) -> Vec<GoTo> {
    let important_other = [
        go_to(
            ListId::Important,
            "important",
            "Important",
            &[],
            &["G", "then", "I"],
            Some("MOVE_TO_IMPORTANT"),
        ),
        go_to(
            ListId::Other,
            "other",
            "Other",
            &[],
            &["G", "then", "O"],
            Some("MOVE_TO_OTHER"),
        ),
    ];
    let important_other = if unified {
        Vec::new()
    } else {
        important_other.into()
    };
    important_other
        .into_iter()
        .chain([
            go_to(
                ListId::All,
                "all",
                "All Mail",
                &["g a"],
                &["G", "then", "A"],
                None,
            ),
            GoTo {
                // `go to Sent` boosts on a lowercase `sent`.
                operative_word: "sent",
                ..go_to(
                    ListId::Sent,
                    "sent",
                    "Sent",
                    &["g t"],
                    &["G", "then", "T"],
                    None,
                )
            },
            go_to(
                ListId::Inbox,
                "inbox",
                "Inbox",
                &[],
                &["G", "then", "I"],
                None,
            ),
            go_to(
                ListId::Trash,
                "trash",
                "Trash",
                &["g #"],
                &["G", "then", "#"],
                Some("TRASH"),
            ),
            go_to(
                ListId::Drafts,
                "draft",
                "Drafts",
                &["g d"],
                &["G", "then", "D"],
                None,
            ),
            go_to(
                ListId::Spam,
                "spam",
                "Spam",
                &["g !"],
                &["G", "then", "!"],
                Some("MARK_SPAM"),
            ),
            go_to(
                ListId::Starred,
                "starred",
                "Starred",
                &["g s"],
                &["G", "then", "S"],
                None,
            ),
        ])
        .collect()
}

/// The system views listed after the user's labels, in the palette's order: Scheduled has no
/// `g` key.
pub fn late_go_tos() -> [GoTo; 4] {
    [
        go_to(
            ListId::Done,
            "done",
            "Done",
            &["g e"],
            &["G", "then", "E"],
            Some("MARK_DONE"),
        ),
        go_to(ListId::Scheduled, "scheduled", "Scheduled", &[], &[], None),
        go_to(
            ListId::Reminders,
            "reminders",
            "Reminders",
            &["g h"],
            &["G", "then", "H"],
            None,
        ),
        go_to(
            ListId::Muted,
            "muted",
            "Muted",
            &["g m", "g shift+m"],
            &["G", "then", "M"],
            None,
        ),
    ]
}

/// The user's labels, which show as chips and can be gone to or added: every user label but the
/// `[Superhuman]/…` and Exo's own state labels, in Gmail's order. Hiding a label from Gmail's
/// label list doesn't hide it here.
pub fn user_labels(labels: &[Label]) -> impl Iterator<Item = &Label> {
    labels.iter().filter(|label| {
        label.kind == LabelKind::User
            && !label.name.starts_with(FOREIGN_STATE_LABEL_PREFIX)
            && !label.name.starts_with(EXO_LABEL_PREFIX)
    })
}

/// The user's labels as the palette lists them to go to, move to or add, and the folder drawer
/// lists them: [`user_labels`] without Auto Labels (`[News]`), which show only as chips.
pub fn listed_labels(labels: &[Label]) -> impl Iterator<Item = &Label> {
    user_labels(labels).filter(|label| !crate::mailbox::is_auto_label(&label.name))
}

/// A label's slug, its `go to` row's id: lowercase, with whitespace, `&`, `(` and `)` as `-`.
pub fn slug(name: &str) -> String {
    name.chars()
        .map(|c| {
            if matches!(c, '&' | '(' | ')') || c.is_whitespace() {
                '-'
            } else {
                c
            }
        })
        .collect::<String>()
        .to_lowercase()
}
