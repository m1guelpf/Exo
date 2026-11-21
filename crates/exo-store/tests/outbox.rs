//! The outbox: undo, persistence across restarts, coalescing, and what happens when Gmail is
//! unreachable, failing, rate limiting or rejecting the token.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use exo_store::{Error, ListId, Mutation, Store, ThreadId, ThreadSummary};
use gmail::{TokenFuture, TokenProvider};
use gmail_mock::DEV_ACCESS_TOKEN;
use serde_json::json;
use support::{ACCOUNT, Harness, options};

async fn inbox_thread(harness: &Harness) -> ThreadSummary {
    harness
        .all(&ListId::Inbox)
        .await
        .into_iter()
        .find(|thread| thread.drafts == 0)
        .unwrap()
}

/// Endpoints where nothing listens.
fn unreachable() -> gmail::Endpoints {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    gmail::Endpoints::mock(&url_for(port))
}

fn url_for(port: u16) -> reqwest::Url {
    reqwest::Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap()
}

/// Flushes until nothing is pending or `rounds` run out, waiting out backoffs.
async fn flush_until_done(store: &Store, rounds: usize) {
    for _ in 0..rounds {
        let report = store.flush().await.unwrap();
        if store.pending().await.unwrap() == 0 {
            return;
        }
        let wait = report
            .retry_at
            .map(|at| at.duration_since(jiff::Timestamp::now()))
            .and_then(|wait| Duration::try_from(wait).ok())
            .unwrap_or_default();
        tokio::time::sleep(wait + Duration::from_millis(5)).await;
    }
    panic!("the outbox never drained");
}

#[tokio::test]
async fn undo_before_sending_drops_the_mutation() {
    let harness = Harness::synced().await;
    let thread = inbox_thread(&harness).await;
    let before = harness.gmail_labels(&thread.id).await;
    let mut changes = harness.store.changes();

    let undo = harness
        .store
        .apply(std::slice::from_ref(&thread.id), Mutation::MarkDone)
        .await
        .unwrap();
    assert!(changes.has_changed().unwrap(), "the UI hears about it");
    changes.mark_unchanged();
    assert!(
        !harness
            .ids(&ListId::Inbox)
            .await
            .contains(thread.id.as_str())
    );

    harness.store.undo(undo).await.unwrap();
    assert!(changes.has_changed().unwrap());
    assert!(
        harness
            .ids(&ListId::Inbox)
            .await
            .contains(thread.id.as_str())
    );
    assert_eq!(harness.store.pending().await.unwrap(), 0);
    assert_eq!(harness.store.flush().await.unwrap().completed, 0);
    assert_eq!(
        harness.gmail_labels(&thread.id).await,
        before,
        "Gmail never heard of it"
    );
}

#[tokio::test]
async fn undo_after_sending_queues_the_inverse() {
    let harness = Harness::synced().await;
    let thread = inbox_thread(&harness).await;
    let undo = harness
        .store
        .apply(std::slice::from_ref(&thread.id), Mutation::MarkDone)
        .await
        .unwrap();
    assert_eq!(harness.store.flush().await.unwrap().completed, 1);
    assert!(
        harness
            .gmail_labels(&thread.id)
            .await
            .iter()
            .all(|(_, l)| !l.contains("INBOX"))
    );

    harness.store.undo(undo).await.unwrap();
    assert!(
        harness
            .ids(&ListId::Inbox)
            .await
            .contains(thread.id.as_str()),
        "back at once"
    );
    assert_eq!(harness.store.pending().await.unwrap(), 1);
    assert_eq!(harness.store.flush().await.unwrap().completed, 1);
    // Undo adds INBOX to the whole thread.
    assert!(
        harness
            .gmail_labels(&thread.id)
            .await
            .iter()
            .all(|(_, l)| l.contains("INBOX"))
    );
}

