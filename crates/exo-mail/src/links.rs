//! Links in a body, and which one unsubscribes.

use std::sync::LazyLock;

use regex::Regex;

use crate::{
    Link,
    content::Content,
    dom::{self, NodeRef},
    quotes::FORWARDED,
};

/// Marks the chosen unsubscribe link, which signature detection treats as a footer.
pub(crate) const UNSUBSCRIBE_CLASS: &str = "exo-unsubscribe-link";

static UNSUBSCRIBE_WORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("(?i)unsubscribe|unsub\\b|avregistrera|désinscription|se désinscrire|me désabonner|se désabonner|gérer abonnement|解除|メール配信を希望されな")
        .expect("valid regex")
});
static APPLE_NEWS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"mynews\.apple\.com/subscriptions").expect("valid regex"));
static ITALIAN_NEWSLETTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)non.*interessato.*newsletter").expect("valid regex"));
static CONTEXT_UNSUBSCRIBE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)unsubscribe|no longer.*receive.*emails").expect("valid regex")
});
static SETTINGS_TEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)((email|communication)\s*(settings|preference))|(disable.*notifications)|((update|manage).*subscription)|(^turn off$)|opt(-|\s*)out")
        .expect("valid regex")
});
static OPT_OUT_CONTEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)opt(-|\s*)out|(not|no longer|stop|do not want|don't want).*((receiv|get|interested).*(e-?mails?|updates))|(hear from (me|us|our))|(not |ir)relevant to you")
        .expect("valid regex")
});

/// Every link under `root`, and the best unsubscribe candidate (marked with
/// [`UNSUBSCRIBE_CLASS`]). `user_domains` are the account's own domains: a `mailto:` to them
/// is never an unsubscribe link.
pub(crate) fn find_links(
    root: &NodeRef,
    content: &Content,
    user_domains: &[String],
) -> (Vec<Link>, Option<Link>) {
    let forwarded = FORWARDED.is_match(&content.to_formatted_string());
    let mut links = Vec::new();
    let mut unsubscribe = None;
    let mut best_rank = u8::MAX;
    for anchor in dom::select(root, |n| dom::is(n, "a")) {
        let Some(url) = dom::attr(&anchor, "href").filter(|href| url::Url::parse(href).is_ok())
        else {
            continue;
        };
        let mut text = dom::text_content(&anchor).trim().to_owned();
        if text.is_empty() {
            text = dom::select_first(&anchor, |n| dom::is(n, "img"))
                .and_then(|image| dom::attr(&image, "alt"))
                .unwrap_or_default()
                .trim()
                .to_owned();
        }
        let link = Link {
            text,
            context: content.context_around(&anchor, 256).unwrap_or_default(),
            url,
        };
        let quoted = dom::closest(&anchor, |n| dom::is(n, "blockquote")).is_some();
        if let Some(rank) = unsubscribe_rank(&link, user_domains, quoted, forwarded)
            && rank <= best_rank
        {
            dom::add_class(&anchor, &[UNSUBSCRIBE_CLASS]);
            unsubscribe = Some(link.clone());
            best_rank = rank;
        }
        links.push(link);
    }
    (links, unsubscribe)
}

/// How likely a link is to unsubscribe, 1 (surely) to 4; `None` when it isn't one.
fn unsubscribe_rank(
    link: &Link,
    user_domains: &[String],
    quoted: bool,
    forwarded: bool,
) -> Option<u8> {
    let Link { text, context, url } = link;
    if forwarded || quoted || url.starts_with("https://mail.superhuman.com") {
        return None;
    }
    if url.starts_with("mailto:")
        && user_domains
            .iter()
            .any(|domain| url.ends_with(domain.as_str()) || !valid_unsubscribe_mailto(url))
    {
        return None;
    }
    if UNSUBSCRIBE_WORDS.is_match(text)
        || UNSUBSCRIBE_WORDS.is_match(url)
        || APPLE_NEWS.is_match(url)
        || ITALIAN_NEWSLETTER.is_match(context)
    {
        Some(1)
    } else if CONTEXT_UNSUBSCRIBE.is_match(context) {
        Some(2)
    } else if SETTINGS_TEXT.is_match(text) {
        Some(3)
    } else if OPT_OUT_CONTEXT.is_match(context) {
        Some(4)
    } else {
        None
    }
}

/// A `mailto:` with at least one recipient, all valid.
fn valid_unsubscribe_mailto(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("mailto:") else {
        return false;
    };
    let (to, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut recipients: Vec<String> = to
        .split(',')
        .map(crate::sanitize::decode_uri_component)
        .collect();
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if matches!(key.to_ascii_lowercase().as_str(), "to" | "cc" | "bcc") {
            recipients.extend(value.split(',').map(str::to_owned));
        }
    }
    let recipients: Vec<&str> = recipients
        .iter()
        .map(|r| r.trim())
        .filter(|r| !r.is_empty())
        .collect();
    !recipients.is_empty()
        && recipients
            .iter()
            .all(|r| crate::sanitize::VALID_EMAIL.is_match(address_of(r)))
}

/// The address in `Name <address>`.
fn address_of(contact: &str) -> &str {
    match (contact.rfind('<'), contact.rfind('>')) {
        (Some(open), Some(close)) if open < close => &contact[open + 1..close],
        _ => contact,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_unsubscribe_link() {
        let document = dom::parse(
            "<p>Read <a href=\"https://n.co/post\">more</a>.</p>\
             <p>Don't want these emails? <a href=\"https://n.co/u?id=1\">Unsubscribe</a> or \
             <a href=\"https://n.co/prefs\">update email preferences</a>.</p>",
        );
        let body = dom::body(&document).unwrap();
        let content = Content::new(&body);
        let (links, unsubscribe) = find_links(&body, &content, &[]);
        assert_eq!(links.len(), 3);
        assert_eq!(unsubscribe.unwrap().url, "https://n.co/u?id=1");
    }
}
