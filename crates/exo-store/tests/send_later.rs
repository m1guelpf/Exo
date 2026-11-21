//! Send later: a draft scheduled for a time stays a Gmail draft, listed in Scheduled and shown in
//! its thread, until the push sends it then (or at the next launch), dated when it goes. An
//! `if no reply` send is called off by a reply.

mod support;

use exo_store::{Address, Draft, DraftAction, DraftRecord, ListId, SendLater, ThreadId};
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use support::Harness;

/// A new message to Ann, dated `dated`, for `send_later`.
fn draft(id: &str, dated: Timestamp, send_later: Option<SendLater>) -> Draft {
    let email = gmail::mime::Email {
        from: Some(gmail::mime::Mailbox::new("Maya Okafor", support::ACCOUNT)),
        to: vec![gmail::mime::Mailbox::new("Ann", "ann@example.com")],
        subject: "Quarterly plan".into(),
        html: Some("<div>Here it is</div>".into()),
        text: Some("Here it is".into()),
        message_id: Some(format!("{id}@exo")),
        date: Some(dated.to_zoned(jiff::tz::TimeZone::UTC)),
        ..gmail::mime::Email::default()
    };
    let record = DraftRecord {
        action: DraftAction::Compose,
        from: Some(Address {
            name: Some("Maya Okafor".into()),
            email: support::ACCOUNT.into(),
        }),
        to: vec![Address {
            name: Some("Ann".into()),
            email: "ann@example.com".into(),
        }],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: "Quarterly plan".into(),
        body: "<div>Here it is</div>".into(),
        quoted_content: None,
        quoted_inlined: false,
        in_reply_to: None,
        in_reply_to_rfc822: None,
        references: Vec::new(),
        rfc822_id: format!("{id}@exo"),
        fingerprint: String::new(),
        attachments: Vec::new(),
        send_later,
        reminder: None,
    };
    Draft {
        id: id.into(),
        thread: None,
        adopted: None,
        record,
        raw: String::from_utf8(email.to_rfc822()).expect("ASCII"),
        complete: true,
        sending: true,
    }
}

fn later(at: Timestamp, abort_on_reply: bool) -> Option<SendLater> {
    Some(SendLater {
        at,
        abort_on_reply,
        current_message_ids: Vec::new(),
    })
}

/// The Gmail messages with Message-ID `id` in Sent.
async fn sent(harness: &Harness, id: &str) -> Vec<gmail::Message> {
    let query = gmail::ListQuery {
        q: Some(format!("rfc822msgid:{id}")),
        label_ids: vec!["SENT".into()],
        ..gmail::ListQuery::default()
    };
    let mut sent = Vec::new();
    for message in harness
        .gmail
        .gmail()
        .messages(&query)
        .await
        .expect("messages.list")
        .messages
    {
        let id = message.id.expect("an id");
        sent.push(
            harness
                .gmail
                .gmail()
                .message(&id, gmail::Format::Metadata, &["Date"])
                .await
                .expect("messages.get"),
        );
    }
    sent
}

fn date_header(message: &gmail::Message) -> Timestamp {
    let header = message
        .payload
        .as_ref()
        .and_then(|payload| payload.headers.iter().find(|header| header.name == "Date"))
        .expect("a Date header");
    jiff::fmt::rfc2822::parse(&header.value)
        .expect("RFC 2822")
        .timestamp()
}

/// Schedules draft `id` (a new message) for `at` and pushes. Returns its thread.
async fn schedule(harness: &Harness, draft: Draft) -> ThreadId {
    let id = draft.id.clone();
    harness.store.save_draft(draft).await.expect("saves");
    harness.store.schedule_draft(&id).await.expect("schedules");
    harness.store.push_drafts().await.expect("pushes");
    harness
        .store
        .draft(&id)
        .await
        .expect("reads")
        .and_then(|draft| draft.thread)
        .expect("a thread")
}