#[tokio::test]
async fn the_outbox_survives_a_restart() {
    let mut harness = Harness::synced().await;
    let thread = inbox_thread(&harness).await;
    harness
        .store
        .apply(std::slice::from_ref(&thread.id), Mutation::MarkDone)
        .await
        .unwrap();

    harness.reopen().await;
    assert_eq!(harness.store.pending().await.unwrap(), 1);
    assert!(
        !harness
            .ids(&ListId::Inbox)
            .await
            .contains(thread.id.as_str()),
        "still applied"
    );
    assert_eq!(harness.store.flush().await.unwrap().completed, 1);
    assert!(
        harness
            .gmail_labels(&thread.id)
            .await
            .iter()
            .all(|(_, l)| !l.contains("INBOX"))
    );
}

#[tokio::test]
async fn offline_changes_wait_for_the_network() {
    let harness = Harness::synced().await;
    let thread = inbox_thread(&harness).await;
    let offline = Store::open_with(
        harness.dir.path(),
        ACCOUNT,
        unreachable(),
        gmail::StaticToken(DEV_ACCESS_TOKEN.into()),
        options(),
    )
    .await
    .unwrap();

    // Reads and mutations work; sending waits.
    assert!(matches!(offline.sync().await, Err(Error::Offline(_))));
    offline
        .apply(std::slice::from_ref(&thread.id), Mutation::Star)
        .await
        .unwrap();
    let report = offline.flush().await.unwrap();
    assert_eq!(report.completed, 0);
    assert!(report.failed.is_empty(), "offline never gives up");
    assert!(report.retry_at.is_some());
    assert!(
        offline
            .list(&ListId::Starred, None, 50)
            .await
            .unwrap()
            .threads
            .iter()
            .any(|t| t.id == thread.id)
    );
    drop(offline);

    // Back online (same file), it goes out.
    flush_until_done(&harness.store, 10).await;
    assert!(
        harness
            .gmail_labels(&thread.id)
            .await
            .iter()
            .any(|(_, l)| l.contains("STARRED"))
    );
}

#[tokio::test]
async fn server_errors_back_off_and_retry() {
    let harness = Harness::synced().await;
    let thread = inbox_thread(&harness).await;
    harness
        .fault(json!({ "method": "POST", "path": "/gmail/v1/users/me/messages/batchModify", "status": 503, "times": 3 }))
        .await;
    harness
        .store
        .apply(std::slice::from_ref(&thread.id), Mutation::MarkDone)
        .await
        .unwrap();

    let first = harness.store.flush().await.unwrap();
    assert_eq!(first.completed, 0);
    assert!(first.retry_at.is_some());
    assert!(
        !harness
            .ids(&ListId::Inbox)
            .await
            .contains(thread.id.as_str()),
        "still applied"
    );
    flush_until_done(&harness.store, 10).await;
    assert!(
        harness
            .gmail_labels(&thread.id)
            .await
            .iter()
            .all(|(_, l)| !l.contains("INBOX"))
    );
}

#[tokio::test]
async fn hopeless_mutations_roll_back() {
    let harness = Harness::synced().await;
    let thread = inbox_thread(&harness).await;
    harness
        .fault(json!({
            "method": "POST",
            "path": "/gmail/v1/users/me/messages/batchModify",
            "status": 400,
            "message": "Invalid label: INBOX",
            "reason": "invalidArgument",
        }))
        .await;
    harness
        .store
        .apply(std::slice::from_ref(&thread.id), Mutation::MarkDone)
        .await
        .unwrap();
    let report = harness.store.flush().await.unwrap();
    assert_eq!(report.failed.len(), 1);
    let failed = &report.failed[0];
    assert_eq!(
        (failed.thread.clone(), failed.mutation.clone(), failed.undo),
        (thread.id.clone(), Mutation::MarkDone, false)
    );
    assert!(failed.error.contains("Invalid label"));
    assert_eq!(harness.store.pending().await.unwrap(), 0);
    assert!(
        harness
            .ids(&ListId::Inbox)
            .await
            .contains(thread.id.as_str()),
        "rolled back"
    );
}

