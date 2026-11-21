//! People v1 handlers under `/v1`.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Extension, Router};
use gmail::people::*;
use serde_json::{Map, Value};

use crate::error::{Failure, Result};
use crate::http::extract::Params;
use crate::oauth::Identity;
use crate::world::App;

type Shared = State<Arc<App>>;
type Reply = Result<axum::Json<Value>>;

pub fn router() -> Router<Arc<App>> {
    Router::new()
        .route("/v1/people/me/connections", get(connections))
        .route("/v1/people/{id}", get(person))
        .route("/v1/otherContacts", get(other_contacts))
        .route("/v1/otherContacts:search", get(search_other_contacts))
        .route("/v1/people:searchContacts", get(search_contacts))
        .route("/v1/people:listDirectoryPeople", get(directory))
}

/// Person fields from the discovery document's `Person` schema.
const PERSON_FIELDS: &[&str] = &[
    "addresses",
    "ageRanges",
    "biographies",
    "birthdays",
    "calendarUrls",
    "clientData",
    "coverPhotos",
    "emailAddresses",
    "events",
    "externalIds",
    "genders",
    "imClients",
    "interests",
    "locales",
    "locations",
    "memberships",
    "metadata",
    "miscKeywords",
    "names",
    "nicknames",
    "occupations",
    "organizations",
    "phoneNumbers",
    "photos",
    "relations",
    "sipAddresses",
    "skills",
    "urls",
    "userDefined",
];

/// The `personFields`/`readMask` parameter, validated like the People API does.
fn mask(params: &Params, name: &str) -> Result<Vec<String>> {
    let docs = "Valid paths are documented at https://developers.google.com/people/api/rest/v1/people/get.";
    let Some(spec) = params.text(name) else {
        return Err(Failure::invalid_argument(format!(
            "{name} mask is required. Please specify one or more valid paths. {docs}"
        )));
    };
    spec.split(',')
        .map(|path| {
            let path = path.trim().trim_start_matches("person.");
            if PERSON_FIELDS.contains(&path) {
                Ok(path.to_owned())
            } else {
                Err(Failure::invalid_argument(format!(
                    "Invalid {name} mask path: \"{path}\". {docs}"
                )))
            }
        })
        .collect()
}

/// A person with only `resourceName`, `etag` and the masked fields.
fn masked(person: &Person, fields: &[String]) -> Value {
    let Value::Object(all) = serde_json::to_value(person).expect("people serialize") else {
        return Value::Null;
    };
    let kept: Map<String, Value> = all
        .into_iter()
        .filter(|(key, _)| key == "resourceName" || key == "etag" || fields.contains(key))
        .collect();
    Value::Object(kept)
}

/// Pagination and sync shared by the list endpoints: `(page, nextPageToken, nextSyncToken)`.
fn list(
    people: &[Person],
    revision: u64,
    params: &Params,
    fields: &[String],
) -> Result<(Vec<Value>, Option<String>, Option<String>)> {
    let sync_token = format!("^MockSync{revision}");
    if let Some(token) = params.text("syncToken") {
        if token != sync_token {
            let mut failure = Failure::failed_precondition(
                "Sync token is expired. Clear local cache and retry call without the sync token.",
            );
            failure.error.details = vec![serde_json::json!({
                "@type": "type.googleapis.com/google.rpc.ErrorInfo",
                "reason": "EXPIRED_SYNC_TOKEN",
                "domain": "people.googleapis.com",
            })];
            return Err(failure);
        }
        // Nothing changes between revisions, so an up-to-date token sees no changes.
        return Ok((Vec::new(), None, Some(sync_token)));
    }
    let size = params
        .u32("pageSize")?
        .filter(|&n| n > 0)
        .unwrap_or(100)
        .min(1000) as usize;
    let offset = match params.text("pageToken") {
        Some(token) => token
            .parse::<usize>()
            .map_err(|_| Failure::invalid_argument("Invalid page token."))?,
        None => 0,
    };
    let page = people
        .iter()
        .skip(offset)
        .take(size)
        .map(|person| masked(person, fields))
        .collect();
    let next = (offset + size < people.len()).then(|| (offset + size).to_string());
    let sync = (next.is_none() && params.bool("requestSyncToken")?).then_some(sync_token);
    Ok((page, next, sync))
}

