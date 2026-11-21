//! Gmail v1 handlers, at `/gmail/v1/users/{userId}/...` and the `/upload/` variants.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use gmail::*;

use super::search::Query;
use super::wire::{draft, thread, thread_list_item, thread_reference};
use super::{NewMessage, Selection, Threading, page_token};
use crate::error::{Failure, Result};
use crate::http::extract::{Json as Body, Params, Upload};
use crate::world::{App, World};

type Shared = State<Arc<App>>;
type Reply<T> = Result<axum::Json<T>>;

pub fn router(app: Arc<App>) -> Router<Arc<App>> {
    let users = Router::new()
        .route("/profile", get(profile))
        .route("/labels", get(list_labels).post(create_label))
        .route(
            "/labels/{id}",
            get(get_label)
                .put(update_label)
                .patch(patch_label)
                .delete(delete_label),
        )
        .route("/threads", get(list_threads))
        .route("/threads/{id}", get(get_thread).delete(delete_thread))
        .route("/threads/{id}/modify", post(modify_thread))
        .route("/threads/{id}/trash", post(trash_thread))
        .route("/threads/{id}/untrash", post(untrash_thread))
        .route("/messages", get(list_messages).post(insert_message))
        .route("/messages/send", post(send_message))
        .route("/messages/import", post(import_message))
        .route("/messages/batchModify", post(batch_modify))
        .route("/messages/batchDelete", post(batch_delete))
        .route("/messages/{id}", get(get_message).delete(delete_message))
        .route("/messages/{id}/modify", post(modify_message))
        .route("/messages/{id}/trash", post(trash_message))
        .route("/messages/{id}/untrash", post(untrash_message))
        .route("/messages/{message}/attachments/{id}", get(get_attachment))
        .route("/drafts", get(list_drafts).post(create_draft))
        .route("/drafts/send", post(send_draft))
        .route(
            "/drafts/{id}",
            get(get_draft).put(update_draft).delete(delete_draft),
        )
        .route("/history", get(list_history))
        .route("/settings/vacation", get(get_vacation).put(update_vacation))
        .route("/settings/sendAs", get(list_send_as))
        .route(
            "/settings/sendAs/{email}",
            get(get_send_as).put(update_send_as).patch(patch_send_as),
        )
        .route("/settings/filters", get(list_filters).post(create_filter))
        .route(
            "/settings/filters/{id}",
            get(get_filter).delete(delete_filter),
        );

    let uploads = Router::new()
        .route("/messages", post(insert_message))
        .route("/messages/send", post(send_message))
        .route("/messages/import", post(import_message))
        .route("/drafts", post(create_draft))
        .route("/drafts/send", post(send_draft))
        .route("/drafts/{id}", put(update_draft));

    // Every route is under `users/{userId}`; checking it once keeps handlers to their logic.
    Router::new()
        .nest("/gmail/v1/users/{user}", users)
        .nest("/upload/gmail/v1/users/{user}", uploads)
        .route_layer(from_fn_with_state(app, check_user))
}

/// Rejects `userId`s other than `me` and the account address.
async fn check_user(
    State(app): Shared,
    Path(params): Path<HashMap<String, String>>,
    request: Request,
    next: Next,
) -> Response {
    if let Some(user) = params.get("user")
        && let Err(failure) = app.lock().mail.check_user(user)
    {
        return failure.into_response();
    }
    next.run(request).await
}

fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

// Profile

/// The mailbox's, reporting the address the token signs in (`oauth::Identity`).
async fn profile(
    State(app): Shared,
    axum::Extension(identity): axum::Extension<crate::oauth::Identity>,
) -> Reply<Profile> {
    let world = app.lock();
    let mail = &world.mail;
    Ok(axum::Json(Profile {
        email_address: Some(identity.0.unwrap_or_else(|| mail.email.clone())),
        messages_total: u32::try_from(mail.message_count()).ok(),
        threads_total: u32::try_from(mail.thread_count()).ok(),
        history_id: Some(mail.history.current()),
    }))
}

// Labels

async fn list_labels(State(app): Shared) -> Reply<ListLabelsResponse> {
    Ok(axum::Json(ListLabelsResponse {
        labels: app.lock().mail.labels().to_vec(),
    }))
}

