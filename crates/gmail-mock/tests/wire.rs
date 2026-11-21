//! Wire-format tests: raw HTTP responses from the mock checked against Google's discovery
//! documents and against Google's documented shapes, without the `gmail` crate's types, so
//! the mock and the client can't silently agree on a wrong shape.

mod support;

use reqwest::Method;
use serde_json::{Value, json};
use support::{Discovery, Raw};

const ME: &str = "/gmail/v1/users/me";

async fn first_id(raw: &Raw, list: &str, key: &str) -> String {
    let (status, body) = raw.get(&format!("{ME}/{list}")).await;
    assert_eq!(status, 200, "{body}");
    body[key][0]["id"].as_str().expect("an id").to_owned()
}

#[tokio::test]
async fn gmail_resources_match_the_discovery_schema() {
    let server = support::server().await;
    let raw = Raw::new(&server);
    let schema = Discovery::gmail();

    let (_, profile) = raw.get(&format!("{ME}/profile")).await;
    schema.assert("Profile", &profile);

    let (_, labels) = raw.get(&format!("{ME}/labels")).await;
    schema.assert("ListLabelsResponse", &labels);
    for label in ["INBOX", "Label_2"] {
        let (_, label) = raw.get(&format!("{ME}/labels/{label}")).await;
        schema.assert("Label", &label);
    }

    let (_, threads) = raw.get(&format!("{ME}/threads?labelIds=INBOX")).await;
    schema.assert("ListThreadsResponse", &threads);
    let attachment_thread = first_id(&raw, "threads?q=has:attachment", "threads").await;
    for format in ["full", "metadata", "minimal"] {
        let (status, thread) = raw
            .get(&format!("{ME}/threads/{attachment_thread}?format={format}"))
            .await;
        assert_eq!(status, 200);
        schema.assert("Thread", &thread);
    }

    let (_, messages) = raw.get(&format!("{ME}/messages?q=has:attachment")).await;
    schema.assert("ListMessagesResponse", &messages);
    let message = messages["messages"][0]["id"].as_str().unwrap();
    for format in ["full", "metadata", "minimal", "raw"] {
        let (_, body) = raw
            .get(&format!("{ME}/messages/{message}?format={format}"))
            .await;
        schema.assert("Message", &body);
    }
    let (_, full) = raw.get(&format!("{ME}/messages/{message}")).await;
    let attachment = find_attachment(&full["payload"]).expect("an attachment part");
    let (_, body) = raw
        .get(&format!("{ME}/messages/{message}/attachments/{attachment}"))
        .await;
    schema.assert("MessagePartBody", &body);

    let (_, drafts) = raw.get(&format!("{ME}/drafts")).await;
    schema.assert("ListDraftsResponse", &drafts);
    let draft = drafts["drafts"][0]["id"].as_str().unwrap();
    let (_, draft) = raw.get(&format!("{ME}/drafts/{draft}")).await;
    schema.assert("Draft", &draft);

    let start = profile["historyId"].as_str().unwrap();
    raw.send(
        Method::POST,
        &format!("{ME}/messages/{message}/modify"),
        json!({"addLabelIds": ["STARRED"], "removeLabelIds": ["INBOX"]}),
    )
    .await;
    let (_, history) = raw
        .get(&format!("{ME}/history?startHistoryId={start}"))
        .await;
    schema.assert("ListHistoryResponse", &history);

    for (path, name) in [
        ("settings/sendAs", "ListSendAsResponse"),
        ("settings/sendAs/maya@brightmoor.co", "SendAs"),
        ("settings/vacation", "VacationSettings"),
        ("settings/filters", "ListFiltersResponse"),
    ] {
        let (status, body) = raw.get(&format!("{ME}/{path}")).await;
        assert_eq!(status, 200, "{path}: {body}");
        schema.assert(name, &body);
    }
}

