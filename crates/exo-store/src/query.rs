//! The search query language, as the search bar and Split Inbox read it: the parse tree, the
//! Gmail query and the local match.
//!
//! [`SplitQuery`] is a split's definition: constrained to the inbox, with the validation the New
//! and Edit dialogs run; free text never matches a split (it can't be matched locally, which the
//! router treats as no match).
//! [`SearchQuery`] is what the search bar runs; the local index it's looked up in is
//! `crate::search`.
//!
//! A few of the parser's and matcher's rules look like slips but are deliberate: a negated
//! group's own exclusion check ignores its negation; `before:` flips `after:`; only some token
//! kinds may define a split.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::LazyLock;

use regex::Regex;

use crate::db::{AttachmentRow, Mailbox};
use crate::view::{ThreadView, ViewMessage};

/// What a token is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Attachment,
    Label,
    Filename,
    Header,
    After,
    Contact,
    Category,
    AutoLabel,
    Shared,
    Text,
}

/// A leaf of the parse tree.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    kind: Kind,
    /// `from`, `subject`, `in`, `is`, `label`…; empty for the kinds without one.
    field: String,
    /// The value with its double quotes removed; `None` for `has:attachment` and `is:shared`,
    /// which carry none.
    value: Option<String>,
    quoted: bool,
    negated: bool,
    /// The token as typed (`str`), which the "can't split on" alert quotes.
    text: String,
}

impl Token {
    fn value(&self) -> &str {
        self.value.as_deref().unwrap_or_default()
    }

    /// The empty text token: what `in:inbox` becomes, and what an empty group simplifies to.
    fn empty() -> Self {
        Self {
            kind: Kind::Text,
            field: String::new(),
            value: Some(String::new()),
            quoted: false,
            negated: false,
            text: String::new(),
        }
    }

    fn is_empty_text(&self) -> bool {
        self.kind == Kind::Text && self.value.as_deref() == Some("")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    And(Vec<Node>, bool),
    Or(Vec<Node>, bool),
    Leaf(Token),
}

impl Node {
    fn negated(&self) -> bool {
        match self {
            Self::And(_, negated) | Self::Or(_, negated) => *negated,
            Self::Leaf(token) => token.negated,
        }
    }

    fn negate(&mut self) {
        match self {
            Self::And(_, negated) | Self::Or(_, negated) => *negated = !*negated,
            Self::Leaf(token) => token.negated = !token.negated,
        }
    }
}

/// The Gmail labels a query can name, by label slug ([`label_slug`]): a split's own `labels`
/// first, then the account's, then the built-in lists, Inbox, Starred and the Primary category,
/// each under the slug of its name and of its id; later entries win. Exo's own lists (`Done`,
/// `Reminders`…) are among the built-in ones, so `important`, `done` and the like resolve to
/// them.
#[derive(Debug, Clone, Default)]
pub struct LabelIndex {
    by_slug: HashMap<String, LabelRef>,
}

/// A label a query names, as a split keeps it (`labels`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LabelRef {
    /// Gmail's id (or an `SH_…` id, for Exo's own lists).
    pub id: String,
    /// Its name.
    pub name: String,
}

/// The lists every account has, Exo's own and Gmail's system labels, by id and name.
const BUILTIN_LABELS: [(&str, &str); 17] = [
    ("SH_IMPORTANT", "Important"),
    ("SH_OTHER", "Other"),
    ("SH_ALL", "All Mail"),
    ("SH_ARCHIVED", "Done"),
    ("SH_SCHEDULED", "Scheduled"),
    ("SH_SNOOZED", "Reminders"),
    ("SH_MUTED", "Muted"),
    ("SH_SNIPPETS", "Snippets"),
    ("SH_KNOWLEDGE_BASE", "Knowledge Base"),
    ("SH_SIGNATURES", "Signatures"),
    ("SENT", "Sent"),
    ("STARRED", "Starred"),
    ("UNREAD", "Unread"),
    ("DRAFT", "Drafts"),
    ("INBOX", "Inbox"),
    ("TRASH", "Trash"),
    ("SPAM", "Spam"),
];

impl LabelIndex {
    /// The index for a split carrying `own` labels, in an account with `account` labels
    /// (`(id, name)`, Gmail's).
    pub fn new<'a>(
        own: impl IntoIterator<Item = LabelRef>,
        account: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Self {
        let mut index = Self::default();
        let builtin = BUILTIN_LABELS.into_iter().chain([
            ("INBOX", "Inbox"),
            ("STARRED", "Starred"),
            ("CATEGORY_PERSONAL", "category_personal"),
        ]);
        let labels =
            own.into_iter().chain(
                account
                    .into_iter()
                    .chain(builtin)
                    .map(|(id, name)| LabelRef {
                        id: id.to_owned(),
                        name: name.to_owned(),
                    }),
            );
        for label in labels {
            index.by_slug.insert(label_slug(&label.name), label.clone());
            index.by_slug.insert(label_slug(&label.id), label);
        }
        for (alias, target) in [
            ("important", "sh_important"),
            ("other", "sh_other"),
            ("all", "sh_all"),
            ("archived", "sh_archived"),
            ("done", "sh_archived"),
            ("snoozed", "sh_snoozed"),
            ("reminders", "sh_snoozed"),
            ("muted", "sh_muted"),
            ("snippets", "sh_snippets"),
        ] {
            match index.by_slug.get(target).cloned() {
                Some(label) => index.by_slug.insert(alias.to_owned(), label),
                None => index.by_slug.remove(alias),
            };
        }
        index
    }

    fn get(&self, value: &str) -> Option<&LabelRef> {
        self.by_slug.get(&label_slug(value))
    }

    /// The label's id, or the value itself when no label has that name.
    fn id<'a>(&'a self, value: &'a str) -> &'a str {
        self.get(value).map_or(value, |label| label.id.as_str())
    }
}

/// A label's slug: quotes dropped, `&`, parentheses and whitespace to `-`, lower-cased.
pub fn label_slug(name: &str) -> String {
    name.chars()
        .filter(|&c| c != '"')
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

/// Why a query can't define a split: what the alert says, and the offending part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalidity {
    /// The alert's text (title `Sorry!`), lines separated by `\n`, indentation included.
    pub text: String,
    /// The tokens or label names at fault.
    pub problem: String,
}

/// A parsed split query.
#[derive(Debug, Clone)]
pub struct SplitQuery {
    raw: String,
    tree: Node,
}

impl SplitQuery {
    /// Parses `raw` as a split's definition: everything is constrained to `in:inbox`, and a
    /// bare address means `from:` or `to:` it.
    pub fn parse(raw: &str) -> Self {
        let mut tree = parse_tree(raw);
        if let Node::And(children, _) | Node::Or(children, _) = &mut tree {
            let mut constrained: Vec<Node> = children.drain(..).map(constrain_to_inbox).collect();
            constrained.insert(0, Node::Leaf(parse_leaf("in:inbox", false)));
            *children = constrained;
        }
        convert_email_addresses(&mut tree);
        Self {
            raw: raw.to_owned(),
            tree: simplify(tree),
        }
    }

    /// The query as typed.
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Why this query can't define a split, if it can't.
    pub fn invalidity(&self, labels: &LabelIndex) -> Option<Invalidity> {
        static AUTO_LABEL: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?i)autolabel:[a-zA-Z0-9_]*").expect("valid regex"));
        static QUOTED_SUBJECT: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#"subject:"(.)*"(.)*""#).expect("valid regex"));
        let auto_labels: Vec<&str> = AUTO_LABEL
            .find_iter(&self.raw)
            .map(|found| found.as_str())
            .collect();
        if !auto_labels.is_empty() {
            return Some(Invalidity {
                text: "We can't split your inbox with Auto Labels within the text definition.\n\n\n               Please use the Auto Labels section to apply an Auto Label for splitting.".into(),
                problem: auto_labels.join(","),
            });
        }
        let mut invalid = Vec::new();
        invalid_tokens(&self.tree, &mut invalid, true);
        let missing = missing_labels(&self.tree, labels);
        if (self.raw.is_empty() || invalid.is_empty()) && missing.is_empty() {
            return None;
        }
        if let [label] = missing.as_slice() {
            return Some(Invalidity {
                text: format!(
                    "We can't split your inbox on labels that don't exist.\n\n            Please create label {label} first or try splitting on a different label."
                ),
                problem: label.clone(),
            });
        }
        if !missing.is_empty() {
            return Some(Invalidity {
                text: "We can't split your inbox on labels that don't exist.\n\n            Please create labels first or try splitting on a different label.".into(),
                problem: missing.join(","),
            });
        }
        if invalid.iter().all(|token| token.kind == Kind::Text) {
            if QUOTED_SUBJECT.is_match(&self.raw) {
                return Some(Invalidity {
                    text: "We can't split your inbox on subjects with quotes.\n\n              Please remove this and try again.".into(),
                    problem: String::new(),
                });
            }
            let text = invalid
                .iter()
                .map(|token| token.value())
                .collect::<Vec<_>>()
                .join(" ");
            return Some(Invalidity {
                text: format!(
                    "We can't split your inbox on text searches.\n\n            Please try subject:\"{text}\", or from:\"{text}\"."
                ),
                problem: text,
            });
        }
        let token = invalid.iter().find(|token| token.kind != Kind::Text)?;
        Some(Invalidity {
            text: format!(
                "We can't split your inbox on {}\n\n            Please remove this and try again.",
                token.text
            ),
            problem: token.text.clone(),
        })
    }

    /// The labels the query names, which a split keeps with itself.
    pub fn labels_used(&self, labels: &LabelIndex) -> Vec<LabelRef> {
        let mut tokens = Vec::new();
        label_tokens(&self.tree, &mut tokens);
        tokens
            .iter()
            .filter_map(|token| labels.get(token.value()).cloned())
            .collect()
    }

    /// The Gmail search for this split, drafts and chats excluded, with `me` spelled out as `me`
    /// (the account's addresses).
    pub fn to_gmail(&self, labels: &LabelIndex, me: &[String]) -> String {
        let body = gmail_query(&self.tree, true, labels, me, None);
        format!("-in:DRAFT -in:CHAT {body}").trim().to_owned()
    }

    /// Whether `thread` belongs in a split with this definition. `attachments` holds at least the
    /// thread's attachment rows.
    pub(crate) fn is_match(
        &self,
        thread: &ThreadView,
        attachments: &[AttachmentRow],
        labels: &LabelIndex,
        me: &[String],
    ) -> bool {
        if self.raw.is_empty() {
            return false;
        }
        let matcher = Matcher::split(thread, attachments, labels, me);
        if matcher.excludes(&self.tree) {
            return false;
        }
        matcher
            .find(&self.tree)
            .is_some_and(|found| !found.is_empty())
    }
}

