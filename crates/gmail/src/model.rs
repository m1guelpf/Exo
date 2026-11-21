//! Gmail v1 resources, transcribed from the discovery document in `discovery/gmail.v1.json`.
//!
//! Every field is optional on the wire: Gmail drops fields per `format`, per resource state and
//! under `fields` partial responses, so scalars are `Option` and lists default to empty. Field
//! order follows what Gmail actually sends. `int64`/`uint64` fields are JSON strings; `int32`
//! fields (sizes, counts) are JSON numbers.

use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as, skip_serializing_none};

use crate::wire::Base64Url;

/// `users.getProfile`.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// The user's email address.
    pub email_address: Option<String>,
    /// Total messages in the mailbox.
    pub messages_total: Option<u32>,
    /// Total threads in the mailbox.
    pub threads_total: Option<u32>,
    /// The mailbox's current history ID.
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub history_id: Option<u64>,
}

/// A label. `labels.list` omits the counts; `labels.get` includes them.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Label {
    /// Immutable ID: the name for system labels (`INBOX`), `Label_N` for user labels.
    pub id: Option<String>,
    /// Display name; `/` separates nesting levels (`Projects/Exo`).
    pub name: Option<String>,
    /// Whether messages with this label show in the message list.
    pub message_list_visibility: Option<MessageListVisibility>,
    /// Whether the label shows in the label list.
    pub label_list_visibility: Option<LabelListVisibility>,
    /// System or user label.
    #[serde(rename = "type")]
    pub kind: Option<LabelType>,
    /// Messages with the label.
    pub messages_total: Option<u32>,
    /// Unread messages with the label.
    pub messages_unread: Option<u32>,
    /// Threads with the label.
    pub threads_total: Option<u32>,
    /// Unread threads with the label.
    pub threads_unread: Option<u32>,
    /// Color; user labels only.
    pub color: Option<LabelColor>,
}

/// A label's color. Both values must come from Gmail's fixed palette ([`LABEL_PALETTE`]).
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelColor {
    /// Text color, `#rrggbb`.
    pub text_color: Option<String>,
    /// Background color, `#rrggbb`.
    pub background_color: Option<String>,
}

/// The only colors Gmail accepts for labels (from the `LabelColor` schema description).
pub const LABEL_PALETTE: &[&str] = &[
    "#000000", "#434343", "#666666", "#999999", "#cccccc", "#efefef", "#f3f3f3", "#ffffff",
    "#fb4c2f", "#ffad47", "#fad165", "#16a766", "#43d692", "#4a86e8", "#a479e2", "#f691b3",
    "#f6c5be", "#ffe6c7", "#fef1d1", "#b9e4d0", "#c6f3de", "#c9daf8", "#e4d7f5", "#fcdee8",
    "#efa093", "#ffd6a2", "#fce8b3", "#89d3b2", "#a0eac9", "#a4c2f4", "#d0bcf1", "#fbc8d9",
    "#e66550", "#ffbc6b", "#fcda83", "#44b984", "#68dfa9", "#6d9eeb", "#b694e8", "#f7a7c0",
    "#cc3a21", "#eaa041", "#f2c960", "#149e60", "#3dc789", "#3c78d8", "#8e63ce", "#e07798",
    "#ac2b16", "#cf8933", "#d5ae49", "#0b804b", "#2a9c68", "#285bac", "#653e9b", "#b65775",
    "#822111", "#a46a21", "#aa8831", "#076239", "#1a764d", "#1c4587", "#41236d", "#83334c",
    "#464646", "#e7e7e7", "#0d3472", "#b6cff5", "#0d3b44", "#98d7e4", "#3d188e", "#e3d7ff",
    "#711a36", "#fbd3e0", "#8a1c0a", "#f2b2a8", "#7a2e0b", "#ffc8af", "#7a4706", "#ffdeb5",
    "#594c05", "#fbe983", "#684e07", "#fdedc1", "#0b4f30", "#b3efd3", "#04502e", "#a2dcc1",
    "#c2c2c2", "#4986e7", "#2da2bb", "#b99aff", "#994a64", "#f691b2", "#ff7537", "#ffad46",
    "#662e37", "#ebdbde", "#cca6ac", "#094228", "#42d692", "#16a765", "#757575", "#1e53b8",
    "#007286", "#7858c3", "#c2185b", "#d93025", "#54240e", "#633e04", "#521d28", "#202124",
    "#083018",
];