#[tokio::test]
async fn a_scheduled_message_waits_in_scheduled_as_a_gmail_draft() {
    let mut harness = Harness::synced().await;
    // Send later's times are whole milliseconds, as JavaScript's are.
    let at = Timestamp::from_millisecond(
        (Timestamp::now() + SignedDuration::from_hours(20)).as_millisecond(),
    )
    .expect("in range");
    let thread = schedule(
        &harness,
        draft("draft00000000000001", Timestamp::now(), later(at, false)),
    )
    .await;
    let report = harness.store.push_drafts().await.expect("pushes");
    assert!(report.sent.is_empty(), "not before its time");
    assert_eq!(report.retry_at, Some(at), "the push wakes then");
    assert!(sent(&harness, "draft00000000000001@exo").await.is_empty());
    let gmail_id = harness
        .store
        .draft("draft00000000000001")
        .await
        .expect("reads")
        .and_then(|draft| draft.gmail_id)
        .expect("written to Gmail");
    let gmail_drafts = harness
        .gmail
        .gmail()
        .drafts(&gmail::DraftQuery::default())
        .await
        .expect("drafts.list")
        .drafts;
    assert!(
        gmail_drafts
            .iter()
            .any(|draft| draft.id.as_deref() == Some(gmail_id.as_str())),
        "Gmail web and the phone see a draft"
    );

    // Listed in Scheduled (`Scheduled: {date}`), not in the Inbox as a new draft would be.
    let row = harness
        .summary(&ListId::Scheduled, &thread)
        .await
        .expect("in Scheduled");
    assert_eq!(
        row.scheduled,
        Some((at, true)),
        "its time, and Gmail has it"
    );
    assert_eq!(row.date, at);
    assert_eq!(row.drafts, 0, "not a draft being written");
    assert!(
        !harness.ids(&ListId::Inbox).await.contains(thread.as_str()),
        "a scheduled new message isn't in the Inbox"
    );
    assert!(
        harness
            .store
            .count(&ListId::Scheduled)
            .await
            .expect("counts")
            .exact
    );

    // Its thread shows it as a message to go then, with its body.
    let detail = harness
        .store
        .thread(&thread)
        .await
        .expect("reads")
        .expect("stored");
    let message = detail
        .messages
        .iter()
        .find(|message| message.scheduled.is_some())
        .expect("the scheduled message");
    assert_eq!(message.scheduled, Some(at));
    assert!(
        message
            .body
            .html
            .as_deref()
            .is_some_and(|html| html.contains("Here it is")),
        "{:?}",
        message.body
    );

    // A relaunch keeps it.
    harness.reopen().await;
    assert!(
        harness
            .store
            .push_drafts()
            .await
            .expect("pushes")
            .sent
            .is_empty()
    );
    assert_eq!(
        harness
            .summary(&ListId::Scheduled, &thread)
            .await
            .and_then(|row| row.scheduled),
        Some((at, true))
    );
}

#[tokio::test]
async fn a_scheduled_send_goes_once_due_dated_when_it_goes() {
    let mut harness = Harness::synced().await;
    // Scheduled an hour ago for a time that has come while Exo was closed.
    let scheduled_at = Timestamp::now() - SignedDuration::from_hours(1);
    let at = Timestamp::now() + SignedDuration::from_secs(2);
    let thread = schedule(
        &harness,
        draft("draft00000000000002", scheduled_at, later(at, false)),
    )
    .await;
    harness.reopen().await;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let report = harness.store.push_drafts().await.expect("pushes");
    assert_eq!(
        report
            .sent
            .iter()
            .map(|sent| sent.id.as_str())
            .collect::<Vec<_>>(),
        ["draft00000000000002"],
        "sent at the next push"
    );
    let sent = sent(&harness, "draft00000000000002@exo").await;
    let [message] = sent.as_slice() else {
        panic!("sent once: {sent:?}")
    };
    let dated = date_header(message);
    assert!(
        dated.duration_until(Timestamp::now()).abs() < SignedDuration::from_mins(1),
        "dated when it went ({dated}), not when it was scheduled ({scheduled_at})"
    );
    assert!(
        harness.summary(&ListId::Scheduled, &thread).await.is_none(),
        "no longer scheduled"
    );
    assert!(
        harness
            .store
            .draft("draft00000000000002")
            .await
            .expect("reads")
            .is_none()
    );
}