/// Words the index's porter stemming matches other words for (`w1`, `wedding` for `wed`), so
/// local results are checked for them as typed.
const STEMMED: [&str; 4] = ["wedding", "news", "university", "universal"];

/// A phrase or word local results are checked for as typed.
#[derive(Debug, Clone)]
pub(crate) struct Recheck {
    /// The text as a whole word (where it starts and ends with one), ignoring case.
    pub regex: Regex,
    /// A `subject:` operator's: only the subject counts.
    pub subject_only: bool,
    /// Results must not have it.
    pub negated: bool,
}

/// A parsed search, as the search bar runs it.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    raw: String,
    tree: Node,
}

/// What a query's last token is, which autocomplete completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastToken {
    /// What it is.
    pub kind: TokenKind,
    /// Its operator, lower-cased (`from`, `in`, `is`, `label`…); empty for text.
    pub field: String,
    /// Its value as typed after the operator, without quotes; empty for `has:attachment`.
    pub value: String,
    /// The value was in double quotes.
    pub quoted: bool,
}

/// The kinds of token autocomplete tells apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// `in:`, `is:`, `label:`, `l:`.
    Label,
    /// `category:`.
    Category,
    /// `from:`, `to:`, `cc:`, `bcc:`, `replyto:`, `deliveredto:`.
    Contact,
    /// `filename:`, `attachment:`.
    Filename,
    /// Free text.
    Text,
    /// Anything else (`subject:`, `after:`, `has:attachment`…).
    Other,
}

impl SearchQuery {
    /// Parses `raw` as the search bar's query.
    pub fn parse(raw: &str) -> Self {
        Self {
            raw: raw.to_owned(),
            tree: simplify(parse_tree(raw)),
        }
    }

    /// The query as typed.
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Whether it's nothing but whitespace and punctuation, so nothing to search for.
    pub fn is_empty(&self) -> bool {
        self.raw
            .chars()
            .all(|c| c.is_whitespace() || "-!$%^&*()_+|~=`{}[]:\";'<>?,./\\@#".contains(c))
    }

    /// The Gmail search, drafts and chats excluded, with `me` spelled out as the account's
    /// addresses and `in:important` as `important`, the Important split's definition.
    pub fn to_gmail(&self, labels: &LabelIndex, me: &[String], important: &str) -> String {
        let body = gmail_query(&self.tree, false, labels, me, Some(important));
        format!("-in:DRAFT -in:CHAT {body}").trim().to_owned()
    }

    /// The words and subject phrases a result's subject and snippet show in bold.
    pub fn highlightable_tokens(&self) -> Vec<String> {
        fn walk(node: &Node, out: &mut Vec<String>) {
            match node {
                Node::And(children, _) | Node::Or(children, _) => {
                    children.iter().for_each(|child| walk(child, out))
                }
                Node::Leaf(token)
                    if (token.kind == Kind::Text
                        || (token.kind == Kind::Header && token.field == "subject"))
                        && !token.value().is_empty() =>
                {
                    out.push(token.value().to_owned())
                }
                Node::Leaf(_) => {}
            }
        }
        let mut out = Vec::new();
        walk(&self.tree, &mut out);
        out
    }

    /// What local results are checked for again: the quoted phrases and the words the index's
    /// stemming matches too loosely ([`STEMMED`]), among the words and subject operators. With
    /// `all`, they are every word there is, and results' highlights are redone with them.
    pub(crate) fn rechecks(&self) -> (Vec<Recheck>, bool) {
        fn walk<'a>(node: &'a Node, out: &mut Vec<&'a Token>) {
            match node {
                Node::And(children, _) | Node::Or(children, _) => {
                    children.iter().for_each(|child| walk(child, out))
                }
                Node::Leaf(token)
                    if (token.kind == Kind::Text
                        || (token.kind == Kind::Header && token.field == "subject"))
                        && !token.value().is_empty() =>
                {
                    out.push(token)
                }
                Node::Leaf(_) => {}
            }
        }
        static PUNCTUATION: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"[~!@#$%^*()+={}\\:;'<>,.?]").expect("valid regex"));
        let mut words = Vec::new();
        walk(&self.tree, &mut words);
        let rechecks: Vec<Recheck> = words
            .iter()
            .filter(|token| {
                (token.quoted && token.text.matches(['"', '\'']).count() >= 2)
                    || STEMMED.contains(&token.value().to_lowercase().as_str())
            })
            .filter_map(|token| {
                let term = PUNCTUATION
                    .replace_all(&token.value().to_lowercase(), "")
                    .into_owned();
                let edge = |c: Option<char>| {
                    if c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                        r"(?-u:\b)"
                    } else {
                        ""
                    }
                };
                let pattern = format!(
                    "(?i){}{}{}",
                    edge(term.chars().next()),
                    regex::escape(&term),
                    edge(term.chars().last())
                );
                Some(Recheck {
                    regex: Regex::new(&pattern).ok()?,
                    subject_only: token.kind == Kind::Header,
                    negated: token.negated,
                })
            })
            .collect();
        let all = !rechecks.is_empty() && rechecks.len() == words.len();
        (rechecks, all)
    }

    /// The values of address operators and words, which results' names show highlighted where
    /// they match.
    pub fn highlightable_contact_tokens(&self) -> Vec<String> {
        fn walk(node: &Node, out: &mut Vec<String>) {
            match node {
                Node::And(children, _) | Node::Or(children, _) => {
                    children.iter().for_each(|child| walk(child, out))
                }
                Node::Leaf(token)
                    if matches!(token.kind, Kind::Contact | Kind::Text)
                        && !token.value().is_empty() =>
                {
                    out.push(token.value().to_owned())
                }
                Node::Leaf(_) => {}
            }
        }
        let mut out = Vec::new();
        walk(&self.tree, &mut out);
        out
    }

    /// The last token, which autocomplete completes; `None` for an empty query.
    pub fn last_token(&self) -> Option<LastToken> {
        let mut node = &self.tree;
        while let Node::And(children, _) | Node::Or(children, _) = node {
            node = children.last()?;
        }
        let Node::Leaf(token) = node else { return None };
        if token.is_empty_text() {
            return None;
        }
        let kind = match token.kind {
            Kind::Label => TokenKind::Label,
            Kind::Category => TokenKind::Category,
            Kind::Contact => TokenKind::Contact,
            Kind::Filename => TokenKind::Filename,
            Kind::Text => TokenKind::Text,
            _ => TokenKind::Other,
        };
        Some(LastToken {
            kind,
            field: token.field.clone(),
            value: token.value().to_owned(),
            quoted: token.quoted,
        })
    }

    /// Why a bulk action can't run on this search's results, as the alert says it: a word,
    /// which Gmail and Exo match differently, or anything else Gmail can't search for as Exo
    /// means it (Exo's own lists, a label that doesn't exist).
    pub fn bulk_refusal(&self, labels: &LabelIndex) -> Option<String> {
        let mut invalid = Vec::new();
        invalid_tokens(&self.tree, &mut invalid, false);
        if invalid.is_empty() && missing_labels(&self.tree, labels).is_empty() {
            return None;
        }
        Some(match invalid.iter().find(|token| token.kind == Kind::Text) {
            Some(word) => {
                // At most 20 characters: whitespace collapsed, cut at a word.
                let value = word.value().split_whitespace().collect::<Vec<_>>().join(" ");
                let cut = if value.chars().count() <= 20 {
                    value
                } else {
                    let head: String = value.chars().take(20).collect();
                    head.rfind(' ').map_or(head.clone(), |space| head[..space].to_owned())
                };
                format!("We can't complete this action on text searches.\n\nPlease try subject:\"{cut}\" or from:\"{cut}\".")
            }
            None => "We can't complete this action on this set of conversations.\n\nPlease unselect these conversations and try again.".into(),
        })
    }

    /// The label ids the query asks for: a result that loses one leaves the results.
    pub(crate) fn included_labels(&self, labels: &LabelIndex) -> Vec<String> {
        let mut tokens = Vec::new();
        label_tokens(&self.tree, &mut tokens);
        tokens
            .iter()
            .filter(|token| !token.negated)
            .map(|token| labels.id(token.value()).to_owned())
            .collect()
    }

    /// Whether the query asks for the label with this slug (`in:trash`, `in:sent`).
    pub fn shows_label(&self, slug: &str) -> bool {
        let mut tokens = Vec::new();
        label_tokens(&self.tree, &mut tokens);
        tokens
            .iter()
            .any(|token| !token.negated && token.value().eq_ignore_ascii_case(slug))
    }

    /// Whether the query has a token that only the thread itself can confirm (a contact), so
    /// index hits are checked with [`is_match`](Self::is_match).
    pub(crate) fn checks_contacts(&self) -> bool {
        fn walk(node: &Node) -> bool {
            match node {
                Node::And(children, _) | Node::Or(children, _) => children.iter().any(walk),
                Node::Leaf(token) => matches!(token.kind, Kind::Contact | Kind::Shared),
            }
        }
        walk(&self.tree)
    }

    /// A quick test that rules `thread` out (a Gmail hit that no longer matches).
    pub(crate) fn excludes(
        &self,
        thread: &ThreadView,
        attachments: &[AttachmentRow],
        labels: &LabelIndex,
        me: &[String],
    ) -> bool {
        Matcher::search(thread, attachments, labels, me).excludes(&self.tree)
    }

    /// Whether `thread` matches, at thread level: `None` when the query has a token only the
    /// index can answer (text, headers).
    pub(crate) fn is_match(
        &self,
        thread: &ThreadView,
        attachments: &[AttachmentRow],
        labels: &LabelIndex,
        me: &[String],
    ) -> Option<bool> {
        let matcher = Matcher::search(thread, attachments, labels, me);
        if matcher.excludes(&self.tree) {
            return Some(false);
        }
        matcher.find(&self.tree).map(|found| !found.is_empty())
    }

    /// The local index lookup for this query. It reads every token as if they were all joined by
    /// AND (an `OR` query only finds threads matching both sides locally; Gmail's results add the
    /// rest), and leaves out Trash, Spam and Snippets unless the query names them or `in:all`.
    pub(crate) fn fts(&self, labels: &LabelIndex, me: &[String]) -> Fts {
        let mut tokens: Vec<&Token> = Vec::new();
        let (mut anywhere, mut trash, mut spam, mut snippets) = (false, false, false, false);
        fn leaves<'a>(node: &'a Node, out: &mut Vec<&'a Token>) {
            match node {
                Node::And(children, _) | Node::Or(children, _) => {
                    children.iter().for_each(|child| leaves(child, out))
                }
                Node::Leaf(token) if token.value.as_deref() == Some("") => {}
                Node::Leaf(token) => out.push(token),
            }
        }
        let mut all = Vec::new();
        leaves(&self.tree, &mut all);
        for token in all {
            if token.kind == Kind::Label {
                match token.value().to_lowercase().as_str() {
                    "all" | "anywhere" => {
                        anywhere = true;
                        continue;
                    }
                    "trash" => trash = true,
                    "spam" => spam = true,
                    "snippets" => snippets = true,
                    _ => {}
                }
            }
            tokens.push(token);
        }
        let defaults: Vec<Token> = [
            ("trash", !anywhere && !trash),
            ("spam", !anywhere && !spam),
            ("snippets", !snippets),
        ]
        .into_iter()
        .filter(|(_, add)| *add)
        .map(|(label, _)| Token {
            kind: Kind::Label,
            field: "label".into(),
            value: Some(label.into()),
            quoted: false,
            negated: true,
            text: format!("label:{label}"),
        })
        .collect();
        let mut fts = Fts::default();
        let mut terms: Vec<Term> = Vec::new();
        for token in tokens.into_iter().chain(&defaults) {
            fts_token(token, labels, me, &mut terms, &mut fts);
        }
        fts.matches.insert(0, (false, full_table_match(terms)));
        fts
    }
}

