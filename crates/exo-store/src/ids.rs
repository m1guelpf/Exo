//! Identifiers: Gmail thread ids and Exo's list ids.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A Gmail thread id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ThreadId(String);

impl ThreadId {
    /// Wraps a Gmail thread id.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id as Gmail spells it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ThreadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for ThreadId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl From<&str> for ThreadId {
    fn from(id: &str) -> Self {
        Self(id.to_owned())
    }
}

/// A thread list, as threads are routed into them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ListId {
    /// The Inbox: [`Important`](Self::Important) and [`Other`](Self::Other) together.
    Inbox,
    /// Inbox threads with an important message (`SH_IMPORTANT`).
    Important,
    /// Every other inbox thread (`SH_OTHER`).
    Other,
    /// Threads not in the inbox (`SH_ARCHIVED`).
    Done,
    /// `STARRED`.
    Starred,
    /// `SENT`.
    Sent,
    /// Threads with a Gmail draft (`DRAFT`).
    Drafts,
    /// `SPAM`.
    Spam,
    /// `TRASH`.
    Trash,
    /// Muted threads (`SH_MUTED`).
    Muted,
    /// Threads with a scheduled send (`SH_SCHEDULED`), soonest first. Exo keeps every one, so
    /// the list is never short of Gmail's.
    Scheduled,
    /// Threads with a reminder set (`SH_SNOOZED`), the soonest first.
    Reminders,
    /// Every thread (`SH_ALL`).
    All,
    /// Any other Gmail label by id, usually a user label (`Label_12`).
    Label(String),
    /// A split inbox, by its id (`SH_SPLIT_INBOX_…`, [`SplitInbox::id`](crate::SplitInbox::id)).
    Split(String),
}

pub(crate) mod labels {
    //! Gmail system label ids and Exo's virtual list ids.

    pub const INBOX: &str = "INBOX";
    pub const UNREAD: &str = "UNREAD";
    pub const STARRED: &str = "STARRED";
    pub const SENT: &str = "SENT";
    pub const DRAFT: &str = "DRAFT";
    pub const SPAM: &str = "SPAM";
    pub const TRASH: &str = "TRASH";
    pub const CHAT: &str = "CHAT";
    pub const CATEGORY_PERSONAL: &str = "CATEGORY_PERSONAL";
    pub const CATEGORY_SOCIAL: &str = "CATEGORY_SOCIAL";
    pub const CATEGORY_FORUMS: &str = "CATEGORY_FORUMS";
    pub const CATEGORY_PROMOTIONS: &str = "CATEGORY_PROMOTIONS";
    pub const CATEGORY_UPDATES: &str = "CATEGORY_UPDATES";

    pub const SH_IMPORTANT: &str = "SH_IMPORTANT";
    pub const SH_OTHER: &str = "SH_OTHER";
    pub const SH_ARCHIVED: &str = "SH_ARCHIVED";
    pub const SH_MUTED: &str = "SH_MUTED";
    pub const SH_SCHEDULED: &str = "SH_SCHEDULED";
    pub const SH_SNOOZED: &str = "SH_SNOOZED";
    pub const SH_ALL: &str = "SH_ALL";
    pub const SH_NO_REPLY: &str = "SH_NO_REPLY";

    /// Exo's hidden labels in Gmail: their parent, and those under it.
    pub const EXO_PARENT: &str = "[Exo]";
    pub const EXO_MUTED: &str = "[Exo]/Muted";
    /// A reminder is set.
    pub const EXO_SNOOZED: &str = "[Exo]/Is Snoozed";
    /// A reminder brought the thread back and it hasn't been read since.
    pub const EXO_REMINDER_UNREAD: &str = "[Exo]/ru";
}

/// A list filter: one at a time narrows the list on screen to the threads in another list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Filter {
    /// `UNREAD`.
    Unread,
    /// `STARRED`.
    Starred,
    /// Important (`SH_IMPORTANT`).
    Important,
    /// The last message is the user's: awaiting a reply (`SH_NO_REPLY`).
    NoReply,
}

impl Filter {
    /// The list a thread must be in to pass.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Unread => labels::UNREAD,
            Self::Starred => labels::STARRED,
            Self::Important => labels::SH_IMPORTANT,
            Self::NoReply => labels::SH_NO_REPLY,
        }
    }
}

impl ListId {
    /// The id this list is stored under, in `list_ids` and `lists`.
    pub(crate) fn key(&self) -> &str {
        use labels::*;
        match self {
            Self::Inbox => INBOX,
            Self::Important => SH_IMPORTANT,
            Self::Other => SH_OTHER,
            Self::Done => SH_ARCHIVED,
            Self::Starred => STARRED,
            Self::Sent => SENT,
            Self::Drafts => DRAFT,
            Self::Spam => SPAM,
            Self::Trash => TRASH,
            Self::Muted => SH_MUTED,
            Self::Scheduled => SH_SCHEDULED,
            Self::Reminders => SH_SNOOZED,
            Self::All => SH_ALL,
            Self::Label(id) | Self::Split(id) => id,
        }
    }

    /// The membership rows a listing reads. The Inbox is Important plus Other; reading the
    /// `INBOX` rows instead would include threads a split takes out of Important/Other.
    pub(crate) fn members(&self) -> Vec<&str> {
        match self {
            Self::Inbox => vec![labels::SH_IMPORTANT, labels::SH_OTHER],
            other => vec![other.key()],
        }
    }
}
