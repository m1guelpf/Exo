//! What the screens show, reduced from the store's threads to the facts the rows and contact
//! pane display: [`ThreadSummary`] is one row of a list, [`ContactCard`] one person.

use std::{cmp::Ordering, collections::HashMap, ops::Range};

pub use exo_store::Attachments;
use exo_store::{Label, ListId, Mutation, ReminderStatus, SendFailure};
use jiff::Timestamp;

use crate::format::{names::Person, row::Participant};

/// The `[Superhuman]/…` state labels (`[Superhuman]/Muted`, …); never shown as chips.
pub(crate) const FOREIGN_STATE_LABEL_PREFIX: &str = "[Superhuman]";
/// Exo's own hidden labels (exo-store's `[Exo]/Muted`).
pub(crate) const EXO_LABEL_PREFIX: &str = "[Exo]";
/// The muted chip's label (`SH_MUTED`, named `Muted`).
const MUTED_CHIP: &str = "Muted";
/// Gmail's spam label, and its chip's name.
const SPAM: &str = "SPAM";
const SPAM_CHIP: &str = "Spam";
/// Gmail's trash label, and its chip's name (shown only in search results).
const TRASH: &str = "TRASH";
const TRASH_CHIP: &str = "Trash";

/// One row of a thread list.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadSummary {
    /// The Gmail thread id.
    pub id: String,
    /// Who the row names, in order, me already dropped: the senders, the recipients in Sent, or
    /// with drafts, the last draft's recipients.
    pub participants: Vec<Participant>,
    /// The participants are recipients (Sent, or a draft): all of them are listed, with no
    /// ` .. ` for the ones that don't fit.
    pub recipients: bool,
    /// How many drafts the row shows: Exo's own, and in Drafts, any.
    pub drafts: usize,
    /// Every message the row shows is a draft (a new message being written).
    pub draft_only: bool,
    /// The first message's subject, or `-`.
    pub subject: String,
    /// The latest message's (or draft's) snippet, as plain text.
    pub snippet: String,
    /// The label chips, in display order.
    pub labels: Vec<String>,
    /// Every Gmail label id on the thread, for the commands that toggle them.
    pub label_ids: Vec<String>,
    /// The thread's date in this list; rows sort and date by it.
    pub received: Timestamp,
    /// Some message is unread.
    pub unread: bool,
    /// Some message is starred.
    pub starred: bool,
    /// Not routed to the inbox: non-inbox lists mark it with a check.
    pub done: bool,
    /// Muted: the mute label, and no newer message added you as a recipient.
    pub muted: bool,
    /// The icon in the row's metadata.
    pub attachment: Attachments,
    /// Whom the contact pane shows while this row has the cursor.
    pub contact: Option<Person>,
    /// The mutations that take the thread out of this list: its row slides out when one is
    /// applied.
    pub removed_by: Vec<Mutation>,
    /// Its messages, drafts included (a search result shows ` (N)` past one).
    pub messages: usize,
    /// A search result's matched words: where `subject` and `snippet` are bold.
    pub search: Option<SearchMarks>,
    /// It has a failed draft, for this reason: the row says so instead of its date.
    pub failure: Option<SendFailure>,
    /// A send of its is held or waiting to go: offline, the row says when it will go.
    pub sending: bool,
    /// Its earliest scheduled send, and whether Gmail has each: the row says when it goes instead
    /// of its date.
    pub scheduled: Option<(jiff::Timestamp, bool)>,
    /// Its reminder, while the row shows it.
    pub reminder: Option<ReminderStatus>,
    /// Its attached files, which a search result shows as cards under its row.
    pub files: Vec<exo_store::Attachment>,
}

impl ThreadSummary {
    /// A reminder is set.
    pub fn has_reminder(&self) -> bool {
        matches!(self.reminder, Some(ReminderStatus::Set { .. }))
    }

    /// Back from a reminder and not read since.
    pub fn reminder_unread(&self) -> bool {
        matches!(
            self.reminder,
            Some(ReminderStatus::Returned { unread: true, .. })
        )
    }
}

/// The bold parts of a search result's subject and snippet (byte ranges).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchMarks {
    /// In [`ThreadSummary::subject`].
    pub subject: Vec<Range<usize>>,
    /// In [`ThreadSummary::snippet`].
    pub snippet: Vec<Range<usize>>,
    /// Words of the query that highlight a participant's name where they match it, or the
    /// whole name where they're its address.
    pub names: Vec<String>,
}