fn respond(
    mut body: Map<String, Value>,
    page: Vec<Value>,
    key: &str,
    next: Option<String>,
    sync: Option<String>,
) -> Reply {
    if !page.is_empty() {
        body.insert(key.into(), Value::Array(page));
    }
    if let Some(next) = next {
        body.insert("nextPageToken".into(), next.into());
    }
    if let Some(sync) = sync {
        body.insert("nextSyncToken".into(), sync.into());
    }
    Ok(axum::Json(Value::Object(body)))
}

async fn person(
    State(app): Shared,
    Extension(identity): Extension<Identity>,
    Path(id): Path<String>,
    params: Params,
) -> Reply {
    let fields = mask(&params, "personFields")?;
    if let (true, Identity(Some(email))) = (id == "me", &identity) {
        return Ok(axum::Json(masked(
            &crate::fixtures::someone_else(email),
            &fields,
        )));
    }
    let world = app.lock();
    let people = &world.people;
    let resource = format!("people/{id}");
    let person = if id == "me" {
        Some(&people.me)
    } else {
        people
            .contacts
            .iter()
            .chain(&people.directory)
            .find(|person| person.resource_name.as_deref() == Some(&resource))
    };
    let person =
        person.ok_or_else(|| Failure::new(404, "Requested entity was not found.", "notFound"))?;
    Ok(axum::Json(masked(person, &fields)))
}

async fn connections(State(app): Shared, params: Params) -> Reply {
    let fields = mask(&params, "personFields")?;
    let world = app.lock();
    let people = &world.people;
    let (page, next, sync) = list(&people.contacts, people.revision, &params, &fields)?;
    let total = Value::from(people.contacts.len());
    let body = Map::from_iter([
        ("totalPeople".into(), total.clone()),
        ("totalItems".into(), total),
    ]);
    respond(body, page, "connections", next, sync)
}

async fn other_contacts(State(app): Shared, params: Params) -> Reply {
    let fields = mask(&params, "readMask")?;
    let world = app.lock();
    let people = &world.people;
    let (page, next, sync) = list(&people.other_contacts, people.revision, &params, &fields)?;
    let body = Map::from_iter([("totalSize".into(), Value::from(people.other_contacts.len()))]);
    respond(body, page, "otherContacts", next, sync)
}

async fn directory(State(app): Shared, params: Params) -> Reply {
    let fields = mask(&params, "readMask")?;
    if params.all("sources").is_empty() {
        return Err(Failure::invalid_argument(
            "Must specify at least one source: DIRECTORY_SOURCE_TYPE_DOMAIN_CONTACT or \
             DIRECTORY_SOURCE_TYPE_DOMAIN_PROFILE.",
        ));
    }
    let world = app.lock();
    let people = &world.people;
    let (page, next, sync) = list(&people.directory, people.revision, &params, &fields)?;
    respond(Map::new(), page, "people", next, sync)
}

async fn search_contacts(State(app): Shared, params: Params) -> Reply {
    let world = app.lock();
    search(&world.people.contacts, &params)
}

async fn search_other_contacts(State(app): Shared, params: Params) -> Reply {
    let world = app.lock();
    search(&world.people.other_contacts, &params)
}

/// Prefix search over names and addresses, as the People API's search does.
fn search(people: &[Person], params: &Params) -> Reply {
    let fields = mask(params, "readMask")?;
    let query = params
        .get("query")
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let size = params
        .u32("pageSize")?
        .filter(|&n| n > 0)
        .unwrap_or(10)
        .min(30) as usize;
    let matches = |person: &Person| {
        let names = person.names.iter().flat_map(|name| {
            [&name.display_name, &name.given_name, &name.family_name]
                .into_iter()
                .flatten()
                .flat_map(|text| text.split_whitespace())
        });
        let emails = person
            .email_addresses
            .iter()
            .filter_map(|email| email.value.as_deref());
        !query.is_empty()
            && names
                .chain(emails)
                .any(|word| word.to_lowercase().starts_with(&query))
    };
    let results: Vec<Value> = people
        .iter()
        .filter(|person| matches(person))
        .take(size)
        .map(|person| serde_json::json!({ "person": masked(person, &fields) }))
        .collect();
    respond(Map::new(), results, "results", None, None)
}
