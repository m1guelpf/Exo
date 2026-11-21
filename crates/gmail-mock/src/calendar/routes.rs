//! Calendar v3 handlers under `/calendar/v3`.

use std::sync::Arc;

use axum::body::{Body as HttpBody, to_bytes};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::middleware::map_response;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Router};
use gmail::calendar::*;
use jiff::Timestamp;

use super::{Calendars, rfc3339_utc};
use crate::error::{Failure, Result};
use crate::http::extract::{Json as Body, Params};
use crate::oauth::Identity;
use crate::world::{App, World};

type Shared = State<Arc<App>>;
type Reply<T> = Result<axum::Json<T>>;

pub fn router() -> Router<Arc<App>> {
    Router::new()
        .route("/calendar/v3/users/me/calendarList", get(calendar_list))
        .route(
            "/calendar/v3/users/me/calendarList/{id}",
            get(calendar_list_entry),
        )
        .route(
            "/calendar/v3/calendars/{calendar}/events",
            get(list_events).post(insert_event),
        )
        .route(
            "/calendar/v3/calendars/{calendar}/events/import",
            post(import_event),
        )
        .route(
            "/calendar/v3/calendars/{calendar}/events/{event}",
            get(get_event)
                .patch(patch_event)
                .put(update_event)
                .delete(delete_event),
        )
        .route(
            "/calendar/v3/calendars/{calendar}/events/{event}/instances",
            get(instances),
        )
        .route("/calendar/v3/freeBusy", post(free_busy))
        .layer(map_response(legacy_errors))
}

/// Calendar still answers with the v1 error body, which has no `status`.
async fn legacy_errors(response: Response) -> Response {
    if response.status().is_success() {
        return response;
    }
    let (parts, body) = response.into_parts();
    let bytes = to_bytes(body, usize::MAX).await.unwrap_or_default();
    let body = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(mut value) => {
            if let Some(error) = value.get_mut("error").and_then(|e| e.as_object_mut()) {
                error.remove("status");
            }
            HttpBody::from(value.to_string())
        }
        Err(_) => HttpBody::from(bytes),
    };
    Response::from_parts(parts, body)
}

fn bad_request(message: &str) -> Failure {
    Failure::new(400, message, "badRequest")
}

/// Refuses a series whose rule ends with the wrong type of `UNTIL` (a date-time on an all-day
/// series, or a date on a timed one).
fn check_recurrence(event: &Event) -> Result<()> {
    if super::recurrence::valid(event) {
        Ok(())
    } else {
        Err(Failure::new(400, "Invalid recurrence rule.", "invalid"))
    }
}

fn sync_token(revision: u64) -> String {
    format!("CPMock{revision}=")
}

fn parse_sync_token(token: &str) -> Result<u64> {
    token
        .strip_prefix("CPMock")
        .and_then(|rest| rest.strip_suffix('='))
        .and_then(|revision| revision.parse().ok())
        .ok_or_else(|| {
            Failure::new(
                410,
                "Sync token is no longer valid, a full sync is required.",
                "fullSyncRequired",
            )
        })
}

fn timestamp(params: &Params, name: &str) -> Result<Option<Timestamp>> {
    params
        .text(name)
        .map(|value| value.parse().map_err(|_| bad_request("Bad Request")))
        .transpose()
}

/// Offset page tokens over a fully materialized result.
fn page<T>(
    items: Vec<T>,
    params: &Params,
    default: u32,
    cap: u32,
) -> Result<(Vec<T>, Option<String>)> {
    let size = params
        .u32("maxResults")?
        .filter(|&n| n > 0)
        .unwrap_or(default)
        .min(cap) as usize;
    let offset = match params.text("pageToken") {
        Some(token) => token
            .parse::<usize>()
            .map_err(|_| bad_request("Invalid page token"))?,
        None => 0,
    };
    let total = items.len();
    let page: Vec<T> = items.into_iter().skip(offset).take(size).collect();
    let next = (offset + size < total).then(|| (offset + size).to_string());
    Ok((page, next))
}