/// The local index lookup for a [`SearchQuery`]: FTS5 expressions over the whole table (all
/// must hold, or for a negated one, not hold) and bounds on the thread's date.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Fts {
    /// `(negated, expression)`; the first is never negated.
    pub matches: Vec<(bool, String)>,
    /// `(before, epoch ms)`: `sort < ms` when `before`, else `sort >= ms`.
    pub dates: Vec<(bool, i64)>,
}

/// A term of the full-table expression.
struct Term {
    negated: bool,
    group: bool,
    text: String,
}

/// The index's anchor term, which every row has: what an all-negative expression starts from.
pub(crate) const FTS_ANCHOR: &str = "thread";

/// The terms joined by `AND`/`NOT`, a positive one first.
fn full_table_match(mut terms: Vec<Term>) -> String {
    match terms.iter().position(|term| !term.negated) {
        Some(0) => {}
        Some(at) => {
            let first = terms.remove(at);
            terms.insert(0, first);
        }
        None => terms.insert(
            0,
            Term {
                negated: false,
                group: false,
                text: format!("meta:{FTS_ANCHOR}"),
            },
        ),
    }
    let many = terms.len() > 1;
    let mut out = String::new();
    for term in terms {
        let text = if term.group && many {
            format!("({})", term.text)
        } else {
            term.text
        };
        if out.is_empty() {
            out = text;
        } else {
            out = format!("{out} {} {text}", if term.negated { "NOT" } else { "AND" });
        }
    }
    out
}

/// One token's part of the lookup.
fn fts_token(
    token: &Token,
    labels: &LabelIndex,
    me: &[String],
    terms: &mut Vec<Term>,
    fts: &mut Fts,
) {
    let negated = token.negated;
    let column = |column: &str, value: &str| Term {
        negated,
        group: false,
        text: column_term(column, value),
    };
    match token.kind {
        Kind::Text if token.quoted && negated => fts
            .matches
            .push((true, fts5_prefix_or_phrase(token.value()))),
        Kind::Text if token.quoted => terms.push(Term {
            negated: false,
            group: false,
            text: fts5_prefix_or_phrase(token.value()),
        }),
        Kind::Text => terms.push(Term {
            negated,
            group: false,
            text: fts5_value(token.value()),
        }),
        Kind::Attachment => terms.push(column("meta", "attachment")),
        Kind::Shared => terms.push(column("meta", "shared")),
        Kind::Filename => column_or_phrase("attachments", token, terms, fts),
        Kind::Header => column_or_phrase(fts_column(&token.field), token, terms, fts),
        Kind::Contact => {
            let fields: &[&str] = if token.field == "to" {
                &["to", "cc", "bcc"]
            } else {
                &[fts_column(&token.field)]
            };
            let values: Option<Vec<String>> = if token.value().eq_ignore_ascii_case("me")
                && !token.quoted
            {
                Some(me.iter().map(|address| tokenize_email(address)).collect())
            } else if EMAIL.is_match(token.value()) {
                Some(vec![tokenize_email(token.value())])
            } else if !token.quoted || token.value().chars().all(|c| c.is_ascii_alphanumeric()) {
                Some(vec![token.value().to_owned()])
            } else {
                None
            };
            let Some(values) = values else {
                return column_or_phrase(fts_column(&token.field), token, terms, fts);
            };
            let each: Vec<String> = values
                .iter()
                .flat_map(|value| fields.iter().map(move |field| column_term(field, value)))
                .collect();
            if negated {
                terms.extend(each.into_iter().map(|text| Term {
                    negated: true,
                    group: false,
                    text,
                }));
            } else if each.len() > 1 {
                terms.push(Term {
                    negated: false,
                    group: true,
                    text: each.join(" OR "),
                });
            } else if let Some(text) = each.into_iter().next() {
                terms.push(Term {
                    negated: false,
                    group: false,
                    text,
                });
            }
        }
        Kind::After => fts.dates.push(match parse_date(token.value()) {
            Some(date) => (negated, date.div_euclid(1000) * 1000),
            None => (true, 0),
        }),
        Kind::Label => terms.push(column("labels", labels.id(token.value()))),
        Kind::Category => terms.push(column("labels", labels.id(category_label(token).value()))),
        // Auto Labels, which Exo hasn't got: none to look up.
        Kind::AutoLabel => {}
    }
}

/// An address the way the index stores it whole: every non-letter replaced by the token
/// character, so the address is one token.
pub(crate) fn tokenize_email(email: &str) -> String {
    email
        .chars()
        .map(|c| {
            if c.is_ascii_alphabetic() {
                c
            } else {
                EMAIL_TOKENIZER
            }
        })
        .collect()
}

/// U+F8F7, a private-use character the index's tokenizer keeps inside a token.
pub(crate) const EMAIL_TOKENIZER: char = '\u{f8f7}';

/// An address anywhere in the value.
static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[^\s<@]+@[a-zA-Z0-9-]+(?:\.[a-zA-Z0-9-]+)+").expect("valid regex")
});

