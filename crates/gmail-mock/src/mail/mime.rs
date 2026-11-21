//! Turns RFC 822 bytes into what Gmail derives from them: the payload tree, attachment bodies,
//! the snippet, and the fields search runs against.

use std::collections::HashMap;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use gmail::{MessagePart, MessagePartBody, MessagePartHeader};
use mail_parser::{Address, HeaderValue, MessageParser, MimeHeaders, PartType};
use sha2::{Digest, Sha256};

/// Everything Gmail computes from a message's bytes.
#[derive(Debug, Clone)]
pub struct Parsed {
    /// The `full` format payload; text bodies inline, attachments by ID.
    pub payload: MessagePart,
    /// Attachment bodies by attachment ID.
    pub attachments: HashMap<String, Vec<u8>>,
    /// Gmail's snippet: the start of the text, whitespace-collapsed and HTML-escaped.
    pub snippet: String,
    /// `Subject:`, decoded.
    pub subject: String,
    /// `Message-ID:` without angle brackets.
    pub rfc822_id: Option<String>,
    /// Message IDs from `In-Reply-To:` and `References:`, used for threading.
    pub ancestors: Vec<String>,
    /// `Date:` in epoch milliseconds, when parseable.
    pub date: Option<i64>,
    /// Addresses in `To`, `Cc` and `Bcc`, lowercased.
    pub recipients: Vec<String>,
    /// Pre-tokenized fields for search.
    pub search: SearchFields,
}

/// Token streams search operators match against. See [`tokenize`].
#[derive(Debug, Clone, Default)]
pub struct SearchFields {
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: Vec<String>,
    pub body: Vec<String>,
    pub filenames: Vec<String>,
    pub list: Vec<String>,
    pub delivered_to: Vec<String>,
    pub has_attachment: bool,
}

/// Parses `raw`. `message_id` seeds attachment IDs. `None` if it isn't a message at all.
pub fn parse(raw: &[u8], message_id: &str) -> Option<Parsed> {
    let message = MessageParser::default().parse(raw)?;
    let mut attachments = HashMap::new();
    let mut filenames = Vec::new();
    let payload = build_part(
        &message,
        0,
        String::new(),
        message_id,
        &mut attachments,
        &mut filenames,
    );

    let text = message
        .body_text(0)
        .map(|text| text.into_owned())
        .or_else(|| {
            message
                .body_html(0)
                .map(|html| mail_parser::decoders::html::html_to_text(&html))
        })
        .unwrap_or_default();

    let addresses = |value: Option<&Address<'_>>| -> Vec<String> {
        value
            .into_iter()
            .flat_map(|address| address.iter())
            .filter_map(|addr| addr.address.as_deref().map(str::to_lowercase))
            .collect()
    };
    let header_tokens = |name: &str| tokenize(payload.header(name).unwrap_or_default());
    let ids = |value: &HeaderValue<'_>| -> Vec<String> {
        match value {
            HeaderValue::Text(id) => vec![id.to_string()],
            HeaderValue::TextList(list) => list.iter().map(ToString::to_string).collect(),
            _ => Vec::new(),
        }
    };
    let mut ancestors = ids(message.references());
    ancestors.extend(ids(message.in_reply_to()));

    let mut recipients = addresses(message.to());
    recipients.extend(addresses(message.cc()));
    recipients.extend(addresses(message.bcc()));

    let search = SearchFields {
        from: header_tokens("From"),
        to: header_tokens("To"),
        cc: header_tokens("Cc"),
        bcc: header_tokens("Bcc"),
        subject: header_tokens("Subject"),
        body: tokenize(&text),
        list: header_tokens("List-Id"),
        delivered_to: header_tokens("Delivered-To"),
        has_attachment: !filenames.is_empty(),
        filenames: filenames.iter().flat_map(|name| tokenize(name)).collect(),
    };

    Some(Parsed {
        snippet: snippet(&text),
        subject: message.subject().unwrap_or_default().to_owned(),
        rfc822_id: message.message_id().map(str::to_owned),
        ancestors,
        date: message.date().map(|date| date.to_timestamp() * 1000),
        recipients,
        search,
        payload,
        attachments,
    })
}

fn build_part(
    message: &mail_parser::Message<'_>,
    index: u32,
    part_id: String,
    message_id: &str,
    attachments: &mut HashMap<String, Vec<u8>>,
    filenames: &mut Vec<String>,
) -> MessagePart {
    let Some(part) = message.part(index) else {
        return MessagePart::default();
    };
    let raw = message.raw_message.as_ref();
    let headers = part
        .headers
        .iter()
        .map(|header| {
            let name = raw
                .get(header.offset_field as usize..header.offset_start as usize)
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            let value = raw
                .get(header.offset_start as usize..header.offset_end as usize)
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            MessagePartHeader {
                name: name.trim_end().trim_end_matches(':').trim().to_owned(),
                value: decode_header(&value, &header.value),
            }
        })
        .collect();
    let mime_type = part.content_type().map_or_else(
        || "text/plain".to_owned(),
        |content_type| {
            let subtype = content_type.subtype().unwrap_or("plain");
            format!("{}/{subtype}", content_type.ctype()).to_ascii_lowercase()
        },
    );
    let filename = part.attachment_name().unwrap_or_default().to_owned();

    let (body, parts) = match &part.body {
        PartType::Multipart(children) => {
            let parts = children
                .iter()
                .enumerate()
                .map(|(position, &child)| {
                    let child_id = if part_id.is_empty() {
                        position.to_string()
                    } else {
                        format!("{part_id}.{position}")
                    };
                    build_part(message, child, child_id, message_id, attachments, filenames)
                })
                .collect();
            (
                MessagePartBody {
                    size: Some(0),
                    ..MessagePartBody::default()
                },
                parts,
            )
        }
        _ => {
            let bytes = part.contents().to_vec();
            let size = Some(u32::try_from(bytes.len()).unwrap_or(u32::MAX));
            let body = if mime_type.starts_with("text/") && filename.is_empty() {
                MessagePartBody {
                    size,
                    data: Some(bytes),
                    ..MessagePartBody::default()
                }
            } else {
                let id = attachment_id(message_id, &part_id);
                attachments.insert(id.clone(), bytes);
                if !filename.is_empty() {
                    filenames.push(filename.clone());
                }
                MessagePartBody {
                    attachment_id: Some(id),
                    size,
                    data: None,
                }
            };
            (body, Vec::new())
        }
    };

    MessagePart {
        part_id: Some(part_id),
        mime_type: Some(mime_type),
        filename: Some(filename),
        headers,
        body: Some(body),
        parts,
    }
}

