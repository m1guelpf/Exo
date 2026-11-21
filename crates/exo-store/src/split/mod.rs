//! Split Inbox: the split definitions as the account's settings store them, the rules for
//! naming and changing one, and the routing that puts inbox threads into the enabled ones.
//!
//! Splits are stored (`splitInboxesV2`, [`order`]) by external id, in one order that places
//! Important, Other and Inbox among them.

mod library;
mod order;

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::db::AttachmentRow;
use crate::ids::labels::{INBOX, SH_IMPORTANT, SH_OTHER};
use crate::view::ThreadView;

pub use library::{
    AutoLabels, LibrarySection, Preview, PreviewThread, Properties, QueryProperty, Section,
    Senders, SplitLibrary, Template,
};
pub(crate) use order::SplitInboxesV2;
pub use order::{SplitEntry, SystemSplit};

pub use crate::query::{Invalidity, LabelIndex, LabelRef, SplitQuery, label_slug};

/// The prefix of a split's id, which is also its list id (`SH_SPLIT_INBOX_`).
pub const SPLIT_ID_PREFIX: &str = "SH_SPLIT_INBOX_";

/// The default Calendar split's definition.
pub const CALENDAR_QUERY: &str = "from:{notifications@calendly.com, calendar-notification@google.com, calendar@superhuman.com} OR filename:ics";

/// A split as the settings store it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitInbox {
    /// `SH_SPLIT_INBOX_<ms><n>`: its list id. A new one is minted whenever the query, whether
    /// it's off, or whether it leaves threads in Important and Other changes.
    pub id: String,
    /// A stable id (`splitInbox_1…`), kept across edits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    /// Threads it takes also stay in Important and Other (or Inbox).
    #[serde(default)]
    pub leave_threads_in_important_other: bool,
    /// Its tab hides while it's empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hide_split_when_empty: Option<bool>,
    /// A built-in kind: `vip`, `team`, `news`, `calendar`, `shared` (or `starred`,
    /// `reminders`); none for the user's own.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Turned off.
    #[serde(default)]
    pub is_disabled: bool,
    /// Its name and definition.
    pub matcher: Matcher,
    /// The labels its definition names, as they were when it was defined.
    #[serde(default)]
    pub labels: Vec<LabelRef>,
    /// iOS notifications for it: Exo has no iOS app, and keeps it as stored.
    #[serde(rename = "notifyIOS", default)]
    pub notify_ios: bool,
    /// The library template it came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
}

/// A split's name and definition.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Matcher {
    /// The tab's name.
    pub name: String,
    /// The definition, in the search bar's syntax ([`SplitQuery`]).
    #[serde(default)]
    pub query: String,
    /// [`query`](Self::query) as a Gmail search, for backfilling the split.
    #[serde(default)]
    pub gmail_query: String,
}

/// Built-in kinds.
const DEFAULT_KINDS: [&str; 5] = ["news", "team", "vip", "calendar", "shared"];

/// What a New or Edit Split Inbox form asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SplitChanges {
    /// The name.
    pub name: Option<String>,
    /// The definition.
    pub query: Option<String>,
    /// Turned off.
    pub is_disabled: Option<bool>,
    /// Leaves its threads in Important and Other.
    pub leave_threads_in_important_other: Option<bool>,
    /// Hides its tab while empty.
    pub hide_split_when_empty: Option<bool>,
}

/// What an account knows when it defines a split: its labels, and its addresses for `me`.
#[derive(Debug, Clone, Copy)]
pub struct Definitions<'a> {
    /// The account's labels.
    pub labels: &'a [crate::Label],
    /// The account's addresses.
    pub me: &'a [String],
}

impl Definitions<'_> {
    fn index(&self) -> LabelIndex {
        LabelIndex::new(
            [],
            self.labels
                .iter()
                .map(|label| (label.id.as_str(), label.name.as_str())),
        )
    }
}