/// The index column for a header or contact field; the parser only produces these.
fn fts_column(field: &str) -> &'static str {
    match field {
        "subject" => "subject",
        "list" => "list",
        "rfc822msgid" => "rfc822msgid",
        "from" => "from",
        "cc" => "cc",
        "bcc" => "bcc",
        "replyto" => "replyto",
        "deliveredto" => "deliveredto",
        _ => "to",
    }
}

/// A quoted value is a phrase looked up on its own; any other is a column term.
fn column_or_phrase(column: &str, token: &Token, terms: &mut Vec<Term>, fts: &mut Fts) {
    if token.quoted {
        fts.matches.push((
            token.negated,
            format!("{column} : {}", fts5_prefix_or_phrase(token.value())),
        ));
    } else {
        terms.push(Term {
            negated: token.negated,
            group: false,
            text: column_term(column, token.value()),
        });
    }
}

/// `column:value`, the value as [`fts5_value`] writes it.
fn column_term(column: &str, value: &str) -> String {
    format!("{column}:{}", fts5_value(value))
}

/// A bare term when FTS5 reads it as one, else a quoted phrase. User text only ever reaches FTS5
/// through here or [`fts5_phrase`].
fn fts5_value(value: &str) -> String {
    let bare = value.strip_suffix('*').unwrap_or(value);
    let plain = !bare.is_empty()
        && bare
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == EMAIL_TOKENIZER);
    let keyword = ["AND", "OR", "NOT", "NEAR"]
        .iter()
        .any(|keyword| value.eq_ignore_ascii_case(keyword));
    if plain && !keyword {
        value.to_owned()
    } else {
        fts5_phrase(value)
    }
}

/// `"word"*` for a trailing star, else a phrase.
fn fts5_prefix_or_phrase(value: &str) -> String {
    match value.strip_suffix('*') {
        Some(head) if !head.ends_with('*') && !head.contains(['*', '"']) && !head.is_empty() => {
            format!("{}*", fts5_phrase(head))
        }
        _ => fts5_phrase(value),
    }
}

/// An FTS5 string literal, double quotes doubled.
fn fts5_phrase(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

/// The tree of `raw`, before a split constrains it or anything simplifies.
fn parse_tree(raw: &str) -> Node {
    let mut tokens: VecDeque<String> = TOKEN
        .find_iter(raw)
        .map(|token| token.as_str().to_owned())
        .collect();
    parse_more(&mut tokens, Mode::And, "", true).unwrap_or_else(|| Node::And(Vec::new(), false))
}

/// The tokens of a query: `- ( ) { } !`, a quoted phrase, a `field:` prefix or a word.
static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"[-(){}!]|"[^"]*(?:"|$)|[a-z0-9]+:|[^\s,;"\-(){}!][^\s,;"(){}]*"#)
        .expect("valid regex")
});

/// Makes a group node from its children and negation (`Node::And` or `Node::Or`).
type Group = fn(Vec<Node>, bool) -> Node;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    And,
    Or,
    /// One node, for the operand after `OR`/`AND`.
    Single,
}

/// Parses `tokens` into a group of `mode` (or one node, for [`Mode::Single`]) up to its closing
/// bracket, which the top level ignores. A value without a `field:` of its own takes
/// `parent_prefix`.
fn parse_more(
    tokens: &mut VecDeque<String>,
    mode: Mode,
    parent_prefix: &str,
    top: bool,
) -> Option<Node> {
    let mut negated = false;
    let mut prefix = String::new();
    let mut children: Vec<Node> = Vec::new();
    while let Some(token) = tokens.pop_front() {
        if token == "-" || token == "!" || token.eq_ignore_ascii_case("not") {
            negated = !negated;
        } else if token.ends_with(':') {
            prefix.push_str(&token);
        } else if token == ")" || token == "}" {
            if !top {
                break;
            }
        } else if token.eq_ignore_ascii_case("or") {
            if mode == Mode::And && !children.is_empty() {
                if !matches!(children.last(), Some(Node::Or(..))) {
                    let last = children.pop().expect("children isn't empty");
                    children.push(Node::Or(vec![last], negated));
                }
                if let Some(next) = parse_more(tokens, Mode::Single, parent_prefix, false)
                    && let Some(Node::Or(group, _)) = children.last_mut()
                {
                    group.push(next);
                }
            }
        } else if token.eq_ignore_ascii_case("and") {
            if mode == Mode::Or && !children.is_empty() {
                if !matches!(children.last(), Some(Node::And(..))) {
                    children = vec![Node::And(
                        vec![Node::Or(std::mem::take(&mut children), false)],
                        false,
                    )];
                }
                if let Some(next) = parse_more(tokens, Mode::Single, parent_prefix, false)
                    && let Some(Node::And(group, _)) = children.last_mut()
                {
                    group.push(next);
                }
            }
        } else if token == "{" || token == "(" {
            let inner_prefix = if prefix.is_empty() {
                parent_prefix.to_owned()
            } else {
                prefix.clone()
            };
            let (inner, wrap): (Mode, Group) = if token == "{" {
                (Mode::Or, Node::Or)
            } else {
                (Mode::And, Node::And)
            };
            let group = parse_more(tokens, inner, &inner_prefix, false)
                .unwrap_or_else(|| wrap(Vec::new(), false));
            children.push(if negated {
                wrap(vec![group], true)
            } else {
                group
            });
            prefix.clear();
            negated = false;
        } else {
            let mut token = token;
            if let Some(rest) = token.strip_prefix('+') {
                token = format!("\"{rest}\"");
            }
            if token.starts_with('"') && !token.ends_with('"') {
                token.push('"');
            }
            let prefix_or_parent = if prefix.is_empty() {
                parent_prefix
            } else {
                prefix.as_str()
            };
            children.push(Node::Leaf(parse_leaf(
                &format!("{prefix_or_parent}{token}"),
                negated,
            )));
            prefix.clear();
            negated = false;
        }
        if mode == Mode::Single && children.len() == 1 {
            break;
        }
    }
    if !prefix.is_empty() {
        children.push(Node::Leaf(parse_leaf(&prefix, negated)));
    }
    match mode {
        Mode::Single => children.into_iter().next(),
        Mode::And => Some(Node::And(children, false)),
        Mode::Or => Some(Node::Or(children, false)),
    }
}

/// The value with its double quotes removed.
fn unquote(value: &str) -> String {
    value.replace('"', "")
}

/// Whether the value has anything but ASCII letters and digits.
fn has_non_alphanumeric(value: &str) -> bool {
    value.chars().any(|c| !c.is_ascii_alphanumeric())
}

/// Whether a yes/no value says yes: it starts with `yes` or contains `true`.
fn says_yes(value: &str) -> bool {
    value.starts_with("yes") || value.contains("true")
}

