//! The search bar's suggestions: the operator the last word starts, labels for
//! `in:`/`is:`/`label:`, Gmail categories, and people from local mail, at most five, mixed and
//! ordered by how much of the text each completes.

use std::sync::LazyLock;

use exo_commands::command_score;
use exo_store::{Label, LabelKind, SearchQuery, TokenKind};
use regex::Regex;

/// Most suggestions shown, and people looked up per word.
pub(crate) const LIMIT: usize = 5;
/// The operators a word completes to.
const KEYWORDS: [&str; 16] = [
    "in",
    "is",
    "label",
    "has:attachment",
    "attachment",
    "from",
    "to",
    "cc",
    "bcc",
    "deliveredto",
    "replyto",
    "filename",
    "subject",
    "category",
    "list",
    "rfc822msgid",
];
/// Gmail's categories as `in:category_…` names them.
const CATEGORIES: [&str; 5] = ["personal", "social", "promotions", "forums", "updates"];

/// The operators the bar recognises, which get lozenges.
pub(crate) const OPERATORS: &str = "(is|in|label|l|has|category|from|to|cc|bcc|subject|filename|list|before|after|older|newer|older_than|newer_than|deliveredto|rfc822msgid|size|larger|smaller):";

/// What a suggestion is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// An operator to finish typing a value after (`syntax-firstpart`: `from:`).
    Operator,
    /// `me`.
    Me,
    /// A label (`in:sent`).
    Label,
    /// A category (`category:social`).
    Category,
    /// Someone from local mail; inserting it inserts their address.
    Contact {
        /// Their name, if mail had one.
        name: Option<String>,
        /// Their address.
        email: String,
        /// Matched by name rather than address, so the bar spells out the address it inserts.
        by_name: bool,
    },
}

/// One suggestion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Suggestion {
    pub kind: Kind,
    /// What it reads and inserts (`replacement`).
    pub replacement: String,
    /// A label's replacement keeping the whitespace typed after the operator (`completion`).
    pub completion: Option<String>,
    /// The typed text it completes (`original`).
    pub original: String,
    /// Where the text it completes starts in the bar.
    pub start: Option<usize>,
}

impl Suggestion {
    fn new(kind: Kind, replacement: String, original: &str) -> Self {
        Self {
            kind,
            replacement,
            completion: None,
            original: original.to_owned(),
            start: None,
        }
    }

    pub fn is_contact(&self) -> bool {
        matches!(self.kind, Kind::Contact { .. })
    }
}

/// A label the bar can suggest: its slug (what `in:` takes) and its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LabelChoice {
    pub slug: String,
    pub name: String,
}

/// The labels `in:` suggests: Exo's own lists, then the account's labels and Gmail's categories, by
/// the names the app gives them.
pub(crate) fn label_choices(labels: &[Label]) -> Vec<LabelChoice> {
    let choice = |slug: &str, name: &str| LabelChoice {
        slug: slug.into(),
        name: name.into(),
    };
    let mut choices = vec![
        choice("all", "All Mail"),
        choice("done", "Done"),
        choice("scheduled", "Scheduled"),
        choice("reminders", "Reminders"),
        choice("muted", "Muted"),
        choice("snippets", "Snippets"),
    ];
    for label in labels {
        let system = |slug: &str, name: &str| Some(choice(slug, name));
        let found = match (label.kind, label.id.as_str()) {
            (LabelKind::System, "INBOX") => system("inbox", "Inbox"),
            (LabelKind::System, "SENT") => system("sent", "Sent"),
            (LabelKind::System, "STARRED") => system("starred", "Starred"),
            (LabelKind::System, "UNREAD") => system("unread", "Unread"),
            (LabelKind::System, "DRAFT") => system("draft", "Drafts"),
            (LabelKind::System, "TRASH") => system("trash", "Trash"),
            (LabelKind::System, "SPAM") => system("spam", "Spam"),
            (LabelKind::System, id) if id.starts_with("CATEGORY_") => Some(LabelChoice {
                slug: exo_store::label_slug(id),
                name: id.to_owned(),
            }),
            (LabelKind::System, _) => None,
            (LabelKind::User, _)
                if crate::lists::user_labels(std::slice::from_ref(label))
                    .next()
                    .is_some() =>
            {
                Some(LabelChoice {
                    slug: exo_store::label_slug(&label.name),
                    name: label.name.clone(),
                })
            }
            (LabelKind::User, _) => None,
        };
        choices.extend(found);
    }
    choices
}