/// `Label.type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LabelType {
    /// Created by Gmail (`INBOX`, `UNREAD`, `CATEGORY_*`, ...).
    System,
    /// Created by the user.
    User,
}

/// `Label.messageListVisibility`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageListVisibility {
    /// Show messages in the message list.
    Show,
    /// Hide messages from the message list.
    Hide,
}

/// `Label.labelListVisibility`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LabelListVisibility {
    /// Always show the label.
    LabelShow,
    /// Show the label only while it has unread messages.
    LabelShowIfUnread,
    /// Never show the label.
    LabelHide,
}

/// `labels.list`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListLabelsResponse {
    /// All labels, system first. Always present (system labels always exist).
    #[serde(default)]
    pub labels: Vec<Label>,
}

/// A message. Which fields are present depends on the requested [`Format`].
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// Immutable message ID.
    pub id: Option<String>,
    /// The thread the message belongs to.
    pub thread_id: Option<String>,
    /// Labels applied to the message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub label_ids: Vec<String>,
    /// Short plain-text excerpt, HTML-escaped (`&#39;`, `&amp;`, ...).
    pub snippet: Option<String>,
    /// Parsed MIME tree (`full` and `metadata` formats).
    pub payload: Option<MessagePart>,
    /// Estimated size in bytes.
    pub size_estimate: Option<u32>,
    /// The whole RFC 822 message (`raw` format, and the request body of send/insert/import).
    #[serde_as(as = "Option<Base64Url>")]
    pub raw: Option<Vec<u8>>,
    /// ID of the last history record that modified this message.
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub history_id: Option<u64>,
    /// When Gmail accepted the message, in epoch milliseconds. Gmail sorts by this, not `Date`.
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub internal_date: Option<i64>,
    /// Workspace classification labels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classification_label_values: Vec<ClassificationLabelValue>,
}

impl Message {
    /// The first top-level header named `name` (case-insensitive), if the payload is present.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.payload.as_ref()?.header(name)
    }
}

/// One MIME part. The root part has `partId` `""`; children are `0`, `1`, then `0.0`, ...
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePart {
    /// Position in the MIME tree.
    pub part_id: Option<String>,
    /// Content type without parameters, e.g. `text/html`.
    pub mime_type: Option<String>,
    /// Attachment filename; `""` for non-attachment parts.
    pub filename: Option<String>,
    /// Headers in message order, RFC 2047-decoded and unfolded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<MessagePartHeader>,
    /// Body: inline `data` for text parts, an `attachmentId` for attachments, empty for
    /// containers.
    pub body: Option<MessagePartBody>,
    /// Children of a `multipart/*` part.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<MessagePart>,
}

impl MessagePart {
    /// The first header named `name` (case-insensitive).
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.as_str())
    }

    /// This part and all of its descendants, depth first.
    pub fn walk(&self) -> impl Iterator<Item = &MessagePart> {
        let mut stack = vec![self];
        std::iter::from_fn(move || {
            let part = stack.pop()?;
            stack.extend(part.parts.iter().rev());
            Some(part)
        })
    }
}

/// A header. Order and duplicates are preserved.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessagePartHeader {
    /// Header name as it appears in the message.
    pub name: String,
    /// Decoded value.
    pub value: String,
}

/// A part's body, or an attachment fetched with `attachments.get`.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePartBody {
    /// Set when the data must be fetched separately with `attachments.get`.
    pub attachment_id: Option<String>,
    /// Decoded body size in bytes. `0` for containers.
    pub size: Option<u32>,
    /// Decoded body bytes (base64url on the wire).
    #[serde_as(as = "Option<Base64Url>")]
    pub data: Option<Vec<u8>>,
}

