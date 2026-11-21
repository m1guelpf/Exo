//! Personal snippets in a draft: applying one with its variables, and how the pickers rank
//! them.

use exo_editor::{Pos, Selection};
use exo_store::{Address, Alias, Snippet};

use super::{contacts, draft::Open, form, recipients};
use crate::format::names;

/// Applies `snippet`: its recipients join the draft's, its alias becomes the From, its
/// subject fills an empty one (variables replaced), its body goes in at the caret (at the
/// start when `at_start`: ⌘; from a recipient field) and its files are added.
pub(crate) fn apply(
    open: &mut Open,
    snippet: &Snippet,
    aliases: &[Alias],
    book: Option<&contacts::Index>,
    at_start: bool,
    now: f64,
) {
    for (list, from) in [
        (&mut open.record.to, &snippet.to),
        (&mut open.record.cc, &snippet.cc),
        (&mut open.record.bcc, &snippet.bcc),
    ] {
        for address in from {
            if !list.iter().any(|kept| kept.email == address.email) {
                list.push(address.clone());
            }
        }
    }
    let alias = snippet.from.as_ref().and_then(|from| {
        aliases
            .iter()
            .find(|alias| alias.active && recipients::same_address(&alias.email, &from.email))
    });
    if let Some(alias) = alias {
        form::choose_from(open, aliases, alias, now);
    }
    if !snippet.cc.is_empty() || !snippet.bcc.is_empty() || alias.is_some() {
        open.state.header_expanded = true;
        open.state.cc_bcc_shown = true;
        open.state.field = super::Field::Body;
    }
    let values = values(open, snippet, book);
    if !snippet.subject.is_empty() && open.record.subject.is_empty() {
        open.record.subject = if open.record.in_reply_to.is_none() {
            replace_in_text(&snippet.subject, &values)
        } else {
            snippet.subject.clone()
        };
    }
    if at_start {
        open.editor.set_selection(Selection::caret(Pos::default()));
    }
    open.editor
        .insert_html(&body_to_insert(&replace_in_html(&snippet.body, &values)));
    open.editor.snippet_inserted();
    for attachment in &snippet.attachments {
        if !open
            .record
            .attachments
            .iter()
            .any(|kept| kept.uuid == attachment.uuid)
        {
            open.record.attachments.push(attachment.clone());
        }
    }
    open.applied_snippets.push(snippet.id.clone());
    open.touch(now);
}

/// The built-in variables and what they read for this draft: the recipient is the first of To
/// (and Cc, for a reply), the sender the From. A value that's empty, or only the address, isn't
/// replaced.
fn values(
    open: &Open,
    snippet: &Snippet,
    book: Option<&contacts::Index>,
) -> Vec<(&'static str, String)> {
    let reply = open.record.in_reply_to.is_some();
    let recipient = open
        .record
        .to
        .iter()
        .chain(open.record.cc.iter().filter(|_| reply))
        .next();
    let sender = open.record.from.as_ref().or(snippet.from.as_ref());
    let empty = Address {
        name: None,
        email: String::new(),
    };
    let (recipient, sender) = (recipient.unwrap_or(&empty), sender.unwrap_or(&empty));
    let mut values = Vec::new();
    for (prefix, person) in [("", recipient), ("sender_", sender)] {
        // Their name, or the address book's.
        let known = person
            .name
            .clone()
            .filter(|name| !name.is_empty())
            .or_else(|| book.and_then(|book| book.name_for(&person.email)))
            .unwrap_or_default();
        let first = if person.email.is_empty() {
            String::new()
        } else {
            names::first_name(&known, "")
        };
        // The last name: what follows the first name, else the whole.
        let full = names::reverse_by_comma(
            person
                .name
                .as_deref()
                .filter(|name| !name.is_empty())
                .unwrap_or(&person.email.to_lowercase())
                .trim(),
        );
        let shown_first = names::first_name(&full, &person.email);
        let last = full
            .split_once(shown_first.as_str())
            .map(|(_, rest)| rest.trim())
            .filter(|rest| !rest.is_empty())
            .unwrap_or(&full)
            .to_owned();
        let whole = person.name.clone().unwrap_or_default();
        for (name, value) in [
            ("first_name", first),
            ("last_name", last),
            ("full_name", whole),
        ] {
            if !value.is_empty() && value != person.email {
                values.push((variable(prefix, name), value));
            }
        }
    }
    values
}