fn find_attachment(part: &Value) -> Option<String> {
    if let Some(id) = part["body"]["attachmentId"].as_str() {
        return Some(id.to_owned());
    }
    part["parts"].as_array()?.iter().find_map(find_attachment)
}

#[tokio::test]
async fn gmail_encodes_int64_as_strings_and_omits_unset_fields() {
    let server = support::server().await;
    let raw = Raw::new(&server);

    let (_, profile) = raw.get(&format!("{ME}/profile")).await;
    assert!(profile["historyId"].is_string());
    assert!(profile["messagesTotal"].is_u64());

    let message = first_id(&raw, "messages?labelIds=INBOX", "messages").await;
    let (_, full) = raw.get(&format!("{ME}/messages/{message}")).await;
    assert!(full["internalDate"].is_string() && full["historyId"].is_string());
    assert!(
        full["sizeEstimate"].is_u64(),
        "sizeEstimate is int32, a JSON number"
    );
    assert_eq!(
        full["payload"]["partId"], "",
        "the root part's ID is empty, not absent"
    );
    assert!(full["payload"]["body"]["size"].is_u64());
    assert!(full.get("raw").is_none());

    let (_, minimal) = raw
        .get(&format!("{ME}/messages/{message}?format=minimal"))
        .await;
    let keys: Vec<&str> = minimal
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "id",
            "threadId",
            "labelIds",
            "snippet",
            "sizeEstimate",
            "historyId",
            "internalDate"
        ]
    );

    // Empty results omit the list entirely.
    let (_, empty) = raw
        .get(&format!("{ME}/messages?q=zzyzx-no-such-word"))
        .await;
    assert_eq!(empty, json!({"resultSizeEstimate": 0}));
    let (_, empty) = raw.get(&format!("{ME}/threads?q=zzyzx-no-such-word")).await;
    assert_eq!(empty, json!({"resultSizeEstimate": 0}));

    // Gmail's own shape for system labels in labels.list.
    let (_, labels) = raw.get(&format!("{ME}/labels")).await;
    let inbox = labels["labels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["id"] == "INBOX")
        .unwrap();
    assert_eq!(
        inbox,
        &json!({"id": "INBOX", "name": "INBOX", "type": "system"})
    );
}

