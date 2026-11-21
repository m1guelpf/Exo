//! What a conversation's Unsubscribe does: the first message with a `List-Unsubscribe` header Exo
//! can use, else the unsubscribe link a rendered body has; whom that unsubscribes from; and how to
//! find all of their mail.
//!
//! Every target is checked first, since someone else wrote it and Exo acts on it for you: a
//! `mailto:` must name one valid address and nothing else to send to, and a web link must be
//! `http`/`https` with no credentials and no control characters. Anything else is left out, as if
//! the header didn't list it.

use exo_store::{MessageDetail, ThreadDetail};
use url::Url;

use crate::compose::recipients;

/// What Unsubscribe does for a conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Unsubscribe {
    /// How it unsubscribes.
    pub target: Target,
    /// Whom from (`sender`): the mailing list the mail came to, else whoever sent it.
    pub sender: String,
    /// The list's name, which the alerts name when there's no sender.
    pub list_name: String,
    /// The message it was found in, whose `Delivered-To` alias a `mailto:` goes from.
    pub message: String,
    /// The search for all of their mail: `to:"{list}"` or `from:"{sender}"`.
    pub query: String,
}

/// How an unsubscribe happens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// Email `to`, with `subject`.
    Mailto {
        /// The one address the link names.
        to: String,
        /// The link's subject on one line, or `Unsubscribe`.
        subject: String,
    },
    /// POST `List-Unsubscribe=One-Click` to it (RFC 8058; `List-Unsubscribe-Post`).
    OneClick(Url),
    /// Open it in the browser.
    Web(Url),
}

/// `{name} <{address}>` inside `List-Id`, and an address without its name.
fn list_id_address(list_id: &str) -> &str {
    match list_id.split_once('<') {
        Some((_, rest)) => rest
            .split_once('>')
            .map_or(rest, |(address, _)| address)
            .trim(),
        None => list_id.trim(),
    }
}

/// The links of a `List-Unsubscribe` header: each `<…>` of the comma-separated list, scheme
/// normalized, `http:`, `https:` and `mailto:` only.
fn parse_links(header: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    let mut split = |part: &str| {
        let inner = part.split_once('<').map_or(part, |(_, rest)| rest);
        let inner = inner
            .rsplit_once('>')
            .map_or(inner, |(link, _)| link)
            .trim();
        let lower = inner.to_ascii_lowercase();
        let link = match lower.strip_prefix("mailto:") {
            Some(_) => format!("mailto:{}", inner[7..].trim_start()),
            None if lower.starts_with("https:") => format!("https{}", &inner[5..]),
            None if lower.starts_with("http:") => format!("http{}", &inner[4..]),
            None => return,
        };
        links.push(link);
    };
    for (at, ch) in header.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                split(&header[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    split(&header[start..]);
    links
}

/// `link` as a target Exo will act on, `one_click` when the message allows a one-click POST.
fn target(link: &str, one_click: bool) -> Option<Target> {
    // A line break or other control character has no business in a link (`Url` would drop
    // tabs and newlines silently).
    if gmail::mime::one_line(link) != link {
        return None;
    }
    if link
        .get(..7)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("mailto:"))
    {
        return mailto(link);
    }
    let url = Url::parse(link).ok().filter(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.has_host()
            && url.username().is_empty()
            && url.password().is_none()
    })?;
    Some(if one_click {
        Target::OneClick(url)
    } else {
        Target::Web(url)
    })
}

/// A `mailto:` naming exactly one valid address and no Cc or Bcc: one is all an unsubscribe needs,
/// and more would let the sender have your account write to whomever they like.
fn mailto(link: &str) -> Option<Target> {
    let fields = crate::compose::mailto_fields(link)?;
    let named = |list: &[exo_store::Address]| {
        list.iter()
            .any(|address| !address.email.is_empty() || address.name.is_some())
    };
    if named(&fields.cc) || named(&fields.bcc) {
        return None;
    }
    let [to] = &fields.to[..] else { return None };
    if !recipients::is_valid(&to.email) {
        return None;
    }
    let subject = gmail::mime::one_line(&fields.subject);
    let subject = if subject.is_empty() {
        "Unsubscribe".to_owned()
    } else {
        subject
    };
    Some(Target::Mailto {
        to: to.email.clone(),
        subject,
    })
}