impl SplitInbox {
    /// A new split: a fresh id and external id, the definition compiled to Gmail and its labels
    /// recorded.
    pub fn new(
        name: &str,
        query: &str,
        kind: Option<&str>,
        template_id: Option<&str>,
        definitions: Definitions<'_>,
    ) -> Self {
        let mut split = Self {
            id: new_id(),
            external_id: Some(new_external_id()),
            leave_threads_in_important_other: false,
            hide_split_when_empty: None,
            kind: kind.map(str::to_owned),
            is_disabled: false,
            matcher: Matcher {
                name: name.to_owned(),
                ..Matcher::default()
            },
            labels: Vec::new(),
            notify_ios: false,
            template_id: template_id.map(str::to_owned),
        };
        split.define(query, definitions);
        split
    }

    /// The default splits, off: Starred, VIP, Team, Calendar and News. Team takes the account's
    /// `team_domain`, or none for a webmail address; Calendar takes [`CALENDAR_QUERY`] and its
    /// template.
    pub fn defaults(team_domain: Option<&str>, definitions: Definitions<'_>) -> Vec<Self> {
        let team = team_domain
            .map(|domain| format!("from:{domain} to:me"))
            .unwrap_or_default();
        [
            ("Starred", "is:starred", "starred", None, false),
            ("VIP", "", "vip", None, true),
            ("Team", team.as_str(), "team", None, true),
            (
                "Calendar",
                CALENDAR_QUERY,
                "calendar",
                Some("splitTemplate_calendar"),
                false,
            ),
            ("News", "", "news", None, false),
        ]
        .into_iter()
        .map(|(name, query, kind, template, leave)| Self {
            is_disabled: true,
            leave_threads_in_important_other: leave,
            ..Self::new(name, query, Some(kind), template, definitions)
        })
        .collect()
    }

    /// The split with `changes` made (`update`): a new id when the definition, whether it's
    /// off or where its threads show changes.
    #[must_use]
    pub fn update(&self, changes: &SplitChanges, definitions: Definitions<'_>) -> Self {
        let mut next = self.clone();
        if let Some(name) = changes.name.as_ref().filter(|name| !name.is_empty()) {
            next.matcher.name.clone_from(name);
        }
        if let Some(query) = changes.query.as_ref().filter(|query| !query.is_empty()) {
            next.matcher.query.clone_from(query);
        }
        if let Some(disabled) = changes.is_disabled {
            next.is_disabled = disabled;
        }
        if let Some(leave) = changes.leave_threads_in_important_other {
            next.leave_threads_in_important_other = leave;
        }
        if let Some(hide) = changes.hide_split_when_empty {
            next.hide_split_when_empty = Some(hide);
        }
        let query_changed = next.matcher.query.trim() != self.matcher.query.trim();
        if query_changed
            || next.is_disabled != self.is_disabled
            || next.leave_threads_in_important_other != self.leave_threads_in_important_other
        {
            next.id = new_id();
        }
        if query_changed {
            let query = std::mem::take(&mut next.matcher.query);
            next.define(&query, definitions);
        }
        next
    }

    /// Sets the definition, its Gmail search and the labels it names.
    fn define(&mut self, query: &str, definitions: Definitions<'_>) {
        let index = definitions.index();
        let parsed = SplitQuery::parse(query);
        self.labels = parsed.labels_used(&index);
        self.matcher.gmail_query = if query.trim().is_empty() {
            String::new()
        } else {
            parsed.to_gmail(&index, definitions.me)
        };
        self.matcher.query = query.to_owned();
    }

    /// Its name.
    pub fn name(&self) -> &str {
        &self.matcher.name
    }

    /// Its definition.
    pub fn query(&self) -> &str {
        &self.matcher.query
    }

    /// Its name as names are compared ([`slugify_name`]).
    pub fn slug(&self) -> String {
        slugify_name(&self.matcher.name)
    }