#[tokio::test]
async fn errors_use_googles_envelope() {
    let server = support::server().await;
    let raw = Raw::new(&server);

    let (status, body) = raw.get(&format!("{ME}/messages/18c0000000000000")).await;
    assert_eq!(status, 404);
    assert_eq!(
        body,
        json!({"error": {
            "code": 404,
            "message": "Requested entity was not found.",
            "errors": [{"message": "Requested entity was not found.", "domain": "global", "reason": "notFound"}],
            "status": "NOT_FOUND"
        }})
    );

    let (status, body) = raw.get(&format!("{ME}/threads/not-an-id")).await;
    assert_eq!(
        (status, body["error"]["message"].as_str()),
        (400, Some("Invalid id value"))
    );

    let (status, body) = raw.get(&format!("{ME}/messages?maxResults=lots")).await;
    assert_eq!(status, 400);
    assert_eq!(body["error"]["status"], "INVALID_ARGUMENT");
    assert_eq!(
        body["error"]["message"],
        "Invalid value at 'max_results' (TYPE_UINT32), \"lots\""
    );
    assert_eq!(
        body["error"]["details"][0]["@type"],
        "type.googleapis.com/google.rpc.BadRequest"
    );

    let (status, body) = raw
        .send(
            Method::POST,
            &format!("{ME}/labels"),
            json!({"name": "Hiring"}),
        )
        .await;
    assert_eq!(
        (status, body["error"]["message"].as_str()),
        (409, Some("Label name exists or conflicts"))
    );

    let (status, body) = raw
        .send(
            Method::POST,
            &format!("{ME}/messages/batchModify"),
            json!({"ids": [], "addLabels": ["X"]}),
        )
        .await;
    assert_eq!(status, 400);
    assert_eq!(
        body["error"]["message"],
        "Invalid JSON payload received. Unknown name \"addLabels\": Cannot find field."
    );

    let (status, body) = raw.get("/gmail/v1/users/someone@example.com/profile").await;
    assert_eq!(
        (status, body["error"]["status"].as_str()),
        (403, Some("PERMISSION_DENIED"))
    );

    // Unknown query parameters are ignored, like a `_nocache` cache buster.
    let (status, _) = raw
        .get(&format!("{ME}/profile?_nocache=0.123&alt=json"))
        .await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn auth_failures_are_googles_401s() {
    let server = support::server().await;
    let raw = Raw::new(&server);
    let url = raw.url(&format!("{ME}/profile"));

    let response = raw.http.get(&url).send().await.unwrap();
    assert_eq!(response.status(), 401);
    assert_eq!(
        response.headers()["www-authenticate"],
        r#"Bearer realm="https://accounts.google.com/""#
    );
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["status"], "UNAUTHENTICATED");
    assert_eq!(body["error"]["errors"][0]["reason"], "required");
    assert_eq!(body["error"]["errors"][0]["location"], "Authorization");
    assert_eq!(body["error"]["details"][0]["reason"], "CREDENTIALS_MISSING");

    let response = raw
        .http
        .get(&url)
        .bearer_auth("ya29.nope")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["errors"][0]["reason"], "authError");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("Request had invalid authentication credentials.")
    );

    // Userinfo also takes the token as a parameter.
    let response = raw
        .http
        .get(raw.url(&format!(
            "/oauth2/v2/userinfo?alt=json&access_token={}",
            gmail_mock::DEV_ACCESS_TOKEN
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["verified_email"], true);
    assert_eq!(body["email"], "maya@brightmoor.co");
}

#[tokio::test]
async fn responses_are_pretty_printed_utf8_json_with_partial_response_support() {
    let server = support::server().await;
    let raw = Raw::new(&server);

    let response = raw
        .http
        .get(raw.url(&format!("{ME}/profile")))
        .bearer_auth(gmail_mock::DEV_ACCESS_TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.headers()["content-type"],
        "application/json; charset=UTF-8"
    );
    let text = response.text().await.unwrap();
    assert!(
        text.starts_with("{\n  \"emailAddress\": \"maya@brightmoor.co\",\n"),
        "{text}"
    );

    let response = raw
        .http
        .get(raw.url(&format!(
            "{ME}/messages?maxResults=2&prettyPrint=false&fields=messages/id,nextPageToken"
        )))
        .bearer_auth(gmail_mock::DEV_ACCESS_TOKEN)
        .send()
        .await
        .unwrap();
    let text = response.text().await.unwrap();
    assert!(!text.contains("\n  "));
    let body: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(body.as_object().unwrap().len(), 2);
    assert_eq!(body["messages"][0].as_object().unwrap().len(), 1);
}

#[tokio::test]
async fn batch_uses_googles_multipart_framing() {
    let server = support::server().await;
    let raw = Raw::new(&server);
    let message = first_id(&raw, "messages?labelIds=INBOX", "messages").await;
    let body = format!(
        "--batch_foobarbaz\r\nContent-Type: application/http\r\nContent-ID: <item1:12930812@barnyard.example.com>\r\n\r\n\
         GET {ME}/messages/{message}?format=minimal\r\n\r\n\
         --batch_foobarbaz\r\nContent-Type: application/http\r\nContent-ID: <item2:12930812@barnyard.example.com>\r\n\r\n\
         POST {ME}/messages/{message}/modify HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{{\"addLabelIds\": [\"STARRED\"]}}\r\n\
         --batch_foobarbaz\r\nContent-Type: application/http\r\nContent-ID: <item3:12930812@barnyard.example.com>\r\n\r\n\
         GET {ME}/messages/0000000000000001\r\n\r\n--batch_foobarbaz--\r\n"
    );
    let response = raw
        .http
        .post(raw.url("/batch/gmail/v1"))
        .bearer_auth(gmail_mock::DEV_ACCESS_TOKEN)
        .header("Content-Type", "multipart/mixed; boundary=batch_foobarbaz")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let content_type = response.headers()["content-type"]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        content_type.starts_with("multipart/mixed; boundary=batch_"),
        "{content_type}"
    );
    let boundary = content_type.split("boundary=").nth(1).unwrap().to_owned();
    let text = response.text().await.unwrap();

    let parts: Vec<&str> = text
        .split(&format!("--{boundary}"))
        .filter(|part| part.trim() != "--" && !part.trim().is_empty())
        .collect();
    assert_eq!(parts.len(), 3, "{text}");
    assert!(parts[0].starts_with("\r\nContent-Type: application/http\r\nContent-ID: <response-item1:12930812@barnyard.example.com>\r\n\r\nHTTP/1.1 200 OK\r\n"), "{}", parts[0]);
    assert!(parts[0].contains("Content-Type: application/json; charset=UTF-8\r\n"));
    assert!(parts[1].contains("HTTP/1.1 200 OK") && parts[1].contains("\"STARRED\""));
    assert!(
        parts[2].contains("<response-item3:12930812@barnyard.example.com>")
            && parts[2].contains("HTTP/1.1 404 Not Found")
    );
    assert!(text.ends_with(&format!("--{boundary}--\r\n")));

    // Inner requests inherit the outer Authorization header, and only that.
    let unauthenticated = raw
        .http
        .post(raw.url("/batch/gmail/v1"))
        .header("Content-Type", "multipart/mixed; boundary=b")
        .body(format!(
            "--b\r\nContent-Type: application/http\r\n\r\nGET {ME}/profile\r\n\r\n--b--\r\n"
        ))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        unauthenticated.contains("HTTP/1.1 401 Unauthorized"),
        "{unauthenticated}"
    );
}