/// Parses one token, its `field:` prefix included.
fn parse_leaf(text: &str, negated: bool) -> Token {
    static PATTERNS: LazyLock<[Regex; 14]> = LazyLock::new(|| {
        [
            r"(?i)^has:\s*attachment$",
            r"(?i)^hasattachment:\s*(yes|no|true|false)$",
            r"(?i)^has:\s*star$",
            r"(?i)^isflagged:\s*(yes|no|true|false)",
            r"(?i)^is:\s*flagged",
            r"(?i)^(filename|attachment):\s*(.*)",
            r"(?i)^(subject|list|rfc822msgid):\s*(.*)",
            r"(?i)^(after|before|older|newer):\s*(.*)",
            r"(?i)^(to|from|cc|bcc|replyto|deliveredto):\s*(.*)",
            r"(?i)^isread:\s*(yes|no|true|false)",
            r"(?i)^(in|is|label|l):\s*(.*)",
            r"(?i)^category:\s*(.*)",
            r"(?i)^autolabel:\s*(.*)",
            r"(?i)^kind:\s*(meetings)",
        ]
        .map(|pattern| Regex::new(pattern).expect("valid regex"))
    });
    let token =
        |kind: Kind, field: &str, value: Option<String>, quoted: bool, negated: bool| Token {
            kind,
            field: field.to_owned(),
            value,
            quoted,
            negated,
            text: text.to_owned(),
        };
    let starred = |negated: bool| token(Kind::Label, "is", Some("starred".into()), false, negated);
    let [
        attachment,
        has_attachment,
        star,
        flagged_yes,
        flagged,
        filename,
        header,
        after,
        contact,
        read,
        label,
        category,
        auto_label,
        meetings,
    ] = &*PATTERNS;
    let group = |pattern: &Regex, at: usize| {
        pattern
            .captures(text)
            .and_then(|captures| captures.get(at))
            .map_or_else(String::new, |found| found.as_str().to_owned())
    };

    if attachment.is_match(text) {
        return token(Kind::Attachment, "", None, false, negated);
    }
    if has_attachment.is_match(text) {
        return token(
            Kind::Attachment,
            "",
            None,
            false,
            if says_yes(&group(has_attachment, 1)) {
                negated
            } else {
                !negated
            },
        );
    }
    if star.is_match(text) {
        return starred(negated);
    }
    if flagged_yes.is_match(text) {
        return starred(if says_yes(&group(flagged_yes, 1)) {
            negated
        } else {
            !negated
        });
    }
    if flagged.is_match(text) {
        return starred(negated);
    }
    if filename.is_match(text) {
        let value = group(filename, 2);
        return token(
            Kind::Filename,
            "",
            Some(unquote(&value)),
            has_non_alphanumeric(&value),
            negated,
        );
    }
    if header.is_match(text) {
        let field = group(header, 1).to_lowercase();
        let mut value = group(header, 2);
        if (field == "list" || field == "rfc822msgid")
            && value.starts_with('<')
            && value.ends_with('>')
            && value.len() >= 2
        {
            value = value[1..value.len() - 1].to_owned();
        }
        return token(
            Kind::Header,
            &field,
            Some(unquote(&value)),
            has_non_alphanumeric(&value),
            negated,
        );
    }
    if after.is_match(text) {
        let operator = group(after, 1);
        let flipped = operator == "before" || operator == "older";
        return token(
            Kind::After,
            "",
            Some(group(after, 2)),
            false,
            negated != flipped,
        );
    }
    if contact.is_match(text) {
        let value = group(contact, 2);
        let quoted = value
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '_'))
            || value.starts_with('"');
        return token(
            Kind::Contact,
            &group(contact, 1).to_lowercase(),
            Some(unquote(&value)),
            quoted,
            negated,
        );
    }
    if read.is_match(text) {
        // `isread:yes` reads as `-is:unread`.
        let negated = if says_yes(&group(read, 1)) {
            !negated
        } else {
            negated
        };
        return token(Kind::Label, "is", Some("unread".into()), false, negated);
    }
    if label.is_match(text) {
        let field = group(label, 1).to_lowercase();
        let mut value = group(label, 2);
        let mut negated = negated;
        if value.eq_ignore_ascii_case("read") {
            negated = !negated;
            value = "unread".into();
        }
        if field == "is" && value.eq_ignore_ascii_case("shared") {
            return token(Kind::Shared, "", None, false, negated);
        }
        let lower = value.to_lowercase();
        if field == "in"
            && CATEGORIES
                .iter()
                .any(|category| lower == format!("category_{category}"))
        {
            // The prefix goes whatever its case; the name keeps its own.
            let name = value["category_".len()..].to_owned();
            return Token {
                kind: Kind::Category,
                field: "category".into(),
                text: format!("category:{name}"),
                value: Some(name),
                quoted: false,
                negated,
            };
        }
        return token(
            Kind::Label,
            &field,
            Some(unquote(&value)),
            value.starts_with('"'),
            negated,
        );
    }
    if category.is_match(text) {
        let value = group(category, 1);
        return token(
            Kind::Category,
            "category",
            Some(unquote(&value)),
            value.starts_with('"'),
            negated,
        );
    }
    if auto_label.is_match(text) {
        let value = group(auto_label, 1);
        return token(
            Kind::AutoLabel,
            "autolabel",
            Some(unquote(&value)),
            value.starts_with('"'),
            negated,
        );
    }
    if meetings.is_match(text) {
        return token(
            Kind::Filename,
            "",
            Some("ics".into()),
            text.starts_with('"'),
            negated,
        );
    }
    token(
        Kind::Text,
        "",
        Some(unquote(text)),
        has_non_alphanumeric(text),
        negated,
    )
}

/// Gmail's categories as `in:category_…` can name them.
const CATEGORIES: [&str; 8] = [
    "primary",
    "social",
    "forums",
    "updates",
    "promotions",
    "reservations",
    "purchases",
    "personal",
];

/// Constrains a split below its top level: an explicit `in:inbox` is dropped (the top adds its
/// own).
fn constrain_to_inbox(node: Node) -> Node {
    match node {
        Node::And(children, negated) => Node::And(
            children.into_iter().map(constrain_to_inbox).collect(),
            negated,
        ),
        Node::Or(children, negated) => Node::Or(
            children.into_iter().map(constrain_to_inbox).collect(),
            negated,
        ),
        Node::Leaf(token)
            if token.kind == Kind::Label && token.value().eq_ignore_ascii_case("inbox") =>
        {
            Node::Leaf(Token::empty())
        }
        leaf => leaf,
    }
}

/// A bare address is `from:` it or `to:` it.
fn convert_email_addresses(node: &mut Node) {
    static ADDRESS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"[^\s<@]+@[a-zA-Z0-9-]+(?:\.[a-zA-Z0-9-]+)+").expect("valid regex")
    });
    let (Node::And(children, _) | Node::Or(children, _)) = node else {
        return;
    };
    for child in children {
        match child {
            Node::Leaf(token) if token.kind == Kind::Text && ADDRESS.is_match(token.value()) => {
                let value = token.value().to_owned();
                *child = Node::Or(
                    vec![
                        Node::Leaf(parse_leaf(&format!("from:{value}"), false)),
                        Node::Leaf(parse_leaf(&format!("to:{value}"), false)),
                    ],
                    false,
                );
            }
            Node::Leaf(_) => {}
            group => convert_email_addresses(group),
        }
    }
}

/// Nested groups of the same kind flattened, empty text dropped, one-child groups unwrapped.
fn simplify(node: Node) -> Node {
    let (children, negated, is_and) = match node {
        Node::And(children, negated) => (children, negated, true),
        Node::Or(children, negated) => (children, negated, false),
        leaf => return leaf,
    };
    let mut queue: VecDeque<Node> = children.into();
    let mut kept = Vec::new();
    while let Some(child) = queue.pop_front() {
        let same = matches!(
            (&child, is_and),
            (Node::And(_, false), true) | (Node::Or(_, false), false)
        );
        if same {
            if let Node::And(grandchildren, _) | Node::Or(grandchildren, _) = child {
                for grandchild in grandchildren.into_iter().rev() {
                    queue.push_front(grandchild);
                }
            }
            continue;
        }
        let child = simplify(child);
        if !matches!(&child, Node::Leaf(token) if token.is_empty_text() && !token.negated) {
            kept.push(child);
        }
    }
    match kept.len() {
        0 => Node::Leaf(Token::empty()),
        1 => {
            let mut only = kept.pop().expect("one child");
            if negated {
                only.negate();
            }
            only
        }
        _ if is_and => Node::And(kept, negated),
        _ => Node::Or(kept, negated),
    }
}

/// The tokens that can't define a split (`for_splits`) or be acted on in bulk.
fn invalid_tokens<'a>(node: &'a Node, out: &mut Vec<&'a Token>, for_splits: bool) {
    // Exo's own lists; a split can't be defined on Unread or All either.
    const UNSEARCHABLE: [&str; 9] = [
        "done",
        "other",
        "important",
        "muted",
        "snippets",
        "reminders",
        "snoozed",
        "snippets",
        "archived",
    ];
    const NOT_SPLITTABLE: [&str; 2] = ["unread", "all"];
    match node {
        Node::And(children, _) | Node::Or(children, _) => children
            .iter()
            .for_each(|child| invalid_tokens(child, out, for_splits)),
        Node::Leaf(token) => {
            let label = token.value().to_lowercase();
            let label_valid = !UNSEARCHABLE.contains(&label.as_str())
                && !(for_splits && NOT_SPLITTABLE.contains(&label.as_str()));
            let valid = matches!(
                token.kind,
                Kind::After
                    | Kind::Contact
                    | Kind::Attachment
                    | Kind::Filename
                    | Kind::Category
                    | Kind::AutoLabel
                    | Kind::Shared
            ) || (token.kind == Kind::Label && label_valid)
                || (token.kind == Kind::Header
                    && (token.field == "subject" || token.field == "list"));
            if !valid {
                out.push(token);
            }
        }
    }
}

/// The label (and category) names in `node` no label answers to.
fn missing_labels(node: &Node, labels: &LabelIndex) -> Vec<String> {
    let mut tokens = Vec::new();
    label_tokens(node, &mut tokens);
    tokens
        .iter()
        .map(|token| label_slug(token.value()))
        .filter(|slug| !labels.by_slug.contains_key(slug) && slug != "all" && slug != "unread")
        .collect()
}

/// Label tokens, with categories as the `in:category_…` labels they are. (Auto Labels aren't
/// among them: Exo hasn't got any.)
fn label_tokens(node: &Node, out: &mut Vec<Token>) {
    match node {
        Node::And(children, _) | Node::Or(children, _) => {
            children.iter().for_each(|child| label_tokens(child, out))
        }
        Node::Leaf(token) if token.kind == Kind::Label => out.push(token.clone()),
        Node::Leaf(token) if token.kind == Kind::Category => out.push(category_label(token)),
        Node::Leaf(_) => {}
    }
}

/// A category token as the `in:category_…` label token it stands for.
fn category_label(token: &Token) -> Token {
    let value = format!("category_{}", token.value());
    Token {
        kind: Kind::Label,
        field: "in".into(),
        text: format!("in:{value}"),
        value: Some(value),
        quoted: false,
        negated: token.negated,
    }
}