    /// Whether its tab hides while it's empty.
    pub fn hides_when_empty(&self) -> bool {
        self.hide_split_when_empty.unwrap_or(false)
    }

    /// Whether it's one of the built-in kinds.
    pub fn is_default(&self) -> bool {
        self.kind
            .as_deref()
            .is_some_and(|kind| DEFAULT_KINDS.contains(&kind))
    }

    /// Whether `name` is its name, as names are compared.
    pub fn has_same_name(&self, name: &str) -> bool {
        self.slug() == slugify_name(name)
    }

    /// Whether `query` is its definition but for case, `in:inbox` and how starred and
    /// invitations are said.
    pub fn has_equivalent_query(&self, query: &str) -> bool {
        fn sanitize(query: &str) -> String {
            let mut query = query.replace("in:inbox", "");
            for starred in ["is:flagged", "isflagged:true", "label:starred"] {
                query = query.replace(starred, "is:starred");
            }
            query
                .replace("kind:meetings", "filename:ics")
                .trim()
                .to_lowercase()
        }
        sanitize(query) == sanitize(&self.matcher.query)
    }
}

/// Whether `name` is one only the built-in splits have, which no split of the user's can take.
/// Calendar isn't one: a split with its definition may.
pub fn is_reserved_name(name: &str) -> bool {
    RESERVED_SLUGS[..RESERVED_SLUGS.len() - 1].contains(&slugify_name(name).as_str())
}

/// Whether `name` is Calendar's.
pub fn is_calendar_name(name: &str) -> bool {
    slugify_name(name) == "calendar"
}

/// `text` trimmed, each run of whitespace one space (an ideographic space ending the run is
/// kept).
pub fn clean_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run: Option<char> = None;
    for c in text.trim().chars() {
        if c.is_whitespace() {
            run = Some(c);
            continue;
        }
        if let Some(last) = run.take() {
            out.push(if last == '\u{3000}' { last } else { ' ' });
        }
        out.push(c);
    }
    out
}

/// A split name's slug: whitespace-cleaned, lower-cased, with whitespace, `:`, `/`, `.`, `@`
/// and `\` as `-`.
pub fn slugify_name(name: &str) -> String {
    clean_whitespace(name)
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_whitespace() || matches!(c, ':' | '/' | '.' | '@' | '\\') {
                '-'
            } else {
                c
            }
        })
        .collect()
}

/// The names kept for the built-in splits, Calendar's last ([`is_reserved_name`] leaves it out).
const RESERVED_SLUGS: [&str; 9] = [
    "inbox",
    "important",
    "other",
    "vip",
    "team",
    "news",
    "docs",
    "shared",
    "calendar",
];

/// The New/Edit form's name error for `name`, against `others` (every split but the one being
/// edited).
pub fn name_error(name: &str, others: &[SplitInbox]) -> Option<&'static str> {
    let name = clean_whitespace(name);
    if name.is_empty() {
        return Some("Please enter a name for this Split Inbox.");
    }
    let slug = slugify_name(&name);
    if RESERVED_SLUGS.contains(&slug.as_str())
        || others.iter().any(|split| split.has_same_name(&name))
    {
        return Some(
            "A Split Inbox with this name already exists. Please choose a different name.",
        );
    }
    None
}

/// The length of a split's search, URL-encoded, past which Gmail refuses it.
const MAX_GMAIL_QUERY: usize = 7800;

/// Whether saving `splits` (the account's, as they'd be) makes a Gmail search too long: the one
/// for `changed`, or Other's, which subtracts every split.
pub fn query_too_long(splits: &[SplitInbox], changed: &SplitInbox, me: &Me<'_>) -> bool {
    let encoded = |query: &str| {
        query
            .bytes()
            .map(|byte| {
                if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
                    1
                } else {
                    3
                }
            })
            .sum::<usize>()
    };
    let other = gmail_search(SH_OTHER, splits, me);
    let own = gmail_search(&changed.id, splits, me);
    encoded(&other) > MAX_GMAIL_QUERY || encoded(&own) > MAX_GMAIL_QUERY
}

