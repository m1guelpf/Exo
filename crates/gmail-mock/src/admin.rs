//! `/__mock/` endpoints for tests and UI work. Unauthenticated; not part of any Google API.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use gmail::mime::{Email, Mailbox};
use gmail::{Format, Message};
use jiff::{Timestamp, Zoned};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{Failure, Result};
use crate::http::extract::Json;
use crate::mail::{NewMessage, Threading};
use crate::world::{App, Fault};

type Shared = State<Arc<App>>;

pub fn router() -> Router<Arc<App>> {
    Router::new()
        .route("/__mock/reset", post(reset))
        .route("/__mock/state", get(state))
        .route("/__mock/deliver", post(deliver))
        .route("/__mock/clock", get(clock).post(set_clock))
        .route("/__mock/history/expire", post(expire_history))
        .route("/__mock/faults", post(add_fault).delete(clear_faults))
        .route("/__mock/quota", get(quota))
        .route("/__mock/unsubscribe/{*list}", post(unsubscribe))
        .route("/__mock/unsubscribes", get(unsubscribes))
        .route("/__mock/photo/{name}", get(photo))
}

/// The account's profile photo (`Config::photo`), answered as Google's image server answers
/// its URLs: `{id}=s{n}` is `n` pixels square (`-c` crops, which a square photo needs none of),
/// and without options it's the 512px original. Public, as Google's are.
async fn photo(State(app): Shared, Path(name): Path<String>) -> Response {
    let (id, options) = name.split_once('=').unwrap_or((&name, ""));
    let ours = app
        .lock()
        .account
        .photo
        .as_deref()
        .is_some_and(|photo| photo.rsplit('/').next() == Some(id));
    if !ours {
        return StatusCode::NOT_FOUND.into_response();
    }
    let size = options
        .split('-')
        .find_map(|option| option.strip_prefix('s')?.parse::<u32>().ok())
        .unwrap_or(512)
        .clamp(1, 1024);
    (
        [(header::CONTENT_TYPE, "image/png")],
        crate::fixtures::portrait_png(size),
    )
        .into_response()
}

/// A one-click unsubscribe (RFC 8058) the mock took, as the list's server would see it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unsubscribe {
    /// The path after `/__mock/unsubscribe/`: the list's link without its scheme.
    pub list: String,
    /// The link's query.
    pub query: Option<String>,
    pub content_type: Option<String>,
    pub body: String,
    /// The headers it had that carry credentials (`Cookie`, `Authorization`), by name.
    pub credentials: Vec<String>,
}

/// A newsletter's one-click unsubscribe endpoint: records the POST and answers 200, or the
/// status in an `answer` query parameter (a 302 points elsewhere, which a client shouldn't
/// follow).
async fn unsubscribe(
    State(app): Shared,
    Path(list): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: String,
) -> Response {
    let credentials = [header::COOKIE, header::AUTHORIZATION]
        .into_iter()
        .filter(|name| headers.contains_key(name))
        .map(|name| name.to_string())
        .collect();
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let answer = query
        .as_deref()
        .and_then(|query| {
            url::form_urlencoded::parse(query.as_bytes()).find(|(key, _)| key == "answer")
        })
        .and_then(|(_, status)| status.parse::<u16>().ok());
    app.lock().unsubscribes.push(Unsubscribe {
        list,
        query,
        content_type,
        body,
        credentials,
    });
    match answer.and_then(|status| StatusCode::from_u16(status).ok()) {
        Some(status) if status.is_redirection() => {
            (status, [(header::LOCATION, "http://127.0.0.1:1/elsewhere")]).into_response()
        }
        Some(status) => status.into_response(),
        None => StatusCode::OK.into_response(),
    }
}