async fn calendar_list(
    State(app): Shared,
    Extension(identity): Extension<Identity>,
    params: Params,
) -> Reply<CalendarList> {
    let show_hidden = params.bool("showHidden")?;
    let world = app.lock_calendar(&identity);
    let entries: Vec<_> = world
        .calendar
        .entries
        .iter()
        .filter(|entry| show_hidden || entry.hidden != Some(true))
        .cloned()
        .collect();
    let (items, next) = page(entries, &params, 100, 250)?;
    let revision = world.calendar.revision();
    Ok(axum::Json(CalendarList {
        kind: Some("calendar#calendarList".into()),
        etag: Some(format!("\"p3{revision:015}\"")),
        next_sync_token: next.is_none().then(|| sync_token(revision)),
        next_page_token: next,
        items,
    }))
}

async fn calendar_list_entry(
    State(app): Shared,
    Extension(identity): Extension<Identity>,
    Path(id): Path<String>,
) -> Reply<CalendarListEntry> {
    Ok(axum::Json(
        app.lock_calendar(&identity).calendar.calendar(&id)?.clone(),
    ))
}

fn calendar_id(calendars: &Calendars, id: &str) -> Result<String> {
    Ok(calendars.calendar(id)?.id.clone().unwrap_or_default())
}

async fn list_events(
    State(app): Shared,
    Extension(identity): Extension<Identity>,
    Path(calendar): Path<String>,
    params: Params,
) -> Reply<Events> {
    let world = app.lock_calendar(&identity);
    let calendars = &world.calendar;
    let entry = match calendars.calendar(&calendar) {
        Ok(entry) => entry.clone(),
        Err(_) if colleague(&world, &calendar) => {
            return colleague_events(&world, &calendar, &params);
        }
        Err(failure) => return Err(failure),
    };
    let id = entry.id.clone().unwrap_or_default();
    let single = params.bool("singleEvents")?;
    let order_by = params.text("orderBy");
    if order_by == Some("startTime") && !single {
        return Err(bad_request(
            "The requested ordering is not available for the particular query.",
        ));
    }
    let mut items = match params.text("syncToken") {
        // Incremental sync returns every change, deletions included, regardless of range.
        Some(token) => calendars.changed_since(&id, parse_sync_token(token)?),
        None => {
            let min = timestamp(&params, "timeMin")?.unwrap_or(Timestamp::MIN);
            let max = timestamp(&params, "timeMax")?.unwrap_or(Timestamp::MAX);
            let mut items = if single {
                calendars.occurrences(&id, min, max)
            } else {
                calendars
                    .stored(&id)
                    .map(|stored| stored.event.clone())
                    .filter(|event| {
                        calendars.start(event) < max
                            && (!event.recurrence.is_empty() || calendars.end(event) > min)
                    })
                    .collect()
            };
            if !params.bool("showDeleted")? {
                items.retain(|event| event.status.as_deref() != Some("cancelled"));
            }
            items
        }
    };
    let event_types = params.all("eventTypes");
    let ical_uid = params.text("iCalUID");
    let q = params.text("q").map(str::to_lowercase);
    items.retain(|event| {
        let kind = event.event_type.as_deref().unwrap_or("default");
        (event_types.is_empty() || event_types.iter().any(|wanted| wanted == kind))
            && ical_uid.is_none_or(|uid| event.ical_uid.as_deref() == Some(uid))
            && q.as_deref().is_none_or(|q| matches_text(event, q))
    });
    if order_by == Some("updated") {
        items.sort_by(|a, b| a.updated.cmp(&b.updated));
    } else {
        items.sort_by_key(|event| calendars.start(event));
    }
    let (items, next) = page(items, &params, 250, 2500)?;
    let revision = calendars.revision();
    Ok(axum::Json(Events {
        kind: Some("calendar#events".into()),
        etag: Some(format!("\"p3{revision:015}\"")),
        summary: entry.summary.clone(),
        description: entry.description.clone(),
        updated: Some(rfc3339_utc(world.clock.now())),
        time_zone: entry.time_zone.clone(),
        access_role: entry.access_role.clone(),
        default_reminders: entry.default_reminders.clone(),
        next_sync_token: next.is_none().then(|| sync_token(revision)),
        next_page_token: next,
        items,
    }))
}

/// `id` is a colleague's address: someone on the account's own workspace, whose primary
/// calendar a workspace shares.
fn colleague(world: &World, id: &str) -> bool {
    let domain = world
        .account
        .email
        .split_once('@')
        .map(|(_, domain)| domain.to_lowercase());
    domain.is_some_and(|domain| id.to_lowercase().ends_with(&format!("@{domain}")))
}