/// The mailing list `message` came through: its last `X-BeenThere`, else its `Mailing-List`'s
/// address.
fn mailing_list(message: &MessageDetail) -> Option<String> {
    let header = |name: &'static str| {
        message
            .headers
            .iter()
            .filter(move |header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.as_str())
    };
    let been_there = header("X-BeenThere").next_back().map(|value| {
        value
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned()
    });
    let mailing_list = || {
        header("Mailing-List").find_map(|value| {
            let address = value.split_once("list ")?.1.split_once(';')?.0;
            recipients::is_valid(address).then(|| address.to_owned())
        })
    };
    been_there
        .filter(|address| !address.is_empty())
        .or_else(mailing_list)
}

/// Whom `message`'s unsubscribe is from, how the alerts name them, and the search for all
/// their mail: the mailing list when the mail was addressed to it, else the sender.
fn sender(message: &MessageDetail) -> (String, String, String) {
    let to_list = mailing_list(message).filter(|list| {
        message
            .to
            .iter()
            .chain(&message.cc)
            .chain(&message.bcc)
            .any(|recipient| recipients::same_address(&recipient.email, list))
    });
    match to_list {
        Some(list) => (list.clone(), list.clone(), format!("to:\"{list}\"")),
        None => {
            let from = message.from.as_ref();
            let email = from.map(|from| from.email.clone()).unwrap_or_default();
            let name = from
                .and_then(|from| from.name.clone())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| email.clone());
            (email.clone(), name, format!("from:\"{email}\""))
        }
    }
}

/// `message`'s header unsubscribe: none for a draft, or for a list of your own domains (`List-Id`
/// at one of `own_domains`). A `mailto:` comes first; then, for one-click, an `https:` link; then
/// whatever's first.
fn from_header(message: &MessageDetail, own_domains: &[String]) -> Option<Unsubscribe> {
    if message.is_draft() {
        return None;
    }
    let header = message.list_unsubscribe.as_deref()?;
    if let Some(list) = message
        .list_id
        .as_deref()
        .map(list_id_address)
        .filter(|list| !list.is_empty())
        && own_domains
            .iter()
            .any(|domain| list.ends_with(domain.as_str()))
    {
        return None;
    }
    let one_click = message
        .list_unsubscribe_post
        .as_deref()
        .is_some_and(|post| post.trim() == "List-Unsubscribe=One-Click");
    let links: Vec<(String, Target)> = parse_links(header)
        .into_iter()
        .filter_map(|link| Some((link.clone(), target(&link, one_click)?)))
        .collect();
    let chosen = links
        .iter()
        .find(|(_, target)| matches!(target, Target::Mailto { .. }))
        .or_else(|| {
            links
                .iter()
                .find(|(link, _)| one_click && link.starts_with("https:"))
        })
        .or_else(|| links.first())?;
    let (sender, list_name, query) = sender(message);
    Some(Unsubscribe {
        target: chosen.1.clone(),
        sender,
        list_name,
        message: message.id.clone(),
        query,
    })
}

/// What Unsubscribe does for `detail`: the first message's header unsubscribe, else the first
/// rendered body's unsubscribe link (`body_link` of a message id), which is never one-click.
/// `own_domains` are the account's.
pub(crate) fn find(
    detail: &ThreadDetail,
    own_domains: &[String],
    body_link: impl Fn(&str) -> Option<String>,
) -> Option<Unsubscribe> {
    let messages = || detail.messages.iter().filter(|message| !message.is_draft());
    messages()
        .find_map(|message| from_header(message, own_domains))
        .or_else(|| {
            messages().find_map(|message| {
                let target = target(&body_link(&message.id)?, false)?;
                let from = message.from.as_ref();
                let email = from.map(|from| from.email.clone()).unwrap_or_default();
                let list_name = from
                    .and_then(|from| from.name.clone())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| email.clone());
                Some(Unsubscribe {
                    target,
                    list_name,
                    query: format!("from:\"{email}\""),
                    sender: email,
                    message: message.id.clone(),
                })
            })
        })
}