impl ThreadSummary {
    /// `thread` as a row of `list`, with chips named from `chips` (user label names by id).
    pub fn new(thread: exo_store::ThreadSummary, list: &ListId, chips: &Chips) -> Self {
        let subject = thread.subject.replace('\u{feff}', "");
        let subject = subject.trim_end();
        Self {
            id: thread.id.to_string(),
            participants: thread
                .participants
                .into_iter()
                .map(|contact| Participant {
                    is_me: contact.is_me,
                    person: Person::new(contact.name.unwrap_or_default(), contact.email),
                })
                .collect(),
            recipients: *list == ListId::Sent || thread.drafts > 0,
            drafts: thread.drafts,
            draft_only: thread.drafts == thread.messages,
            subject: if subject.is_empty() {
                "-".into()
            } else {
                subject.to_owned()
            },
            snippet: thread.snippet,
            labels: chips.for_thread(&thread.labels, thread.muted, list),
            received: thread.date,
            unread: thread.unread,
            starred: thread.starred,
            // Spam doesn't count as done: no check on its rows.
            done: thread.done && !thread.labels.iter().any(|label| label == SPAM),
            muted: thread.muted,
            attachment: thread.attachments,
            contact: thread
                .contact
                .map(|contact| Person::new(contact.name.unwrap_or_default(), contact.email)),
            label_ids: thread.labels,
            removed_by: thread.removed_by,
            messages: thread.messages,
            search: None,
            failure: thread.failure,
            sending: thread.sending,
            scheduled: thread.scheduled,
            reminder: thread.reminder,
            files: thread.files,
        }
    }

    /// A search result's row: its subject and excerpt as the search found them, with Trash among
    /// its chips and the labels the query asks for left out.
    pub fn search_result(
        result: exo_store::SearchResult,
        queried: &[String],
        names: &[String],
        chips: &Chips,
    ) -> Self {
        let mut row = Self::new(result.thread, &ListId::All, chips);
        row.labels = chips.for_search(&row.label_ids, row.muted, queried);
        let mut marks = SearchMarks {
            names: names.to_vec(),
            ..SearchMarks::default()
        };
        if let Some(subject) = result.subject {
            let trimmed = subject.text.replace('\u{feff}', "");
            let trimmed = trimmed.trim_end();
            marks.subject = subject
                .bold
                .into_iter()
                .filter(|range| range.end <= trimmed.len())
                .collect();
            row.subject = trimmed.to_owned();
        }
        if let Some(snippet) = result.snippet {
            marks.snippet = snippet.bold;
            row.snippet = snippet.text;
        }
        row.search = Some(marks);
        row
    }

    /// Has the Gmail label `id`.
    pub fn has_label(&self, id: &str) -> bool {
        self.label_ids.iter().any(|label| label == id)
    }
}

/// Which labels show as chips on rows: the user's own labels and Auto Labels, by id, each with
/// the name it sorts by and the name it shows.
#[derive(Clone, Debug, Default)]
pub struct Chips(HashMap<String, (String, String)>);

impl Chips {
    /// The chips among `labels`: user labels and Auto Labels, not the `[Superhuman]/…` or Exo's
    /// own state labels.
    pub fn new(labels: &[Label]) -> Self {
        Self(
            labels
                .iter()
                .filter(|label| label.kind == exo_store::LabelKind::User)
                .filter_map(|label| {
                    Some((
                        label.id.clone(),
                        (label.name.clone(), chip_name(&label.name)?),
                    ))
                })
                .collect(),
        )
    }

    /// A search result's chips: Trash shows, and the labels the query asks for (`queried`, ids)
    /// don't.
    pub fn for_search(&self, ids: &[String], muted: bool, queried: &[String]) -> Vec<String> {
        let shown: Vec<String> = ids
            .iter()
            .filter(|id| !queried.contains(id))
            .cloned()
            .collect();
        let mut chips = self.sorted(&shown, &ListId::All);
        if shown.iter().any(|id| id == TRASH) {
            chips.push((TRASH_CHIP, TRASH_CHIP));
            chips.sort_by(|a, b| compare_label_names(a.0, b.0));
        }
        lead_with_muted(chips, muted && !queried.iter().any(|id| id == "SH_MUTED"))
    }

    /// The open thread's header chips: like a row's, without leaving out the label being viewed.
    pub fn header_chips(&self, ids: &[String], muted: bool) -> Vec<String> {
        self.for_thread(ids, muted, &ListId::Inbox)
    }

