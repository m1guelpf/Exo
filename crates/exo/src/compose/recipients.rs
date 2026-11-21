//! Addresses as compose reads and writes them: parsing what's typed or pasted into a recipient
//! field, formatting a chip for editing, and what counts as a valid address.

use std::sync::LazyLock;

use exo_store::Address;
use regex::Regex;

/// The shape of a valid address.
static VALID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^[a-z0-9!#$%&'*+/=?^_`{|}~-]+(?:\.[a-z0-9!#$%&'*+/=?^_`{|}~-]+)*@(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+[a-z0-9](?:[a-z0-9-]*[a-z0-9])$").expect("valid regex")
});
/// `"Name" <address>`.
static NAMED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""?((?:[^\\"]|\\.)*)"?\s*<\s*([^<>]+@[^@<>]+)\s*>"#).expect("valid regex")
});
/// Name suffixes that keep a comma: `Smith, Jr` isn't `Jr Smith`.
static SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i),\s?(Jr|Junior|Sr|Senior|CEO|CPA|CSPO|DACVS|DDS|DVM|Esq|Inc|Director|Founder|Co-?founder|J\.?D|LLC|MBA|M\.?D|M\.?Ed|MBA|PCC|Ph\.?D|Psy\.?D|R\.?N)[\s.]*").expect("valid regex")
});
static VIA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[ ](\||[Vv]ia|[Ff]rom) .*").expect("valid regex"));
static TO_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^to:\s*").expect("valid regex"));

/// Gmail's limit on recipients per message.
pub(crate) const MAX_RECIPIENTS: usize = 100;

/// A valid address: non-empty, at most 900 characters, and shaped like an address.
pub(crate) fn is_valid(email: &str) -> bool {
    !email.is_empty() && email.len() <= 900 && VALID.is_match(email)
}

/// The same address, ignoring case.
pub(crate) fn same_address(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// What's typed as a recipient: `Name <address>`, `"Last, First" <address>` or a bare
/// address. Text without an `@` past its start becomes a name with no address: an invalid chip
/// showing it.
pub(crate) fn parse(text: &str) -> Address {
    if text.find('@').is_some_and(|at| at > 0) {
        let (email, name) = match NAMED.captures(text) {
            Some(captures) => {
                let email = captures[2].to_owned();
                let name = unescape(captures[1].trim());
                let name = if name == email { String::new() } else { name };
                (email, name)
            }
            None => (text.to_owned(), String::new()),
        };
        let name = sanitize_name(&name, true);
        return Address {
            name: Some(name).filter(|name| !name.is_empty()),
            email: email.trim().to_owned(),
        };
    }
    Address {
        name: Some(text.to_owned()).filter(|name| !name.is_empty()),
        email: String::new(),
    }
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            out.extend(chars.next());
        } else {
            out.push(ch);
        }
    }
    out
}

/// A contact's name tidied: `Last, First` becomes `First Last` (unless the part after the comma
/// is a suffix like `Jr`), and ` via …`, ` | …`, ` from …`, a trailing `(…)` or `,`, a leading
/// `to:` and surrounding quotes go.
pub(crate) fn sanitize_name(name: &str, strip_via: bool) -> String {
    let mut name = match name.find(", ") {
        Some(at) if !SUFFIX.is_match(name) => format!("{} {}", &name[at + 2..], &name[..at]),
        _ => name.to_owned(),
    };
    if strip_via {
        name = VIA.replace(&name, "").into_owned();
    }
    // Cut at the first `(`, or a trailing `,`, and a space before it.
    let cut = name.char_indices().find_map(|(at, ch)| match ch {
        '(' => Some(if name[..at].ends_with(' ') {
            at - 1
        } else {
            at
        }),
        ',' if at + 1 == name.len() => Some(if name[..at].ends_with(' ') {
            at - 1
        } else {
            at
        }),
        _ => None,
    });
    if let Some(cut) = cut {
        name.truncate(cut);
    }
    let name = TO_PREFIX.replace(&name, "").into_owned();
    for (open, close) in [('"', '"'), ('\'', '\''), ('“', '”'), ('‘', '’')] {
        if let Some(inner) = name.strip_prefix(open) {
            let inner = inner.strip_suffix(close).unwrap_or(inner);
            if !inner.is_empty() && !inner.contains(close) {
                return inner.to_owned();
            }
        }
    }
    name
}