/// The Gmail search for `node`. `top` is a split's top level, whose AND isn't parenthesized;
/// `important` is the Important split's definition, which `in:important` and `in:other` stand
/// for in a search.
fn gmail_query(
    node: &Node,
    top: bool,
    labels: &LabelIndex,
    me: &[String],
    important: Option<&str>,
) -> String {
    let minus = if node.negated() { "-" } else { "" };
    let join = |children: &[Node]| {
        children
            .iter()
            .map(|child| gmail_query(child, false, labels, me, important))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let token = match node {
        Node::And(children, negated) if top && !negated => return join(children),
        Node::And(children, _) => return format!("{minus}({})", join(children)),
        Node::Or(children, _) => return format!("{minus}{{{}}}", join(children)),
        Node::Leaf(token) => token,
    };
    if token.value.as_deref() == Some("") {
        return String::new();
    }
    let value = if token.quoted {
        format!("\"{}\"", token.value())
    } else {
        token.value().to_owned()
    };
    match token.kind {
        Kind::Label if important.is_some() && token.value().eq_ignore_ascii_case("important") => {
            format!("{minus}{}", important.unwrap_or_default())
        }
        Kind::Label if important.is_some() && token.value().eq_ignore_ascii_case("other") => {
            format!(
                "{}{}",
                if token.negated { "" } else { "-" },
                important.unwrap_or_default()
            )
        }
        Kind::Label => {
            let slug = label_slug(token.value());
            match labels.by_slug.get(&slug) {
                Some(label) if label_slug(&label.name) != slug => {
                    format!("{minus}label:{}", label_slug(&label.name))
                }
                _ if token
                    .value()
                    .chars()
                    .any(|c| matches!(c, '!'..='/' | '@' | ';')) =>
                {
                    format!("{minus}label:{slug}")
                }
                _ => format!("{minus}{}:{value}", token.field),
            }
        }
        // Auto Labels; Exo has none to name.
        Kind::AutoLabel => String::new(),
        Kind::Contact if token.value().eq_ignore_ascii_case("me") && !token.quoted => {
            format!("{}:{minus}{{{}}}", token.field, me.join(" "))
        }
        Kind::Contact if is_dot_me(token.value()) => format!("{minus}{}:*.me", token.field),
        Kind::Contact | Kind::Header => format!("{minus}{}:{value}", token.field),
        Kind::Attachment => format!("{minus}has:attachment"),
        Kind::Shared => format!("{minus}from:\"sharing@superhuman.com\""),
        Kind::Filename => format!("{minus}filename:{value}"),
        Kind::After => format!("{minus}after:{value}"),
        Kind::Category => format!("{minus}category:{}", token.value()),
        Kind::Text => format!("{minus}{}", value.replace('\'', "")),
    }
}

/// Whether the value is `me` after nothing but punctuation (`.me`, `*.me`), ignoring case.
fn is_dot_me(value: &str) -> bool {
    let lower = value.to_lowercase();
    lower.strip_suffix("me").is_some_and(|rest| {
        rest.chars()
            .all(|c| !(c.is_ascii_alphanumeric() || c == '_'))
    })
}

/// A thread as a query matches it. A split queries the messages that aren't drafts one by one;
/// a search queries every message, and a group matches when its parts match anywhere in the
/// thread.
struct Matcher<'a> {
    thread: &'a ThreadView,
    messages: Vec<&'a ViewMessage>,
    attachments: &'a [AttachmentRow],
    labels: &'a LabelIndex,
    me: &'a [String],
    message_level: bool,
}

/// Indexes into [`Matcher::messages`].
type Found = BTreeSet<usize>;

impl<'a> Matcher<'a> {
    fn split(
        thread: &'a ThreadView,
        attachments: &'a [AttachmentRow],
        labels: &'a LabelIndex,
        me: &'a [String],
    ) -> Self {
        Self {
            messages: thread.messages.iter().filter(|m| !m.is_draft()).collect(),
            thread,
            attachments,
            labels,
            me,
            message_level: true,
        }
    }

    fn search(
        thread: &'a ThreadView,
        attachments: &'a [AttachmentRow],
        labels: &'a LabelIndex,
        me: &'a [String],
    ) -> Self {
        Self {
            messages: thread.messages.iter().collect(),
            thread,
            attachments,
            labels,
            me,
            message_level: false,
        }
    }

    fn all(&self) -> Found {
        (0..self.messages.len()).collect()
    }

    fn attachments_of(&self, message: &ViewMessage) -> impl Iterator<Item = &'a AttachmentRow> {
        let id = message.row.id.clone();
        self.attachments
            .iter()
            .filter(move |row| row.message_id == id)
    }

    /// Whether `message` has an attachment that shows: not inline, and not a calendar file.
    fn has_visible_attachment(&self, message: &ViewMessage) -> bool {
        self.attachments_of(message).any(|row| {
            !row.inline
                && !matches!(
                    row.mime_type.to_ascii_lowercase().as_str(),
                    "text/calendar" | "application/ics"
                )
        })
    }

    /// The thread's time for `after:`: its latest message's, drafts aside.
    fn thread_time(&self) -> i64 {
        self.thread
            .messages
            .iter()
            .filter(|m| !m.is_draft())
            .map(|m| m.row.internal_date)
            .max()
            .unwrap_or(i64::MIN)
    }

    /// A quick rejection before matching message by message.
    fn excludes(&self, node: &Node) -> bool {
        let token = match node {
            Node::And(children, _) => return children.iter().any(|child| self.excludes(child)),
            Node::Or(children, _) => return children.iter().all(|child| self.excludes(child)),
            Node::Leaf(token) => token,
        };
        match token.kind {
            Kind::After => {
                let Some(date) = parse_date(token.value()) else {
                    return true;
                };
                let time = self.thread_time();
                time != date && (time >= date) == token.negated
            }
            Kind::Attachment => {
                self.thread
                    .messages
                    .iter()
                    .any(|m| self.has_visible_attachment(m))
                    == token.negated
            }
            Kind::Label if token.negated && !token.value().is_empty() => {
                let id = self.labels.id(token.value());
                self.messages.iter().any(|m| m.has(id))
            }
            // Auto Labels: unknown here, so excluded.
            Kind::AutoLabel => true,
            // No thread is shared without teams.
            Kind::Shared => !token.negated,
            _ => false,
        }
    }

    /// The messages `node` matches, message by message for a split. `None` is a token that
    /// can't be matched locally.
    fn find(&self, node: &Node) -> Option<Found> {
        let found = match node {
            Node::Leaf(token) if token.value.as_deref() == Some("") => return Some(self.all()),
            Node::And(children, _) => {
                let sets = children
                    .iter()
                    .map(|child| self.find(child))
                    .collect::<Option<Vec<_>>>()?;
                if !self.message_level {
                    return Some(
                        self.whole(sets.iter().all(|set| !set.is_empty()), node.negated()),
                    );
                }
                let mut sets = sets.into_iter();
                let first = sets.next().unwrap_or_default();
                sets.fold(first, |acc, set| acc.intersection(&set).copied().collect())
            }
            Node::Or(children, _) => {
                let sets = children
                    .iter()
                    .map(|child| self.find(child))
                    .collect::<Option<Vec<_>>>()?;
                if !self.message_level {
                    return Some(
                        self.whole(sets.iter().any(|set| !set.is_empty()), node.negated()),
                    );
                }
                sets.into_iter().flatten().collect()
            }
            Node::Leaf(token) => self.find_token(token)?,
        };
        Some(match (node.negated(), self.message_level) {
            (false, _) => found,
            (true, true) => self.all().difference(&found).copied().collect(),
            (true, false) => self.whole(!found.is_empty(), true),
        })
    }

    /// Thread-level matching: every message when the node matched (or didn't, negated).
    fn whole(&self, matched: bool, negated: bool) -> Found {
        if matched != negated {
            self.all()
        } else {
            Found::new()
        }
    }

    fn find_token(&self, token: &Token) -> Option<Found> {
        let indexes = |keep: &dyn Fn(&ViewMessage) -> bool| -> Found {
            self.messages
                .iter()
                .enumerate()
                .filter(|(_, m)| keep(m))
                .map(|(at, _)| at)
                .collect()
        };
        Some(match token.kind {
            Kind::After => match parse_date(token.value()) {
                Some(date) => indexes(&|m| {
                    if token.negated {
                        date < m.row.internal_date
                    } else {
                        date <= m.row.internal_date
                    }
                }),
                None => Found::new(),
            },
            Kind::Label => self.label(token.value()),
            Kind::Category => self.label(category_label(token).value()),
            Kind::AutoLabel | Kind::Shared => Found::new(),
            Kind::Attachment => indexes(&|m| self.has_visible_attachment(m)),
            Kind::Filename => {
                let wanted = token.value().to_lowercase();
                indexes(&|m| {
                    self.attachments_of(m)
                        .any(|row| filename_matches(&wanted, row))
                })
            }
            Kind::Contact => indexes(&|m| self.contact_matches(token, m)),
            Kind::Header if token.field == "subject" => {
                if word_match(
                    &normalize_subject(token.value()),
                    &normalize_subject(&self.thread.subject()),
                ) {
                    self.all()
                } else {
                    Found::new()
                }
            }
            Kind::Header if token.field == "list" => {
                let wanted = collapse_whitespace(token.value());
                if self.messages.iter().any(|m| {
                    m.row
                        .list_id
                        .as_deref()
                        .is_some_and(|list| word_match(&wanted, list))
                }) {
                    self.all()
                } else {
                    Found::new()
                }
            }
            Kind::Header | Kind::Text => return None,
        })
    }

    /// The whole thread when any message carries the label.
    fn label(&self, value: &str) -> Found {
        let wanted = label_slug(self.labels.id(value));
        if self
            .messages
            .iter()
            .any(|m| m.labels.iter().any(|label| label_slug(label) == wanted))
        {
            self.all()
        } else {
            Found::new()
        }
    }

    /// Whether one of `message`'s addresses in `token`'s field matches its value.
    fn contact_matches(&self, token: &Token, message: &ViewMessage) -> bool {
        let values: Vec<String> = if token.value().eq_ignore_ascii_case("me") && !token.quoted {
            self.me.to_vec()
        } else {
            vec![token.value().to_owned()]
        };
        let values: Vec<String> = values
            .iter()
            .map(|value| {
                value
                    .trim_start_matches([
                        '*', '.', '^', '/', '@', ',', '#', '$', '%', '\\', '|', ';',
                    ])
                    .to_lowercase()
            })
            .collect();
        let row = &message.row;
        let delivered: Vec<Mailbox>;
        let mailboxes: Vec<&Mailbox> = match token.field.as_str() {
            "from" => row.sender.0.iter().collect(),
            "to" => row
                .to
                .iter()
                .chain(row.cc.iter())
                .chain(row.bcc.iter())
                .collect(),
            "cc" => row.cc.iter().collect(),
            "bcc" => row.bcc.iter().collect(),
            "replyto" => row.reply_to.iter().collect(),
            "deliveredto" => {
                delivered = row
                    .headers
                    .iter()
                    .filter(|header| header.name.eq_ignore_ascii_case("Delivered-To"))
                    .map(|header| Mailbox {
                        name: None,
                        email: header.value.trim().to_owned(),
                    })
                    .collect();
                delivered.iter().collect()
            }
            _ => Vec::new(),
        };
        mailboxes
            .into_iter()
            .any(|mailbox| contact_matches_field(&values, mailbox))
    }
}