    /// A thread's chips in `list`: a leading `muted` chip outside Muted, then its labels in
    /// natural order of their Gmail names, without the label being viewed, and `spam` in Spam
    /// (where every thread is spam through and through).
    fn for_thread(&self, ids: &[String], muted: bool, list: &ListId) -> Vec<String> {
        lead_with_muted(self.sorted(ids, list), muted && *list != ListId::Muted)
    }

    /// `ids`' chips in `list` as (sort name, shown name), in order.
    fn sorted<'a>(&'a self, ids: &[String], list: &ListId) -> Vec<(&'a str, &'a str)> {
        let mut chips: Vec<(&str, &str)> = ids
            .iter()
            .filter(|id| !matches!(list, ListId::Label(viewed) if viewed == *id))
            .filter_map(|id| self.0.get(id))
            .map(|(name, shown)| (name.as_str(), shown.as_str()))
            .collect();
        if *list == ListId::Spam && ids.iter().any(|id| id == SPAM) {
            chips.push((SPAM_CHIP, SPAM_CHIP));
        }
        chips.sort_by(|a, b| compare_label_names(a.0, b.0));
        chips.dedup_by(|a, b| a.1 == b.1);
        chips
    }
}

/// The chips' shown names, after a `muted` chip when `muted`.
fn lead_with_muted(chips: Vec<(&str, &str)>, muted: bool) -> Vec<String> {
    muted
        .then_some(MUTED_CHIP)
        .into_iter()
        .chain(chips.into_iter().map(|(_, shown)| shown))
        .map(str::to_owned)
        .collect()
}

/// The names the built-in Auto Labels have.
const AUTO_LABELS: &[&str] = &[
    "Pitch",
    "Marketing",
    "Social",
    "News",
    "Respond",
    "Waiting",
    "Signature",
    "Calendar",
    "FYI",
    "Contract",
    "Invoice",
    "Login",
    "Meeting",
    "Purchase",
    "Shipping",
    "Tax",
    "Travel",
    "Notifications",
    "Notification",
    "Promotions",
    "Promotion",
];

/// What the chip of a user label named `name` says, or `None` when it's a `[Superhuman]/…` or
/// Exo's own state label, which never show.
///
/// An Auto Label is known by its name in Gmail: `[Superhuman]/AI/{name}`, and for the built-in
/// ones `{name}`, `[{name}]`, `[{n}] {name}` or `[{n}. {name}]`. Its chip shows `{name}`.
pub(crate) fn chip_name(name: &str) -> Option<String> {
    if let Some(auto) = name.strip_prefix("[Superhuman]/AI/") {
        return Some(auto.trim().to_owned());
    }
    if name.starts_with(FOREIGN_STATE_LABEL_PREFIX) || name.starts_with(EXO_LABEL_PREFIX) {
        return None;
    }
    /// `{name}` from `{n}. {name}` or `{n}] {name}`.
    fn numbered(inner: &str) -> Option<&str> {
        let (n, rest) = inner.split_once(['.', ']'])?;
        (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).then(|| rest.trim())
    }
    let auto = name
        .strip_prefix('[')
        .and_then(|inner| match inner.strip_suffix(']') {
            // `[News]` or `[3. News]`.
            Some(inner) => Some(numbered(inner).unwrap_or(inner)),
            // `[3] News`.
            None => numbered(inner),
        })
        .filter(|auto| {
            AUTO_LABELS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(auto))
        });
    Some(auto.unwrap_or(name).to_owned())
}

/// Whether the user label named `name` is an Auto Label, as [`chip_name`] recognizes them.
pub(crate) fn is_auto_label(name: &str) -> bool {
    chip_name(name).is_some_and(|shown| shown != name)
}

/// What the contact pane knows about someone, from the account's own mail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactCard {
    /// Their name and address.
    pub person: Person,
    /// Their latest conversations (the pane's Mail section), newest first.
    pub recent: Vec<RecentThread>,
}

/// A conversation in the contact pane's Mail section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentThread {
    /// The Gmail thread id.
    pub id: String,
    /// Its subject, or `(no subject)`.
    pub subject: String,
    /// Its latest message, which the row shows while hovered.
    pub date: Timestamp,
}

impl RecentThread {
    /// `thread` as a row of the Mail section.
    pub fn new(thread: exo_store::RecentThread) -> Self {
        let subject = thread.subject.replace('\u{feff}', "");
        let subject = subject.trim_end();
        Self {
            id: thread.id.to_string(),
            subject: if subject.is_empty() {
                "(no subject)".into()
            } else {
                subject.to_owned()
            },
            date: thread.date,
        }
    }
}