fn variable(prefix: &str, name: &str) -> &'static str {
    match (prefix, name) {
        ("", "first_name") => "{first_name}",
        ("", "last_name") => "{last_name}",
        ("", "full_name") => "{full_name}",
        (_, "first_name") => "{sender_first_name}",
        (_, "last_name") => "{sender_last_name}",
        _ => "{sender_full_name}",
    }
}

/// `text` with each variable replaced, matched case-insensitively.
fn replace_in_text(text: &str, values: &[(&str, String)]) -> String {
    values
        .iter()
        .fold(text.to_owned(), |text, (variable, value)| {
            replace_all(&text, variable, value)
        })
}

/// `html` with the variables replaced in its text, never in its tags; one cut by a tag (half of
/// it bold) stays as it is: only a variable in a single text node is replaced. Values are
/// escaped.
fn replace_in_html(html: &str, values: &[(&str, String)]) -> String {
    let escaped: Vec<(&str, String)> = values
        .iter()
        .map(|(variable, value)| {
            (
                *variable,
                value
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;"),
            )
        })
        .collect();
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while !rest.is_empty() {
        let text_end = rest.find('<').unwrap_or(rest.len());
        out.push_str(&replace_in_text(&rest[..text_end], &escaped));
        rest = &rest[text_end..];
        let tag_end = rest.find('>').map_or(rest.len(), |end| end + 1);
        out.push_str(&rest[..tag_end]);
        rest = &rest[tag_end..];
    }
    out
}