/// A Workspace classification label value on a message.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationLabelValue {
    /// The classification label ID.
    pub label_id: Option<String>,
    /// Field values for the label.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<ClassificationLabelFieldValue>,
}

/// One field of a [`ClassificationLabelValue`].
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationLabelFieldValue {
    /// The field ID.
    pub field_id: Option<String>,
    /// Selected option ID, for selection fields.
    pub selection: Option<String>,
}

/// `messages.get` / `threads.get` / `drafts.get` `format` parameter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Format {
    /// IDs, labels, snippet, size, history ID and internal date only.
    Minimal,
    /// Everything, with a fully parsed payload. The default.
    #[default]
    Full,
    /// Everything except the payload, plus the RFC 822 message in `raw`. Not valid for threads.
    Raw,
    /// Like minimal, plus top-level headers (optionally filtered by `metadataHeaders`).
    Metadata,
}

/// `messages.list`. Items carry only `id` and `threadId`.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListMessagesResponse {
    /// The page of messages, newest first. Omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<Message>,
    /// Token for the next page; absent on the last page.
    pub next_page_token: Option<String>,
    /// Estimated total number of results.
    pub result_size_estimate: Option<u32>,
}

/// A conversation.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    /// Immutable thread ID (the ID of its first message).
    pub id: Option<String>,
    /// Snippet of the latest message (`threads.list` only).
    pub snippet: Option<String>,
    /// ID of the last history record that modified the thread.
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub history_id: Option<u64>,
    /// Messages, oldest first (`threads.get` only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<Message>,
}

/// `threads.list`. Items carry `id`, `snippet` and `historyId`.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListThreadsResponse {
    /// The page of threads, most recently active first. Omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub threads: Vec<Thread>,
    /// Token for the next page; absent on the last page.
    pub next_page_token: Option<String>,
    /// Estimated total number of results.
    pub result_size_estimate: Option<u32>,
}

/// `messages.modify` body.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModifyMessageRequest {
    /// Labels to add.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_label_ids: Vec<String>,
    /// Labels to remove.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove_label_ids: Vec<String>,
    /// Classification labels to add.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_classification_labels: Vec<ClassificationLabelValue>,
    /// Classification labels to remove.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove_classification_label_ids: Vec<String>,
}

/// `threads.modify` body.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModifyThreadRequest {
    /// Labels to add to every message in the thread.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_label_ids: Vec<String>,
    /// Labels to remove from every message in the thread.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove_label_ids: Vec<String>,
}

/// `messages.batchModify` body.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchModifyMessagesRequest {
    /// Messages to modify (at most 1000).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<String>,
    /// Labels to add.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_label_ids: Vec<String>,
    /// Labels to remove.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove_label_ids: Vec<String>,
    /// Classification labels to add.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_classification_labels: Vec<ClassificationLabelValue>,
    /// Classification labels to remove.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove_classification_label_ids: Vec<String>,
}

/// `messages.batchDelete` body.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchDeleteMessagesRequest {
    /// Messages to delete permanently (at most 1000).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<String>,
}

/// `messages.insert` / `messages.import` `internalDateSource` parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InternalDateSource {
    /// Use the time Gmail received the message.
    ReceivedTime,
    /// Use the message's `Date` header when it is valid.
    DateHeader,
}

/// A draft: an ID plus a message labelled `DRAFT`.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    /// Immutable draft ID (`r` followed by digits).
    pub id: Option<String>,
    /// The draft's message. Its ID changes every time the draft is updated.
    pub message: Option<Message>,
}

/// `drafts.list`.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDraftsResponse {
    /// The page of drafts, newest first. Omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drafts: Vec<Draft>,
    /// Token for the next page; absent on the last page.
    pub next_page_token: Option<String>,
    /// Estimated total number of results.
    pub result_size_estimate: Option<u32>,
}