/// Natural order for label names: case-insensitive, digit runs compared as numbers.
pub fn compare_label_names(a: &str, b: &str) -> Ordering {
    fn chunks(text: &str) -> Vec<(bool, String)> {
        let mut chunks: Vec<(bool, String)> = Vec::new();
        for c in text.to_lowercase().chars() {
            let digit = c.is_ascii_digit();
            match chunks.last_mut() {
                Some((is_digit, chunk)) if *is_digit == digit => chunk.push(c),
                _ => chunks.push((digit, c.to_string())),
            }
        }
        chunks
    }
    let numeric = |chunk: &str| chunk.trim_start_matches('0').len();
    for ((a_digit, a), (b_digit, b)) in chunks(a).iter().zip(&chunks(b)) {
        let order = if *a_digit && *b_digit {
            numeric(a).cmp(&numeric(b)).then_with(|| a.cmp(b))
        } else {
            a.cmp(b)
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    a.len().cmp(&b.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_names_sort_naturally() {
        assert_eq!(
            compare_label_names("Clients/Calloway", "hiring"),
            Ordering::Less
        );
        assert_eq!(compare_label_names("Item 9", "Item 10"), Ordering::Less);
    }

    #[test]
    fn chips_leave_out_the_viewed_label_and_lead_with_muted() {
        let label = |id: &str, name: &str, kind| Label {
            id: id.into(),
            name: name.into(),
            kind,
            text_color: None,
            background_color: None,
            hidden: false,
        };
        let chips = Chips::new(&[
            label("L1", "hiring", exo_store::LabelKind::User),
            label("L2", "Clients", exo_store::LabelKind::User),
            label("INBOX", "INBOX", exo_store::LabelKind::System),
        ]);
        let ids = ["INBOX", "L1", "L2"].map(String::from);
        assert_eq!(
            chips.for_thread(&ids, false, &ListId::Inbox),
            ["Clients", "hiring"]
        );
        assert_eq!(
            chips.for_thread(&ids, true, &ListId::Label("L1".into())),
            ["Muted", "Clients"]
        );
        assert_eq!(
            chips.for_thread(&ids, true, &ListId::Muted),
            ["Clients", "hiring"]
        );
    }

    #[test]
    fn chips_show_hidden_labels_but_not_state_labels() {
        let label = |id: &str, name: &str, hidden| Label {
            id: id.into(),
            name: name.into(),
            kind: exo_store::LabelKind::User,
            text_color: None,
            background_color: None,
            hidden,
        };
        let chips = Chips::new(&[
            label("L1", "Input/Needs Action", true),
            label("L2", "[Superhuman]/Muted", true),
            label("L3", "[Exo]/Muted", true),
            label("L4", "Receipts", false),
        ]);
        let ids = ["L1", "L2", "L3", "L4"].map(String::from);
        assert_eq!(
            chips.for_thread(&ids, false, &ListId::Inbox),
            ["Input/Needs Action", "Receipts"]
        );
    }

    #[test]
    fn auto_labels_show_their_names_in_gmail_name_order() {
        let label = |id: &str, name: &str| Label {
            id: id.into(),
            name: name.into(),
            kind: exo_store::LabelKind::User,
            text_color: None,
            background_color: None,
            hidden: false,
        };
        // Chips sort by their Gmail names, so Auto Labels' bracketed ones come first.
        let chips = Chips::new(&[
            label("L1", "Receipts"),
            label("L2", "[Superhuman]/AI/Marketing"),
            label("L3", "Clients/Acme"),
            label("L4", "[News]"),
        ]);
        let ids = ["L1", "L2", "L3", "L4"].map(String::from);
        assert_eq!(
            chips.for_thread(&ids, false, &ListId::Inbox),
            ["News", "Marketing", "Clients/Acme", "Receipts"]
        );
        assert_eq!(chip_name("[2] Pitch").as_deref(), Some("Pitch"));
        assert_eq!(chip_name("[3. Travel]").as_deref(), Some("Travel"));
        assert_eq!(
            chip_name("[Clients]").as_deref(),
            Some("[Clients]"),
            "not an Auto Label's name"
        );
        assert_eq!(chip_name("[Superhuman]/Is Snoozed"), None);
        // Listed nowhere but on their chips.
        let labels = [
            label("L1", "Receipts"),
            label("L2", "[News]"),
            label("L3", "[Clients]"),
        ];
        let listed: Vec<&str> = crate::lists::listed_labels(&labels)
            .map(|label| label.name.as_str())
            .collect();
        assert_eq!(listed, ["Receipts", "[Clients]"]);
    }
}