/// The events of the user's calendars that colleague `person` organizes or attends, from
/// `min` to `max`, as their own calendar has them (instances, their attendee entry `self`).
fn colleague_occurrences(
    world: &World,
    person: &str,
    min: Timestamp,
    max: Timestamp,
) -> Vec<Event> {
    let calendars = &world.calendar;
    let theirs =
        |email: Option<&str>| email.is_some_and(|email| email.eq_ignore_ascii_case(person));
    let mut seen = std::collections::HashSet::new();
    calendars
        .entries
        .iter()
        .filter_map(|entry| entry.id.clone())
        .flat_map(|id| calendars.occurrences(&id, min, max))
        .filter(|event| {
            theirs(
                event
                    .organizer
                    .as_ref()
                    .and_then(|organizer| organizer.email.as_deref()),
            ) || event
                .attendees
                .iter()
                .any(|attendee| theirs(attendee.email.as_deref()))
        })
        // An event on two of the user's calendars is one on theirs.
        .filter(|event| seen.insert(event.id.clone()))
        .map(|mut event| {
            for attendee in &mut event.attendees {
                attendee.is_self = theirs(attendee.email.as_deref()).then_some(true);
            }
            if let Some(organizer) = &mut event.organizer {
                organizer.is_self = theirs(organizer.email.as_deref()).then_some(true);
            }
            event
        })
        .collect()
}

/// `events.list` on a colleague's primary calendar (`singleEvents` only): what a workspace
/// that shares event details shows a colleague, one page.
fn colleague_events(world: &World, person: &str, params: &Params) -> Reply<Events> {
    if !params.bool("singleEvents")? {
        return Err(bad_request(
            "Only single events are listed for another person's calendar.",
        ));
    }
    let min = timestamp(params, "timeMin")?.unwrap_or(Timestamp::MIN);
    let max = timestamp(params, "timeMax")?.unwrap_or(Timestamp::MAX);
    let calendars = &world.calendar;
    let mut items: Vec<Event> = colleague_occurrences(world, person, min, max)
        .into_iter()
        .filter(|event| event.status.as_deref() != Some("cancelled"))
        .collect();
    items.sort_by_key(|event| calendars.start(event));
    let (items, next) = page(items, params, 250, 2500)?;
    Ok(axum::Json(Events {
        kind: Some("calendar#events".into()),
        summary: Some(person.to_owned()),
        updated: Some(rfc3339_utc(world.clock.now())),
        access_role: Some("reader".into()),
        next_page_token: next,
        items,
        ..Events::default()
    }))
}

fn matches_text(event: &Event, q: &str) -> bool {
    [&event.summary, &event.description, &event.location]
        .into_iter()
        .flatten()
        .chain(event.attendees.iter().filter_map(|a| a.email.as_ref()))
        .any(|text| text.to_lowercase().contains(q))
}

async fn get_event(
    State(app): Shared,
    Extension(identity): Extension<Identity>,
    Path((calendar, event)): Path<(String, String)>,
) -> Reply<Event> {
    let world = app.lock_calendar(&identity);
    let id = calendar_id(&world.calendar, &calendar)?;
    Ok(axum::Json(world.calendar.event(&id, &event)?))
}

/// Server-assigned fields of a new event.
fn stamp_new(world: &mut World, mut event: Event) -> Result<Event> {
    if event.start.is_none() {
        return Err(Failure::new(400, "Missing start time.", "required"));
    }
    if event.end.is_none() {
        return Err(Failure::new(400, "Missing end time.", "required"));
    }
    check_recurrence(&event)?;
    let now = rfc3339_utc(world.clock.now());
    let id = event.id.clone().unwrap_or_else(|| world.calendar.next_id());
    let me = EventPerson {
        email: Some(world.account.email.clone()),
        is_self: Some(true),
        ..EventPerson::default()
    };
    event.kind = Some("calendar#event".into());
    event.html_link = Some(format!("https://www.google.com/calendar/event?eid={id}"));
    event.ical_uid = event.ical_uid.or_else(|| Some(format!("{id}@google.com")));
    event.id = Some(id);
    event.status = event.status.or_else(|| Some("confirmed".into()));
    event.created = Some(now.clone());
    event.creator = event.creator.or_else(|| Some(me.clone()));
    event.organizer = event.organizer.or(Some(me));
    event.sequence = event.sequence.or(Some(0));
    event.event_type = event.event_type.or_else(|| Some("default".into()));
    event.reminders = event.reminders.or_else(|| {
        Some(EventReminders {
            use_default: Some(true),
            overrides: Vec::new(),
        })
    });
    Ok(touch(world, conference(event)))
}

