//! A `.eml` attachment read for the viewer: its subject, From/To/Cc/Bcc, date, body (the HTML part
//! with its inline images as `data:` URLs, or the text part in a `<pre>`) and its own files.

use base64::Engine as _;
use exo_store::Contact;
use mail_parser::{MessageParser, MimeHeaders};

/// A message read from an attachment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Eml {
    pub(crate) subject: String,
    pub(crate) from: Option<Contact>,
    pub(crate) to: Vec<Contact>,
    pub(crate) cc: Vec<Contact>,
    pub(crate) bcc: Vec<Contact>,
    /// When it was sent, or the `Date` header as written when it isn't a date.
    pub(crate) date: Option<Date>,
    /// What the body shows, as HTML.
    pub(crate) html: String,
    /// Its files, not its inline parts: name, type and bytes.
    pub(crate) files: Vec<(String, String, Vec<u8>)>,
}

/// An `.eml`'s `Date`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Date {
    At(jiff::Timestamp),
    Unparsed(String),
}

/// Reads `bytes` as a message, or `None` when they aren't one (`Unable to load .eml preview.`).
pub(crate) fn parse(bytes: &[u8]) -> Option<Eml> {
    let message = MessageParser::default().parse(bytes)?;
    let contacts = |address: Option<&mail_parser::Address<'_>>| -> Vec<Contact> {
        address
            .into_iter()
            .flat_map(mail_parser::Address::iter)
            .filter_map(|addr| {
                let email = addr.address()?.to_owned();
                Some(Contact {
                    name: addr
                        .name()
                        .map(str::to_owned)
                        .filter(|name| !name.is_empty()),
                    email,
                    is_me: false,
                })
            })
            .collect()
    };
    let kind = |part: &mail_parser::MessagePart<'_>| {
        part.content_type()
            .map(|kind| {
                format!(
                    "{}/{}",
                    kind.ctype(),
                    kind.subtype().unwrap_or("octet-stream")
                )
            })
            .unwrap_or_else(|| "application/octet-stream".into())
    };
    let cid = |part: &mail_parser::MessagePart<'_>| {
        part.content_id()
            .map(|cid| cid.trim_matches(['<', '>']).to_owned())
    };
    // Parts the HTML shows by Content-ID go into it as `data:` URLs, and aren't files.
    let mut shown = Vec::new();
    let html = match message
        .html_bodies()
        .find(|part| part.is_text_html())
        .and_then(|part| part.text_contents())
    {
        Some(html) => message
            .attachments()
            .fold(html.to_owned(), |html, part| match cid(part) {
                Some(id)
                    if html
                        .to_lowercase()
                        .contains(&format!("cid:{id}").to_lowercase()) =>
                {
                    shown.push(id.clone());
                    replace_cid(
                        &html,
                        &id,
                        &format!(
                            "data:{};base64,{}",
                            kind(part),
                            base64::engine::general_purpose::STANDARD.encode(part.contents())
                        ),
                    )
                }
                _ => html,
            }),
        None => message
            .body_text(0)
            .map(|text| format!("<pre>{}</pre>", escape(&text)))
            .unwrap_or_default(),
    };
    let files = message
        .attachments()
        .filter(|part| cid(part).is_none_or(|id| !shown.contains(&id)))
        .map(|part| {
            (
                part.attachment_name().unwrap_or("attachment").to_owned(),
                kind(part),
                part.contents().to_vec(),
            )
        })
        .collect();
    let date = message.date().map(
        |date| match jiff::Timestamp::from_second(date.to_timestamp()) {
            Ok(at) => Date::At(at),
            Err(_) => Date::Unparsed(date.to_rfc3339()),
        },
    );
    Some(Eml {
        subject: message.subject().unwrap_or_default().to_owned(),
        from: contacts(message.from()).into_iter().next(),
        to: contacts(message.to()),
        cc: contacts(message.cc()),
        bcc: contacts(message.bcc()),
        date,
        html,
        files,
    })
}

/// `html` with every `cid:{cid}` (any case) as `url`.
fn replace_cid(html: &str, cid: &str, url: &str) -> String {
    let needle = format!("cid:{cid}").to_lowercase();
    let lower = html.to_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut at = 0;
    while let Some(found) = lower[at..].find(&needle) {
        out.push_str(&html[at..at + found]);
        out.push_str(url);
        at += found + needle.len();
    }
    out.push_str(&html[at..]);
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    const EML: &str = "From: Millbrook Inn <reservations@millbrookinn.example.com>\r\nTo: Priya Raman <priya@brightmoor.co>, ops@brightmoor.co\r\nSubject: Your room block is confirmed\r\nDate: Mon, 16 Nov 2026 16:20:00 -0500\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: multipart/related; boundary=r\r\n\r\n--r\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Dear Priya,</p><img src=\"CID:logo@inn\">\r\n--r\r\nContent-Type: image/png\r\nContent-ID: <logo@inn>\r\nContent-Disposition: inline\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0K\r\n--r--\r\n--b\r\nContent-Type: text/csv; name=rooms.csv\r\nContent-Disposition: attachment; filename=rooms.csv\r\n\r\na,b\r\n--b--\r\n";

    #[test]
    fn an_eml_is_read_for_the_viewer() {
        let eml = parse(EML.as_bytes()).expect("a message");
        assert_eq!(eml.subject, "Your room block is confirmed");
        assert_eq!(
            eml.from
                .as_ref()
                .map(|from| (from.name.as_deref(), from.email.as_str())),
            Some((
                Some("Millbrook Inn"),
                "reservations@millbrookinn.example.com"
            ))
        );
        assert_eq!(
            eml.to
                .iter()
                .map(|to| to.email.as_str())
                .collect::<Vec<_>>(),
            ["priya@brightmoor.co", "ops@brightmoor.co"]
        );
        assert_eq!(
            eml.date,
            Some(Date::At("2026-11-16T21:20:00Z".parse().expect("a time")))
        );
        assert!(
            eml.html
                .contains("<img src=\"data:image/png;base64,iVBORw0K\">"),
            "{}",
            eml.html
        );
        assert_eq!(
            eml.files
                .iter()
                .map(|(name, kind, _)| (name.as_str(), kind.as_str()))
                .collect::<Vec<_>>(),
            [("rooms.csv", "text/csv")],
            "the inline image isn't a file"
        );
    }

    #[test]
    fn a_text_message_shows_in_a_pre() {
        let eml = parse(b"Subject: Hi\r\n\r\n<b> & more").expect("a message");
        assert_eq!(eml.html, "<pre>&lt;b&gt; &amp; more</pre>");
        assert!(parse(b"").is_none_or(|eml| eml.html.is_empty()));
    }
}