async fn get_label(State(app): Shared, Path((_, id)): Path<(String, String)>) -> Reply<Label> {
    Ok(axum::Json(app.lock().mail.label(&id)?))
}

async fn create_label(State(app): Shared, Body(label): Body<Label>) -> Reply<Label> {
    Ok(axum::Json(app.lock().mail.create_label(label)?))
}

async fn update_label(
    State(app): Shared,
    Path((_, id)): Path<(String, String)>,
    Body(label): Body<Label>,
) -> Reply<Label> {
    Ok(axum::Json(app.lock().mail.update_label(&id, label, false)?))
}

async fn patch_label(
    State(app): Shared,
    Path((_, id)): Path<(String, String)>,
    Body(label): Body<Label>,
) -> Reply<Label> {
    Ok(axum::Json(app.lock().mail.update_label(&id, label, true)?))
}

async fn delete_label(
    State(app): Shared,
    Path((_, id)): Path<(String, String)>,
) -> Result<Response> {
    app.lock().mail.delete_label(&id)?;
    Ok(no_content())
}

// Threads

/// `q`, `labelIds` and `includeSpamTrash`, shared by the list endpoints.
fn selection(world: &World, params: &Params) -> Result<Selection> {
    Ok(Selection {
        query: params
            .text("q")
            .map(|q| Query::compile(q, &world.mail.context(world.clock.now()))),
        label_ids: params.all("labelIds"),
        include_spam_trash: params.bool("includeSpamTrash")?,
    })
}

fn format(params: &Params, allow_raw: bool) -> Result<(Format, Vec<String>)> {
    const PROTO: &str = "type.googleapis.com/gmail.v1.Format";
    let format = params
        .enumeration::<Format>("format", PROTO)?
        .unwrap_or_default();
    if format == Format::Raw && !allow_raw {
        return Err(Failure::bad_value(
            "format",
            PROTO,
            params.get("format").unwrap_or_default(),
        ));
    }
    Ok((format, params.all("metadataHeaders")))
}

fn count(n: usize) -> Option<u32> {
    Some(u32::try_from(n).unwrap_or(u32::MAX))
}

async fn list_threads(State(app): Shared, params: Params) -> Reply<ListThreadsResponse> {
    let world = app.lock();
    let selection = selection(&world, &params)?;
    let max = params.max_results(100, 500)?;
    let hits = world.mail.select_threads(&selection);
    let total = hits.len();
    let (page, next) = page_token::page(hits, |(_, key)| *key, params.text("pageToken"), max)?;
    let threads = page
        .into_iter()
        .map(|(id, _)| Ok(thread_list_item(id, &world.mail.thread(id)?)))
        .collect::<Result<_>>()?;
    Ok(axum::Json(ListThreadsResponse {
        threads,
        next_page_token: next,
        result_size_estimate: count(total),
    }))
}

async fn get_thread(
    State(app): Shared,
    Path((_, id)): Path<(String, String)>,
    params: Params,
) -> Reply<Thread> {
    let (format, headers) = format(&params, false)?;
    let world = app.lock();
    Ok(axum::Json(thread(
        &id,
        &world.mail.thread(&id)?,
        format,
        &headers,
    )))
}

async fn modify_thread(
    State(app): Shared,
    Path((_, id)): Path<(String, String)>,
    Body(request): Body<ModifyThreadRequest>,
) -> Reply<Thread> {
    let mut world = app.lock();
    world
        .mail
        .modify_thread(&id, &request.add_label_ids, &request.remove_label_ids)?;
    Ok(axum::Json(thread_reference(&id, &world.mail.thread(&id)?)))
}

async fn trash_thread(State(app): Shared, Path((_, id)): Path<(String, String)>) -> Reply<Thread> {
    let mut world = app.lock();
    for message in world.mail.thread_message_ids(&id)? {
        world.mail.trash(&message)?;
    }
    Ok(axum::Json(thread_reference(&id, &world.mail.thread(&id)?)))
}

async fn untrash_thread(
    State(app): Shared,
    Path((_, id)): Path<(String, String)>,
) -> Reply<Thread> {
    let mut world = app.lock();
    for message in world.mail.thread_message_ids(&id)? {
        world.mail.untrash(&message)?;
    }
    Ok(axum::Json(thread_reference(&id, &world.mail.thread(&id)?)))
}