/// One history record: a change to one or more messages.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    /// The record's ID; later records have larger IDs, with gaps.
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub id: Option<u64>,
    /// Messages touched by this record (`id` and `threadId` only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<Message>,
    /// Messages added to the mailbox.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages_added: Vec<HistoryMessageAdded>,
    /// Messages permanently deleted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages_deleted: Vec<HistoryMessageDeleted>,
    /// Labels added to messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels_added: Vec<HistoryLabelAdded>,
    /// Labels removed from messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels_removed: Vec<HistoryLabelRemoved>,
}

/// `History.messagesAdded` entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryMessageAdded {
    /// The message (`id`, `threadId`, `labelIds`).
    pub message: Message,
}

/// `History.messagesDeleted` entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryMessageDeleted {
    /// The message (`id`, `threadId`).
    pub message: Message,
}

/// `History.labelsAdded` entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryLabelAdded {
    /// The message (`id`, `threadId`, `labelIds` after the change).
    pub message: Message,
    /// Labels that were added.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub label_ids: Vec<String>,
}

/// `History.labelsRemoved` entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryLabelRemoved {
    /// The message (`id`, `threadId`, `labelIds` after the change).
    pub message: Message,
    /// Labels that were removed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub label_ids: Vec<String>,
}

/// `history.list` `historyTypes` parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HistoryType {
    /// `messagesAdded` records.
    MessageAdded,
    /// `messagesDeleted` records.
    MessageDeleted,
    /// `labelsAdded` records.
    LabelAdded,
    /// `labelsRemoved` records.
    LabelRemoved,
}

/// `history.list`.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListHistoryResponse {
    /// Records after `startHistoryId`, oldest first. Omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<History>,
    /// Token for the next page; absent on the last page.
    pub next_page_token: Option<String>,
    /// The mailbox's current history ID; store it for the next sync.
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub history_id: Option<u64>,
}

/// A send-as alias (the primary address is one too). Carries the signature.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendAs {
    /// The address that appears in `From:`.
    pub send_as_email: Option<String>,
    /// The name that appears in `From:`; empty means the account name.
    pub display_name: Option<String>,
    /// Default `Reply-To:`; empty for none.
    pub reply_to_address: Option<String>,
    /// HTML signature.
    pub signature: Option<String>,
    /// Whether this is the account's primary address.
    pub is_primary: Option<bool>,
    /// Whether this address is the default `From:` for new mail.
    pub is_default: Option<bool>,
    /// Whether Gmail treats this address as an alias of the account.
    pub treat_as_alias: Option<bool>,
    /// SMTP relay settings, for external aliases.
    pub smtp_msa: Option<SmtpMsa>,
    /// Verification state; non-primary addresses only.
    pub verification_status: Option<VerificationStatus>,
}

/// `SendAs.smtpMsa`.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmtpMsa {
    /// SMTP host.
    pub host: Option<String>,
    /// SMTP port.
    pub port: Option<u16>,
    /// Username (write-only).
    pub username: Option<String>,
    /// Password (write-only).
    pub password: Option<String>,
    /// Transport security.
    pub security_mode: Option<SecurityMode>,
}

/// `SmtpMsa.securityMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecurityMode {
    /// Unspecified.
    SecurityModeUnspecified,
    /// No TLS.
    None,
    /// Implicit TLS.
    Ssl,
    /// STARTTLS.
    Starttls,
}

/// `SendAs.verificationStatus` (and forwarding addresses').
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VerificationStatus {
    /// Unspecified.
    VerificationStatusUnspecified,
    /// Verified.
    Accepted,
    /// Waiting for the user to verify.
    Pending,
}

/// `settings.sendAs.list`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSendAsResponse {
    /// All send-as aliases, primary first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub send_as: Vec<SendAs>,
}

/// Vacation responder settings.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VacationSettings {
    /// Whether the responder is on.
    pub enable_auto_reply: Option<bool>,
    /// Subject prefix of replies.
    pub response_subject: Option<String>,
    /// Plain-text reply body.
    pub response_body_plain_text: Option<String>,
    /// HTML reply body.
    pub response_body_html: Option<String>,
    /// Only reply to contacts.
    pub restrict_to_contacts: Option<bool>,
    /// Only reply within the domain (Workspace).
    pub restrict_to_domain: Option<bool>,
    /// Start, epoch milliseconds.
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub start_time: Option<i64>,
    /// End, epoch milliseconds.
    #[serde_as(as = "Option<DisplayFromStr>")]
    pub end_time: Option<i64>,
}