#[tokio::test]
async fn taking_a_scheduled_send_back_makes_it_a_draft_again() {
    let harness = Harness::synced().await;
    let at = Timestamp::now() + SignedDuration::from_hours(20);
    let thread = schedule(
        &harness,
        draft("draft00000000000003", Timestamp::now(), later(at, false)),
    )
    .await;
    assert!(
        harness
            .store
            .cancel_send("draft00000000000003")
            .await
            .expect("cancels"),
        "it was still held"
    );
    let stored = harness
        .store
        .draft("draft00000000000003")
        .await
        .expect("reads")
        .expect("kept");
    assert_eq!(
        (
            stored.send_at,
            stored.record.send_later.map(|later| later.at)
        ),
        (None, Some(at)),
        "a draft again, its time kept for compose"
    );
    assert!(harness.summary(&ListId::Scheduled, &thread).await.is_none());
    let detail = harness
        .store
        .thread(&thread)
        .await
        .expect("reads")
        .expect("stored");
    assert!(
        detail
            .messages
            .iter()
            .all(|message| message.scheduled.is_none())
    );
}

/// A reply to `thread`'s last message, scheduled `if no reply` for tomorrow.
async fn scheduled_reply(harness: &Harness, thread: &ThreadId, id: &str) -> Draft {
    let detail = harness
        .store
        .thread(thread)
        .await
        .expect("reads")
        .expect("stored");
    let last = detail
        .messages
        .iter()
        .rev()
        .find(|message| !message.is_draft())
        .expect("a message");
    let mut reply = draft(
        id,
        Timestamp::now(),
        later(Timestamp::now() + SignedDuration::from_hours(20), true),
    );
    let email = gmail::mime::Email {
        to: vec![gmail::mime::Mailbox::address("ann@example.com")],
        subject: gmail::mime::reply_subject(&detail.subject),
        html: Some("<div>Sounds good</div>".into()),
        message_id: Some(format!("{id}@exo")),
        in_reply_to: last.rfc822_id.clone(),
        references: last
            .references
            .iter()
            .cloned()
            .chain(last.rfc822_id.clone())
            .collect(),
        ..gmail::mime::Email::default()
    };
    reply.raw = String::from_utf8(email.to_rfc822()).expect("ASCII");
    reply.thread = Some(thread.clone());
    reply.record.action = DraftAction::Reply;
    reply.record.in_reply_to = Some(last.id.clone());
    reply
}

#[tokio::test]
async fn a_reply_calls_off_an_if_no_reply_send() {
    let harness = Harness::synced().await;
    let thread = harness
        .all(&ListId::Inbox)
        .await
        .into_iter()
        .find(|row| row.drafts == 0)
        .expect("a thread")
        .id;
    schedule(
        &harness,
        scheduled_reply(&harness, &thread, "draft00000000000004").await,
    )
    .await;
    let stored = harness
        .store
        .draft("draft00000000000004")
        .await
        .expect("reads")
        .expect("kept");
    let ids = stored
        .record
        .send_later
        .as_ref()
        .map(|later| later.current_message_ids.len())
        .unwrap_or_default();
    assert!(ids > 0, "the thread's messages are recorded");
    assert!(harness.summary(&ListId::Scheduled, &thread).await.is_some());

    // The user's own message from Gmail web isn't a reply that calls it off.
    harness.deliver(json!({ "from": format!("Maya Okafor <{}>", support::ACCOUNT), "threadId": thread.as_str(), "text": "One more thing", "labelIds": ["SENT"] })).await;
    harness.store.sync().await.expect("syncs");
    assert!(
        harness
            .store
            .draft("draft00000000000004")
            .await
            .expect("reads")
            .expect("kept")
            .send_at
            .is_some(),
        "still scheduled"
    );

    harness.deliver(json!({ "from": "Ann <ann@example.com>", "threadId": thread.as_str(), "text": "Actually, never mind" })).await;
    harness.store.sync().await.expect("syncs");
    let stored = harness
        .store
        .draft("draft00000000000004")
        .await
        .expect("reads")
        .expect("kept");
    assert_eq!(stored.send_at, None, "called off");
    assert!(
        stored.record.send_later.is_some(),
        "its time stays, for `Unscheduled: Ann replied`"
    );
    assert!(harness.summary(&ListId::Scheduled, &thread).await.is_none());
    assert_eq!(
        harness
            .summary(&ListId::Inbox, &thread)
            .await
            .expect("in the Inbox")
            .drafts,
        1,
        "a draft being written again"
    );
}

