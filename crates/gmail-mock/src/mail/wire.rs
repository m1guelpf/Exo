//! Stored state rendered as Gmail resources, per `format`.

use gmail::{Draft, Format, Message, MessagePart, Thread};

use super::StoredMessage;

impl StoredMessage {
    /// The message as `messages.get` returns it in `format`. `metadata_headers` filters the
    /// headers of the `metadata` format (case-insensitively); empty means all.
    pub fn to_wire(&self, format: Format, metadata_headers: &[String]) -> Message {
        let mut message = Message {
            id: Some(self.id.clone()),
            thread_id: Some(self.thread_id.clone()),
            label_ids: self.labels.iter().cloned().collect(),
            snippet: Some(self.mime.snippet.clone()),
            size_estimate: Some(u32::try_from(self.raw.len()).unwrap_or(u32::MAX)),
            history_id: Some(self.history_id),
            internal_date: Some(self.internal_date),
            ..Message::default()
        };
        match format {
            Format::Minimal => {}
            Format::Full => message.payload = Some(self.mime.payload.clone()),
            Format::Raw => message.raw = Some(self.raw.clone()),
            Format::Metadata => {
                let headers = self
                    .mime
                    .payload
                    .headers
                    .iter()
                    .filter(|header| {
                        metadata_headers.is_empty()
                            || metadata_headers
                                .iter()
                                .any(|wanted| wanted.eq_ignore_ascii_case(&header.name))
                    })
                    .cloned()
                    .collect();
                message.payload = Some(MessagePart {
                    mime_type: self.mime.payload.mime_type.clone(),
                    headers,
                    ..MessagePart::default()
                });
            }
        }
        message
    }

    /// The short form mutations return: `id`, `threadId`, `labelIds`.
    pub fn to_reference(&self) -> Message {
        Message {
            id: Some(self.id.clone()),
            thread_id: Some(self.thread_id.clone()),
            label_ids: self.labels.iter().cloned().collect(),
            ..Message::default()
        }
    }

    /// The form list endpoints return: `id` and `threadId` only.
    pub fn to_list_item(&self) -> Message {
        Message {
            id: Some(self.id.clone()),
            thread_id: Some(self.thread_id.clone()),
            ..Message::default()
        }
    }
}

/// `threads.get`: the thread ID, its latest history ID, and every message in `format`.
pub fn thread(id: &str, messages: &[&StoredMessage], format: Format, headers: &[String]) -> Thread {
    Thread {
        id: Some(id.to_owned()),
        history_id: messages.iter().map(|m| m.history_id).max(),
        messages: messages
            .iter()
            .map(|m| m.to_wire(format, headers))
            .collect(),
        ..Thread::default()
    }
}

/// `threads.list` item: ID, the latest message's snippet, and the thread's history ID.
pub fn thread_list_item(id: &str, messages: &[&StoredMessage]) -> Thread {
    Thread {
        id: Some(id.to_owned()),
        snippet: messages.last().map(|m| m.mime.snippet.clone()),
        history_id: messages.iter().map(|m| m.history_id).max(),
        ..Thread::default()
    }
}

/// Mutations on threads return the thread with each message in short form.
pub fn thread_reference(id: &str, messages: &[&StoredMessage]) -> Thread {
    Thread {
        id: Some(id.to_owned()),
        history_id: messages.iter().map(|m| m.history_id).max(),
        messages: messages.iter().map(|m| m.to_reference()).collect(),
        ..Thread::default()
    }
}

/// A draft resource around an already rendered message.
pub fn draft(id: &str, message: Message) -> Draft {
    Draft {
        id: Some(id.to_owned()),
        message: Some(message),
    }
}