/// Answers a Meet `createRequest` with a call, as Calendar does with `conferenceDataVersion=1`:
/// a `meet.google.com` link derived from the event's id.
fn conference(mut event: Event) -> Event {
    let Some(request) = event
        .conference_data
        .as_ref()
        .and_then(|data| data.get("createRequest"))
        .filter(|_| {
            event
                .conference_data
                .as_ref()
                .is_some_and(|data| data.get("entryPoints").is_none())
        })
        .cloned()
    else {
        return event;
    };
    let seed: u32 = event
        .id
        .as_deref()
        .unwrap_or_default()
        .bytes()
        .fold(7, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(u32::from(byte))
        });
    let letter = |shift: u32| char::from(b'a' + ((seed >> shift) % 26) as u8);
    let code = format!(
        "{}{}{}-{}{}{}{}-{}{}{}",
        letter(0),
        letter(3),
        letter(6),
        letter(9),
        letter(12),
        letter(15),
        letter(18),
        letter(21),
        letter(24),
        letter(27)
    );
    let uri = format!("https://meet.google.com/{code}");
    let mut request = request;
    request["status"] = serde_json::json!({ "statusCode": "success" });
    event.conference_data = Some(serde_json::json!({
        "createRequest": request,
        "entryPoints": [{ "entryPointType": "video", "uri": uri, "label": format!("meet.google.com/{code}") }],
        "conferenceSolution": { "key": { "type": "hangoutsMeet" }, "name": "Google Meet" },
        "conferenceId": code,
    }));
    event.hangout_link = Some(uri);
    event
}

/// Bumps `updated` and `etag` for a change.
fn touch(world: &World, mut event: Event) -> Event {
    let now = world.clock.now();
    event.updated = Some(rfc3339_utc(now));
    event.etag = Some(format!("\"{}\"", now.as_millisecond() * 1000));
    event
}

async fn insert_event(
    State(app): Shared,
    Extension(identity): Extension<Identity>,
    Path(calendar): Path<String>,
    Body(event): Body<Event>,
) -> Reply<Event> {
    let mut world = app.lock_calendar(&identity);
    let id = calendar_id(&world.calendar, &calendar)?;
    // A client may choose the id: 5–1024 characters of base32hex, unique in the calendar.
    if let Some(chosen) = &event.id {
        let valid = (5..=1024).contains(&chosen.len())
            && chosen
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'v'));
        if !valid {
            return Err(bad_request("Invalid resource id value."));
        }
        if world
            .calendar
            .stored(&id)
            .any(|stored| stored.event.id.as_ref() == Some(chosen))
        {
            return Err(Failure::new(
                409,
                "The requested identifier already exists.",
                "duplicate",
            ));
        }
    }
    let event = stamp_new(&mut world, event)?;
    Ok(axum::Json(world.calendar.put(&id, event)))
}

/// Adds a private copy of an invite, or updates the copy with the same `iCalUID`.
async fn import_event(
    State(app): Shared,
    Extension(identity): Extension<Identity>,
    Path(calendar): Path<String>,
    Body(event): Body<Event>,
) -> Reply<Event> {
    let mut world = app.lock_calendar(&identity);
    let id = calendar_id(&world.calendar, &calendar)?;
    let uid = event
        .ical_uid
        .clone()
        .ok_or_else(|| Failure::new(400, "Missing iCalUID.", "required"))?;
    let existing = world
        .calendar
        .stored(&id)
        .find(|stored| stored.event.ical_uid.as_deref() == Some(&uid))
        .map(|stored| stored.event.clone());
    let event = match existing {
        Some(existing) => touch(
            &world,
            Event {
                id: existing.id,
                created: existing.created,
                kind: existing.kind,
                html_link: existing.html_link,
                ..event
            },
        ),
        None => stamp_new(&mut world, Event { id: None, ..event })?,
    };
    Ok(axum::Json(world.calendar.put(&id, event)))
}