#[tokio::test]
async fn undoing_a_mutation_given_up_on_sends_nothing() {
    let harness = Harness::synced().await;
    let thread = inbox_thread(&harness).await;
    let before = harness.gmail_labels(&thread.id).await;
    harness
        .fault(json!({ "method": "POST", "path": "/gmail/v1/users/me/messages/batchModify", "status": 400, "message": "Invalid label: INBOX", "reason": "invalidArgument" }))
        .await;
    let undo = harness
        .store
        .apply(std::slice::from_ref(&thread.id), Mutation::MarkDone)
        .await
        .unwrap();
    assert_eq!(harness.store.flush().await.unwrap().failed.len(), 1);
    // Its toast's Undo, after the failure: Gmail never had the change to take back.
    harness.store.undo(undo).await.unwrap();
    assert_eq!(
        harness.store.pending().await.unwrap(),
        0,
        "no inverse queued"
    );
    assert!(
        harness
            .ids(&ListId::Inbox)
            .await
            .contains(thread.id.as_str())
    );
    assert_eq!(harness.gmail_labels(&thread.id).await, before);
}

#[tokio::test]
async fn reading_again_rewrites_the_queued_read() {
    let harness = Harness::synced().await;
    let thread = harness
        .all(&ListId::Inbox)
        .await
        .into_iter()
        .find(|t| t.unread && t.drafts == 0)
        .unwrap();
    harness
        .store
        .apply(std::slice::from_ref(&thread.id), Mutation::MarkRead)
        .await
        .unwrap();

    // New mail makes the thread unread again; reading it again replaces the queued read.
    harness
        .deliver(json!({ "from": "Ana <ana@example.com>", "threadId": thread.id.as_str(), "text": "More" }))
        .await;
    harness.store.sync().await.unwrap();
    assert!(
        harness
            .summary(&ListId::Inbox, &thread.id)
            .await
            .unwrap()
            .unread
    );
    harness
        .store
        .apply(std::slice::from_ref(&thread.id), Mutation::MarkRead)
        .await
        .unwrap();
    assert_eq!(harness.store.pending().await.unwrap(), 1, "coalesced");
    assert_eq!(harness.store.flush().await.unwrap().completed, 1);
    assert!(
        harness
            .gmail_labels(&thread.id)
            .await
            .iter()
            .all(|(_, l)| !l.contains("UNREAD"))
    );
}

/// A token provider that counts how often Gmail rejected its token.
#[derive(Clone, Default)]
struct CountingTokens(Arc<AtomicUsize>);

impl TokenProvider for CountingTokens {
    fn access_token(&self) -> TokenFuture<'_> {
        Box::pin(async { Ok(DEV_ACCESS_TOKEN.to_owned()) })
    }

    fn invalidate(&self, _token: &str) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[tokio::test]
async fn a_rejected_token_is_refreshed_once() {
    let harness = Harness::start().await;
    let tokens = CountingTokens::default();
    let store = Store::open_with(
        harness.dir.path(),
        "other",
        harness.server.endpoints(),
        tokens.clone(),
        options(),
    )
    .await
    .unwrap();
    harness
        .fault(json!({ "method": "GET", "path": "/gmail/v1/users/me/profile", "status": 401 }))
        .await;
    store.sync().await.unwrap();
    assert_eq!(tokens.0.load(Ordering::Relaxed), 1);

    // Rejected again after the refresh: the store gives up (the account must sign in again).
    harness
        .fault(json!({ "method": "GET", "path": "/gmail/v1/users/me/history", "status": 401, "times": 2 }))
        .await;
    assert!(matches!(store.sync().await, Err(Error::Unauthorized(_))));
    assert_eq!(tokens.0.load(Ordering::Relaxed), 3);
}

