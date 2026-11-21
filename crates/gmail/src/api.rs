//! Gmail v1 calls, for the authenticated user (`userId` `me`).

use reqwest::Method;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::client::{Body, Call, Client, decode, random_boundary};
use crate::endpoints::join;
use crate::error::Error;
use crate::model::*;
use crate::multipart;

/// Most calls Gmail accepts in one batch.
pub const MAX_BATCH: usize = 100;

/// `threads.list` / `messages.list` parameters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListQuery {
    /// Gmail search query.
    pub q: Option<String>,
    /// Only results carrying every one of these labels.
    pub label_ids: Vec<String>,
    /// Page size; Gmail defaults to 100 and caps at 500.
    pub max_results: Option<u32>,
    /// `nextPageToken` from the previous page.
    pub page_token: Option<String>,
    /// Include `SPAM` and `TRASH`.
    pub include_spam_trash: bool,
}

/// `drafts.list` parameters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DraftQuery {
    /// Gmail search query over draft messages.
    pub q: Option<String>,
    /// Page size; Gmail defaults to 100 and caps at 500.
    pub max_results: Option<u32>,
    /// `nextPageToken` from the previous page.
    pub page_token: Option<String>,
    /// Include drafts in `SPAM` and `TRASH`.
    pub include_spam_trash: bool,
}

/// `history.list` parameters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryQuery {
    /// Return records after this ID. A 404 means it expired: do a full sync.
    pub start_history_id: u64,
    /// Only these record types.
    pub history_types: Vec<HistoryType>,
    /// Only messages with this label.
    pub label_id: Option<String>,
    /// Page size; Gmail defaults to 100 and caps at 500.
    pub max_results: Option<u32>,
    /// `nextPageToken` from the previous page.
    pub page_token: Option<String>,
}

/// Options for `messages.insert` and `messages.import`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InsertOptions {
    /// Labels to apply (e.g. `INBOX`, `UNREAD`).
    pub label_ids: Vec<String>,
    /// Thread to add the message to; Gmail also requires matching `Subject` and threading
    /// headers.
    pub thread_id: Option<String>,
    /// Defaults to `receivedTime` for insert and `dateHeader` for import.
    pub internal_date_source: Option<InternalDateSource>,
}

/// Gmail calls for the authenticated user. Get one with [`Client::gmail`].
#[derive(Debug, Clone, Copy)]
pub struct Gmail<'a> {
    client: &'a Client,
}

impl Client {
    /// Gmail calls for the authenticated user.
    #[must_use]
    pub fn gmail(&self) -> Gmail<'_> {
        Gmail { client: self }
    }
}