/// The words to look people up by: what follows the last address operator or separator, and each
/// tail of it after a space outside quotes.
pub(crate) fn contact_words(raw: &str) -> Vec<String> {
    static SPLIT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?:\b(from|to|cc|bcc|replyto|deliveredto):)|[;|,()]").expect("valid regex")
    });
    let last = SPLIT.split(raw).last().unwrap_or_default().trim_start();
    increasing_substrings(last, false)
}

/// The tokens a suggestion can complete: `text`, then what follows each space (outside double
/// quotes unless `everywhere`).
pub(crate) fn increasing_substrings(text: &str, everywhere: bool) -> Vec<String> {
    let mut out = vec![text.to_owned()];
    let mut quoted = false;
    for (at, c) in text.char_indices() {
        if c == ' ' && (everywhere || !quoted) {
            out.push(text[at + 1..].to_owned());
        } else if c == '"' {
            quoted = !quoted;
        }
    }
    out
}

/// Whether the people found answer `raw`: the bar asks for them only when its last word could be
/// someone (`text` or an address operator's value).
pub(crate) fn wants_contacts(raw: &str) -> bool {
    last_word(raw).is_some_and(|(kind, value, _)| {
        matches!(kind, TokenKind::Text | TokenKind::Contact) && !value.trim().is_empty()
    })
}

/// The last token's kind, value (with a `!` or `(` typed before it kept on it) and operator.
fn last_word(raw: &str) -> Option<(TokenKind, String, String)> {
    static BANG: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?-u:\B)!\w+").expect("valid regex"));
    static PAREN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?-u:\B)\(\w+").expect("valid regex"));
    let token = SearchQuery::parse(raw).last_token()?;
    let mut value = token.value.clone();
    let has_value = token.kind != TokenKind::Other || !token.field.is_empty();
    if has_value && !token.quoted && BANG.is_match(raw) {
        value = format!("!{value}");
    }
    if has_value && !token.quoted && PAREN.is_match(raw) {
        value = format!("({value}");
    }
    Some((token.kind, value, token.field))
}

/// The suggestions for `raw`, given `labels` and, for each of [`contact_words`]`(raw)`, the people
/// local mail has (`(email, name)`).
pub(crate) fn suggest(
    raw: &str,
    labels: &[LabelChoice],
    contacts: &[Vec<(String, Option<String>)>],
) -> Vec<Suggestion> {
    let Some((kind, value, field)) = last_word(raw) else {
        return Vec::new();
    };
    if kind == TokenKind::Filename {
        return Vec::new();
    }
    let people = if matches!(kind, TokenKind::Text | TokenKind::Contact) && !value.trim().is_empty()
    {
        contact_suggestions(&contact_words(raw), contacts)
    } else {
        Vec::new()
    };
    let syntax = syntax_suggestions(kind, &field, &value, labels, !people.is_empty());
    let mut merged: Vec<Suggestion> = match (syntax.is_empty(), people.is_empty()) {
        (false, false) if people.len() >= 3 => syntax
            .into_iter()
            .take(2)
            .chain(people.into_iter().take(3))
            .collect(),
        (false, false) => syntax
            .into_iter()
            .take(3)
            .chain(people.into_iter().take(2))
            .collect(),
        (false, true) => syntax,
        _ => people,
    };
    // Longer typed text first; at equal length, people last.
    merged.sort_by(|a, b| {
        b.original
            .len()
            .cmp(&a.original.len())
            .then_with(|| a.is_contact().cmp(&b.is_contact()))
    });
    merged
}