#[tokio::test]
async fn rate_limits_wait_or_pause() {
    let harness = Harness::synced().await;
    let retry_in = |seconds: i64| {
        let at = jiff::Timestamp::now() + jiff::SignedDuration::from_secs(seconds);
        format!("User-rate limit exceeded.  Retry after {at}")
    };

    // A short wait is sat through.
    harness
        .fault(json!({ "method": "GET", "path": "/gmail/v1/users/me/history", "status": 429, "message": retry_in(1) }))
        .await;
    let started = std::time::Instant::now();
    harness.store.sync().await.unwrap();
    assert!(
        started.elapsed() >= Duration::from_millis(500),
        "{:?}",
        started.elapsed()
    );

    // A long one pauses every call, without asking Gmail again.
    harness
        .fault(json!({ "method": "GET", "path": "/gmail/v1/users/me/history", "status": 403, "message": retry_in(120) }))
        .await;
    let Err(Error::Throttled { until }) = harness.store.sync().await else {
        panic!("expected a pause");
    };
    assert!(until > jiff::Timestamp::now() + jiff::SignedDuration::from_secs(60));
    assert!(matches!(
        harness.store.sync().await,
        Err(Error::Throttled { .. })
    ));

    // Queued mutations wait it out without counting failures.
    let thread = inbox_thread(&harness).await;
    harness
        .store
        .apply(std::slice::from_ref(&thread.id), Mutation::Star)
        .await
        .unwrap();
    let report = harness.store.flush().await.unwrap();
    assert!(report.failed.is_empty());
    assert_eq!(report.retry_at, Some(until));
    assert!(
        harness
            .gmail_labels(&ThreadId::new(thread.id.as_str()))
            .await
            .iter()
            .all(|(_, l)| !l.contains("STARRED"))
    );
}

#[tokio::test]
async fn a_cancelled_flush_leaves_nothing_stuck() {
    let mut harness = Harness::synced().await;
    let gate = harness.reopen_gated().await;
    let thread = inbox_thread(&harness).await;
    harness
        .store
        .apply(std::slice::from_ref(&thread.id), Mutation::MarkDone)
        .await
        .unwrap();

    // The app stops waiting for a flush while its request is in flight.
    gate.hold_after(0);
    tokio::select! {
        _ = harness.store.flush() => panic!("the request is held"),
        () = gate.held() => {}
    }
    gate.release();
    assert_eq!(harness.store.flush().await.unwrap().completed, 1);
}

#[tokio::test]
async fn backoff_starts_over_once_a_step_goes_through() {
    let harness = Harness::synced().await;
    let base = Duration::from_millis(50);
    let store = Store::open_with(
        harness.dir.path(),
        ACCOUNT,
        harness.server.endpoints(),
        gmail::StaticToken(DEV_ACCESS_TOKEN.into()),
        exo_store::Options {
            backoff: base,
            ..options()
        },
    )
    .await
    .unwrap();
    let thread = inbox_thread(&harness).await;
    // Trash is a batchModify, then a threads.modify: the first fails twice, the second once.
    harness
        .fault(json!({ "method": "POST", "path": "/gmail/v1/users/me/messages/batchModify", "status": 503, "times": 2 }))
        .await;
    harness
        .fault(json!({ "method": "POST", "path": "/gmail/v1/users/me/threads/", "status": 503 }))
        .await;
    store
        .apply(std::slice::from_ref(&thread.id), Mutation::Trash)
        .await
        .unwrap();
    for _ in 0..2 {
        let retry_at = store.flush().await.unwrap().retry_at.unwrap();
        let wait = retry_at.duration_since(jiff::Timestamp::now());
        tokio::time::sleep(Duration::try_from(wait).unwrap_or_default()).await;
    }

    // The batchModify goes through; the threads.modify failing after it is a first failure.
    let started = jiff::Timestamp::now();
    let retry_at = store.flush().await.unwrap().retry_at.unwrap();
    let backoff = Duration::try_from(retry_at.duration_since(started)).unwrap();
    assert!(backoff < base * 5, "{backoff:?}");
}