async fn delete_thread(
    State(app): Shared,
    Path((_, id)): Path<(String, String)>,
) -> Result<Response> {
    let mut world = app.lock();
    for message in world.mail.thread_message_ids(&id)? {
        world.mail.delete(&message)?;
    }
    Ok(no_content())
}

// Messages

async fn list_messages(State(app): Shared, params: Params) -> Reply<ListMessagesResponse> {
    let world = app.lock();
    let selection = selection(&world, &params)?;
    let max = params.max_results(100, 500)?;
    let hits = world.mail.select_messages(&selection);
    let total = hits.len();
    let (page, next) = page_token::page(hits, |m| m.sort_key(), params.text("pageToken"), max)?;
    Ok(axum::Json(ListMessagesResponse {
        messages: page.into_iter().map(|m| m.to_list_item()).collect(),
        next_page_token: next,
        result_size_estimate: count(total),
    }))
}

async fn get_message(
    State(app): Shared,
    Path((_, id)): Path<(String, String)>,
    params: Params,
) -> Reply<Message> {
    let (format, headers) = format(&params, true)?;
    Ok(axum::Json(
        app.lock().mail.message(&id)?.to_wire(format, &headers),
    ))
}

async fn modify_message(
    State(app): Shared,
    Path((_, id)): Path<(String, String)>,
    Body(request): Body<ModifyMessageRequest>,
) -> Reply<Message> {
    let mut world = app.lock();
    world
        .mail
        .modify(&id, &request.add_label_ids, &request.remove_label_ids)?;
    Ok(axum::Json(world.mail.message(&id)?.to_reference()))
}

async fn trash_message(
    State(app): Shared,
    Path((_, id)): Path<(String, String)>,
) -> Reply<Message> {
    let mut world = app.lock();
    world.mail.trash(&id)?;
    Ok(axum::Json(world.mail.message(&id)?.to_reference()))
}

async fn untrash_message(
    State(app): Shared,
    Path((_, id)): Path<(String, String)>,
) -> Reply<Message> {
    let mut world = app.lock();
    world.mail.untrash(&id)?;
    Ok(axum::Json(world.mail.message(&id)?.to_reference()))
}

async fn delete_message(
    State(app): Shared,
    Path((_, id)): Path<(String, String)>,
) -> Result<Response> {
    app.lock().mail.delete(&id)?;
    Ok(no_content())
}

/// Most IDs `batchModify`/`batchDelete` accept.
const MAX_BATCH_IDS: usize = 1000;

async fn batch_modify(
    State(app): Shared,
    Body(request): Body<BatchModifyMessagesRequest>,
) -> Result<Response> {
    if request.ids.len() > MAX_BATCH_IDS {
        return Err(Failure::invalid_argument(format!(
            "Too many ids; at most {MAX_BATCH_IDS} are allowed"
        )));
    }
    let mut world = app.lock();
    // Validate everything first so a bad label changes nothing.
    for id in &request.ids {
        world.mail.message(id)?;
    }
    for id in &request.ids {
        world
            .mail
            .modify(id, &request.add_label_ids, &request.remove_label_ids)?;
    }
    Ok(no_content())
}

async fn batch_delete(
    State(app): Shared,
    Body(request): Body<BatchDeleteMessagesRequest>,
) -> Result<Response> {
    if request.ids.len() > MAX_BATCH_IDS {
        return Err(Failure::invalid_argument(format!(
            "Too many ids; at most {MAX_BATCH_IDS} are allowed"
        )));
    }
    let mut world = app.lock();
    for id in &request.ids {
        world.mail.message(id)?;
    }
    for id in &request.ids {
        world.mail.delete(id)?;
    }
    Ok(no_content())
}

fn require_raw(message: &mut Message) -> Result<Vec<u8>> {
    message.raw.take().ok_or_else(|| {
        Failure::invalid_argument(
            "'raw' RFC822 payload message string or uploading message via /upload/* URL required",
        )
    })
}