#[cfg(test)]
mod tests {
    use exo_store::{Contact, Header, ThreadId};

    use super::*;

    fn contact(email: &str, name: Option<&str>) -> Contact {
        Contact {
            name: name.map(str::to_owned),
            email: email.into(),
            is_me: false,
        }
    }

    fn message(id: &str, unsubscribe: Option<&str>, post: bool) -> MessageDetail {
        MessageDetail {
            id: id.into(),
            labels: vec!["INBOX".into()],
            date: jiff::Timestamp::UNIX_EPOCH,
            from: Some(contact("news@shop.example", Some("Shop"))),
            to: vec![contact("maya@brightmoor.co", None)],
            cc: Vec::new(),
            bcc: Vec::new(),
            reply_to: Vec::new(),
            subject: "Deals".into(),
            snippet: String::new(),
            rfc822_id: None,
            in_reply_to: None,
            references: Vec::new(),
            list_id: Some("Shop News <news.shop.example>".into()),
            list_unsubscribe: unsubscribe.map(str::to_owned),
            list_unsubscribe_post: post.then(|| "List-Unsubscribe=One-Click".to_owned()),
            headers: Vec::new(),
            body: exo_store::Body::default(),
            attachments: Vec::new(),
            failure: None,
            scheduled: None,
            sending: false,
        }
    }

    fn thread(messages: Vec<MessageDetail>) -> ThreadDetail {
        ThreadDetail {
            id: ThreadId::new("t1".to_owned()),
            subject: "Deals".into(),
            labels: vec!["INBOX".into()],
            muted: false,
            messages,
            reminder: None,
        }
    }

    fn found(header: &str, post: bool) -> Option<Target> {
        find(
            &thread(vec![message("m1", Some(header), post)]),
            &[],
            |_| None,
        )
        .map(|found| found.target)
    }

    fn web(url: &str) -> Url {
        Url::parse(url).expect("a URL")
    }

    #[test]
    fn reads_each_bracketed_link_with_a_web_or_mailto_scheme() {
        assert_eq!(
            parse_links("<mailto:u@list.example?subject=unsub>, <https://list.example/u/1>"),
            [
                "mailto:u@list.example?subject=unsub",
                "https://list.example/u/1"
            ]
        );
        assert_eq!(
            parse_links(" < MAILTO: u@x.example > ,<HTTPS://x.example/a,b>"),
            ["mailto:u@x.example", "https://x.example/a,b"]
        );
        assert!(parse_links("<ftp://x.example>, garbage").is_empty());
    }

    #[test]
    fn prefers_mailto_then_https_for_one_click() {
        let both = "<https://list.example/u/1>, <mailto:u@list.example?subject=unsub-1>";
        assert_eq!(
            found(both, true),
            Some(Target::Mailto {
                to: "u@list.example".into(),
                subject: "unsub-1".into()
            })
        );
        assert_eq!(
            found("<http://list.example/a>, <https://list.example/b>", true),
            Some(Target::OneClick(web("https://list.example/b")))
        );
        assert_eq!(
            found("<http://list.example/a>, <https://list.example/b>", false),
            Some(Target::Web(web("http://list.example/a")))
        );
        assert_eq!(
            found("<mailto:u@list.example>", false),
            Some(Target::Mailto {
                to: "u@list.example".into(),
                subject: "Unsubscribe".into()
            })
        );
    }