#[tokio::test]
async fn a_reply_sync_missed_still_calls_it_off_when_due() {
    let harness = Harness::synced().await;
    let thread = harness
        .all(&ListId::Inbox)
        .await
        .into_iter()
        .find(|row| row.drafts == 0)
        .expect("a thread")
        .id;
    let mut reply = scheduled_reply(&harness, &thread, "draft00000000000005").await;
    reply.record.send_later.as_mut().expect("scheduled").at =
        Timestamp::now() + SignedDuration::from_secs(1);
    schedule(&harness, reply).await;
    // Ann replies while Exo is closed; the push runs before sync at the next launch.
    harness.deliver(json!({ "from": "Ann <ann@example.com>", "threadId": thread.as_str(), "text": "Actually, never mind" })).await;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let report = harness.store.push_drafts().await.expect("pushes");
    assert!(report.sent.is_empty(), "not sent");
    assert!(sent(&harness, "draft00000000000005@exo").await.is_empty());
    assert_eq!(
        harness
            .store
            .draft("draft00000000000005")
            .await
            .expect("reads")
            .expect("kept")
            .send_at,
        None,
        "called off"
    );
}

#[tokio::test]
async fn a_scheduled_draft_edited_in_gmail_goes_as_gmail_has_it() {
    let harness = Harness::synced().await;
    let at = Timestamp::now() + SignedDuration::from_secs(1);
    schedule(
        &harness,
        draft(
            "draft00000000000006",
            Timestamp::now() - SignedDuration::from_hours(1),
            later(at, false),
        ),
    )
    .await;
    // Gmail web rewrites it.
    let gmail_id = harness
        .store
        .draft("draft00000000000006")
        .await
        .expect("reads")
        .and_then(|draft| draft.gmail_id)
        .expect("in Gmail");
    let mut edited = draft("draft00000000000006", Timestamp::now(), None);
    edited.raw = edited.raw.replace("Here it is", "Here it is, edited");
    harness
        .gmail
        .gmail()
        .update_draft(&gmail_id, edited.raw.into_bytes(), None)
        .await
        .expect("drafts.update");
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    assert_eq!(
        harness
            .store
            .push_drafts()
            .await
            .expect("pushes")
            .sent
            .len(),
        1
    );
    let query = gmail::ListQuery {
        q: Some("rfc822msgid:draft00000000000006@exo".into()),
        label_ids: vec!["SENT".into()],
        ..gmail::ListQuery::default()
    };
    let id = harness
        .gmail
        .gmail()
        .messages(&query)
        .await
        .expect("messages.list")
        .messages
        .remove(0)
        .id
        .expect("an id");
    let sent = harness
        .gmail
        .gmail()
        .message(&id, gmail::Format::Full, &[])
        .await
        .expect("messages.get");
    assert!(
        sent.snippet
            .as_deref()
            .is_some_and(|snippet| snippet.contains("edited")),
        "the edit made elsewhere went: {:?}",
        sent.snippet
    );
}

#[tokio::test]
async fn a_scheduled_reply_with_a_reminder_sets_it_on_the_messages_it_was_scheduled_with() {
    let harness = Harness::synced().await;
    let thread = harness
        .all(&ListId::Inbox)
        .await
        .into_iter()
        .find(|row| row.drafts == 0)
        .expect("a thread")
        .id;
    // Scheduled `regardless`, to go in a second, with an `if no reply` reminder in two days.
    let mut reply = scheduled_reply(&harness, &thread, "draft00000000000006").await;
    let later = reply.record.send_later.as_mut().expect("scheduled");
    later.at = Timestamp::now() + SignedDuration::from_secs(1);
    later.abort_on_reply = false;
    let at = Timestamp::now() + SignedDuration::from_hours(48);
    reply.record.reminder = Some(exo_store::DraftReminder {
        at,
        keep_on_reply: false,
        automatic: false,
    });
    schedule(&harness, reply).await;
    assert!(
        harness.summary(&ListId::Reminders, &thread).await.is_some(),
        "in Reminders from scheduling"
    );

    // Ann answers before it goes: the send goes all the same, and the reminder, set on the
    // messages it was scheduled with, is over, and isn't given again.
    harness.deliver(json!({ "from": "Ann <ann@example.com>", "threadId": thread.as_str(), "text": "Already on it" })).await;
    harness.store.sync().await.expect("syncs");
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let report = harness.store.push_drafts().await.expect("pushes");
    assert_eq!(report.sent.len(), 1, "sent");
    let row = harness
        .summary(&ListId::Inbox, &thread)
        .await
        .expect("in the inbox");
    assert!(
        matches!(
            row.reminder,
            Some(exo_store::ReminderStatus::Aborted { .. })
        ),
        "{:?}",
        row.reminder
    );
}