async fn send_message(State(app): Shared, Upload(mut message): Upload<Message>) -> Reply<Message> {
    let raw = require_raw(&mut message)?;
    let mut world = app.lock();
    let now = world.clock.now_ms();
    let id = world.mail.send(raw, message.thread_id, now)?;
    Ok(axum::Json(world.mail.message(&id)?.to_reference()))
}

async fn insert_message(
    State(app): Shared,
    params: Params,
    Upload(message): Upload<Message>,
) -> Reply<Message> {
    add_message(
        &app,
        &params,
        message,
        InternalDateSource::ReceivedTime,
        false,
    )
}

async fn import_message(
    State(app): Shared,
    params: Params,
    Upload(message): Upload<Message>,
) -> Reply<Message> {
    add_message(&app, &params, message, InternalDateSource::DateHeader, true)
}

/// `messages.insert` and `messages.import`, which differ in the default date source and in
/// import running the user's filters. `deleted`, `neverMarkSpam` and `processForCalendar` are
/// accepted and ignored.
fn add_message(
    app: &App,
    params: &Params,
    mut message: Message,
    default_source: InternalDateSource,
    import: bool,
) -> Reply<Message> {
    let raw = require_raw(&mut message)?;
    let source = params
        .enumeration(
            "internalDateSource",
            "type.googleapis.com/gmail.v1.InternalDateSource",
        )?
        .unwrap_or(default_source);
    let mut world = app.lock();
    let now = world.clock.now_ms();
    let internal_date = match source {
        InternalDateSource::ReceivedTime => now,
        InternalDateSource::DateHeader => super::mime::parse(&raw, "probe")
            .and_then(|parsed| parsed.date)
            .unwrap_or(now),
    };
    let id = world.mail.insert(NewMessage {
        raw,
        labels: message.label_ids,
        threading: Threading::Requested(message.thread_id),
        internal_date,
        apply_filters: import,
    })?;
    Ok(axum::Json(world.mail.message(&id)?.to_reference()))
}

async fn get_attachment(
    State(app): Shared,
    Path((_, message, id)): Path<(String, String, String)>,
) -> Reply<MessagePartBody> {
    let world = app.lock();
    let data = world.mail.attachment(&message, &id)?;
    Ok(axum::Json(MessagePartBody {
        attachment_id: None,
        size: count(data.len()),
        data: Some(data.to_vec()),
    }))
}

// Drafts

async fn list_drafts(State(app): Shared, params: Params) -> Reply<ListDraftsResponse> {
    let world = app.lock();
    let selection = selection(&world, &params)?;
    let max = params.max_results(100, 500)?;
    let matching: Vec<_> = world
        .mail
        .select_messages(&Selection {
            label_ids: vec!["DRAFT".into()],
            ..selection
        })
        .into_iter()
        .map(|message| message.id.clone())
        .collect();
    let drafts: Vec<_> = world
        .mail
        .drafts()
        .into_iter()
        .filter(|(_, message)| matching.contains(&message.id))
        .collect();
    let total = drafts.len();
    let (page, next) =
        page_token::page(drafts, |(_, m)| m.sort_key(), params.text("pageToken"), max)?;
    Ok(axum::Json(ListDraftsResponse {
        drafts: page
            .into_iter()
            .map(|(id, message)| draft(id, message.to_list_item()))
            .collect(),
        next_page_token: next,
        result_size_estimate: count(total),
    }))
}

async fn get_draft(
    State(app): Shared,
    Path((_, id)): Path<(String, String)>,
    params: Params,
) -> Reply<Draft> {
    let (format, headers) = format(&params, true)?;
    let world = app.lock();
    Ok(axum::Json(draft(
        &id,
        world.mail.draft(&id)?.to_wire(format, &headers),
    )))
}

fn draft_parts(request: &mut Draft) -> Result<(Vec<u8>, Option<String>)> {
    let message = request.message.get_or_insert_default();
    Ok((require_raw(message)?, message.thread_id.clone()))
}

async fn create_draft(State(app): Shared, Upload(mut request): Upload<Draft>) -> Reply<Draft> {
    let (raw, thread_id) = draft_parts(&mut request)?;
    let mut world = app.lock();
    let now = world.clock.now_ms();
    let id = world.mail.create_draft(raw, thread_id, now)?;
    Ok(axum::Json(draft(
        &id,
        world.mail.draft(&id)?.to_reference(),
    )))
}