/// Whether `mailbox` matches one of `values`: its whole name or address, a part of the address,
/// or words of its name.
fn contact_matches_field(values: &[String], mailbox: &Mailbox) -> bool {
    let name = mailbox.name.as_deref().unwrap_or_default().to_lowercase();
    let email = mailbox.email.to_lowercase();
    let canonical = canonical_gmail_address(&email);
    if !values.iter().any(|value| {
        name.contains(value.as_str())
            || email.contains(value.as_str())
            || canonical.contains(value.as_str())
    }) {
        return false;
    }
    let name_words = words(&name);
    let at_parts: Vec<&str> = email.split('@').collect();
    let dot_parts: Vec<&str> = email.split(['.', '@']).collect();
    values.iter().any(|value| {
        name == *value
            || email == *value
            || canonical == *value
            || (!value.contains('@') && email.ends_with(&format!(".{value}")))
            || at_parts.contains(&value.as_str())
            || dot_parts.contains(&value.as_str())
            || words(value).iter().all(|word| name_words.contains(word))
    })
}

/// The canonical Gmail address: no `+tag`, no dots before the `@`, lower-cased.
fn canonical_gmail_address(email: &str) -> String {
    let mut parts: Vec<String> = email.split('@').map(str::to_owned).collect();
    if let Some(local) = parts.first_mut() {
        if let Some(plus) = local.find('+') {
            local.truncate(plus);
        }
        *local = local.replace('.', "");
    }
    parts.join("@").to_lowercase()
}

/// Runs of anything but ASCII letters and digits separate words, and one at either end leaves an
/// empty word there.
fn words(text: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut start = 0;
    let mut in_separator = false;
    for (at, c) in text.char_indices() {
        let separator = c.is_ascii() && !c.is_ascii_alphanumeric();
        if separator && !in_separator {
            words.push(&text[start..at]);
        }
        if !separator && in_separator {
            start = at;
        }
        in_separator = separator;
    }
    words.push(if in_separator { "" } else { &text[start..] });
    words
}

/// Whether attachment `row` is the one `wanted` names: its whole filename or MIME type, or words
/// of them.
fn filename_matches(wanted: &str, row: &AttachmentRow) -> bool {
    let name = row.filename.to_lowercase();
    let mime = row.mime_type.to_lowercase();
    if name == wanted || mime == wanted {
        return true;
    }
    let (name_words, mime_words) = (words(&name), words(&mime));
    words(wanted)
        .iter()
        .all(|word| name_words.contains(word) || mime_words.contains(word))
}

/// Collapses whitespace runs to one space and trims.
fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A subject as `subject:` compares it: punctuation to spaces, whitespace collapsed.
fn normalize_subject(text: &str) -> String {
    const PUNCTUATION: &str = "`~!@#$%^*)(+-={\\:;'<>,.?/";
    collapse_whitespace(
        &text
            .chars()
            .map(|c| if PUNCTUATION.contains(c) { ' ' } else { c })
            .collect::<String>(),
    )
}

/// Whether `needle` is in `haystack` between word boundaries, ignoring case.
fn word_match(needle: &str, haystack: &str) -> bool {
    let bound = r"(?:^|(?-u:\b)|\s|$)";
    Regex::new(&format!("(?i){bound}{}{bound}", regex::escape(needle)))
        .is_ok_and(|pattern| pattern.is_match(haystack))
}

