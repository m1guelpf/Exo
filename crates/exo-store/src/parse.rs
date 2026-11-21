//! Turning Gmail's messages into the rows the store keeps.

use gmail::{Message, MessagePart};
use toasty::Json;

use crate::db::{AttachmentRow, BodyRow, Mailbox, MessageRow};
use crate::ids::labels::CHAT;
use crate::model::Header;

/// A message flattened into its rows.
pub(crate) struct Parsed {
    pub message: MessageRow,
    pub attachments: Vec<AttachmentRow>,
    /// `None` for a message fetched for its row alone.
    pub body: Option<BodyRow>,
}

/// Flattens a message fetched with `format=full`, or when not `full`, with `format=metadata`:
/// its row alone, from the headers Gmail sent, with no body or attachments. `None` for a chat
/// message (Exo ignores chats), or without an id or thread id.
pub(crate) fn parse(message: &Message, full: bool) -> Option<Parsed> {
    if message.label_ids.iter().any(|label| label == CHAT) {
        return None;
    }
    let id = message.id.clone()?;
    let thread_id = message.thread_id.clone()?;
    let empty = MessagePart::default();
    let payload = message.payload.as_ref().unwrap_or(&empty);
    let header = |name: &str| payload.header(name).map(str::to_owned);
    let mailboxes = |name: &str| payload.header(name).map(addresses).unwrap_or_default();

    let mut attachments = Vec::new();
    let mut calendar = false;
    let (mut html, mut text) = (Vec::new(), Vec::new());
    // A message fetched for its row comes with headers alone: no parts to walk.
    for part in payload.walk().filter(|_| full) {
        let mime_type = part.mime_type.as_deref().unwrap_or_default();
        calendar |= mime_type.eq_ignore_ascii_case("text/calendar");
        let filename = part.filename.as_deref().unwrap_or_default();
        let body = part.body.as_ref();
        let as_attachment = part.header("Content-Disposition").is_some_and(|value| {
            value
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("attachment")
        });
        let attachment_id = body.and_then(|body| body.attachment_id.clone());
        // Any part with a name or an attachment id is a file: inline images often come without a
        // name. A big HTML or text body Gmail moved out of the message (sync puts it back) is the
        // body, not a file.
        let is_body = !as_attachment
            && (mime_type.eq_ignore_ascii_case("text/html")
                || mime_type.eq_ignore_ascii_case("text/plain"));
        if !filename.is_empty() || (attachment_id.is_some() && !is_body) {
            let content_id = part.header("Content-ID").map(strip_angles);
            attachments.push(AttachmentRow {
                message_id: id.clone(),
                part_id: part.part_id.clone().unwrap_or_default(),
                attachment_id,
                filename: if filename.is_empty() {
                    default_name(mime_type)
                } else {
                    filename.to_owned()
                },
                mime_type: mime_type.to_owned(),
                size: body.and_then(|body| body.size).unwrap_or_default(),
                inline: false,
                content_id,
            });
            continue;
        }
        if as_attachment {
            continue;
        }
        let Some(data) = body.and_then(|body| body.data.as_deref()) else {
            continue;
        };
        if mime_type.eq_ignore_ascii_case("text/html") {
            html.push(String::from_utf8_lossy(data).into_owned());
        } else if mime_type.eq_ignore_ascii_case("text/plain") {
            text.push(String::from_utf8_lossy(data).into_owned());
        }
    }
    // A file is inline when the HTML shows it (`cid:` + its Content-ID) or it's a Gmail emoji,
    // whatever its Content-Disposition says: senders often mark the images they show as
    // attachments.
    let shown = shown_content_ids(&html);
    for attachment in &mut attachments {
        attachment.inline = attachment.content_id.as_deref().is_some_and(|cid| {
            shown.contains(&cid) || cid.strip_suffix("@goomoji.gmail").is_some_and(is_lower_hex)
        });
    }
    let joined = |parts: Vec<String>| (!parts.is_empty()).then(|| parts.join("\n"));

    Some(Parsed {
        message: MessageRow {
            id: id.clone(),
            thread_id,
            history_id: message.history_id.unwrap_or_default(),
            internal_date: message.internal_date.unwrap_or_default(),
            label_ids: message.label_ids.clone(),
            subject: header("Subject").unwrap_or_default(),
            snippet: message.snippet.clone().unwrap_or_default(),
            sender: Json(mailboxes("From").into_iter().next()),
            sender_raw: header("From").unwrap_or_default(),
            to: Json(mailboxes("To")),
            cc: Json(mailboxes("Cc")),
            bcc: Json(mailboxes("Bcc")),
            reply_to: Json(mailboxes("Reply-To")),
            rfc822_id: payload.header("Message-ID").map(strip_angles),
            in_reply_to: payload.header("In-Reply-To").map(strip_angles),
            references: payload
                .header("References")
                .map(|value| value.split_whitespace().map(strip_angles).collect())
                .unwrap_or_default(),
            list_id: header("List-Id"),
            list_unsubscribe: header("List-Unsubscribe"),
            list_unsubscribe_post: header("List-Unsubscribe-Post"),
            headers: Json(
                payload
                    .headers
                    .iter()
                    .map(|header| Header {
                        name: header.name.clone(),
                        value: header.value.clone(),
                    })
                    .collect(),
            ),
            size: message.size_estimate.unwrap_or_default(),
            has_attachments: calendar || attachments.iter().any(|attachment| !attachment.inline),
        },
        attachments,
        body: full.then(|| BodyRow {
            message_id: id,
            html: joined(html),
            text: joined(text),
            visible: None,
        }),
    })
}