async fn update_draft(
    State(app): Shared,
    Path((_, id)): Path<(String, String)>,
    Upload(mut request): Upload<Draft>,
) -> Reply<Draft> {
    let (raw, thread_id) = draft_parts(&mut request)?;
    let mut world = app.lock();
    let now = world.clock.now_ms();
    world.mail.update_draft(&id, raw, thread_id, now)?;
    Ok(axum::Json(draft(
        &id,
        world.mail.draft(&id)?.to_reference(),
    )))
}

async fn delete_draft(
    State(app): Shared,
    Path((_, id)): Path<(String, String)>,
) -> Result<Response> {
    app.lock().mail.delete_draft(&id)?;
    Ok(no_content())
}

/// Sends `id`, first saving the message in the request if it carries one.
async fn send_draft(State(app): Shared, Upload(mut request): Upload<Draft>) -> Reply<Message> {
    let id = request
        .id
        .clone()
        .ok_or_else(|| Failure::invalid_argument("Invalid draft"))?;
    let mut world = app.lock();
    let now = world.clock.now_ms();
    if request.message.as_ref().is_some_and(|m| m.raw.is_some()) {
        let (raw, thread_id) = draft_parts(&mut request)?;
        world.mail.update_draft(&id, raw, thread_id, now)?;
    }
    let sent = world.mail.send_draft(&id, now)?;
    Ok(axum::Json(world.mail.message(&sent)?.to_reference()))
}

// History

async fn list_history(State(app): Shared, params: Params) -> Reply<ListHistoryResponse> {
    let start = params
        .u64("startHistoryId")?
        .ok_or_else(|| Failure::invalid_argument("startHistoryId is required"))?;
    let types: Vec<HistoryType> =
        params.enumerations("historyTypes", "type.googleapis.com/gmail.v1.HistoryType")?;
    let label = params.text("labelId");
    let max = params.max_results(100, 500)?;
    let after = match params.text("pageToken") {
        Some(token) => token
            .parse::<u64>()
            .map_err(|_| Failure::invalid_argument("Invalid pageToken"))?,
        None => start,
    };
    let world = app.lock();
    let history = &world.mail.history;
    // An expired start is a 404, even mid-pagination.
    if history.since(start).is_none() {
        return Err(Failure::not_found());
    }
    let mut records = history
        .since(after)
        .ok_or_else(Failure::not_found)?
        .filter(|record| types.is_empty() || types.contains(&record.kind()))
        .filter(|record| label.is_none_or(|label| record.involves_label(label)))
        .peekable();
    let page: Vec<_> = records.by_ref().take(max).collect();
    let next = records
        .peek()
        .and(page.last())
        .map(|last| last.id.to_string());
    Ok(axum::Json(ListHistoryResponse {
        history: page.iter().map(|record| record.to_wire()).collect(),
        next_page_token: next,
        history_id: Some(history.current()),
    }))
}

// Settings

async fn get_vacation(State(app): Shared) -> Reply<VacationSettings> {
    Ok(axum::Json(app.lock().mail.settings.vacation.clone()))
}

async fn update_vacation(
    State(app): Shared,
    Body(mut settings): Body<VacationSettings>,
) -> Reply<VacationSettings> {
    // Gmail keeps one body: given both, the HTML one ("If both responseBodyPlainText and
    // responseBodyHtml are specified, responseBodyHtml will be used").
    if settings
        .response_body_html
        .as_deref()
        .is_some_and(|html| !html.is_empty())
    {
        settings.response_body_plain_text = None;
    }
    let mut world = app.lock();
    world.mail.settings.vacation = settings.clone();
    Ok(axum::Json(settings))
}

async fn list_send_as(State(app): Shared) -> Reply<ListSendAsResponse> {
    Ok(axum::Json(ListSendAsResponse {
        send_as: app.lock().mail.settings.send_as.clone(),
    }))
}

fn send_as_index(world: &World, email: &str) -> Result<usize> {
    world
        .mail
        .settings
        .send_as
        .iter()
        .position(|alias| {
            alias
                .send_as_email
                .as_deref()
                .is_some_and(|other| other.eq_ignore_ascii_case(email))
        })
        .ok_or_else(Failure::not_found)
}