async fn patch_event(
    State(app): Shared,
    Extension(identity): Extension<Identity>,
    Path((calendar, event)): Path<(String, String)>,
    Body(patch): Body<serde_json::Value>,
) -> Reply<Event> {
    let mut world = app.lock_calendar(&identity);
    let id = calendar_id(&world.calendar, &calendar)?;
    let current = world.calendar.event(&id, &event)?;
    // Clearing the conference takes its Meet link with it.
    let cleared = patch
        .get("conferenceData")
        .is_some_and(serde_json::Value::is_null);
    let merged = merge_patch(current, patch)?;
    check_recurrence(&merged)?;
    let mut merged = touch(&world, conference(merged));
    if cleared {
        merged.hangout_link = None;
    }
    Ok(axum::Json(world.calendar.put(&id, merged)))
}

/// Applies a patch body. With `attendeesOmitted`, listed attendees are updated in place (how
/// an RSVP patches only the user's own response) instead of replacing the list.
fn merge_patch(current: Event, patch: serde_json::Value) -> Result<Event> {
    let serde_json::Value::Object(patch) = patch else {
        return Err(Failure::invalid_json("Expected an object."));
    };
    let partial: Event = crate::http::extract::parse_json(
        serde_json::Value::Object(patch.clone())
            .to_string()
            .as_bytes(),
    )?;
    let mut merged = serde_json::to_value(&current).expect("events serialize");
    let attendees_omitted = partial.attendees_omitted == Some(true);
    for (key, value) in patch {
        if key == "attendees" && attendees_omitted {
            continue;
        }
        merged[key] = value;
    }
    let mut merged: Event = serde_json::from_value(merged).map_err(Failure::invalid_json)?;
    if attendees_omitted {
        merged.attendees = current.attendees;
        for update in partial.attendees {
            match merged
                .attendees
                .iter_mut()
                .find(|attendee| attendee.email == update.email)
            {
                Some(attendee) => {
                    attendee.response_status =
                        update.response_status.or(attendee.response_status.take());
                    attendee.comment = update.comment.or(attendee.comment.take());
                }
                None => merged.attendees.push(update),
            }
        }
        merged.attendees_omitted = None;
    }
    Ok(merged)
}

async fn update_event(
    State(app): Shared,
    Extension(identity): Extension<Identity>,
    Path((calendar, event)): Path<(String, String)>,
    Body(replacement): Body<Event>,
) -> Reply<Event> {
    let mut world = app.lock_calendar(&identity);
    let id = calendar_id(&world.calendar, &calendar)?;
    let current = world.calendar.event(&id, &event)?;
    let replaced = touch(
        &world,
        Event {
            kind: current.kind,
            id: current.id,
            created: current.created,
            html_link: current.html_link,
            ical_uid: current.ical_uid,
            creator: current.creator,
            organizer: replacement.organizer.clone().or(current.organizer),
            recurring_event_id: current.recurring_event_id,
            original_start_time: current.original_start_time,
            ..replacement
        },
    );
    check_recurrence(&replaced)?;
    Ok(axum::Json(world.calendar.put(&id, replaced)))
}