/// The Content-IDs the HTML references: what follows each `cid:` up to a quote, a parenthesis
/// or whitespace.
fn shown_content_ids(html: &[String]) -> Vec<&str> {
    html.iter()
        .flat_map(|html| html.split("cid:").skip(1))
        .map(|rest| {
            let end = rest
                .find(|c: char| matches!(c, '(' | '\'' | '"' | ')') || c.is_whitespace())
                .unwrap_or(rest.len());
            &rest[..end]
        })
        .filter(|cid| !cid.is_empty())
        .collect()
}

/// The name of a file sent without one, from its MIME type.
fn default_name(mime_type: &str) -> String {
    let extension = [
        ("json", ".json"),
        ("pdf", ".pdf"),
        ("html", ".html"),
        ("jpeg", ".jpg"),
        ("gif", ".gif"),
        ("plain", ".txt"),
        ("png", ".png"),
        ("message/rfc822", ".eml"),
    ]
    .into_iter()
    .find(|(kind, _)| mime_type.contains(kind))
    .map_or("", |(_, extension)| extension);
    format!("untitled{extension}")
}

fn is_lower_hex(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Splits an address-list header (`To`, `Cc`, ...) on the commas between addresses, not the
/// ones inside quoted names. Entries without an address (`undisclosed-recipients:;`) are
/// dropped.
fn addresses(value: &str) -> Vec<Mailbox> {
    let mut entries = Vec::new();
    let (mut current, mut quoted, mut angle) = (String::new(), false, false);
    for ch in value.chars() {
        match ch {
            '"' => quoted = !quoted,
            '<' if !quoted => angle = true,
            '>' if !quoted => angle = false,
            ',' if !quoted && !angle => {
                entries.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }
    entries.push(current);
    entries
        .iter()
        .map(|entry| gmail::mime::Mailbox::parse(entry.trim()))
        .filter(|mailbox| mailbox.email.contains('@'))
        .map(|mailbox| Mailbox {
            name: mailbox.name,
            email: mailbox.email,
        })
        .collect()
}

fn strip_angles(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_owned()
}

/// Decodes the HTML entities Gmail leaves in snippets (`&#39;`, `&amp;`, ...).
pub(crate) fn unescape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let decoded = rest.find(';').filter(|&end| end <= 10).and_then(|end| {
            let entity = &rest[1..end];
            let ch = match entity {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => '\u{a0}',
                _ => {
                    let code = match entity.strip_prefix('#')? {
                        hex if hex.starts_with(['x', 'X']) => u32::from_str_radix(&hex[1..], 16),
                        decimal => decimal.parse(),
                    };
                    char::from_u32(code.ok()?)?
                }
            };
            Some((ch, end))
        });
        match decoded {
            Some((ch, end)) => {
                out.push(ch);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_address_lists_outside_quotes() {
        let list = addresses(
            r#""Lovelace, Ada" <ada@example.com>, bob@example.com, undisclosed-recipients:;"#,
        );
        assert_eq!(
            list,
            vec![
                Mailbox {
                    name: Some("Lovelace, Ada".into()),
                    email: "ada@example.com".into()
                },
                Mailbox {
                    name: None,
                    email: "bob@example.com".into()
                },
            ]
        );
    }

    #[test]
    fn files_the_html_shows_are_inline_whatever_their_disposition() {
        let part = |mime: &str, filename: &str, headers: &[(&str, &str)], data: &str| MessagePart {
            mime_type: Some(mime.into()),
            filename: Some(filename.into()),
            headers: headers
                .iter()
                .map(|(name, value)| gmail::MessagePartHeader {
                    name: (*name).into(),
                    value: (*value).into(),
                })
                .collect(),
            body: Some(gmail::MessagePartBody {
                data: (!data.is_empty()).then(|| data.as_bytes().to_vec()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let message = Message {
            id: Some("m".into()),
            thread_id: Some("t".into()),
            payload: Some(MessagePart {
                mime_type: Some("multipart/mixed".into()),
                parts: vec![
                    part(
                        "text/html",
                        "",
                        &[],
                        "<img src=\"cid:comic=\"><img src='cid:1f600@goomoji.gmail'>",
                    ),
                    part(
                        "image/png",
                        "comic.png",
                        &[
                            ("Content-Disposition", "attachment"),
                            ("Content-ID", "<comic=>"),
                        ],
                        "",
                    ),
                    part(
                        "image/png",
                        "logo.png",
                        &[("Content-Disposition", "inline"), ("Content-ID", "<logo>")],
                        "",
                    ),
                    part(
                        "image/png",
                        "emoji.png",
                        &[("Content-ID", "<1f600@goomoji.gmail>")],
                        "",
                    ),
                ],
                ..Default::default()
            }),
            ..Default::default()
        };
        let parsed = parse(&message, true).unwrap();
        let inline: Vec<_> = parsed
            .attachments
            .iter()
            .map(|attachment| (attachment.filename.as_str(), attachment.inline))
            .collect();
        assert_eq!(
            inline,
            [
                ("comic.png", true),
                ("logo.png", false),
                ("emoji.png", true)
            ]
        );
        assert!(
            parsed.message.has_attachments,
            "the unreferenced logo is a file"
        );
    }

    #[test]
    fn nameless_parts_gmail_stores_apart_are_files() {
        let part =
            |mime: &str, headers: &[(&str, &str)], attachment_id: Option<&str>| MessagePart {
                mime_type: Some(mime.into()),
                filename: Some(String::new()),
                headers: headers
                    .iter()
                    .map(|(name, value)| gmail::MessagePartHeader {
                        name: (*name).into(),
                        value: (*value).into(),
                    })
                    .collect(),
                body: Some(gmail::MessagePartBody {
                    attachment_id: attachment_id.map(str::to_owned),
                    data: attachment_id
                        .is_none()
                        .then(|| b"<img src=\"cid:chart\">".to_vec()),
                    ..Default::default()
                }),
                ..Default::default()
            };
        let message = Message {
            id: Some("m".into()),
            thread_id: Some("t".into()),
            payload: Some(MessagePart {
                mime_type: Some("multipart/related".into()),
                parts: vec![
                    part("text/html", &[], None),
                    part("image/png", &[("Content-ID", "<chart>")], Some("A1")),
                    part("text/html", &[], Some("A2")),
                ],
                ..Default::default()
            }),
            ..Default::default()
        };
        let parsed = parse(&message, true).unwrap();
        let files: Vec<_> = parsed
            .attachments
            .iter()
            .map(|attachment| (attachment.filename.as_str(), attachment.inline))
            .collect();
        assert_eq!(
            files,
            [("untitled.png", true)],
            "a big HTML body isn't a file"
        );
        assert_eq!(default_name("application/octet-stream"), "untitled");
        assert_eq!(default_name("image/jpeg"), "untitled.jpg");
    }

    #[test]
    fn a_row_alone_has_no_body_or_files() {
        let message = Message {
            id: Some("m".into()),
            thread_id: Some("t".into()),
            payload: Some(MessagePart {
                mime_type: Some("multipart/mixed".into()),
                headers: vec![gmail::MessagePartHeader {
                    name: "Subject".into(),
                    value: "Q4".into(),
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let parsed = parse(&message, false).unwrap();
        assert!(parsed.body.is_none() && parsed.attachments.is_empty());
        assert!(!parsed.message.has_attachments);
        assert_eq!(parsed.message.subject, "Q4");
        assert!(parse(&message, true).unwrap().body.is_some());
    }

    #[test]
    fn unescapes_snippets() {
        assert_eq!(
            unescape_html("it&#39;s &lt;b&gt; &amp; &#x263A; &bogus; & done"),
            "it's <b> & ☺ &bogus; & done"
        );
    }
}