/// `text` with every `from` replaced by `to`, ignoring case.
fn replace_all(text: &str, from: &str, to: &str) -> String {
    let lower = text.to_lowercase();
    if lower.len() != text.len() {
        // Lowercasing moved byte offsets (rare scripts): match exactly instead.
        return text.replace(from, to);
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(found) = lower[at..].find(from) {
        out.push_str(&text[at..at + found]);
        out.push_str(to);
        at += found + from.len();
    }
    out.push_str(&text[at..]);
    out
}

/// What a snippet's body inserts: a body that's one `<div>` of inline text (`span a b i u s tt
/// sup sub`, optionally ending in a `<br>`) joins the line at the caret instead of making its
/// own.
fn body_to_insert(body: &str) -> String {
    const INLINE: [&str; 9] = ["span", "a", "b", "i", "u", "s", "tt", "sup", "sub"];
    let inner = body
        .strip_prefix("<div>")
        .and_then(|inner| inner.strip_suffix("</div>"))
        .map(|inner| inner.strip_suffix("<br>").unwrap_or(inner));
    let Some(inner) = inner.filter(|inner| !inner.contains('\n')) else {
        return body.to_owned();
    };
    let tags = inner.split('<').skip(1).map(|tag| {
        tag.trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .next()
            .unwrap_or_default()
            .to_lowercase()
    });
    if tags.into_iter().all(|tag| INLINE.contains(&tag.as_str())) {
        inner.to_owned()
    } else {
        body.to_owned()
    }
}

/// What a snippet is called in lists: its name, or `-`.
pub(crate) fn name(snippet: &Snippet) -> &str {
    if snippet.name.trim().is_empty() {
        "-"
    } else {
        &snippet.name
    }
}

/// Its body as text, for the pickers' previews.
pub(crate) fn preview(snippet: &Snippet) -> String {
    super::plain::from_html(&snippet.body)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The inline picker's rows for `query`: with nothing typed, by when each was last used (10 for
/// one never used); otherwise by `command-score` on the name. Five at most; ties go by name,
/// later first.
pub(crate) fn rank<'a>(
    snippets: &'a [Snippet],
    last_used: &std::collections::BTreeMap<String, String>,
    query: &str,
) -> Vec<&'a Snippet> {
    let mut scored: Vec<(f64, &Snippet)> = snippets
        .iter()
        .map(|snippet| {
            let score = if query.is_empty() {
                last_used
                    .get(&snippet.id)
                    .and_then(|at| at.parse::<jiff::Timestamp>().ok())
                    .map_or(10.0, |at| at.as_millisecond() as f64)
            } else {
                let name: String = snippet.name.chars().take(200).collect();
                exo_commands::command_score(if name.is_empty() { "-" } else { &name }, query)
            };
            (score, snippet)
        })
        .filter(|(score, _)| *score > 0.0)
        .collect();
    scored.sort_by(|(a, first), (b, second)| {
        b.total_cmp(a).then_with(|| second.name.cmp(&first.name))
    });
    scored
        .into_iter()
        .take(5)
        .map(|(_, snippet)| snippet)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use exo_store::{DraftAction, DraftRecord};

    fn address(name: &str, email: &str) -> Address {
        Address {
            name: (!name.is_empty()).then(|| name.to_owned()),
            email: email.into(),
        }
    }

    fn draft(to: Vec<Address>, subject: &str) -> Open {
        let record = DraftRecord {
            action: DraftAction::Compose,
            from: Some(address("Maya Okafor", "maya@brightmoor.co")),
            to,
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: subject.into(),
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
        Open::new("draft00aaaaaaaaaaaaaa".into(), None, record)
    }

    #[test]
    fn variables_take_the_first_recipient_and_the_sender() {
        let snippet = Snippet {
            id: "s".into(),
            subject: "Hi {First_Name}".into(),
            body: "<div>Dear {first_name} {last_name}, from <b>{sender_first_name}</b> ({full_name})</div><div>{first_<b>name</b>} {custom}</div>".into(),
            cc: vec![address("", "dan@brightmoor.co")],
            ..Snippet::default()
        };
        let mut open = draft(vec![address("Tom Becker", "tom@brightmoor.co")], "");
        apply(&mut open, &snippet, &[], None, false, 0.0);
        assert_eq!(
            open.record.subject, "Hi Tom",
            "case-insensitive, in an empty subject"
        );
        let html = open.editor.html();
        assert!(
            html.contains("Dear Tom Becker, from <b>Maya</b> (Tom Becker)"),
            "{html}"
        );
        assert!(
            html.contains("{first_<b>name</b>}") && html.contains("{custom}"),
            "a cut variable and a placeholder stay: {html}"
        );
        assert_eq!(open.record.cc, [address("", "dan@brightmoor.co")]);
        assert!(
            open.state.header_expanded && open.state.cc_bcc_shown,
            "Cc shows it"
        );
    }

    #[test]
    fn unknown_names_stay_as_variables() {
        let snippet = Snippet {
            id: "s".into(),
            body: "<div>Hi {first_name}</div>".into(),
            ..Snippet::default()
        };
        let mut open = draft(vec![address("", "ops@x.com")], "Kept");
        apply(&mut open, &snippet, &[], None, false, 0.0);
        assert!(
            open.editor.html().contains("Hi {first_name}"),
            "{}",
            open.editor.html()
        );
        assert_eq!(open.record.subject, "Kept");
    }

    #[test]
    fn a_one_line_body_joins_the_caret_line() {
        assert_eq!(
            body_to_insert("<div>See <b>you</b><br></div>"),
            "See <b>you</b>"
        );
        assert_eq!(
            body_to_insert("<div>One</div><div>Two</div>"),
            "<div>One</div><div>Two</div>"
        );
        assert_eq!(
            body_to_insert("<div><ul><li>x</li></ul></div>"),
            "<div><ul><li>x</li></ul></div>"
        );
    }

    #[test]
    fn the_inline_picker_ranks_by_use_then_by_name() {
        let snippet = |id: &str, name: &str| Snippet {
            id: id.into(),
            name: name.into(),
            ..Snippet::default()
        };
        let snippets = [
            snippet("a", "intro"),
            snippet("b", "follow up"),
            snippet("c", "invoice"),
        ];
        let used =
            std::collections::BTreeMap::from([("b".to_owned(), "2026-11-17T10:00:00Z".to_owned())]);
        let names = |found: Vec<&Snippet>| {
            found
                .into_iter()
                .map(|snippet| snippet.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(rank(&snippets, &used, "")),
            ["follow up", "invoice", "intro"]
        );
        assert_eq!(
            names(rank(&snippets, &used, "in")),
            ["invoice", "intro"],
            "a tie, the later name first"
        );
    }
}