/// The account's addresses and name, for the Gmail searches that build Important.
#[derive(Debug, Clone, Copy)]
pub struct Me<'a> {
    /// The account's addresses.
    pub addresses: &'a [String],
    /// Its display name.
    pub name: Option<&'a str>,
}

/// The Gmail search that lists `list` (Inbox, Important, Other or a split's id) when `splits` are
/// the account's (enabled or not).
pub fn gmail_search(list: &str, splits: &[SplitInbox], me: &Me<'_>) -> String {
    let enabled: Vec<&SplitInbox> = splits.iter().filter(|split| !split.is_disabled).collect();
    let remove = remove_splits(&enabled, me.addresses);
    let important = important_definition(me);
    match list {
        "SH_INBOX" | INBOX => format!("label:INBOX {remove}").trim().to_owned(),
        SH_IMPORTANT => format!("label:INBOX {important} {remove}")
            .trim()
            .to_owned(),
        SH_OTHER => format!("label:INBOX -{important} {remove}")
            .trim()
            .to_owned(),
        id => enabled
            .iter()
            .find(|split| split.id == id)
            .map_or_else(String::new, |split| {
                split_search(split, me.addresses, false)
            }),
    }
}

/// Important's definition as a Gmail query.
pub(crate) fn important_definition(me: &Me<'_>) -> String {
    let mut from: Vec<String> = me.addresses.to_vec();
    from.extend(me.name.map(|name| format!("\"{name}\"")));
    let from = if from.is_empty() {
        String::new()
    } else {
        format!(" OR from:({})", from.join(" OR "))
    };
    format!("((category:personal){from} OR from:\"via Google\")")
}

/// What Inbox, Important and Other leave out: what splits take.
fn remove_splits(enabled: &[&SplitInbox], me: &[String]) -> String {
    let taken: Vec<String> = enabled
        .iter()
        .filter(|split| !split.leave_threads_in_important_other && !split.matcher.query.is_empty())
        .map(|split| {
            let search = split_search(split, me, true);
            if search.contains(' ') {
                format!("({search})")
            } else {
                search
            }
        })
        .collect();
    if taken.is_empty() {
        String::new()
    } else {
        format!("-({})", taken.join(" OR "))
    }
}

/// The split's stored Gmail search, recompiled when the account has several addresses for `me`;
/// negated, without the draft and inbox parts.
fn split_search(split: &SplitInbox, me: &[String], negated: bool) -> String {
    let mut search = split.matcher.gmail_query.clone();
    if me.len() > 1 {
        let index = LabelIndex::new(split.labels.iter().cloned(), []);
        search = SplitQuery::parse(&split.matcher.query).to_gmail(&index, me);
        search = search
            .strip_prefix("-in:DRAFT -in:CHAT ")
            .map_or(search.clone(), str::to_owned);
    }
    if search.is_empty() {
        return match split.matcher.query.as_str() {
            "" => "label:INBOX -label:INBOX".to_owned(),
            query if negated => query.to_owned(),
            query => format!("label:INBOX {query}"),
        };
    }
    if negated {
        let search = search
            .strip_prefix("-in:DRAFT -in:CHAT ")
            .unwrap_or(&search);
        // Every `in:inbox `, whatever its case, goes.
        let mut out = String::with_capacity(search.len());
        let mut rest = search;
        while let Some(at) = rest.to_ascii_lowercase().find("in:inbox ") {
            out.push_str(&rest[..at]);
            rest = &rest[at + "in:inbox ".len()..];
        }
        out.push_str(rest);
        return out;
    }
    search
}

/// The account's splits as the store holds them: as defined, and the enabled ones ready to
/// route threads.
#[derive(Debug, Default)]
pub(crate) struct Splits {
    pub defined: Vec<SplitInbox>,
    pub routed: Vec<RoutedSplit>,
}