/// The one-click unsubscribes taken since the world was built, oldest first.
async fn unsubscribes(State(app): Shared) -> axum::Json<Vec<Unsubscribe>> {
    axum::Json(app.lock().unsubscribes.clone())
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Reset {
    seed: Option<u64>,
    /// RFC 3339 with a time zone annotation, e.g. `2026-11-17T09:00:00-05:00[America/New_York]`.
    now: Option<String>,
}

/// Regenerates the fixtures, optionally with a new seed or time.
async fn reset(State(app): Shared, Json(request): Json<Reset>) -> Result<Response> {
    let mut world = app.lock();
    let mut config = world.config.clone();
    if let Some(seed) = request.seed {
        config.seed = seed;
    }
    if let Some(now) = request.now {
        config.now = parse_time(&now, config.now.time_zone())?;
    }
    *world = crate::fixtures::world(&config);
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn parse_time(text: &str, tz: &jiff::tz::TimeZone) -> Result<Zoned> {
    crate::world::parse_time(text, tz)
        .map_err(|error| Failure::invalid_argument(format!("Invalid time {text:?}: {error}")))
}

/// A summary of the mock's state: account, clock, labels with counts, and every thread.
async fn state(State(app): Shared) -> axum::Json<Value> {
    let world = app.lock();
    let mail = &world.mail;
    let labels: Vec<_> = mail
        .labels()
        .iter()
        .filter_map(|label| mail.label(label.id.as_deref()?).ok())
        .collect();
    let selection = crate::mail::Selection {
        query: None,
        label_ids: Vec::new(),
        include_spam_trash: true,
    };
    let threads: Vec<Value> = mail
        .select_threads(&selection)
        .into_iter()
        .filter_map(|(id, _)| {
            let messages = mail.thread(id).ok()?;
            Some(json!({
                "id": id,
                "subject": messages.first().map(|m| m.mime.subject.clone()),
                "messages": messages.iter().map(|m| json!({
                    "id": m.id,
                    "from": m.mime.payload.header("From"),
                    "to": m.mime.payload.header("To"),
                    "date": m.mime.payload.header("Date"),
                    "labelIds": m.labels,
                    "snippet": m.mime.snippet,
                })).collect::<Vec<_>>(),
            }))
        })
        .collect();
    axum::Json(json!({
        "account": { "email": world.account.email, "name": world.account.name() },
        "now": world.clock.now().to_zoned(world.account.tz.clone()).to_string(),
        "seed": world.config.seed,
        "historyId": mail.history.current().to_string(),
        "messagesTotal": mail.message_count(),
        "threadsTotal": mail.thread_count(),
        "drafts": mail.drafts().len(),
        "labels": labels,
        "filters": mail.settings.filters,
        "calendarEvents": world.calendar.entries.iter().filter_map(|entry| {
            let id = entry.id.as_deref()?;
            Some((id.to_owned(), world.calendar.stored(id).count()))
        }).collect::<std::collections::BTreeMap<_, _>>(),
        "contacts": world.people.contacts.len(),
        "threads": threads,
    }))
}

/// An incoming message: either `raw` (base64url RFC 822) or fields to compose one from.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Delivery {
    raw: Option<String>,
    /// `Name <address>` or a bare address.
    from: Option<String>,
    /// Defaults to the account.
    to: Option<Vec<String>>,
    cc: Option<Vec<String>>,
    subject: Option<String>,
    text: Option<String>,
    html: Option<String>,
    /// Reply into this thread (sets `In-Reply-To`/`References` and a `Re:` subject).
    thread_id: Option<String>,
    /// Defaults to `INBOX`, `UNREAD`, `CATEGORY_PERSONAL`.
    label_ids: Option<Vec<String>>,
}

/// Simulates a message arriving by SMTP: threaded by headers, filters applied.
async fn deliver(
    State(app): Shared,
    Json(delivery): Json<Delivery>,
) -> Result<axum::Json<Message>> {
    let mut world = app.lock();
    let now = world.clock.now();
    let raw = match &delivery.raw {
        Some(raw) => gmail::wire::decode_base64url(raw)
            .map_err(|error| Failure::invalid_argument(format!("Invalid raw: {error}")))?,
        None => compose(&world, &delivery, now)?,
    };
    let labels = delivery.label_ids.unwrap_or_else(|| {
        ["INBOX", "UNREAD", "CATEGORY_PERSONAL"]
            .map(String::from)
            .to_vec()
    });
    let id = world.mail.insert(NewMessage {
        raw,
        labels,
        threading: Threading::Headers,
        internal_date: now.as_millisecond(),
        apply_filters: true,
    })?;
    Ok(axum::Json(
        world.mail.message(&id)?.to_wire(Format::Minimal, &[]),
    ))
}

fn compose(world: &crate::world::World, delivery: &Delivery, now: Timestamp) -> Result<Vec<u8>> {
    let from = Mailbox::parse(
        delivery
            .from
            .as_deref()
            .unwrap_or("Notifications <notifications@example.com>"),
    );
    let mut email = Email {
        to: match &delivery.to {
            Some(to) => to.iter().map(|to| Mailbox::parse(to)).collect(),
            None => vec![Mailbox::new(world.account.name(), &world.account.email)],
        },
        cc: delivery
            .cc
            .iter()
            .flatten()
            .map(|cc| Mailbox::parse(cc))
            .collect(),
        subject: delivery.subject.clone().unwrap_or_default(),
        text: delivery.text.clone(),
        html: delivery.html.clone(),
        date: Some(now.to_zoned(world.account.tz.clone())),
        message_id: Some(format!(
            "{}.{}@{}",
            now.as_millisecond(),
            world.mail.message_count(),
            from.email
                .split_once('@')
                .map_or("example.com", |(_, domain)| domain)
        )),
        headers: vec![
            ("Delivered-To".into(), world.account.email.clone()),
            (
                "Received".into(),
                format!(
                    "by 2002:a05:6a00:mock with SMTP id mock; {}",
                    jiff::fmt::rfc2822::to_string(&now.to_zoned(world.account.tz.clone()))
                        .unwrap_or_default()
                ),
            ),
        ],
        from: Some(from),
        ..Email::default()
    };
    if let Some(thread_id) = &delivery.thread_id {
        let thread = world.mail.thread(thread_id)?;
        let last = thread.last().ok_or_else(Failure::not_found)?;
        email.reply_to_message(&last.to_wire(Format::Metadata, &[]));
        if email.subject.is_empty() {
            email.subject = gmail::mime::reply_subject(&last.mime.subject);
        }
    }
    Ok(email.to_rfc822())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetClock {
    /// RFC 3339 instant.
    now: String,
}

async fn clock(State(app): Shared) -> axum::Json<Value> {
    let world = app.lock();
    axum::Json(json!({ "now": world.clock.now().to_string() }))
}

/// Moves the clock; it keeps running from the new time.
async fn set_clock(
    State(app): Shared,
    axum::Json(request): axum::Json<SetClock>,
) -> Result<Response> {
    let mut world = app.lock();
    let at = parse_time(&request.now, &world.account.tz)?.timestamp();
    world.clock.set(at);
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Forgets all history, so every earlier `startHistoryId` gets a 404 (forcing a full sync).
async fn expire_history(State(app): Shared) -> Response {
    app.lock().mail.history.expire();
    StatusCode::NO_CONTENT.into_response()
}

/// Arms a fault: the next `times` requests whose path starts with `path` (and whose method
/// matches, if given) fail with `status`; with `after`, once they've taken effect.
async fn add_fault(State(app): Shared, axum::Json(fault): axum::Json<Fault>) -> Response {
    app.lock().faults.push(fault);
    StatusCode::NO_CONTENT.into_response()
}

/// The quota and what the account spent against it: units in the last minute, and units,
/// calls and refused calls since the world was built.
async fn quota(State(app): Shared) -> axum::Json<Value> {
    axum::Json(app.lock().quota.summary(std::time::Instant::now()))
}

async fn clear_faults(State(app): Shared) -> Response {
    app.lock().faults.clear();
    StatusCode::NO_CONTENT.into_response()
}
