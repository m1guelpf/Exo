//! New mail (`SyncReport::arrived`): what a round of history brought that the store didn't
//! have, and none from the first sync or a backfill.

mod support;

use exo_store::{Arrival, ListId, Mutation, ThreadId};
use serde_json::json;
use support::{ACCOUNT, Harness};

async fn arrived(harness: &Harness) -> Vec<Arrival> {
    harness.store.sync().await.expect("sync").arrived
}

#[tokio::test]
async fn the_first_sync_and_the_backfill_bring_no_new_mail() {
    let harness = Harness::start().await;
    for _ in 0..50 {
        let report = harness.store.sync().await.expect("sync");
        assert!(report.arrived.is_empty(), "{:?}", report.arrived);
        if !report.backfilling {
            return;
        }
    }
    panic!("backfill never finished");
}

#[tokio::test]
async fn delivered_mail_arrives_with_what_a_notification_shows() {
    let harness = Harness::synced().await;
    let delivered = harness.deliver(json!({ "from": "Priya Raman <priya@brightmoor.co>", "subject": "Launch checklist", "text": "Can you look at the checklist before 3? It's <short>." })).await;
    let arrivals = arrived(&harness).await;
    let [arrival] = arrivals.as_slice() else {
        panic!("{arrivals:?}")
    };
    assert_eq!(Some(&arrival.message), delivered.id.as_ref());
    assert_eq!(Some(&arrival.thread), delivered.thread_id.as_ref());
    let from = arrival.from.as_ref().expect("a sender");
    assert_eq!(
        (from.name.as_deref(), from.email.as_str(), from.is_me),
        (Some("Priya Raman"), "priya@brightmoor.co", false)
    );
    assert_eq!(arrival.subject, "Launch checklist");
    assert!(
        arrival
            .snippet
            .starts_with("Can you look at the checklist before 3? It's <short>."),
        "unescaped: {}",
        arrival.snippet
    );
    assert!(arrival.unread && arrival.inbox && arrival.important && !arrival.muted);
    assert!(arrived(&harness).await.is_empty(), "once");

    // Promotions are Other, and mail filed away isn't in the inbox.
    harness.deliver(json!({ "from": "deals@shop.example", "subject": "Sale", "labelIds": ["INBOX", "UNREAD", "CATEGORY_PROMOTIONS"] })).await;
    harness.deliver(json!({ "from": "list@lists.example", "subject": "Digest", "labelIds": ["UNREAD", "CATEGORY_UPDATES"] })).await;
    let arrivals = arrived(&harness).await;
    let flags: Vec<(&str, bool, bool)> = arrivals
        .iter()
        .map(|arrival| (arrival.subject.as_str(), arrival.inbox, arrival.important))
        .collect();
    assert_eq!(
        flags,
        [("Sale", true, false), ("Digest", false, false)],
        "oldest first"
    );
}

#[tokio::test]
async fn mail_already_stored_is_not_new() {
    let harness = Harness::synced().await;
    let seen = harness
        .deliver(json!({ "from": "ana@example.com", "subject": "Seen" }))
        .await;
    // The contact pane fetches Ana's latest conversations before history gets to it.
    harness
        .store
        .fetch_recent_threads("ana@example.com")
        .await
        .expect("fetches");
    assert!(
        harness
            .ids(&ListId::Inbox)
            .await
            .contains(seen.thread_id.as_deref().expect("thread")),
        "stored"
    );
    assert!(arrived(&harness).await.is_empty());
}

#[tokio::test]
async fn my_own_mail_drafts_and_deleted_mail() {
    let harness = Harness::synced().await;
    let gmail = harness.gmail.gmail();
    let raw = |subject: &str| {
        format!("From: Maya <{ACCOUNT}>\r\nTo: ana@example.com\r\nSubject: {subject}\r\n\r\nHi")
            .into_bytes()
    };
    let sent = gmail.send(raw("Sent elsewhere"), None).await.expect("send");
    gmail.create_draft(raw("Draft"), None).await.expect("draft");
    let gone = harness
        .deliver(json!({ "from": "ana@example.com", "subject": "Gone" }))
        .await;
    gmail
        .delete_message(gone.id.as_deref().expect("id"))
        .await
        .expect("delete");

    let arrivals = arrived(&harness).await;
    let [arrival] = arrivals.as_slice() else {
        panic!("only the sent message: {arrivals:?}")
    };
    assert_eq!(Some(&arrival.message), sent.id.as_ref());
    assert!(
        arrival.from.as_ref().is_some_and(|from| from.is_me),
        "from me"
    );
    assert!(!arrival.inbox && !arrival.unread);
}

#[tokio::test]
async fn a_reply_in_a_muted_thread_arrives_muted() {
    let harness = Harness::synced().await;
    let first = harness
        .deliver(json!({ "from": "ana@example.com", "subject": "Noisy" }))
        .await;
    let thread = first.thread_id.clone().expect("thread");
    arrived(&harness).await;
    harness
        .store
        .apply(&[ThreadId::new(thread.clone())], Mutation::Mute)
        .await
        .expect("mute");
    harness.store.flush().await.expect("flush");
    harness
        .deliver(json!({ "from": "ana@example.com", "threadId": thread, "text": "More" }))
        .await;
    let arrivals = arrived(&harness).await;
    let [arrival] = arrivals.as_slice() else {
        panic!("{arrivals:?}")
    };
    assert!(arrival.muted && !arrival.inbox);
}

#[tokio::test]
async fn a_threads_latest_message_as_it_stands() {
    let harness = Harness::synced().await;
    let first = harness
        .deliver(json!({ "from": "ana@example.com", "subject": "Lunch" }))
        .await;
    let thread = first.thread_id.clone().expect("thread");
    let reply = harness
        .deliver(json!({ "from": "Bea <bea@example.com>", "threadId": thread, "text": "Noon?" }))
        .await;
    arrived(&harness).await;
    harness
        .store
        .apply(&[ThreadId::new(thread.clone())], Mutation::MarkRead)
        .await
        .expect("read");
    let latest = harness
        .store
        .latest_arrival(&thread)
        .await
        .expect("reads")
        .expect("stored");
    assert_eq!(Some(&latest.message), reply.id.as_ref());
    assert_eq!(
        latest.from.map(|from| from.email),
        Some("bea@example.com".to_owned())
    );
    assert!(!latest.unread, "as it stands");
    assert!(
        harness
            .store
            .latest_arrival("missing")
            .await
            .expect("reads")
            .is_none()
    );
}