impl Splits {
    /// `splits` in an account whose labels are `account` (`(id, name)`).
    pub fn new<'a>(
        splits: Vec<SplitInbox>,
        account: impl IntoIterator<Item = (&'a str, &'a str)> + Clone,
    ) -> Self {
        let routed = RoutedSplit::enabled(&splits, account);
        Self {
            defined: splits,
            routed,
        }
    }

    /// The enabled splits' ids: when these change, every inbox thread routes again.
    pub fn routed_ids(&self) -> Vec<&str> {
        self.routed.iter().map(|split| split.id.as_str()).collect()
    }
}

/// An enabled split as the router uses it: its definition parsed, with the labels it can name.
#[derive(Debug, Clone)]
pub(crate) struct RoutedSplit {
    pub id: String,
    pub leave_threads_in_important_other: bool,
    query: SplitQuery,
    labels: LabelIndex,
}

impl RoutedSplit {
    /// The enabled ones of `splits`, for an account whose labels are `account`.
    pub fn enabled<'a>(
        splits: &[SplitInbox],
        account: impl IntoIterator<Item = (&'a str, &'a str)> + Clone,
    ) -> Vec<Self> {
        splits
            .iter()
            .filter(|split| !split.is_disabled)
            .map(|split| Self {
                id: split.id.clone(),
                leave_threads_in_important_other: split.leave_threads_in_important_other,
                query: SplitQuery::parse(&split.matcher.query),
                labels: LabelIndex::new(split.labels.iter().cloned(), account.clone()),
            })
            .collect()
    }

    pub fn is_match(
        &self,
        thread: &ThreadView,
        attachments: &[AttachmentRow],
        me: &[String],
    ) -> bool {
        self.query.is_match(thread, attachments, &self.labels, me)
    }
}

/// A new list id: `SH_SPLIT_INBOX_` + the time in milliseconds + a counter.
fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
    format!(
        "{SPLIT_ID_PREFIX}{}{n}",
        jiff::Timestamp::now().as_millisecond()
    )
}