/// Gmail returns header values unfolded and with RFC 2047 encoded words decoded.
fn decode_header(raw: &str, parsed: &HeaderValue<'_>) -> String {
    let unfolded = raw.replace("\r\n", "").replace('\n', "");
    let unfolded = unfolded.trim();
    if !unfolded.contains("=?") {
        return unfolded.to_owned();
    }
    match parsed {
        HeaderValue::Text(text) => text.to_string(),
        HeaderValue::TextList(list) => list.join(", "),
        HeaderValue::Address(address) => address
            .iter()
            .map(|addr| {
                let email = addr.address.as_deref().unwrap_or_default();
                gmail::mime::Mailbox {
                    name: addr.name.as_deref().map(str::to_owned),
                    email: email.to_owned(),
                }
                .to_string()
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => unfolded.to_owned(),
    }
}

/// Gmail's attachment IDs are long opaque strings; ours are stable per message and part.
fn attachment_id(message_id: &str, part_id: &str) -> String {
    let digest = Sha256::digest(format!("{message_id}/{part_id}").as_bytes());
    format!("ANGjdJ{}", URL_SAFE_NO_PAD.encode(digest))
}

const SNIPPET_CHARS: usize = 200;

fn snippet(text: &str) -> String {
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = collapsed.chars().take(SNIPPET_CHARS).collect();
    escape_html(&cut)
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Lowercased runs of letters and digits. Search matches query tokens as a contiguous run of
/// field tokens, so `from:acme.com` (`acme`, `com`) matches `Ada <ada@acme.com>`.
pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &str = "From: =?UTF-8?Q?Jos=C3=A9?= <jose@example.com>\r\n\
To: Ada <ada@example.com>\r\n\
Subject: =?UTF-8?B?Q2Fmw6kg4piV?=\r\n\
Message-ID: <m1@example.com>\r\n\
References: <a@x>\r\n <b@x>\r\n\
Date: Tue, 17 Nov 2026 09:00:00 -0500\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"outer\"\r\n\
\r\n\
--outer\r\n\
Content-Type: multipart/alternative; boundary=\"inner\"\r\n\
\r\n\
--inner\r\n\
Content-Type: text/plain; charset=\"UTF-8\"\r\n\
\r\n\
It's <here> & ready.\r\n\
--inner\r\n\
Content-Type: text/html; charset=\"UTF-8\"\r\n\
\r\n\
<p>It's here</p>\r\n\
--inner--\r\n\
--outer\r\n\
Content-Type: application/pdf; name=\"q3.pdf\"\r\n\
Content-Disposition: attachment; filename=\"q3.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
JVBERi0=\r\n\
--outer--\r\n";

    #[test]
    fn builds_gmail_payload_tree() {
        let parsed = parse(RAW.as_bytes(), "18f").unwrap();
        let root = &parsed.payload;
        assert_eq!(root.part_id.as_deref(), Some(""));
        assert_eq!(root.mime_type.as_deref(), Some("multipart/mixed"));
        assert_eq!(root.header("subject"), Some("Café ☕"));
        assert_eq!(root.header("From"), Some("José <jose@example.com>"));
        assert_eq!(root.header("References"), Some("<a@x> <b@x>"));
        assert_eq!(root.body.as_ref().unwrap().size, Some(0));

        let alternative = &root.parts[0];
        assert_eq!(alternative.part_id.as_deref(), Some("0"));
        let text = &alternative.parts[0];
        assert_eq!(text.part_id.as_deref(), Some("0.0"));
        assert_eq!(text.filename.as_deref(), Some(""));
        assert_eq!(
            text.body.as_ref().unwrap().data.as_deref(),
            Some(b"It's <here> & ready.".as_slice())
        );

        let pdf = &root.parts[1];
        assert_eq!(pdf.part_id.as_deref(), Some("1"));
        assert_eq!(pdf.filename.as_deref(), Some("q3.pdf"));
        let id = pdf.body.as_ref().unwrap().attachment_id.clone().unwrap();
        assert_eq!(parsed.attachments[&id], b"%PDF-");
        assert_eq!(pdf.body.as_ref().unwrap().size, Some(5));

        assert_eq!(parsed.snippet, "It&#39;s &lt;here&gt; &amp; ready.");
        assert_eq!(parsed.rfc822_id.as_deref(), Some("m1@example.com"));
        assert_eq!(parsed.ancestors, ["a@x", "b@x"]);
        assert_eq!(parsed.date, Some(1_794_924_000_000));
        assert!(parsed.search.has_attachment);
        assert_eq!(parsed.search.filenames, ["q3", "pdf"]);
    }
}