/// Cancels the event (instances become cancelled exceptions). Deleting twice is a 410.
async fn delete_event(
    State(app): Shared,
    Extension(identity): Extension<Identity>,
    Path((calendar, event)): Path<(String, String)>,
) -> Result<Response> {
    let mut world = app.lock_calendar(&identity);
    let id = calendar_id(&world.calendar, &calendar)?;
    let current = world.calendar.event(&id, &event)?;
    if current.status.as_deref() == Some("cancelled") {
        return Err(Failure::new(410, "Resource has been deleted", "deleted"));
    }
    let cancelled = touch(
        &world,
        Event {
            status: Some("cancelled".into()),
            ..current
        },
    );
    world.calendar.put(&id, cancelled);
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn instances(
    State(app): Shared,
    Extension(identity): Extension<Identity>,
    Path((calendar, event)): Path<(String, String)>,
    params: Params,
) -> Reply<Events> {
    let world = app.lock_calendar(&identity);
    let calendars = &world.calendar;
    let entry = calendars.calendar(&calendar)?;
    let id = entry.id.clone().unwrap_or_default();
    let series = calendars.event(&id, &event)?;
    let min = timestamp(&params, "timeMin")?.unwrap_or(Timestamp::MIN);
    let max = timestamp(&params, "timeMax")?.unwrap_or(Timestamp::MAX);
    let original = timestamp(&params, "originalStart")?;
    let show_deleted = params.bool("showDeleted")?;
    let items: Vec<Event> = calendars
        .occurrences(&id, min, max)
        .into_iter()
        .filter(|instance| instance.recurring_event_id == series.id)
        .filter(|instance| show_deleted || instance.status.as_deref() != Some("cancelled"))
        .filter(|instance| {
            original.is_none_or(|original| {
                instance
                    .original_start_time
                    .as_ref()
                    .and_then(|start| start.timestamp(&calendars.tz))
                    == Some(original)
            })
        })
        .collect();
    let (items, next) = page(items, &params, 250, 2500)?;
    Ok(axum::Json(Events {
        kind: Some("calendar#events".into()),
        summary: entry.summary.clone(),
        time_zone: entry.time_zone.clone(),
        access_role: entry.access_role.clone(),
        updated: Some(rfc3339_utc(world.clock.now())),
        next_page_token: next,
        items,
        ..Events::default()
    }))
}

/// Busy intervals: confirmed, opaque events the user hasn't declined, merged.
async fn free_busy(
    State(app): Shared,
    Extension(identity): Extension<Identity>,
    Body(request): Body<FreeBusyRequest>,
) -> Reply<FreeBusyResponse> {
    let parse = |value: &Option<String>, name: &str| -> Result<Timestamp> {
        value
            .as_deref()
            .ok_or_else(|| Failure::new(400, format!("Missing {name} parameter."), "required"))?
            .parse()
            .map_err(|_| bad_request("Bad Request"))
    };
    let min = parse(&request.time_min, "timeMin")?;
    let max = parse(&request.time_max, "timeMax")?;
    let world = app.lock_calendar(&identity);
    let calendars = &world.calendar;
    let mut response = FreeBusyResponse {
        kind: Some("calendar#freeBusy".into()),
        time_min: request.time_min.clone(),
        time_max: request.time_max.clone(),
        ..FreeBusyResponse::default()
    };
    for item in &request.items {
        let busy = |event: &Event, declined: &dyn Fn(&EventAttendee) -> bool| {
            event.status.as_deref() != Some("cancelled")
                && event.transparency.as_deref() != Some("transparent")
                && !event.attendees.iter().any(|attendee| {
                    declined(attendee) && attendee.response_status.as_deref() == Some("declined")
                })
        };
        let mut spans: Vec<(Timestamp, Timestamp)> = match calendars.calendar(&item.id) {
            Ok(entry) => {
                let id = entry.id.clone().unwrap_or_default();
                calendars
                    .occurrences(&id, min, max)
                    .into_iter()
                    .filter(|event| busy(event, &|attendee| attendee.is_self == Some(true)))
                    .map(|event| (calendars.start(&event), calendars.end(&event)))
                    .collect()
            }
            // A colleague's primary calendar, as a workspace shares free/busy: the events of the
            // user's calendars they organize or attend and haven't declined.
            Err(_) if colleague(&world, &item.id) => {
                colleague_occurrences(&world, &item.id, min, max)
                    .into_iter()
                    .filter(|event| busy(event, &|attendee| attendee.is_self == Some(true)))
                    .map(|event| (calendars.start(&event), calendars.end(&event)))
                    .collect()
            }
            Err(_) => {
                response.calendars.insert(
                    item.id.clone(),
                    FreeBusyCalendar {
                        errors: vec![CalendarError {
                            domain: "global".into(),
                            reason: "notFound".into(),
                        }],
                        busy: Vec::new(),
                    },
                );
                continue;
            }
        };
        for span in &mut spans {
            *span = (span.0.max(min), span.1.min(max));
        }
        spans.sort();
        let mut merged: Vec<(Timestamp, Timestamp)> = Vec::new();
        for (start, end) in spans {
            match merged.last_mut() {
                Some(last) if start <= last.1 => last.1 = last.1.max(end),
                _ => merged.push((start, end)),
            }
        }
        response.calendars.insert(
            item.id.clone(),
            FreeBusyCalendar {
                errors: Vec::new(),
                busy: merged
                    .into_iter()
                    .map(|(start, end)| TimePeriod {
                        start: start.to_string(),
                        end: end.to_string(),
                    })
                    .collect(),
            },
        );
    }
    Ok(axum::Json(response))
}