impl<'a> Gmail<'a> {
    fn call<T>(&self, method: Method, path: &[&str]) -> Call<'a, T> {
        let base = ["gmail", "v1", "users", "me"];
        let url = join(
            &self.client.endpoints.gmail,
            base.iter().chain(path).copied(),
        );
        Call::new(self.client, method, url)
    }

    fn upload_call<T>(&self, method: Method, path: &[&str]) -> Call<'a, T> {
        let base = ["upload", "gmail", "v1", "users", "me"];
        let url = join(
            &self.client.endpoints.gmail,
            base.iter().chain(path).copied(),
        );
        Call::new(self.client, method, url)
    }

    /// `users.getProfile`.
    pub fn profile(&self) -> Call<'a, Profile> {
        self.call(Method::GET, &["profile"])
    }

    // Labels

    /// `labels.list` (no counts; use [`Gmail::label`] for those).
    pub fn labels(&self) -> Call<'a, ListLabelsResponse> {
        self.call(Method::GET, &["labels"])
    }

    /// `labels.get`, including message and thread counts.
    pub fn label(&self, id: &str) -> Call<'a, Label> {
        self.call(Method::GET, &["labels", id])
    }

    /// `labels.create`.
    pub fn create_label(&self, label: &Label) -> Call<'a, Label> {
        self.call(Method::POST, &["labels"]).json(label)
    }

    /// `labels.patch`: changes only the fields set in `label`.
    pub fn patch_label(&self, id: &str, label: &Label) -> Call<'a, Label> {
        self.call(Method::PATCH, &["labels", id]).json(label)
    }

    /// `labels.update`: replaces the label.
    pub fn update_label(&self, id: &str, label: &Label) -> Call<'a, Label> {
        self.call(Method::PUT, &["labels", id]).json(label)
    }

    /// `labels.delete`: also removes the label from every message.
    pub fn delete_label(&self, id: &str) -> Call<'a, ()> {
        self.call(Method::DELETE, &["labels", id])
    }

    // Threads

    /// `threads.list`.
    pub fn threads(&self, query: &ListQuery) -> Call<'a, ListThreadsResponse> {
        list(self.call(Method::GET, &["threads"]), query)
    }

    /// `threads.get`. `format` must not be [`Format::Raw`]; `metadata_headers` only applies to
    /// [`Format::Metadata`].
    pub fn thread(&self, id: &str, format: Format, metadata_headers: &[&str]) -> Call<'a, Thread> {
        get(
            self.call(Method::GET, &["threads", id]),
            format,
            metadata_headers,
        )
    }

    /// `threads.modify`: changes labels on every message in the thread.
    pub fn modify_thread(&self, id: &str, request: &ModifyThreadRequest) -> Call<'a, Thread> {
        self.call(Method::POST, &["threads", id, "modify"])
            .json(request)
    }

    /// `threads.trash`.
    pub fn trash_thread(&self, id: &str) -> Call<'a, Thread> {
        self.call(Method::POST, &["threads", id, "trash"])
    }

    /// `threads.untrash`.
    pub fn untrash_thread(&self, id: &str) -> Call<'a, Thread> {
        self.call(Method::POST, &["threads", id, "untrash"])
    }

    /// `threads.delete`: permanent.
    pub fn delete_thread(&self, id: &str) -> Call<'a, ()> {
        self.call(Method::DELETE, &["threads", id])
    }

    // Messages

    /// `messages.list`.
    pub fn messages(&self, query: &ListQuery) -> Call<'a, ListMessagesResponse> {
        list(self.call(Method::GET, &["messages"]), query)
    }

    /// `messages.get`. `metadata_headers` only applies to [`Format::Metadata`].
    pub fn message(
        &self,
        id: &str,
        format: Format,
        metadata_headers: &[&str],
    ) -> Call<'a, Message> {
        get(
            self.call(Method::GET, &["messages", id]),
            format,
            metadata_headers,
        )
    }

    /// `messages.modify`.
    pub fn modify_message(&self, id: &str, request: &ModifyMessageRequest) -> Call<'a, Message> {
        self.call(Method::POST, &["messages", id, "modify"])
            .json(request)
    }

    /// `messages.batchModify`: same label change on up to 1000 messages.
    pub fn batch_modify_messages(&self, request: &BatchModifyMessagesRequest) -> Call<'a, ()> {
        self.call(Method::POST, &["messages", "batchModify"])
            .json(request)
    }

    /// `messages.batchDelete`: permanent.
    pub fn batch_delete_messages(&self, ids: Vec<String>) -> Call<'a, ()> {
        self.call(Method::POST, &["messages", "batchDelete"])
            .json(&BatchDeleteMessagesRequest { ids })
    }

    /// `messages.trash`.
    pub fn trash_message(&self, id: &str) -> Call<'a, Message> {
        self.call(Method::POST, &["messages", id, "trash"])
    }

    /// `messages.untrash`.
    pub fn untrash_message(&self, id: &str) -> Call<'a, Message> {
        self.call(Method::POST, &["messages", id, "untrash"])
    }

    /// `messages.delete`: permanent.
    pub fn delete_message(&self, id: &str) -> Call<'a, ()> {
        self.call(Method::DELETE, &["messages", id])
    }

    /// `messages.send` (media upload) of an RFC 822 message, e.g. from [`crate::mime::Email`].
    /// To reply in a thread, pass its ID and set `In-Reply-To`/`References` and a matching
    /// `Subject` in the message.
    pub fn send(&self, rfc822: Vec<u8>, thread_id: Option<&str>) -> Call<'a, Message> {
        let metadata = Message {
            thread_id: thread_id.map(str::to_owned),
            ..Message::default()
        };
        self.upload_call(Method::POST, &["messages", "send"])
            .upload(&metadata, rfc822)
    }

    /// `messages.insert` (media upload): adds a message without sending or scanning it.
    pub fn insert_message(&self, rfc822: Vec<u8>, options: &InsertOptions) -> Call<'a, Message> {
        insert(
            self.upload_call(Method::POST, &["messages"]),
            rfc822,
            options,
        )
    }

    /// `messages.import` (media upload): adds a message as if it was delivered by SMTP.
    pub fn import_message(&self, rfc822: Vec<u8>, options: &InsertOptions) -> Call<'a, Message> {
        insert(
            self.upload_call(Method::POST, &["messages", "import"]),
            rfc822,
            options,
        )
    }

    /// `messages.attachments.get`. The data is in `data`.
    pub fn attachment(&self, message_id: &str, attachment_id: &str) -> Call<'a, MessagePartBody> {
        self.call(
            Method::GET,
            &["messages", message_id, "attachments", attachment_id],
        )
    }

    // Drafts

    /// `drafts.list`.
    pub fn drafts(&self, query: &DraftQuery) -> Call<'a, ListDraftsResponse> {
        self.call(Method::GET, &["drafts"])
            .query_opt("q", query.q.as_ref())
            .query_opt("maxResults", query.max_results)
            .query_opt("pageToken", query.page_token.as_ref())
            .query_opt("includeSpamTrash", query.include_spam_trash.then_some(true))
    }

    /// `drafts.get`.
    pub fn draft(&self, id: &str, format: Format) -> Call<'a, Draft> {
        get(self.call(Method::GET, &["drafts", id]), format, &[])
    }

    /// `drafts.create` (media upload).
    pub fn create_draft(&self, rfc822: Vec<u8>, thread_id: Option<&str>) -> Call<'a, Draft> {
        self.upload_call(Method::POST, &["drafts"])
            .upload(&draft_metadata(None, thread_id), rfc822)
    }

    /// `drafts.update` (media upload): replaces the draft's message.
    pub fn update_draft(
        &self,
        id: &str,
        rfc822: Vec<u8>,
        thread_id: Option<&str>,
    ) -> Call<'a, Draft> {
        self.upload_call(Method::PUT, &["drafts", id])
            .upload(&draft_metadata(Some(id), thread_id), rfc822)
    }

    /// `drafts.delete`: permanent.
    pub fn delete_draft(&self, id: &str) -> Call<'a, ()> {
        self.call(Method::DELETE, &["drafts", id])
    }

    /// `drafts.send` of an existing draft as last saved.
    pub fn send_draft(&self, id: &str) -> Call<'a, Message> {
        self.call(Method::POST, &["drafts", "send"])
            .json(&draft_metadata(Some(id), None))
    }

    // History

    /// `history.list`. A 404 ([`Error::is_not_found`]) means `start_history_id` expired.
    pub fn history(&self, query: &HistoryQuery) -> Call<'a, ListHistoryResponse> {
        self.call(Method::GET, &["history"])
            .query("startHistoryId", query.start_history_id)
            .query_all("historyTypes", query.history_types.iter().map(enum_value))
            .query_opt("labelId", query.label_id.as_ref())
            .query_opt("maxResults", query.max_results)
            .query_opt("pageToken", query.page_token.as_ref())
    }

    // Settings

    /// `settings.sendAs.list`: addresses the user can send as, with signatures.
    pub fn send_as_list(&self) -> Call<'a, ListSendAsResponse> {
        self.call(Method::GET, &["settings", "sendAs"])
    }

    /// `settings.sendAs.get`.
    pub fn send_as(&self, email: &str) -> Call<'a, SendAs> {
        self.call(Method::GET, &["settings", "sendAs", email])
    }

    /// `settings.sendAs.patch`, e.g. to change the signature.
    pub fn patch_send_as(&self, email: &str, send_as: &SendAs) -> Call<'a, SendAs> {
        self.call(Method::PATCH, &["settings", "sendAs", email])
            .json(send_as)
    }

    /// `settings.getVacation`.
    pub fn vacation(&self) -> Call<'a, VacationSettings> {
        self.call(Method::GET, &["settings", "vacation"])
    }

    /// `settings.updateVacation`.
    pub fn update_vacation(&self, settings: &VacationSettings) -> Call<'a, VacationSettings> {
        self.call(Method::PUT, &["settings", "vacation"])
            .json(settings)
    }

    /// `settings.filters.list`.
    pub fn filters(&self) -> Call<'a, ListFiltersResponse> {
        self.call(Method::GET, &["settings", "filters"])
    }

    /// `settings.filters.get`.
    pub fn filter(&self, id: &str) -> Call<'a, Filter> {
        self.call(Method::GET, &["settings", "filters", id])
    }

    /// `settings.filters.create`.
    pub fn create_filter(&self, filter: &Filter) -> Call<'a, Filter> {
        self.call(Method::POST, &["settings", "filters"])
            .json(filter)
    }

    /// `settings.filters.delete`.
    pub fn delete_filter(&self, id: &str) -> Call<'a, ()> {
        self.call(Method::DELETE, &["settings", "filters", id])
    }

    // Batch

    /// Runs up to [`MAX_BATCH`] calls in one HTTP request (`POST /batch/gmail/v1`).
    ///
    /// The outer `Result` fails when the batch itself can't be sent or read; each inner one is
    /// that call's own outcome, in the order given. Media uploads can't be batched.
    ///
    /// # Errors
    /// [`Error::Batch`] for too many calls, uploads or a malformed response; transport errors.
    pub async fn batch<T: DeserializeOwned>(
        &self,
        calls: Vec<Call<'_, T>>,
    ) -> Result<Vec<Result<T, Error>>, Error> {
        if calls.len() > MAX_BATCH {
            return Err(Error::Batch(format!(
                "{} calls; Gmail accepts at most {MAX_BATCH}",
                calls.len()
            )));
        }
        let mut writer = multipart::Writer::new(random_boundary("batch"));
        for (index, call) in calls.iter().enumerate() {
            let request = &call.request;
            let url = &request.url;
            let target = match url.query() {
                Some(query) => format!("{}?{query}", url.path()),
                None => url.path().to_owned(),
            };
            let mut inner = format!("{} {target} HTTP/1.1\r\n", request.method).into_bytes();
            match &request.body {
                Body::Empty => inner.extend_from_slice(b"\r\n"),
                Body::Json(json) => {
                    inner.extend_from_slice(
                        format!(
                            "Content-Type: application/json; charset=UTF-8\r\n\
                             Content-Length: {}\r\n\r\n",
                            json.len()
                        )
                        .as_bytes(),
                    );
                    inner.extend_from_slice(json);
                }
                Body::Upload { .. } => {
                    return Err(Error::Batch("media uploads can't be batched".into()));
                }
            }
            let content_id = format!("<item{index}>");
            writer.part(
                &[
                    ("Content-Type", "application/http"),
                    ("Content-ID", &content_id),
                ],
                &inner,
            );
        }
        let url = join(&self.client.endpoints.gmail, ["batch", "gmail", "v1"]);
        log::debug!("POST {url} ({} calls)", calls.len());
        let token = self.client.token().await?;
        let response = self
            .client
            .http
            .post(url)
            .bearer_auth(&token)
            .header(reqwest::header::CONTENT_TYPE, writer.content_type("mixed"))
            .body(writer.finish())
            .send()
            .await?;
        let status = response.status();
        self.client.invalidate_on_401(status, &token);
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body = response.bytes().await?;
        let Some(boundary) = multipart::boundary(&content_type) else {
            // The whole batch was rejected: Google sends a plain JSON error.
            return Err(decode::<serde_json::Value>(status, &body)
                .err()
                .unwrap_or_else(|| Error::Batch(format!("not multipart: {content_type}"))));
        };
        let parts = multipart::split(&body, boundary)
            .ok_or_else(|| Error::Batch("no parts in response".into()))?;

        let mut results: Vec<Option<Result<T, Error>>> = calls.iter().map(|_| None).collect();
        for part in parts {
            let index = part
                .header("Content-ID")
                .and_then(|id| id.trim_matches(['<', '>']).strip_prefix("response-item"))
                .and_then(|index| index.parse::<usize>().ok())
                .filter(|index| *index < results.len())
                .ok_or_else(|| Error::Batch("response part without a known Content-ID".into()))?;
            let (status_line, inner) = multipart::parse_http(part.body)
                .ok_or_else(|| Error::Batch("empty response part".into()))?;
            let status = status_line
                .split_whitespace()
                .nth(1)
                .and_then(|code| code.parse::<u16>().ok())
                .and_then(|code| reqwest::StatusCode::from_u16(code).ok())
                .ok_or_else(|| Error::Batch(format!("bad status line {status_line:?}")))?;
            self.client.invalidate_on_401(status, &token);
            results[index] = Some(decode(status, inner.body));
        }
        results
            .into_iter()
            .enumerate()
            .map(|(index, result)| {
                result.ok_or_else(|| Error::Batch(format!("no response for call {index}")))
            })
            .collect()
    }
}