#[tokio::test]
async fn media_uploads_accept_media_and_multipart() {
    let server = support::server().await;
    let raw = Raw::new(&server);
    let message = "From: Rachel Kim <rachel.kim@callowayfreight.com>\r\nTo: maya@brightmoor.co\r\nSubject: Upload test\r\nMessage-ID: <upload-1@callowayfreight.com>\r\n\r\nHello from an upload.\r\n";

    let response = raw
        .http
        .post(raw.url("/upload/gmail/v1/users/me/messages?uploadType=media"))
        .bearer_auth(gmail_mock::DEV_ACCESS_TOKEN)
        .header("Content-Type", "message/rfc822")
        .body(message)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let inserted: Value = response.json().await.unwrap();
    assert!(inserted["id"].is_string());
    assert!(
        inserted.get("labelIds").is_none(),
        "insert without labelIds applies none"
    );

    let body = format!(
        "--foo_bar_baz\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{{\"labelIds\": [\"INBOX\", \"UNREAD\"]}}\r\n--foo_bar_baz\r\nContent-Type: message/rfc822\r\n\r\n{message}\r\n--foo_bar_baz--"
    );
    let response = raw
        .http
        .post(raw.url("/upload/gmail/v1/users/me/messages/import?uploadType=multipart"))
        .bearer_auth(gmail_mock::DEV_ACCESS_TOKEN)
        .header("Content-Type", "multipart/related; boundary=foo_bar_baz")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let imported: Value = response.json().await.unwrap();
    let mut labels: Vec<&str> = imported["labelIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap())
        .collect();
    labels.sort_unstable();
    // The Calloway filter from the fixtures applies on import.
    assert_eq!(labels, ["INBOX", "Label_5", "UNREAD"]);

    let (status, body) = raw
        .send(Method::POST, &format!("{ME}/messages/send"), json!({}))
        .await;
    assert_eq!(status, 400);
    assert_eq!(
        body["error"]["message"],
        "'raw' RFC822 payload message string or uploading message via /upload/* URL required"
    );
}

#[tokio::test]
async fn cors_allows_browser_clients() {
    let server = support::server().await;
    let raw = Raw::new(&server);
    let response = raw
        .http
        .request(Method::OPTIONS, raw.url(&format!("{ME}/threads")))
        .header("Origin", "https://mail.example.test")
        .header("Access-Control-Request-Method", "GET")
        .header("Access-Control-Request-Headers", "authorization")
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    assert_eq!(
        response.headers()["access-control-allow-origin"],
        "https://mail.example.test"
    );
    assert!(
        response.headers()["access-control-allow-headers"]
            .to_str()
            .unwrap()
            .contains("authorization")
    );
}

#[tokio::test]
async fn calendar_and_people_match_their_discovery_schemas() {
    let server = support::server().await;
    let raw = Raw::new(&server);
    let calendar = Discovery::calendar();
    let people = Discovery::people();

    let (_, list) = raw
        .get("/calendar/v3/users/me/calendarList?showHidden=true")
        .await;
    calendar.assert("CalendarList", &list);
    let (_, events) = raw
        .get("/calendar/v3/calendars/primary/events?singleEvents=true&orderBy=startTime&timeMin=2026-11-01T00:00:00Z&timeMax=2026-12-31T00:00:00Z")
        .await;
    calendar.assert("Events", &events);
    assert!(events["items"].as_array().unwrap().len() > 10);
    let (_, freebusy) = raw
        .send(Method::POST, "/calendar/v3/freeBusy", json!({"timeMin": "2026-11-16T00:00:00Z", "timeMax": "2026-11-21T00:00:00Z", "items": [{"id": "primary"}, {"id": "nobody@example.com"}]}))
        .await;
    calendar.assert("FreeBusyResponse", &freebusy);
    assert_eq!(
        freebusy["calendars"]["nobody@example.com"]["errors"][0]["reason"],
        "notFound"
    );

    let (status, body) = raw
        .get("/calendar/v3/calendars/primary/events?syncToken=garbage")
        .await;
    assert_eq!(status, 410);
    assert_eq!(body["error"]["errors"][0]["reason"], "fullSyncRequired");
    assert!(
        body["error"].get("status").is_none(),
        "Calendar errors have no status"
    );

    let (_, me) = raw
        .get("/v1/people/me?personFields=names,emailAddresses,photos")
        .await;
    people.assert("Person", &me);
    let (_, connections) = raw.get("/v1/people/me/connections?pageSize=500&personFields=emailAddresses,names,metadata&requestSyncToken=true").await;
    people.assert("ListConnectionsResponse", &connections);
    assert!(connections["nextSyncToken"].is_string());
    let (_, other) = raw
        .get("/v1/otherContacts?pageSize=500&readMask=emailAddresses,names,metadata")
        .await;
    people.assert("ListOtherContactsResponse", &other);
    let (_, directory) = raw
        .get("/v1/people:listDirectoryPeople?readMask=emailAddresses,names&sources=DIRECTORY_SOURCE_TYPE_DOMAIN_PROFILE")
        .await;
    people.assert("ListDirectoryPeopleResponse", &directory);
    let (_, search) = raw
        .get("/v1/people:searchContacts?query=rach&readMask=names,emailAddresses")
        .await;
    people.assert("SearchResponse", &search);

    let (status, body) = raw
        .get("/v1/people/me/connections?personFields=names&syncToken=stale")
        .await;
    assert_eq!(status, 400);
    assert_eq!(body["error"]["status"], "FAILED_PRECONDITION");
    assert_eq!(body["error"]["details"][0]["reason"], "EXPIRED_SYNC_TOKEN");
    let (status, _) = raw.get("/v1/people/me").await;
    assert_eq!(status, 400, "personFields is required");
}