/// An `after:`/`before:` value in epoch milliseconds: nine or ten digits are seconds; ISO
/// `YYYY-MM-DD` is UTC midnight, and `YYYY/M/D`, `YYYY-M-D` and `M/D/YYYY` local midnight.
/// Anything else is an invalid date.
fn parse_date(value: &str) -> Option<i64> {
    static SECONDS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[0-9]{9,10}").expect("valid regex"));
    if SECONDS.is_match(value) {
        // Only the leading digits count.
        let digits: String = value
            .trim_start()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        return digits.parse::<i64>().ok().map(|seconds| seconds * 1000);
    }
    let value = value.trim();
    let separator = if value.contains('-') { '-' } else { '/' };
    let parts: Vec<&str> = value.split(separator).collect();
    let [first, second, third] = parts[..] else {
        return None;
    };
    let number = |part: &str| part.parse::<i16>().ok();
    let (year, month, day) = if first.len() == 4 {
        (number(first)?, number(second)?, number(third)?)
    } else {
        (number(third)?, number(first)?, number(second)?)
    };
    let date =
        jiff::civil::Date::new(year, i8::try_from(month).ok()?, i8::try_from(day).ok()?).ok()?;
    let iso = separator == '-' && first.len() == 4 && second.len() == 2 && third.len() == 2;
    let zone = if iso {
        jiff::tz::TimeZone::UTC
    } else {
        jiff::tz::TimeZone::system()
    };
    date.to_zoned(zone)
        .ok()
        .map(|zoned| zoned.timestamp().as_millisecond())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::tests::{ME, message, thread};

    fn labels() -> LabelIndex {
        LabelIndex::new(
            [],
            [
                ("Label_1", "Work"),
                ("Label_2", "Team Updates"),
                ("CATEGORY_SOCIAL", "CATEGORY_SOCIAL"),
                ("IMPORTANT", "IMPORTANT"),
                ("UNREAD", "UNREAD"),
            ],
        )
    }

    fn gmail(query: &str) -> String {
        SplitQuery::parse(query).to_gmail(&labels(), &[ME.to_owned()])
    }

    fn invalid(query: &str) -> Option<String> {
        SplitQuery::parse(query)
            .invalidity(&labels())
            .map(|invalidity| invalidity.text)
    }

    #[test]
    fn split_definitions_become_gmail_queries() {
        // A Team split, and the default Calendar split.
        assert_eq!(
            gmail("from:brightmoor.co to:me"),
            format!("-in:DRAFT -in:CHAT in:inbox from:\"brightmoor.co\" to:{{{ME}}}")
        );
        assert_eq!(
            gmail("is:starred"),
            "-in:DRAFT -in:CHAT in:inbox is:starred"
        );
        assert_eq!(
            gmail("filename:ics"),
            "-in:DRAFT -in:CHAT in:inbox filename:ics"
        );
        assert_eq!(
            gmail(
                "from:{notifications@calendly.com, calendar-notification@google.com, calendar@superhuman.com} OR filename:ics"
            ),
            // `OR` after a `{…}` group joins that group rather than wrapping it.
            "-in:DRAFT -in:CHAT in:inbox {from:\"notifications@calendly.com\" from:\"calendar-notification@google.com\" from:\"calendar@superhuman.com\" filename:ics}"
        );
        assert_eq!(
            gmail("from:a@x.com OR from:b@y.com"),
            "-in:DRAFT -in:CHAT in:inbox {from:\"a@x.com\" from:\"b@y.com\"}"
        );
        assert_eq!(
            gmail("(from:a OR from:b) to:me"),
            format!("-in:DRAFT -in:CHAT in:inbox {{from:a from:b}} to:{{{ME}}}")
        );
        assert_eq!(
            gmail("label:Label_1 -subject:report"),
            "-in:DRAFT -in:CHAT in:inbox label:work -subject:report"
        );
        assert_eq!(
            gmail("label:\"Team Updates\""),
            "-in:DRAFT -in:CHAT in:inbox label:\"Team Updates\""
        );
        assert_eq!(
            gmail("in:inbox alice@example.com"),
            "-in:DRAFT -in:CHAT in:inbox {from:\"alice@example.com\" to:\"alice@example.com\"}"
        );
        assert_eq!(gmail(""), "-in:DRAFT -in:CHAT in:inbox");
    }

    #[test]
    fn invalid_definitions_get_an_alert_saying_why() {
        assert_eq!(invalid("from:a@b.com"), None);
        assert_eq!(invalid("subject:\"survey results\""), None);
        assert_eq!(
            invalid("label:Work in:category_social after:2024/01/01 has:attachment list:x"),
            None
        );
        assert_eq!(invalid("budget"), Some("We can't split your inbox on text searches.\n\n            Please try subject:\"budget\", or from:\"budget\".".into()));
        assert_eq!(invalid("subject:\"a\"b\""), Some("We can't split your inbox on subjects with quotes.\n\n              Please remove this and try again.".into()));
        assert_eq!(invalid("label:Nope"), Some("We can't split your inbox on labels that don't exist.\n\n            Please create label nope first or try splitting on a different label.".into()));
        assert_eq!(invalid("label:a label:b"), Some("We can't split your inbox on labels that don't exist.\n\n            Please create labels first or try splitting on a different label.".into()));
        assert_eq!(invalid("is:unread"), Some("We can't split your inbox on is:unread\n\n            Please remove this and try again.".into()));
        assert_eq!(invalid("in:done"), Some("We can't split your inbox on in:done\n\n            Please remove this and try again.".into()));
        assert!(
            invalid("autolabel:x")
                .is_some_and(|text| text.starts_with("We can't split your inbox with Auto Labels"))
        );
    }

    #[test]
    fn splits_match_threads_locally() {
        let me = [ME.to_owned()];
        let labels = labels();
        let matches = |query: &str, view: &ThreadView| {
            SplitQuery::parse(query).is_match(view, &[], &labels, &me)
        };
        let mut from_github = message(
            "m1",
            1_700_000_000_000,
            "notifications@github.com",
            &[ME],
            &["INBOX", "Label_1"],
        );
        from_github.subject = "[repo] Fix the build (#12)".into();
        let view = thread(vec![from_github]);
        assert!(matches("from:github.com", &view));
        assert!(matches("from:notifications@github.com", &view));
        assert!(matches("from:github", &view));
        assert!(!matches("from:hub.com", &view));
        assert!(matches("to:me", &view));
        assert!(matches("label:Work", &view));
        assert!(matches("subject:\"fix the build\"", &view));
        assert!(!matches("subject:fix -label:work", &view));
        assert!(matches("from:nobody OR label:work", &view));
        assert!(!matches("budget", &view), "text can't be matched locally");
        assert!(!matches("", &view), "an empty definition matches nothing");
        assert!(matches("after:2023/01/01", &view));
        assert!(!matches("before:2023/01/01", &view));
        let archived = thread(vec![message(
            "m2",
            1,
            "notifications@github.com",
            &[ME],
            &[],
        )]);
        assert!(!matches("from:github.com", &archived), "only inbox threads");
    }

    fn fts(query: &str) -> Fts {
        SearchQuery::parse(query).fts(&labels(), &[ME.to_owned()])
    }

    #[test]
    fn searches_become_gmail_queries() {
        let to_gmail = |query: &str| {
            SearchQuery::parse(query).to_gmail(
                &labels(),
                &[ME.to_owned()],
                "((category:personal) OR from:\"via Google\")",
            )
        };
        assert_eq!(to_gmail("budget"), "-in:DRAFT -in:CHAT budget");
        assert_eq!(
            to_gmail("from:ana budget"),
            "-in:DRAFT -in:CHAT (from:ana budget)"
        );
        assert_eq!(
            to_gmail("in:important"),
            "-in:DRAFT -in:CHAT ((category:personal) OR from:\"via Google\")"
        );
        assert_eq!(
            to_gmail("in:other"),
            "-in:DRAFT -in:CHAT -((category:personal) OR from:\"via Google\")"
        );
        assert_eq!(
            to_gmail("to:me is:unread"),
            format!("-in:DRAFT -in:CHAT (to:{{{ME}}} is:unread)")
        );
        assert_eq!(
            to_gmail("label:\"Team Updates\" \"be brilliant\""),
            "-in:DRAFT -in:CHAT (label:\"Team Updates\" \"be brilliant\")"
        );
    }

    #[test]
    fn searches_read_the_index_as_one_and() {
        assert_eq!(
            fts("budget").matches,
            [(
                false,
                "budget NOT labels:TRASH NOT labels:SPAM NOT labels:SH_SNIPPETS".to_owned()
            )]
        );
        assert_eq!(
            fts("in:trash -from:ana").matches[0].1,
            "labels:TRASH NOT from:ana NOT labels:SPAM NOT labels:SH_SNIPPETS"
        );
        assert_eq!(
            fts("in:all").matches[0].1,
            "meta:thread NOT labels:SH_SNIPPETS"
        );
        let me = tokenize_email(ME);
        assert_eq!(
            fts("to:me").matches[0].1,
            format!(
                "(to:{me} OR cc:{me} OR bcc:{me}) NOT labels:TRASH NOT labels:SPAM NOT labels:SH_SNIPPETS"
            )
        );
        assert_eq!(
            fts("from:a OR budget").matches[0].1,
            "from:a AND budget NOT labels:TRASH NOT labels:SPAM NOT labels:SH_SNIPPETS",
            "OR reads as AND locally"
        );
        let phrase = fts("\"be brilliant\" -\"no way\" subject:\"q4 plan\"");
        assert_eq!(
            phrase.matches[0].1,
            "\"be brilliant\" NOT labels:TRASH NOT labels:SPAM NOT labels:SH_SNIPPETS"
        );
        assert_eq!(
            phrase.matches[1..],
            [
                (true, "\"no way\"".to_owned()),
                (false, "subject : \"q4 plan\"".to_owned())
            ]
        );
        assert_eq!(fts("before:2000/01/01").dates.len(), 1);
        assert!(fts("before:2000/01/01").dates[0].0, "before");
        assert_eq!(
            fts("after:nonsense").dates,
            [(true, 0)],
            "a bad date matches nothing"
        );
        // Whatever is typed stays inside one FTS5 string.
        assert_eq!(
            fts("NEAR(a\"").matches[0].1,
            "\"NEAR\" AND a NOT labels:TRASH NOT labels:SPAM NOT labels:SH_SNIPPETS"
        );
        assert_eq!(
            fts("subject:\"x\"\" OR 1\"").matches[1].1,
            "subject : \"x\""
        );
        assert_eq!(
            fts("OR").matches[0].1,
            "meta:thread NOT labels:TRASH NOT labels:SPAM NOT labels:SH_SNIPPETS"
        );
        assert_eq!(
            fts("budget*").matches[0].1,
            "\"budget\"* NOT labels:TRASH NOT labels:SPAM NOT labels:SH_SNIPPETS"
        );
    }

    #[test]
    fn the_last_token_is_what_autocomplete_completes() {
        let last = |query: &str| {
            SearchQuery::parse(query)
                .last_token()
                .map(|token| (token.kind, token.field, token.value))
        };
        assert_eq!(
            last("fr"),
            Some((TokenKind::Text, String::new(), "fr".into()))
        );
        assert_eq!(
            last("budget from:"),
            Some((TokenKind::Contact, "from".into(), String::new()))
        );
        assert_eq!(
            last("in:sen"),
            Some((TokenKind::Label, "in".into(), "sen".into()))
        );
        assert_eq!(last(""), None);
        assert!(SearchQuery::parse("  -- ").is_empty());
        assert!(!SearchQuery::parse("a").is_empty());
        assert!(SearchQuery::parse("in:trash").shows_label("trash"));
        assert!(!SearchQuery::parse("-in:trash").shows_label("trash"));
    }

    #[test]
    fn words_split_at_punctuation_keeping_empty_ends() {
        assert_eq!(words("a.b@c"), ["a", "b", "c"]);
        assert_eq!(words("-a-"), ["", "a", ""]);
        assert_eq!(words("plain"), ["plain"]);
    }

    #[test]
    fn bulk_actions_refuse_words_and_exos_own_lists() {
        let refusal = |query: &str| SearchQuery::parse(query).bulk_refusal(&labels());
        assert_eq!(refusal("from:ann@x.com in:work"), None);
        assert_eq!(
            refusal("is:unread subject:report after:2026/11/01"),
            None,
            "Unread can be acted on in bulk"
        );
        assert_eq!(refusal("quarterly report from:ann"), Some("We can't complete this action on text searches.\n\nPlease try subject:\"quarterly\" or from:\"quarterly\".".into()));
        let generic = Some("We can't complete this action on this set of conversations.\n\nPlease unselect these conversations and try again.".into());
        assert_eq!(refusal("in:done from:ann"), generic);
        assert_eq!(
            refusal("label:nowhere"),
            generic,
            "a label that doesn't exist"
        );
    }
}