/// A new external id of kind `splitInbox` (generation 1: six base-62 digits of seconds, a
/// four-digit shard key and seven of entropy). There's no user id to take the shard key from, so
/// it's random too.
fn new_external_id() -> String {
    const DIGITS: &[u8; 62] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut seconds = u64::try_from(jiff::Timestamp::now().as_second()).unwrap_or_default();
    let mut time = [b'0'; 6];
    for digit in time.iter_mut().rev() {
        *digit = DIGITS[usize::try_from(seconds % 62).unwrap_or_default()];
        seconds /= 62;
    }
    let mut random = [0u8; 11];
    getrandom::fill(&mut random).expect("the system has a source of randomness");
    let random: String = random
        .iter()
        .map(|byte| DIGITS[usize::from(*byte) % 62] as char)
        .collect();
    format!("splitInbox_1{}{random}", String::from_utf8_lossy(&time))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definitions() -> Definitions<'static> {
        Definitions {
            labels: &[],
            me: &[],
        }
    }

    #[test]
    fn splits_keep_their_stored_shape() {
        let json = serde_json::json!({
            "id": "SH_SPLIT_INBOX_16575585288602",
            "matcher": { "name": "Team", "query": "from:brightmoor.co to:me", "gmailQuery": "-in:DRAFT -in:CHAT in:inbox from:\"brightmoor.co\" to:{maya@brightmoor.co}" },
            "isDisabled": true,
            "notifyIOS": false,
            "type": "team",
            "leaveThreadsInImportantOther": true,
            "labels": [{ "id": "INBOX", "name": "Inbox" }]
        });
        let split: SplitInbox = serde_json::from_value(json.clone()).expect("parses");
        assert_eq!(split.name(), "Team");
        assert!(split.is_default());
        assert_eq!(serde_json::to_value(&split).expect("serializes"), json);
    }

    #[test]
    fn editing_mints_a_new_id_only_when_routing_changes() {
        let split = SplitInbox::new("GitHub", "from:github.com", None, None, definitions());
        assert!(split.id.starts_with(SPLIT_ID_PREFIX));
        assert!(split.external_id.as_deref().is_some_and(
            |id| id.starts_with("splitInbox_1") && id.len() == "splitInbox_1".len() + 17
        ));
        assert_eq!(
            split.matcher.gmail_query,
            "-in:DRAFT -in:CHAT in:inbox from:\"github.com\""
        );
        let renamed = split.update(
            &SplitChanges {
                name: Some("Code".into()),
                hide_split_when_empty: Some(true),
                ..SplitChanges::default()
            },
            definitions(),
        );
        assert_eq!(
            (
                renamed.id.as_str(),
                renamed.name(),
                renamed.hides_when_empty()
            ),
            (split.id.as_str(), "Code", true)
        );
        let requery = split.update(
            &SplitChanges {
                query: Some("from:gitlab.com".into()),
                ..SplitChanges::default()
            },
            definitions(),
        );
        assert_ne!(requery.id, split.id);
        assert_eq!(requery.external_id, split.external_id);
        assert_eq!(
            requery.matcher.gmail_query,
            "-in:DRAFT -in:CHAT in:inbox from:\"gitlab.com\""
        );
        let off = split.update(
            &SplitChanges {
                is_disabled: Some(true),
                ..SplitChanges::default()
            },
            definitions(),
        );
        assert_ne!(off.id, split.id);
    }

    #[test]
    fn names_must_be_new_and_not_built_in() {
        let existing = [SplitInbox::new(
            "Git Hub",
            "from:github.com",
            None,
            None,
            definitions(),
        )];
        assert_eq!(
            name_error("  ", &existing),
            Some("Please enter a name for this Split Inbox.")
        );
        assert!(name_error("git  hub", &existing).is_some());
        assert!(
            name_error("Calendar", &[]).is_some(),
            "built-in names are taken"
        );
        assert!(name_error("VIP", &[]).is_some());
        assert_eq!(name_error("GitHub", &existing), None);
        assert_eq!(slugify_name(" A.B  c@d "), "a-b-c-d");
    }

    #[test]
    fn inbox_lists_leave_out_what_splits_take() {
        let me = Me {
            addresses: &["maya@brightmoor.co".to_owned()],
            name: Some("Maya"),
        };
        let github = SplitInbox::new("GitHub", "from:github.com", None, None, definitions());
        let mut team = SplitInbox::new(
            "Team",
            "from:brightmoor.co to:me",
            None,
            None,
            Definitions {
                labels: &[],
                me: &["maya@brightmoor.co".to_owned()],
            },
        );
        team.leave_threads_in_important_other = true;
        let empty = SplitInbox::new("Empty", "", None, None, definitions());
        let splits = [github.clone(), team, empty.clone()];
        assert_eq!(
            gmail_search(INBOX, &splits, &me),
            "label:INBOX -(from:\"github.com\")"
        );
        assert_eq!(
            gmail_search(SH_OTHER, &splits, &me),
            "label:INBOX -((category:personal) OR from:(maya@brightmoor.co OR \"Maya\") OR from:\"via Google\") -(from:\"github.com\")"
        );
        assert_eq!(
            gmail_search(&github.id, &splits, &me),
            "-in:DRAFT -in:CHAT in:inbox from:\"github.com\""
        );
        assert_eq!(
            gmail_search(&empty.id, &splits, &me),
            "label:INBOX -label:INBOX"
        );
        assert_eq!(gmail_search(INBOX, &[], &me), "label:INBOX");
    }

    #[test]
    fn whitespace_runs_become_one_space() {
        assert_eq!(clean_whitespace("  a \n\t b  "), "a b");
        assert_eq!(clean_whitespace("a \u{3000}b"), "a\u{3000}b");
    }
}
