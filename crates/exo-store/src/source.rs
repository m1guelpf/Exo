//! A thread's source as the raw source viewer shows it: Gmail's `threads.get` response, its
//! snippets' HTML entities decoded, as `JSON.stringify(thread, null, 4)` writes it.

use std::fmt;

use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

use crate::{error::Result, parse::unescape_html, quota::Lane, store::Store};

impl Store {
    /// Thread `id`'s source for the raw source viewer, from `threads.get` (format `full`, 10
    /// units).
    ///
    /// # Errors
    /// [`crate::Error::Offline`] without a network; [`crate::Error::Gmail`] when Gmail refuses.
    pub async fn thread_source(&self, id: &str) -> Result<String> {
        let mut thread: Json = self
            .inner
            .api
            .call(Lane::Act, 3, |gmail| {
                gmail
                    .thread(id, gmail::Format::Full, &[])
                    .returning::<Json>()
            })
            .await?;
        normalize(&mut thread);
        Ok(thread.to_string())
    }
}

/// JSON with its members in the order they came, which `serde_json::Value` doesn't keep.
#[derive(Clone, Debug, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    /// As written.
    Number(String),
    String(String),
    Array(Vec<Self>),
    Object(Vec<(String, Self)>),
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_any(JsonVisitor)
    }
}

struct JsonVisitor;

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = Json;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON")
    }

    fn visit_unit<E>(self) -> std::result::Result<Json, E> {
        Ok(Json::Null)
    }

    fn visit_bool<E>(self, value: bool) -> std::result::Result<Json, E> {
        Ok(Json::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> std::result::Result<Json, E> {
        Ok(Json::Number(value.to_string()))
    }

    fn visit_u64<E>(self, value: u64) -> std::result::Result<Json, E> {
        Ok(Json::Number(value.to_string()))
    }

    fn visit_f64<E>(self, value: f64) -> std::result::Result<Json, E> {
        Ok(Json::Number(value.to_string()))
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Json, E> {
        Ok(Json::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Json, E> {
        Ok(Json::String(value))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Json, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Json::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Json, A::Error> {
        let mut members = Vec::new();
        while let Some(member) = map.next_entry()? {
            members.push(member);
        }
        Ok(Json::Object(members))
    }
}

/// Decodes the HTML entities in each message's `snippet`.
fn normalize(thread: &mut Json) {
    let Json::Object(members) = thread else {
        return;
    };
    let messages = members
        .iter_mut()
        .filter(|(key, _)| key == "messages")
        .filter_map(|(_, value)| match value {
            Json::Array(messages) => Some(messages),
            _ => None,
        });
    for message in messages.flatten() {
        let Json::Object(fields) = message else {
            continue;
        };
        for (key, value) in fields.iter_mut() {
            if let (true, Json::String(snippet)) = (key == "snippet", &mut *value) {
                *snippet = unescape_html(snippet);
            }
        }
    }
}

impl fmt::Display for Json {
    /// `JSON.stringify(value, null, 4)`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write(formatter, 0)
    }
}

impl Json {
    fn write(&self, out: &mut fmt::Formatter<'_>, depth: usize) -> fmt::Result {
        let indent = |out: &mut fmt::Formatter<'_>, depth: usize| {
            write!(out, "{:width$}", "", width = 4 * depth)
        };
        match self {
            Self::Null => out.write_str("null"),
            Self::Bool(value) => write!(out, "{value}"),
            Self::Number(value) => out.write_str(value),
            Self::String(value) => quote(out, value),
            Self::Array(items) if items.is_empty() => out.write_str("[]"),
            Self::Object(members) if members.is_empty() => out.write_str("{}"),
            Self::Array(items) => {
                out.write_str("[\n")?;
                for (at, item) in items.iter().enumerate() {
                    indent(out, depth + 1)?;
                    item.write(out, depth + 1)?;
                    out.write_str(if at + 1 < items.len() { ",\n" } else { "\n" })?;
                }
                indent(out, depth)?;
                out.write_str("]")
            }
            Self::Object(members) => {
                out.write_str("{\n")?;
                for (at, (key, value)) in members.iter().enumerate() {
                    indent(out, depth + 1)?;
                    quote(out, key)?;
                    out.write_str(": ")?;
                    value.write(out, depth + 1)?;
                    out.write_str(if at + 1 < members.len() { ",\n" } else { "\n" })?;
                }
                indent(out, depth)?;
                out.write_str("}")
            }
        }
    }
}

/// A string as `JSON.stringify` quotes it: only quotes, backslashes and control characters
/// escaped.
fn quote(out: &mut fmt::Formatter<'_>, text: &str) -> fmt::Result {
    out.write_str("\"")?;
    for ch in text.chars() {
        match ch {
            '"' => out.write_str("\\\"")?,
            '\\' => out.write_str("\\\\")?,
            '\u{8}' => out.write_str("\\b")?,
            '\u{c}' => out.write_str("\\f")?,
            '\n' => out.write_str("\\n")?,
            '\r' => out.write_str("\\r")?,
            '\t' => out.write_str("\\t")?,
            control if u32::from(control) < 0x20 => write!(out, "\\u{:04x}", u32::from(control))?,
            ch => write!(out, "{ch}")?,
        }
    }
    out.write_str("\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(json: &str) -> String {
        let mut thread: Json = serde_json::from_str(json).expect("valid JSON");
        normalize(&mut thread);
        thread.to_string()
    }

    #[test]
    fn writes_what_gmail_sent_in_its_order_four_spaces_deep() {
        let json = r#"{"id":"t1","historyId":"42","messages":[{"id":"m1","labelIds":["INBOX","UNREAD"],"sizeEstimate":1234,"payload":{"headers":[],"parts":[{"partId":"0","body":{"size":0}}]},"draft":false,"x":null}]}"#;
        let expected = r#"{
    "id": "t1",
    "historyId": "42",
    "messages": [
        {
            "id": "m1",
            "labelIds": [
                "INBOX",
                "UNREAD"
            ],
            "sizeEstimate": 1234,
            "payload": {
                "headers": [],
                "parts": [
                    {
                        "partId": "0",
                        "body": {
                            "size": 0
                        }
                    }
                ]
            },
            "draft": false,
            "x": null
        }
    ]
}"#;
        assert_eq!(source(json), expected);
    }

    #[test]
    fn snippets_are_decoded_and_strings_quoted_as_json_stringify_does() {
        let json = r#"{"messages":[{"snippet":"today&#39;s <b> &amp; more","payload":{"headers":[{"name":"To","value":"\"Maya\" <m@x.co>,\tPriya\u0001 é"}]}}]}"#;
        let shown = source(json);
        assert!(
            shown.contains(r#""snippet": "today's <b> & more""#),
            "{shown}"
        );
        assert!(
            shown.contains(r#""value": "\"Maya\" <m@x.co>,\tPriya\u0001 é""#),
            "{shown}"
        );
    }
}