    #[test]
    fn a_mailto_names_one_address_and_nothing_else_to_send_to() {
        for crafted in [
            "<mailto:u@list.example?bcc=victim@else.example>",
            "<mailto:u@list.example?cc=victim@else.example>",
            "<mailto:u@list.example,victim@else.example>",
            "<mailto:u@list.example?to=victim@else.example>",
            "<mailto:?subject=x>",
            "<mailto:not-an-address>",
            "<mailto:u@list.example%0D%0ABcc:%20victim@else.example>",
        ] {
            assert_eq!(found(crafted, false), None, "{crafted}");
        }
        // A crafted mailto is left out, and the next link used.
        assert_eq!(
            found(
                "<mailto:u@list.example?bcc=v@else.example>, <https://list.example/u>",
                true
            ),
            Some(Target::OneClick(web("https://list.example/u")))
        );
        // A line break in the subject can't start a header.
        let Some(Target::Mailto { subject, .. }) = found(
            "<mailto:u@list.example?subject=hi%0D%0ABcc:%20v@else.example%00>",
            false,
        ) else {
            panic!("a mailto")
        };
        assert_eq!(subject, "hiBcc: v@else.example");
    }

    #[test]
    fn a_web_link_is_http_without_credentials() {
        for crafted in [
            "<https://user:pass@list.example/u>",
            "<https://list.example/u\r\nX: y>",
            "<https://list.example/u\tv>",
            "<javascript:alert(1)>",
            "<file:///etc/passwd>",
            "<https://>",
        ] {
            assert_eq!(found(crafted, true), None, "{crafted:?}");
        }
        // Spaces are escaped, as the browser would.
        assert_eq!(
            found("<https://list.example/a b>", true),
            Some(Target::OneClick(web("https://list.example/a%20b")))
        );
    }

    #[test]
    fn mail_to_a_list_unsubscribes_from_the_list() {
        let mut to_list = message("m1", Some("<https://lists.example/u>"), false);
        to_list.to = vec![contact("team@lists.example", None)];
        to_list.headers = vec![Header {
            name: "X-BeenThere".into(),
            value: "team@lists.example; contact admin".into(),
        }];
        let found = find(&thread(vec![to_list]), &[], |_| None).expect("an unsubscribe");
        assert_eq!(
            (found.sender.as_str(), found.query.as_str()),
            ("team@lists.example", "to:\"team@lists.example\"")
        );

        let found = find(
            &thread(vec![message(
                "m1",
                Some("<https://lists.example/u>"),
                false,
            )]),
            &[],
            |_| None,
        )
        .expect("an unsubscribe");
        assert_eq!(
            (
                found.sender.as_str(),
                found.list_name.as_str(),
                found.query.as_str()
            ),
            ("news@shop.example", "Shop", "from:\"news@shop.example\"")
        );
    }

    #[test]
    fn your_own_lists_and_drafts_have_none_and_bodies_come_last() {
        let own = ["shop.example".to_owned()];
        assert_eq!(
            find(
                &thread(vec![message("m1", Some("<https://shop.example/u>"), true)]),
                &own,
                |_| None
            ),
            None
        );
        let mut draft = message("m1", Some("<https://shop.example/u>"), true);
        draft.labels.push("DRAFT".into());
        assert_eq!(find(&thread(vec![draft]), &[], |_| None), None);

        let plain = thread(vec![message("m1", None, false), message("m2", None, false)]);
        let found = find(&plain, &[], |id| {
            (id == "m2").then(|| "https://shop.example/footer-unsub".to_owned())
        })
        .expect("the body's link");
        assert_eq!(
            (found.target, found.message.as_str()),
            (Target::Web(web("https://shop.example/footer-unsub")), "m2")
        );
        // The first message with a header wins over any body.
        let mixed = thread(vec![
            message("m1", None, false),
            message("m2", Some("<https://shop.example/h>"), false),
        ]);
        assert_eq!(
            find(&mixed, &[], |_| Some("https://shop.example/body".into()))
                .map(|found| found.target),
            Some(Target::Web(web("https://shop.example/h")))
        );
    }
}
