//! What the copy commands put on the clipboard: a message or a conversation as text and HTML, a
//! message's recipients and a thread's subject; and ⌘C in the list ([`listed`]).

mod listed;
mod turndown;

use exo_store::{Contact, MessageDetail};
use jiff::Zoned;
use regex::Regex;

use crate::format::dates;
pub(crate) use listed::{Listed, conversation_hint, message_hint};
use turndown::{Kind, Tree};

/// What a copy puts on the clipboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contents {
    /// `text/plain`.
    pub text: String,
    /// `text/html`.
    pub html: String,
}

/// `message` as copying it puts it on the clipboard, its new part given by `unquoted_html`.
pub fn message(message: &MessageDetail, unquoted_html: &str, now: &Zoned) -> Contents {
    conversation(&[(message, unquoted_html)], now)
}

/// A conversation as copying it puts it on the clipboard: each message as [`message`] does,
/// oldest first, with `<br><hr>` between them. The HTML is sanitized, which drops the links'
/// `target`.
pub fn conversation(messages: &[(&MessageDetail, &str)], now: &Zoned) -> Contents {
    Contents {
        text: messages
            .iter()
            .map(|(detail, html)| message_text(detail, html, now))
            .collect::<Vec<_>>()
            .join("\n\n------\n\n"),
        html: exo_mail::sanitize_fragment(
            &messages
                .iter()
                .map(|(detail, html)| format!("{}{html}", html_header(detail, now)))
                .collect::<Vec<_>>()
                .join("<br><hr>"),
        ),
    }
}

/// Everyone on `message` but you, as `Name <address>`.
pub fn recipients(message: &MessageDetail) -> String {
    message
        .from
        .iter()
        .chain(&message.to)
        .chain(&message.cc)
        .chain(&message.bcc)
        .filter(|contact| !contact.is_me)
        .map(format)
        .collect::<Vec<_>>()
        .join(", ")
}

/// `Name <address>`, the name quoted when it isn't a plain word.
fn format(contact: &Contact) -> String {
    match contact.name.as_deref().filter(|name| !name.is_empty()) {
        Some(name) if !contact.email.is_empty() => {
            let plain = name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "$%&'*+.-/=?^_`{}|~ ".contains(c));
            let name = if plain {
                name.to_owned()
            } else {
                format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
            };
            format!("{name} <{}>", contact.email)
        }
        Some(name) => name.to_owned(),
        None => contact.email.clone(),
    }
}