/// A filter: criteria plus actions applied to matching incoming mail.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filter {
    /// Server-assigned ID.
    pub id: Option<String>,
    /// What to match.
    pub criteria: Option<FilterCriteria>,
    /// What to do.
    pub action: Option<FilterAction>,
}

/// `Filter.criteria`.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterCriteria {
    /// Sender.
    pub from: Option<String>,
    /// Recipient (`To`, `Cc`, `Bcc`).
    pub to: Option<String>,
    /// Subject words.
    pub subject: Option<String>,
    /// Search query that must match.
    pub query: Option<String>,
    /// Search query that must not match.
    pub negated_query: Option<String>,
    /// Has an attachment.
    pub has_attachment: Option<bool>,
    /// Skip chats.
    pub exclude_chats: Option<bool>,
    /// Size threshold in bytes, compared with `sizeComparison`.
    pub size: Option<u32>,
    /// How to compare `size`.
    pub size_comparison: Option<SizeComparison>,
}

/// `FilterCriteria.sizeComparison`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SizeComparison {
    /// Unspecified.
    Unspecified,
    /// Smaller than `size`.
    Smaller,
    /// Larger than `size`.
    Larger,
}

/// `Filter.action`.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterAction {
    /// Labels to add.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_label_ids: Vec<String>,
    /// Labels to remove.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove_label_ids: Vec<String>,
    /// Forward to this verified address.
    pub forward: Option<String>,
}

/// `settings.filters.list`. Note the singular field name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListFiltersResponse {
    /// All filters. Omitted when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filter: Vec<Filter>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A `messages.get` response in the shape Gmail documents and sends.
    fn sample() -> serde_json::Value {
        json!({
            "id": "18bfe6c39a7a4b21",
            "threadId": "18bfe6c39a7a4b21",
            "labelIds": ["UNREAD", "IMPORTANT", "CATEGORY_PERSONAL", "INBOX"],
            "snippet": "Hi Maya, it&#39;s ready",
            "payload": {
                "partId": "",
                "mimeType": "multipart/alternative",
                "filename": "",
                "headers": [{"name": "Subject", "value": "Ready"}],
                "body": {"size": 0},
                "parts": [{
                    "partId": "0",
                    "mimeType": "text/plain",
                    "filename": "",
                    "headers": [{"name": "Content-Type", "value": "text/plain; charset=\"UTF-8\""}],
                    "body": {"size": 5, "data": "SGVsbG8="}
                }]
            },
            "sizeEstimate": 5678,
            "historyId": "1234567",
            "internalDate": "1700000000000"
        })
    }

    #[test]
    fn reads_gmail_json() {
        let message: Message = serde_json::from_value(sample()).unwrap();
        assert_eq!(message.history_id, Some(1_234_567));
        assert_eq!(message.internal_date, Some(1_700_000_000_000));
        assert_eq!(message.header("subject"), Some("Ready"));
        let text = &message.payload.as_ref().unwrap().parts[0];
        assert_eq!(
            text.body.as_ref().unwrap().data.as_deref(),
            Some(b"Hello".as_slice())
        );
    }

    #[test]
    fn writes_gmail_json() {
        let message: Message = serde_json::from_value(sample()).unwrap();
        assert_eq!(
            serde_json::to_value(&message).unwrap(),
            sample(),
            "round trips exactly"
        );
        let empty = serde_json::to_value(Message::default()).unwrap();
        assert_eq!(empty, json!({}), "unset fields are omitted, never null");
    }

    #[test]
    fn ignores_fields_it_does_not_model() {
        let mut value = sample();
        value["someFutureField"] = json!({"nested": true});
        assert!(serde_json::from_value::<Message>(value).is_ok());
    }
}