/// The suggestions for the last token's syntax: labels, categories, or for plain text the operator
/// it starts, else (with no people found) the labels it names.
fn syntax_suggestions(
    kind: TokenKind,
    field: &str,
    value: &str,
    labels: &[LabelChoice],
    has_people: bool,
) -> Vec<Suggestion> {
    let mut found = match kind {
        TokenKind::Label => label_suggestions(field, value, labels),
        TokenKind::Category => category_suggestions(field, value),
        TokenKind::Text => {
            let lower = value.to_lowercase();
            match KEYWORDS.iter().find(|keyword| keyword.starts_with(&lower)) {
                Some(keyword) if !value.is_empty() => {
                    let replacement = match *keyword {
                        "has:attachment" | "attachment" => "has:attachment".to_owned(),
                        keyword => format!("{keyword}:"),
                    };
                    vec![Suggestion::new(Kind::Operator, replacement, value)]
                }
                _ if !value.is_empty() && !has_people => label_suggestions("in", value, labels),
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    };
    found.truncate(LIMIT);
    found
}

/// Labels whose slug or name contains the value, plus `all` and `read`, best match first, then by
/// the system labels' priority.
fn label_suggestions(field: &str, value: &str, labels: &[LabelChoice]) -> Vec<Suggestion> {
    const PRIORITY: [(&str, i32); 10] = [
        ("sent", 100),
        ("inbox", 99),
        ("unread", 98),
        ("spam", 96),
        ("trash", 95),
        ("starred", 54),
        ("read", 53),
        ("all", 52),
        ("draft", 51),
        ("important", 50),
    ];
    let original = format!("{field}:{value}");
    let wanted = value.trim().to_lowercase();
    let mut items: Vec<(Suggestion, String)> = Vec::new();
    for label in labels {
        if wanted.is_empty() || label.slug.contains(&wanted) {
            items.push((
                label_item(field, value, &label.slug, &original),
                label.slug.clone(),
            ));
        } else if label.name.to_lowercase().contains(&wanted) {
            items.push((
                label_item(field, value, &label.name, &original),
                label.slug.clone(),
            ));
        }
    }
    for extra in ["all", "read"] {
        if wanted.is_empty() || extra.starts_with(&wanted) {
            items.push((label_item(field, value, extra, &original), extra.to_owned()));
        }
    }
    let priority = |slug: &str| {
        PRIORITY
            .iter()
            .find(|(name, _)| *name == slug)
            .map_or(50, |(_, priority)| *priority)
    };
    let mut scored: Vec<(f64, i32, Suggestion)> = items
        .into_iter()
        .map(|(item, slug)| {
            (
                command_score(&item.replacement, &original),
                priority(&slug),
                item,
            )
        })
        .collect();
    scored.sort_by(|a, b| {
        if a.0 == b.0 {
            b.1.cmp(&a.1)
        } else {
            b.0.total_cmp(&a.0)
        }
    });
    scored.into_iter().map(|(_, _, item)| item).collect()
}

/// A label suggestion: a name with spaces or punctuation in quotes; `in:category_…` as
/// `category:…`.
fn label_item(field: &str, value: &str, name: &str, original: &str) -> Suggestion {
    let name = if name
        .chars()
        .any(|c| "!(){}-".contains(c) || c.is_whitespace())
    {
        format!("\"{name}\"")
    } else {
        name.to_owned()
    };
    let trimmed = value.trim();
    let lead = if trimmed.is_empty() {
        value
    } else {
        &value[..value.find(trimmed).unwrap_or_default()]
    };
    if field == "in"
        && let Some(category) = name
            .strip_prefix("category_")
            .filter(|category| CATEGORIES.contains(category))
    {
        return Suggestion {
            completion: Some(format!("category:{lead}{category}")),
            ..Suggestion::new(Kind::Category, format!("category:{category}"), original)
        };
    }
    Suggestion {
        completion: Some(format!("{field}:{lead}{name}")),
        ..Suggestion::new(Kind::Label, format!("{field}:{name}"), original)
    }
}

/// The categories that start with the value.
fn category_suggestions(field: &str, value: &str) -> Vec<Suggestion> {
    let original = format!("{field}:{value}");
    let wanted = value.trim().to_lowercase();
    let trimmed = value.trim();
    let lead = if trimmed.is_empty() {
        value
    } else {
        &value[..value.find(trimmed).unwrap_or_default()]
    };
    CATEGORIES
        .iter()
        .filter(|category| wanted.is_empty() || category.starts_with(&wanted))
        .map(|category| Suggestion {
            completion: Some(format!("{field}:{lead}{category}")),
            ..Suggestion::new(Kind::Category, format!("{field}:{category}"), &original)
        })
        .collect()
}

/// `me` for `m`/`me`, then up to five people, the first word's first, by address.
fn contact_suggestions(
    words: &[String],
    found: &[Vec<(String, Option<String>)>],
) -> Vec<Suggestion> {
    let last = words.last().map(String::as_str).unwrap_or_default();
    let mut out = Vec::new();
    if last == "m" || last == "me" {
        out.push(Suggestion::new(Kind::Me, "me".into(), last));
    }
    let mut people: Vec<Suggestion> = Vec::new();
    for (word, contacts) in words.iter().zip(found) {
        if word.trim().is_empty() || word.contains('"') || people.len() >= LIMIT {
            continue;
        }
        for (email, name) in contacts {
            if !people.iter().any(|person| matches!(&person.kind, Kind::Contact { email: known, .. } if known == email)) {
                people.push(format_for_search(email, name.as_deref(), word, words));
            }
        }
    }
    out.extend(people);
    out.truncate(LIMIT);
    out
}

/// A person suggestion, matched by address, by domain, or by a word of the name.
fn format_for_search(email: &str, name: Option<&str>, word: &str, words: &[String]) -> Suggestion {
    let word = word.trim().to_lowercase();
    let lower = email.to_lowercase();
    let domain = lower.split_once('@').map_or("", |(_, domain)| domain);
    let contact = |by_name: bool| Kind::Contact {
        name: name.map(str::to_owned),
        email: email.to_owned(),
        by_name,
    };
    if lower.starts_with(&word) || domain.starts_with(&word) {
        return Suggestion::new(contact(false), email.to_owned(), &word);
    }
    if let Some(name) = name.filter(|name| !email.trim().eq_ignore_ascii_case(name.trim())) {
        let parts = increasing_substrings(name, true);
        for typed in words {
            if parts
                .iter()
                .any(|part| part.to_lowercase().contains(&typed.to_lowercase()))
            {
                return Suggestion::new(contact(true), name.to_owned(), typed);
            }
        }
    }
    Suggestion::new(
        contact(true),
        email.to_owned(),
        words.last().map(String::as_str).unwrap_or_default(),
    )
}

/// `text` cut where the typed `original` (or its part after the last operator) matches,
/// case-insensitively: `(text, matched)` runs.
pub(crate) fn highlight<'a>(text: &'a str, original: &str) -> Vec<(&'a str, bool)> {
    static OPERATOR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(OPERATORS).expect("valid regex"));
    let original = original.trim().to_lowercase();
    if original.is_empty() {
        return vec![(text, false)];
    }
    let lower = text.to_lowercase();
    // Lower-casing kept every byte where it was; otherwise nothing is highlighted.
    if lower.len() != text.len() {
        return vec![(text, false)];
    }
    let found = lower
        .find(&original)
        .map(|at| (at, at + original.len()))
        .or_else(|| {
            let tail = OPERATOR
                .split(&original)
                .last()
                .filter(|tail| !tail.is_empty())?;
            lower.find(tail).map(|at| (at, at + tail.len()))
        });
    let Some((start, end)) =
        found.filter(|(start, end)| text.is_char_boundary(*start) && text.is_char_boundary(*end))
    else {
        return vec![(text, false)];
    };
    [
        (&text[..start], false),
        (&text[start..end], true),
        (&text[end..], false),
    ]
    .into_iter()
    .filter(|(part, _)| !part.is_empty())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> Vec<LabelChoice> {
        [
            "inbox",
            "sent",
            "starred",
            "unread",
            "draft",
            "trash",
            "spam",
            "done",
            "all",
            "family",
            "newsletters",
        ]
        .into_iter()
        .map(|slug| LabelChoice {
            slug: slug.into(),
            name: slug.into(),
        })
        .collect()
    }

    fn replacements(suggestions: &[Suggestion]) -> Vec<&str> {
        suggestions
            .iter()
            .map(|suggestion| suggestion.replacement.as_str())
            .collect()
    }

    #[test]
    fn a_word_starting_an_operator_suggests_it_before_people() {
        let freight = vec![vec![(
            "newsletter@freightsignals.com".to_owned(),
            Some("Freight Signals".to_owned()),
        )]];
        let suggestions = suggest("fr", &labels(), &freight);
        assert_eq!(
            replacements(&suggestions),
            ["from:", "newsletter@freightsignals.com"]
        );
        assert_eq!(suggestions[0].kind, Kind::Operator);
        assert_eq!(
            replacements(&suggest("att", &labels(), &[vec![]])),
            ["has:attachment"]
        );
    }

    #[test]
    fn people_match_by_address_domain_or_name() {
        let ana = vec![vec![(
            "ana@letters.anaruiz.com".to_owned(),
            Some("Ana Ruiz".to_owned()),
        )]];
        let by_address = suggest("an", &labels(), &ana);
        assert_eq!(replacements(&by_address), ["ana@letters.anaruiz.com"]);
        assert!(matches!(
            &by_address[0].kind,
            Kind::Contact { by_name: false, .. }
        ));
        let words = contact_words("ruiz");
        let by_name = suggest("ruiz", &labels(), &[ana[0].clone(); 1][..words.len()]);
        assert_eq!(replacements(&by_name), ["Ana Ruiz"]);
        assert!(matches!(
            &by_name[0].kind,
            Kind::Contact { by_name: true, .. }
        ));
        assert_eq!(contact_words("budget from:ana ruiz"), ["ana ruiz", "ruiz"]);
        assert_eq!(replacements(&suggest("me", &labels(), &[vec![]])), ["me"]);
    }

    #[test]
    fn labels_follow_in_is_and_label() {
        assert_eq!(replacements(&suggest("in:se", &labels(), &[])), ["in:sent"]);
        let all = suggest("in:", &labels(), &[]);
        assert_eq!(all.len(), 5);
        assert_eq!(all[0].replacement, "in:sent", "priority breaks score ties");
        assert_eq!(
            replacements(&suggest("is:unr", &labels(), &[])),
            ["is:unread"]
        );
        // A word nobody answers to suggests labels instead.
        assert_eq!(
            replacements(&suggest("fam", &labels(), &[vec![]])),
            ["in:family"]
        );
        assert!(suggest("filename:pd", &labels(), &[]).is_empty());
    }

    #[test]
    fn in_suggests_exos_own_lists() {
        let choices = label_choices(&[]);
        let suggested = |typed: &str| {
            replacements(&suggest(typed, &choices, &[]))
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(suggested("in:rem"), ["in:reminders"]);
        assert_eq!(suggested("in:snip"), ["in:snippets"]);
        assert_eq!(
            suggested("in:s"),
            ["in:scheduled", "in:snippets", "in:reminders"],
            "by `commandScore`"
        );
    }

    #[test]
    fn matches_are_cut_out_for_highlighting() {
        assert_eq!(highlight("from:", "fr"), [("fr", true), ("om:", false)]);
        assert_eq!(
            highlight("newsletter@freightsignals.com", "fr"),
            [
                ("newsletter@", false),
                ("fr", true),
                ("eightsignals.com", false)
            ]
        );
        assert_eq!(
            highlight("in:sent", "in:se"),
            [("in:se", true), ("nt", false)]
        );
        assert_eq!(highlight("Ana Ruiz", "zz"), [("Ana Ruiz", false)]);
    }
}
