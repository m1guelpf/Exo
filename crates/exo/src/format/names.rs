//! How people are named: first names and display formats. Which participants a row lists is
//! exo-store's.

use std::sync::LazyLock;

use regex::Regex;

use super::lists::{POPULAR_DOMAINS, ROLE_USERNAMES};

/// A name and address, as a message header gives them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    /// Display name; empty if the header had none.
    pub name: String,
    /// Email address.
    pub email: String,
}

impl Person {
    /// A person from a name and an address.
    pub fn new(name: impl Into<String>, email: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            email: email.into(),
        }
    }
}

/// Whether `email`'s local part names a role (`info@`, `dev@`, `no-reply@`) rather than a person.
pub fn is_role_based(email: &str) -> bool {
    static NO_REPLY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:no[-_]?reply|do[-_]?not[-_]?reply|auto[-_]?(?:reply|confirm|shipping|email|mated)|expiry[-_]?notice|paper[-_]?free)")
            .expect("valid regex")
    });
    let user = email.to_lowercase();
    let user = user.split('@').next().unwrap_or_default();
    ROLE_USERNAMES.binary_search(&user).is_ok() || NO_REPLY.is_match(user)
}

/// Whether `domain` is a webmail provider (no company to link to).
pub fn is_popular_domain(domain: &str) -> bool {
    POPULAR_DOMAINS.binary_search(&domain).is_ok()
}

/// `Doe, Jane` → `Jane Doe`, unless the part after the comma is a suffix.
pub fn reverse_by_comma(name: &str) -> String {
    static LAST_FIRST: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[^,\s]+,\s[^,]+$").expect("valid regex"));
    static SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i),\s?(Jr|Junior|Sr|Senior|CEO|CPA|CSPO|DACVS|DDS|DVM|Esq|Inc|Director|Founder|Co-?founder|J\.?D|LLC|MBA|M\.?D|M\.?Ed|MBA|PCC|Ph\.?D|Psy\.?D|R\.?N)[\s.]*")
            .expect("valid regex")
    });
    const PARTICLES: [&str; 11] = [
        "van", "de", "da", "di", "le", "la", "du", "des", "del", "della", "vander",
    ];
    let starts_with_particle = PARTICLES
        .iter()
        .any(|particle| name.starts_with(&format!("{particle} ")));
    if (LAST_FIRST.is_match(name) || starts_with_particle) && !SUFFIX.is_match(name) {
        let mut parts: Vec<&str> = name.split(", ").collect();
        parts.reverse();
        parts.join(" ")
    } else {
        name.to_owned()
    }
}

/// The name to call someone by.
pub fn first_name(name: &str, email: &str) -> String {
    static HONORIFIC: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(Mr|Mrs|Miss|Ms)(\.|\s)").expect("valid regex"));
    static TO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^to:\s*").expect("valid regex"));
    static TITLE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(Dr|Rev|The Rev|Prof|Capt|Rabbi)(\s|\.|\.\s)([^\s]+)").expect("valid regex")
    });
    static DOUBLE_QUOTED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"^(?:[^"]*)(?:\s|^)"([^"]+)"(?:\s|$)(?:[^"]*)$"#).expect("valid regex")
    });
    static SINGLE_QUOTED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:[^']*)(?:\s|^)'([^']+)'(?:\s|$)(?:[^']*)$").expect("valid regex")
    });
    static SEPARATOR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[,\s()]").expect("valid regex"));

    match name {
        "Jon Jon Wesolowski" => return "Jon Jon".into(),
        "Seu Mei Fung" => return "Seu Mei".into(),
        _ => {}
    }
    if !email.is_empty() && is_role_based(email) {
        return name.to_owned();
    }
    let name = HONORIFIC.replace_all(name, "");
    let name = TO.replace(name.trim(), "").into_owned();
    let title = TITLE.find(&name).map(|title| title.as_str().to_owned());
    let name = reverse_by_comma(&name);
    let first = if let Some(quoted) = DOUBLE_QUOTED
        .captures(&name)
        .or_else(|| SINGLE_QUOTED.captures(&name))
    {
        quoted[1].to_owned()
    } else if let Some(title) = title {
        title
    } else {
        let lower = name.to_lowercase();
        if SEPARATOR
            .split(&lower)
            .any(|word| ["of", "the", "&", "and", "+"].contains(&word))
        {
            return name;
        }
        SEPARATOR.split(&name).next().unwrap_or_default().to_owned()
    };
    if first.replacen('.', "", 1).encode_utf16().count() < 2 {
        name
    } else {
        first
    }
}

/// Which display format to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameFormat {
    /// The default: first name, or `Me`.
    First,
    /// `fullname-excluding-me`: the whole name, or `Me`.
    Full,
}

/// A person's display name. Without a name, the lower-cased address stands in for it, and
/// `shorten_email` cuts that to its local part.
pub fn display_name(
    person: &Person,
    is_me: bool,
    format: NameFormat,
    shorten_email: bool,
) -> String {
    if is_me {
        return "Me".into();
    }
    let has_name = !person.name.is_empty();
    let full = if has_name {
        person.name.clone()
    } else {
        person.email.to_lowercase()
    };
    let full = reverse_by_comma(full.trim());
    let first = first_name(&full, &person.email);
    let shorten = |name: String| {
        if shorten_email && !has_name {
            name.split('@').next().unwrap_or_default().to_owned()
        } else {
            name
        }
    };
    match format {
        NameFormat::First => shorten(first),
        NameFormat::Full => shorten(full),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_names() {
        assert_eq!(first_name("Tom Becker", "tom@brightmoor.co"), "Tom");
        assert_eq!(first_name("Becker, Tom", ""), "Tom");
        assert_eq!(first_name("Dr. Jane Doe", ""), "Dr. Jane");
        assert_eq!(first_name("Jane \"JJ\" Doe", ""), "JJ");
        assert_eq!(first_name("Bank of Nowhere", ""), "Bank of Nowhere");
        assert_eq!(
            first_name("J Doe", ""),
            "J Doe",
            "one-letter first names keep the whole name"
        );
        assert_eq!(
            first_name("Dev Malhotra", "dev@callowayfreight.com"),
            "Dev Malhotra",
            "role-based addresses keep the whole name"
        );
    }

    #[test]
    fn display_names() {
        let tom = Person::new("Tom Becker", "tom@brightmoor.co");
        assert_eq!(display_name(&tom, false, NameFormat::First, true), "Tom");
        assert_eq!(
            display_name(&tom, false, NameFormat::Full, false),
            "Tom Becker"
        );
        assert_eq!(display_name(&tom, true, NameFormat::First, true), "Me");
        let anonymous = Person::new("", "Someone@Example.com");
        assert_eq!(
            display_name(&anonymous, false, NameFormat::First, true),
            "someone"
        );
        assert_eq!(
            display_name(&anonymous, false, NameFormat::Full, false),
            "someone@example.com"
        );
    }
}