/// `contacts` as [`format`] has them, comma-separated.
fn format_all(contacts: &[Contact]) -> String {
    contacts
        .iter()
        .map(|contact| format(contact).trim().to_owned())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// `message` as plain text: the addresses, the date and the subject, then the new part as Markdown
/// with links and images numbered, and their URLs after `--`.
fn message_text(message: &MessageDetail, unquoted_html: &str, now: &Zoned) -> String {
    let mut lines = vec![format!(
        "From: {}",
        message.from.as_ref().map(format).unwrap_or_default()
    )];
    let reply_to = format_all(&message.reply_to);
    if !reply_to.is_empty() {
        lines.push(format!("Reply-To: {reply_to}"));
    }
    for (label, contacts) in [
        ("To:   ", &message.to),
        ("Cc:   ", &message.cc),
        ("Bcc:  ", &message.bcc),
    ] {
        if !contacts.is_empty() {
            lines.push(format!("{label}{}", format_all(contacts)));
        }
    }
    lines.push(format!("Date: {}", dates::expanded(message.date, now)));
    lines.push(String::new());
    lines.push(format!("Subject: {}", message.subject));
    lines.push(String::new());

    let (body, footnotes) = body_markdown(message, unquoted_html);
    lines.push(body);
    if !footnotes.is_empty() {
        lines.push("\n--".into());
        lines.extend(
            footnotes
                .iter()
                .enumerate()
                .map(|(i, url)| format!("[{}] {url}", i + 1)),
        );
    }
    lines.join("\n")
}

/// The new part as Markdown, and the URLs its links and images point to, numbered in order.
fn body_markdown(message: &MessageDetail, unquoted_html: &str) -> (String, Vec<String>) {
    let mut tree = Tree::parse(unquoted_html);
    let mut footnotes = Vec::new();
    // Every image and link becomes its text, in document order; one inside a link that's
    // already gone still takes a number.
    for id in tree.elements(&["IMG", "A"]) {
        let text = if tree.name(id) == Some("IMG") {
            let alt = tree
                .attribute(id, "alt")
                .filter(|alt| !alt.is_empty())
                .or_else(|| tree.attribute(id, "title"))
                .unwrap_or_default()
                .to_owned();
            let src = tree.attribute(id, "src").unwrap_or_default().to_owned();
            let attachment = src.strip_prefix("cid:").and_then(|cid| {
                message
                    .attachments
                    .iter()
                    .find(|attachment| attachment.content_id.as_deref() == Some(cid))
            });
            match attachment {
                Some(attachment) => format!(
                    "\"{}\"",
                    if alt.is_empty() {
                        attachment.filename.as_str()
                    } else {
                        &alt
                    }
                ),
                None if !src.is_empty() => {
                    footnotes.push(src);
                    format!("{alt} [{}]", footnotes.len())
                }
                None => String::new(),
            }
        } else {
            let inner = tree.text(id);
            let href = tree.attribute(id, "href").unwrap_or_default().to_owned();
            if !href.is_empty() && href != format!("mailto:{inner}") {
                footnotes.push(href);
                format!("{inner} [{}]", footnotes.len())
            } else {
                inner
            }
        };
        let span = tree.create("SPAN");
        tree.set_inner_text(span, &text);
        tree.replace(id, span);
    }
    for style in tree.elements(&["STYLE"]) {
        tree.detach(style);
    }
    debug_assert!(matches!(tree.kind(0), Kind::Element { .. }));
    let markdown = turndown::turndown(tree);
    let trailing = Regex::new(r"(?m)[\s\x{feff}]+$").expect("a valid pattern");
    let blank_runs = Regex::new(r"\n{3,}").expect("a valid pattern");
    let text = trailing.replace_all(&markdown, "");
    let text = blank_runs
        .replace_all(&text, "\n\n")
        .replace("\\[", "[")
        .replace("\\]", "]");
    (text, footnotes)
}

/// `html_header`: the addresses, date and subject above a copied message's HTML.
fn html_header(message: &MessageDetail, now: &Zoned) -> String {
    let line =
        |label: &str, value: &str| format!("<div><strong>{label}</strong> {}</div>", escape(value));
    let join = |contacts: &[Contact]| contacts.iter().map(format).collect::<Vec<_>>().join(", ");
    let mut html = String::from("<div>");
    html += &line(
        "From:",
        &message.from.as_ref().map(format).unwrap_or_default(),
    );
    html += &line("To:", &join(&message.to));
    if !message.cc.is_empty() {
        html += &line("Cc:", &join(&message.cc));
    }
    if !message.bcc.is_empty() {
        html += &line("Bcc:", &join(&message.bcc));
    }
    // The header's `<time>`.
    let datetime = message.date.strftime("%Y-%m-%dT%H:%M:%S%.3fZ");
    html += &format!(
        "<div><strong>Date:</strong> <time datetime=\"{datetime}\" class=\"DateTime\">{}</time></div>",
        escape(&dates::expanded(message.date, now))
    );
    html += &line("Subject:", &message.subject);
    html + "<br></div>"
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use exo_store::{Attachment, Body};

    fn contact(name: &str, email: &str, is_me: bool) -> Contact {
        Contact {
            name: (!name.is_empty()).then(|| name.to_owned()),
            email: email.into(),
            is_me,
        }
    }

    fn now() -> Zoned {
        "2026-11-17T09:00:00[America/New_York]"
            .parse()
            .expect("valid time")
    }

    fn detail() -> MessageDetail {
        MessageDetail {
            id: "m1".into(),
            labels: vec!["INBOX".into()],
            date: "2026-11-16T14:05:00Z".parse().expect("valid time"),
            from: Some(contact(
                "Rachel Kim",
                "rachel.kim@callowayfreight.com",
                false,
            )),
            to: vec![
                contact("Maya Okafor", "maya@brightmoor.co", true),
                contact("O'Neil, Dev", "dev@callowayfreight.com", false),
            ],
            cc: vec![contact("", "ops@callowayfreight.com", false)],
            bcc: Vec::new(),
            reply_to: Vec::new(),
            subject: "Q4 shipping volumes".into(),
            snippet: String::new(),
            rfc822_id: None,
            in_reply_to: None,
            references: Vec::new(),
            list_id: None,
            list_unsubscribe: None,
            list_unsubscribe_post: None,
            headers: Vec::new(),
            body: Body::default(),
            attachments: vec![Attachment {
                message_id: "m1".into(),
                attachment_id: None,
                part_id: "1.2".into(),
                filename: "chart.png".into(),
                mime_type: "image/png".into(),
                size: 10,
                content_id: Some("chart@calloway".into()),
                inline: true,
            }],
            failure: None,
            scheduled: None,
            sending: false,
        }
    }

    #[test]
    fn a_message_copies_with_its_header_and_numbered_links() {
        let html = "<div>Hi Maya,</div><div><br></div><div>The <b>numbers</b> are in the <a href=\"https://calloway.co/q4\">deck</a>. \
                    Mail <a href=\"mailto:ops@callowayfreight.com\">ops@callowayfreight.com</a> with questions.</div>\
                    <div><img src=\"cid:chart@calloway\"><img src=\"https://cdn.calloway.co/logo.png\" alt=\"Calloway\"></div>";
        let copy = message(&detail(), html, &now());
        assert_eq!(
            copy.text,
            "From: Rachel Kim <rachel.kim@callowayfreight.com>\n\
             To:   Maya Okafor <maya@brightmoor.co>, \"O'Neil, Dev\" <dev@callowayfreight.com>\n\
             Cc:   ops@callowayfreight.com\n\
             Date: Monday, November 16 2026 at 9:05 AM EST\n\
             \n\
             Subject: Q4 shipping volumes\n\
             \n\
             Hi Maya,\n\
             The **numbers** are in the deck [1]. Mail ops@callowayfreight.com with questions.\n\
             \"chart.png\"Calloway [2]\n\
             \n\
             --\n\
             [1] https://calloway.co/q4\n\
             [2] https://cdn.calloway.co/logo.png"
        );
        assert!(copy.html.starts_with("<div><div><strong>From:</strong> Rachel Kim &lt;rachel.kim@callowayfreight.com&gt;</div>"));
        assert!(copy.html.contains("<div><strong>Date:</strong> <time datetime=\"2026-11-16T14:05:00.000Z\" class=\"DateTime\">Monday, November 16 2026 at 9:05 AM EST</time></div>"));
        assert!(copy.html.ends_with(&format!("<br></div>{html}")));
    }

    #[test]
    fn a_conversation_joins_its_messages() {
        let first = detail();
        let mut second = detail();
        second.subject = "Re: Q4 shipping volumes".into();
        let copy = conversation(&[(&first, "<p>One</p>"), (&second, "<p>Two</p>")], &now());
        let parts: Vec<&str> = copy.text.split("\n\n------\n\n").collect();
        assert_eq!(parts.len(), 2);
        assert!(parts[0].ends_with("\n\nOne") && parts[1].ends_with("\n\nTwo"));
        assert_eq!(copy.html.matches("<br><hr>").count(), 1);
    }

    #[test]
    fn copied_html_is_sanitized() {
        let copy = message(
            &detail(),
            "<a href=\"https://calloway.co/q4\" target=\"_blank\" onclick=\"x()\">deck</a>",
            &now(),
        );
        assert!(
            copy.html
                .ends_with("<a href=\"https://calloway.co/q4\">deck</a>"),
            "{}",
            copy.html
        );
    }

    #[test]
    fn recipients_leave_you_out() {
        assert_eq!(
            recipients(&detail()),
            "Rachel Kim <rachel.kim@callowayfreight.com>, \"O'Neil, Dev\" <dev@callowayfreight.com>, ops@callowayfreight.com"
        );
    }
}