/// A chip as text to edit: `Name <address>`, the name quoted (with `"` and `\` escaped) unless
/// it only has the characters an RFC 822 word may.
pub(crate) fn format(address: &Address) -> String {
    match (
        address.name.as_deref().filter(|name| !name.is_empty()),
        address.email.is_empty(),
    ) {
        (Some(name), false) => {
            let plain = name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || "$%&'*+.-/=?^_`{}|~ ".contains(ch));
            if plain {
                format!("{name} <{}>", address.email)
            } else {
                format!(
                    "\"{}\" <{}>",
                    name.replace('\\', "\\\\").replace('"', "\\\""),
                    address.email
                )
            }
        }
        (Some(name), true) => name.to_owned(),
        (None, _) => address.email.clone(),
    }
}

/// Pasted addresses: with a `<` in it, split at `,` `;` and newlines outside quotes;
/// otherwise at any of those or whitespace.
pub(crate) fn split_pasted(text: &str) -> Vec<String> {
    let parts: Vec<String> = if text.contains('<') {
        let mut parts = Vec::new();
        let mut current = String::new();
        let mut quoted = false;
        for ch in text.chars() {
            match ch {
                '"' => {
                    quoted = !quoted;
                    current.push(ch);
                }
                ',' | ';' | '\n' if !quoted => parts.push(std::mem::take(&mut current)),
                _ => current.push(ch),
            }
        }
        parts.push(current);
        parts
    } else {
        text.split([',', ';', ' ', '\t', '\n'])
            .map(str::to_owned)
            .collect()
    };
    parts
        .into_iter()
        .map(|part| part.trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect()
}

/// Whether `address` can be sent to: it has an address, and a valid one.
pub(crate) fn sendable(address: &Address) -> bool {
    is_valid(&address.email)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str, email: &str) -> Address {
        Address {
            name: Some(name.into()),
            email: email.into(),
        }
    }

    #[test]
    fn parses_what_people_type_and_paste() {
        assert_eq!(
            parse("ann@x.com"),
            Address {
                name: None,
                email: "ann@x.com".into()
            }
        );
        assert_eq!(parse("Ann Lee <ann@x.com>"), named("Ann Lee", "ann@x.com"));
        assert_eq!(
            parse("\"Lee, Ann\" <ann@x.com>"),
            named("Ann Lee", "ann@x.com")
        );
        assert_eq!(
            parse("\"Smith, Jr\" <j@x.com>"),
            named("Smith, Jr", "j@x.com")
        );
        assert_eq!(
            parse("ann@x.com <ann@x.com>"),
            Address {
                name: None,
                email: "ann@x.com".into()
            }
        );
        assert_eq!(parse("Ann via Docs <ann@x.com>"), named("Ann", "ann@x.com"));
        assert_eq!(
            parse("bob"),
            Address {
                name: Some("bob".into()),
                email: String::new()
            },
            "an invalid chip showing the text"
        );
    }

    #[test]
    fn validity_needs_an_address_with_a_dotted_domain() {
        assert!(is_valid("ann@x.com") && is_valid("a.b+c@sub.example.co"));
        assert!(
            !is_valid("")
                && !is_valid("ann")
                && !is_valid("ann@x")
                && !is_valid("ann@@x.com")
                && !is_valid(".ann@x.com")
        );
    }

    #[test]
    fn formats_names_that_need_quoting() {
        assert_eq!(
            format(&named("Ann Lee", "ann@x.com")),
            "Ann Lee <ann@x.com>"
        );
        assert_eq!(
            format(&named("Lee, Ann", "ann@x.com")),
            "\"Lee, Ann\" <ann@x.com>"
        );
        assert_eq!(
            format(&Address {
                name: None,
                email: "ann@x.com".into()
            }),
            "ann@x.com"
        );
    }

    #[test]
    fn splits_pasted_lists() {
        assert_eq!(
            split_pasted("a@x.com, b@x.com;c@x.com d@x.com"),
            ["a@x.com", "b@x.com", "c@x.com", "d@x.com"]
        );
        assert_eq!(
            split_pasted("\"Lee, Ann\" <ann@x.com>, Bob <b@x.com>"),
            ["\"Lee, Ann\" <ann@x.com>", "Bob <b@x.com>"]
        );
    }
}
