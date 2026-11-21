//! Turning URLs and addresses in text into links, and letting long link text wrap.

use std::{ops::Range, sync::LazyLock};

use regex::Regex;

use crate::dom::{self, NodeRef};

/// Characters that end a URL: controls, whitespace, Latin-1 punctuation, general punctuation
/// and full-width forms.
const STOP: &str =
    r"\x{0000}-\x{001F}\s\x{0080}-\x{00BF}\x{2000}-\x{2012}\x{2014}-\x{206F}\x{FF01}-\x{FF60}";
const SCHEME: &str = r"[a-z][a-z0-9]*://";
const USER_INFO: &str = r"(?:(?:[-_.!~*'()a-zA-Z\d;:&=+$,]|%[a-fA-F0-9]{2})*@)?";
const IPV4: &str = r"[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}";
const IPV6: &str = r"\[(?:(?:[a-fA-F\d]{1,4}:)*(?:[a-fA-F\d]{1,4}|\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3})|(?:(?:[a-fA-F\d]{1,4}:)*[a-fA-F\d]{1,4})?::(?:(?:[a-fA-F\d]{1,4}:)*(?:[a-fA-F\d]{1,4}|\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}))?)\]";

/// Punctuation around URLs, inside a character class.
fn punctuation() -> String {
    format!(r#"`!()\[\]\{{\}};:'".,<>?{STOP}"#)
}

/// Anything that might be a URL.
static CANDIDATE: LazyLock<Regex> = LazyLock::new(|| {
    let b = punctuation();
    let host = format!(r"(?:[^/{b}]+\.)*[^/'{b}]+");
    Regex::new(&format!(
        r"(?i)(?:{SCHEME}{USER_INFO}{host}|{SCHEME}{USER_INFO}{IPV4}|{SCHEME}{USER_INFO}{IPV6}|{host})(?::[0-9]+)?(?:[/#?](?:(?:[^<>{STOP}]*|[^{b}]+)+)?)?;?"
    ))
    .expect("valid regex")
});

/// A known top-level domain followed by the end of the host.
static TOP_LEVEL_DOMAIN: LazyLock<Regex> = LazyLock::new(|| {
    let tlds: Vec<&str> = include_str!("tlds.txt").lines().collect();
    Regex::new(&format!(
        r"(?i)\.({})([&?#:/{}]|$)",
        tlds.join("|"),
        punctuation()
    ))
    .expect("valid regex")
});
static IP_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!("{SCHEME}{USER_INFO}{IPV4}")).expect("valid regex"));
static IPV6_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(IPV6).expect("valid regex"));
/// An address. Its local part may hold every character from `+` to `/`, `,` and `.` included.
static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([a-zA-Z0-9!#$%&'*+,\-./=?^_`{|}~]+@[a-zA-Z0-9_-]+(?:\.[a-zA-Z0-9_-]+)+)$")
        .expect("valid regex")
});
static TRAILING_PUNCTUATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"["'!?.,:;)\]}>]+$"#).expect("valid regex"));
static NOT_A_URL_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\+|@|-|\x{2212}|\x{2013}|\x{2014})").expect("valid regex"));
static HTTP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^https?://").expect("valid regex"));
static ANY_SCHEME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9]+://").expect("valid regex"));
static DEFAULT_HTTP_PORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(http://[^/]+):80([^0-9]|$)").expect("valid regex"));
static DEFAULT_HTTPS_PORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(https://[^/]+):443([^0-9]|$)").expect("valid regex"));
static BARE_HOST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(https?://[^/#?]+)(\?|#|$)").expect("valid regex"));
static ORIGIN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^https?://[^/]+").expect("valid regex"));

/// Links URLs in every text node under `root`, except inside links and stylesheets
/// (`Linkify.element`).
pub(crate) fn linkify(root: &NodeRef) {
    let mut linked = Vec::new();
    collect_links(root, &mut linked);
    dom::replace_all(linked);
}

fn collect_links(root: &NodeRef, linked: &mut Vec<(NodeRef, Vec<NodeRef>)>) {
    for child in dom::children(root) {
        if let Some(text) = dom::text(&child) {
            if let Some(pieces) = linkify_text(&text) {
                linked.push((child, pieces));
            }
        } else if dom::is_element(&child) && !dom::is(&child, "a") && !dom::is(&child, "style") {
            collect_links(&child, linked);
        }
    }
}

/// The links a text node gets when a message renders or is sent: each URL's byte range in `text`,
/// without the punctuation that follows it, and the address it opens as a browser's `URL` prints
/// it.
pub fn links(text: &str) -> Vec<(Range<usize>, String)> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    CANDIDATE
        .find_iter(text)
        .filter_map(|found| {
            let (url_text, _) = separate_tail(found.as_str());
            let href = absolute_url(url_text).and_then(|url| browser_url(&url))?;
            Some((found.start()..found.start() + url_text.len(), href))
        })
        .collect()
}

/// The first link in `text` as the compose editor's autolinking finds it: the URL's byte range
/// and its address, not normalized. Text that is entirely an email address is one link.
pub fn match_link(text: &str) -> Option<(Range<usize>, String)> {
    if text.starts_with('\u{2212}') {
        return None;
    }
    if EMAIL.is_match(text)
        && let Some(href) = absolute_url(text)
    {
        return Some((0..text.len(), href));
    }
    CANDIDATE.find_iter(text).find_map(|found| {
        let (url_text, _) = separate_tail(found.as_str());
        let href = absolute_url(url_text)?;
        Some((found.start()..found.start() + url_text.len(), href))
    })
}

/// The text and links `text` becomes, if it has a URL.
fn linkify_text(text: &str) -> Option<Vec<NodeRef>> {
    let found = links(text);
    if found.is_empty() {
        return None;
    }
    let mut pieces: Vec<NodeRef> = Vec::new();
    let mut last = 0;
    for (range, href) in found {
        if range.start > last {
            pieces.push(dom::text_node(&text[last..range.start]));
        }
        let mut attributes = vec![("target", "_blank")];
        if href.starts_with("http:") || href.starts_with("https:") || href.starts_with("mailto:") {
            attributes.push(("rel", "noopener noreferrer"));
        }
        attributes.push(("href", &href));
        let link = dom::element("a", &attributes);
        dom::append(&link, &dom::text_node(&text[range.clone()]));
        pieces.push(link);
        last = range.end;
    }
    if last < text.len() {
        pieces.push(dom::text_node(&text[last..]));
    }
    Some(pieces)
}

/// The URL as a browser's `URL` would print it, after fixing `http:/x` to `http://x`.
fn browser_url(url: &str) -> Option<String> {
    static ONE_SLASH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^http(s)?:(?:([^/])|/([^/]))").expect("valid regex"));
    let fixed = ONE_SLASH.replace(url, |found: &regex::Captures| {
        let s = found.get(1).map_or("", |m| m.as_str());
        let next = found.get(2).or(found.get(3)).map_or("", |m| m.as_str());
        format!("http{s}://{next}")
    });
    url::Url::parse(&fixed).ok().map(String::from)
}

/// Splits trailing punctuation off a URL, keeping closing brackets that match openers inside
/// it.
fn separate_tail(candidate: &str) -> (&str, &str) {
    let Some(found) = TRAILING_PUNCTUATION.find(candidate) else {
        return (candidate, "");
    };
    let (body, tail) = candidate.split_at(found.start());
    let mut expected: Vec<char> = Vec::new();
    for c in body.chars() {
        match c {
            '(' => expected.push(')'),
            '{' => expected.push('}'),
            '<' => expected.push('>'),
            '[' => expected.push(']'),
            _ if expected.last() == Some(&c) => {
                expected.pop();
            }
            _ => {}
        }
    }
    let mut kept = 0;
    for (i, c) in tail.char_indices() {
        if expected.last() == Some(&c) {
            expected.pop();
            kept = i + c.len_utf8();
        }
    }
    candidate.split_at(body.len() + kept)
}

/// The URL a candidate stands for, if it is one.
fn absolute_url(candidate: &str) -> Option<String> {
    let has_separator = candidate.contains(['.', ':']);
    if !has_separator || NOT_A_URL_START.is_match(candidate) {
        return None;
    }
    if !(TOP_LEVEL_DOMAIN.is_match(candidate)
        || IP_URL.is_match(candidate)
        || IPV6_URL.is_match(candidate))
    {
        return None;
    }
    if HTTP.is_match(candidate) {
        return Some(clean_http_url(candidate));
    }
    if EMAIL.is_match(candidate) {
        return Some(format!("mailto:{candidate}"));
    }
    if !ANY_SCHEME.is_match(candidate) && candidate.contains('.') {
        return Some(clean_http_url(&format!("http://{candidate}")));
    }
    None
}

fn clean_http_url(url: &str) -> String {
    let url = DEFAULT_HTTP_PORT.replace(url, "$1$2");
    let url = DEFAULT_HTTPS_PORT.replace(&url, "$1$2");
    let url = BARE_HOST.replace(&url, "$1/$2");
    ORIGIN
        .replace(&url, |found: &regex::Captures| found[0].to_lowercase())
        .into_owned()
}

/// Lets link text break after `.`, `@` and `/` by inserting `<wbr>` there, for links whose
/// content is a single text node.
pub(crate) fn add_word_breaks(root: &NodeRef) {
    for link in dom::select(root, |n| dom::is(n, "a")) {
        dom::normalize(&link);
        let Some(text_node) = only_text(&link) else {
            continue;
        };
        let text = dom::text(&text_node).unwrap_or_default();
        let mut pieces = Vec::new();
        let mut start = 0;
        for (i, separator) in text.match_indices(['.', '@', '/']) {
            if start < i {
                pieces.push(dom::text_node(&text[start..i]));
            }
            pieces.push(dom::text_node(separator));
            pieces.push(dom::element("wbr", &[]));
            start = i + separator.len();
        }
        if start < text.len() {
            pieces.push(dom::text_node(&text[start..]));
        }
        dom::replace_all(vec![(text_node, pieces)]);
        dom::normalize(&link);
    }
}

/// The text node `link`'s content comes down to through only children, within 11 levels.
fn only_text(link: &NodeRef) -> Option<NodeRef> {
    let mut node = link.clone();
    for _ in 0..=11 {
        if dom::is_text(&node) {
            return Some(node);
        }
        let [child] = dom::children(&node).try_into().ok()?;
        node = child;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linked(html: &str) -> String {
        let document = dom::parse(html);
        let body = dom::body(&document).unwrap();
        linkify(&body);
        add_word_breaks(&body);
        dom::inner_html(&body)
    }

    #[test]
    fn links_urls_domains_and_addresses() {
        assert_eq!(
            linked("see https://example.com/a?b=1."),
            "see <a target=\"_blank\" rel=\"noopener noreferrer\" href=\"https://example.com/a?b=1\">https:/<wbr>/<wbr>example.<wbr>com/<wbr>a?b=1</a>."
        );
        assert_eq!(
            linked("mail ann@x.co or visit acme.io"),
            "mail <a target=\"_blank\" rel=\"noopener noreferrer\" href=\"mailto:ann@x.co\">ann@<wbr>x.<wbr>co</a> or visit \
             <a target=\"_blank\" rel=\"noopener noreferrer\" href=\"http://acme.io/\">acme.<wbr>io</a>"
        );
    }

    #[test]
    fn leaves_non_urls_and_existing_links() {
        assert_eq!(
            linked("version 1.2 and file.txtx"),
            "version 1.2 and file.txtx"
        );
        assert_eq!(
            linked("<a href=\"https://a.com\">a.com</a>"),
            "<a href=\"https://a.com\">a.<wbr>com</a>"
        );
    }

    #[test]
    fn autolinking_matches_one_unnormalized_link() {
        // `links` prints the address as a URL; `match_link` leaves it as written.
        assert_eq!(
            match_link("go to Example.com/ü now"),
            Some((6..20, "http://example.com/ü".to_owned()))
        );
        assert_eq!(
            links("go to Example.com/ü now"),
            [(6..20, "http://example.com/%C3%BC".to_owned())]
        );
        // A text that is only an address is linked whole, punctuation and all.
        assert_eq!(
            match_link("a!b@x.co"),
            Some((0..8, "mailto:a!b@x.co".to_owned()))
        );
        assert_eq!(links("a!b@x.co"), [(2..8, "mailto:b@x.co".to_owned())]);
        assert_eq!(match_link("\u{2212}x.com"), None);
        // A comma is part of an address.
        assert_eq!(
            match_link("a,b@x.co"),
            Some((0..8, "mailto:a,b@x.co".to_owned()))
        );
        assert_eq!(
            links("x/a,b@x.co"),
            [(0..10, "mailto:x/a,b@x.co".to_owned())]
        );
        assert_eq!(match_link("version 1.2"), None);
    }

    #[test]
    fn keeps_balanced_brackets() {
        assert_eq!(
            separate_tail("https://w.org/A_(b))."),
            ("https://w.org/A_(b)", ").")
        );
    }
}