fn list<'a, T>(call: Call<'a, T>, query: &ListQuery) -> Call<'a, T> {
    call.query_opt("q", query.q.as_ref())
        .query_all("labelIds", &query.label_ids)
        .query_opt("maxResults", query.max_results)
        .query_opt("pageToken", query.page_token.as_ref())
        .query_opt("includeSpamTrash", query.include_spam_trash.then_some(true))
}

fn get<'a, T>(call: Call<'a, T>, format: Format, metadata_headers: &[&str]) -> Call<'a, T> {
    call.query("format", enum_value(&format))
        .query_all("metadataHeaders", metadata_headers)
}

fn insert<'a>(
    call: Call<'a, Message>,
    rfc822: Vec<u8>,
    options: &InsertOptions,
) -> Call<'a, Message> {
    let metadata = Message {
        thread_id: options.thread_id.clone(),
        label_ids: options.label_ids.clone(),
        ..Message::default()
    };
    call.query_opt(
        "internalDateSource",
        options.internal_date_source.as_ref().map(enum_value),
    )
    .upload(&metadata, rfc822)
}

fn draft_metadata(id: Option<&str>, thread_id: Option<&str>) -> Draft {
    Draft {
        id: id.map(str::to_owned),
        message: thread_id.map(|thread_id| Message {
            thread_id: Some(thread_id.to_owned()),
            ..Message::default()
        }),
    }
}

/// The wire spelling of a unit enum (`Format::Metadata` → `metadata`).
fn enum_value(value: &impl Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(text)) => text,
        _ => unreachable!("query enums serialize as strings"),
    }
}