async fn get_send_as(
    State(app): Shared,
    Path((_, email)): Path<(String, String)>,
) -> Reply<SendAs> {
    let world = app.lock();
    let index = send_as_index(&world, &email)?;
    Ok(axum::Json(world.mail.settings.send_as[index].clone()))
}

async fn update_send_as(
    State(app): Shared,
    Path((_, email)): Path<(String, String)>,
    Body(request): Body<SendAs>,
) -> Reply<SendAs> {
    change_send_as(&app, &email, request, false)
}

async fn patch_send_as(
    State(app): Shared,
    Path((_, email)): Path<(String, String)>,
    Body(request): Body<SendAs>,
) -> Reply<SendAs> {
    change_send_as(&app, &email, request, true)
}

/// Updates the user-settable fields of an alias; making one the default clears the others.
fn change_send_as(app: &App, email: &str, request: SendAs, patch: bool) -> Reply<SendAs> {
    let mut world = app.lock();
    let index = send_as_index(&world, email)?;
    let aliases = &mut world.mail.settings.send_as;
    let keep = |new: Option<String>, old: &Option<String>| {
        if patch {
            new.or_else(|| old.clone())
        } else {
            new.or_else(|| Some(String::new()))
        }
    };
    let alias = &mut aliases[index];
    alias.display_name = keep(request.display_name, &alias.display_name);
    alias.reply_to_address = keep(request.reply_to_address, &alias.reply_to_address);
    alias.signature = keep(request.signature, &alias.signature);
    if request.treat_as_alias.is_some() && alias.is_primary != Some(true) {
        alias.treat_as_alias = request.treat_as_alias;
    }
    if request.is_default == Some(true) {
        for (position, other) in aliases.iter_mut().enumerate() {
            other.is_default = Some(position == index);
        }
    }
    Ok(axum::Json(aliases[index].clone()))
}

async fn list_filters(State(app): Shared) -> Reply<ListFiltersResponse> {
    Ok(axum::Json(ListFiltersResponse {
        filter: app.lock().mail.settings.filters.clone(),
    }))
}

async fn get_filter(State(app): Shared, Path((_, id)): Path<(String, String)>) -> Reply<Filter> {
    let world = app.lock();
    world
        .mail
        .settings
        .filters
        .iter()
        .find(|filter| filter.id.as_deref() == Some(&id))
        .cloned()
        .map(axum::Json)
        .ok_or_else(Failure::not_found)
}

/// Creates a filter. A client-supplied `id` is output-only and ignored.
async fn create_filter(State(app): Shared, Body(request): Body<Filter>) -> Reply<Filter> {
    let criteria = request.criteria.unwrap_or_default();
    let action = request.action.unwrap_or_default();
    if criteria == FilterCriteria::default() {
        return Err(Failure::failed_precondition(
            "Filter doesn't have any criteria",
        ));
    }
    if action == FilterAction::default() {
        return Err(Failure::failed_precondition(
            "Filter doesn't have any actions",
        ));
    }
    let mut world = app.lock();
    for label in action.add_label_ids.iter().chain(&action.remove_label_ids) {
        world
            .mail
            .label(label)
            .map_err(|_| Failure::invalid_argument(format!("Invalid label: {label}")))?;
    }
    let duplicate = world.mail.settings.filters.iter().any(|filter| {
        filter.criteria.as_ref() == Some(&criteria) && filter.action.as_ref() == Some(&action)
    });
    if duplicate {
        return Err(Failure::failed_precondition("Filter already exists"));
    }
    let filter = Filter {
        id: Some(world.mail.next_opaque_id("ANe1Bm")),
        criteria: Some(criteria),
        action: Some(action),
    };
    world.mail.settings.filters.push(filter.clone());
    Ok(axum::Json(filter))
}

async fn delete_filter(
    State(app): Shared,
    Path((_, id)): Path<(String, String)>,
) -> Result<Response> {
    let mut world = app.lock();
    let filters = &mut world.mail.settings.filters;
    let before = filters.len();
    filters.retain(|filter| filter.id.as_deref() != Some(&id));
    if filters.len() == before {
        return Err(Failure::not_found());
    }
    Ok(no_content())
}
