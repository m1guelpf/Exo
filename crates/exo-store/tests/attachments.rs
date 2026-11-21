//! Fetching attachments' bytes: Gmail hands out a new attachment id every time it returns a
//! message, so the store reads a part again from the message when Gmail refuses the stored id;
//! and someone is waiting on them, so a short rate-limit pause is waited out.

mod support;

use std::time::{Duration, Instant};

use exo_store::{Attachment, Attachments, Error, ListId};
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use support::Harness;

/// A stored attachment, and its bytes as Gmail serves them.
async fn some_attachment(harness: &Harness) -> (Attachment, Vec<u8>) {
    let thread = harness
        .all(&ListId::All)
        .await
        .into_iter()
        .find(|thread| thread.attachments == Attachments::Files)
        .expect("the fixtures have a thread with files");
    let detail = harness
        .store
        .thread(&thread.id)
        .await
        .expect("reads")
        .expect("stored");
    let attachment = detail
        .messages
        .into_iter()
        .flat_map(|message| message.attachments)
        .find(|attachment| attachment.attachment_id.is_some())
        .expect("a file Gmail serves by id");
    let bytes = harness
        .gmail
        .gmail()
        .attachment(
            &attachment.message_id,
            attachment.attachment_id.as_deref().unwrap(),
        )
        .await
        .expect("Gmail serves it")
        .data
        .expect("with data");
    (attachment, bytes)
}

#[tokio::test]
async fn reads_a_part_by_its_id_or_from_the_message() {
    let harness = Harness::synced().await;
    let (attachment, bytes) = some_attachment(&harness).await;
    let part = |id: Option<&'static str>| {
        harness
            .store
            .part_data(&attachment.message_id, id, &attachment.part_id)
    };
    let stored = harness.store.part_data(
        &attachment.message_id,
        attachment.attachment_id.as_deref(),
        &attachment.part_id,
    );
    assert_eq!(stored.await.expect("by the stored id"), bytes);
    assert_eq!(
        part(Some("ANGjdJ-an-id-from-an-earlier-fetch"))
            .await
            .expect("from the message"),
        bytes,
        "an id Gmail no longer accepts"
    );
    assert_eq!(
        part(None).await.expect("from the message"),
        bytes,
        "no id: the data came inline"
    );
    let gone = harness
        .store
        .part_data(&attachment.message_id, None, "9.9")
        .await;
    assert!(matches!(gone, Err(Error::MissingPart { .. })), "{gone:?}");
}

#[tokio::test]
async fn a_short_rate_limit_pause_is_waited_out() {
    let harness = Harness::synced().await;
    let (attachment, bytes) = some_attachment(&harness).await;
    let fetch = || {
        harness.store.part_data(
            &attachment.message_id,
            attachment.attachment_id.as_deref(),
            &attachment.part_id,
        )
    };

    // Sync runs out of retries against Gmail's rate limit, which pauses every call for a
    // moment: an image or file asked for meanwhile arrives once the pause is over.
    harness
        .fault(json!({ "method": "GET", "path": "/gmail/v1/users/me/history", "status": 429, "times": 5 }))
        .await;
    let Err(Error::Throttled { until }) = harness.store.sync().await else {
        panic!("expected a pause");
    };
    assert_eq!(fetch().await.expect("after the pause"), bytes);
    assert!(Timestamp::now() >= until, "fetched during the pause");

    // A longer pause fails the fetch at once.
    let retry_at = Timestamp::now() + SignedDuration::from_secs(120);
    harness
        .fault(json!({
            "method": "GET",
            "path": "/gmail/v1/users/me/history",
            "status": 429,
            "message": format!("User-rate limit exceeded.  Retry after {retry_at}"),
        }))
        .await;
    assert!(matches!(
        harness.store.sync().await,
        Err(Error::Throttled { .. })
    ));
    let started = Instant::now();
    assert!(matches!(fetch().await, Err(Error::Throttled { .. })));
    assert!(started.elapsed() < Duration::from_secs(1));
}

/// A file of a thread with files in `list`, and that thread.
async fn file_in(harness: &Harness, list: &ListId) -> (exo_store::ThreadId, Attachment) {
    for thread in harness
        .all(list)
        .await
        .into_iter()
        .filter(|thread| thread.attachments == Attachments::Files)
    {
        let detail = harness
            .store
            .thread(&thread.id)
            .await
            .expect("reads")
            .expect("stored");
        if let Some(file) = detail
            .messages
            .iter()
            .flat_map(|message| message.attachments.clone())
            .find(|attachment| !attachment.inline)
        {
            return (thread.id, file);
        }
    }
    panic!("{list:?} has a thread with files");
}

#[tokio::test]
async fn the_inboxs_files_are_kept_ahead_and_open_offline() {
    let mut harness = Harness::synced().await;
    let parts = support::TempDir::new();
    harness
        .store
        .cache_parts(exo_store::PartsCache::new(parts.path().to_owned()));
    let (_, file) = file_in(&harness, &ListId::Inbox).await;
    assert!(
        harness
            .store
            .cached_part(&file.message_id, &file.part_id)
            .is_none()
    );
    assert!(
        harness
            .store
            .prefetch_parts(1000)
            .await
            .expect("prefetches")
            > 0
    );
    let kept = harness
        .store
        .cached_part(&file.message_id, &file.part_id)
        .expect("kept ahead");
    assert_eq!(
        harness
            .store
            .prefetch_parts(1000)
            .await
            .expect("prefetches"),
        0,
        "nothing left to fetch"
    );
    harness.server.disconnect().await;
    let offline = harness
        .store
        .part_data(
            &file.message_id,
            file.attachment_id.as_deref(),
            &file.part_id,
        )
        .await
        .expect("from the cache");
    assert_eq!(offline, kept);
}

#[tokio::test]
async fn a_kept_part_goes_with_its_conversations_bodies() {
    let harness = Harness::synced().await;
    let parts = support::TempDir::new();
    harness
        .store
        .cache_parts(exo_store::PartsCache::new(parts.path().to_owned()));
    let (thread, file) = file_in(&harness, &ListId::Done).await;
    harness.store.load_bodies(&thread).await.expect("opens");
    harness
        .store
        .part_data(
            &file.message_id,
            file.attachment_id.as_deref(),
            &file.part_id,
        )
        .await
        .expect("fetches");
    assert!(
        harness
            .store
            .cached_part(&file.message_id, &file.part_id)
            .is_some(),
        "what's fetched is kept"
    );
    // Past the budget, the bodies go and the rows stay.
    let bodies = exo_store::Eviction {
        age: SignedDuration::from_hours(24 * 365 * 100),
        budget: 0,
    };
    harness.store.evict(&bodies).await.expect("evicts");
    assert!(!harness.store.bodies_stored(&thread).await.expect("reads"));
    assert!(
        harness
            .store
            .cached_part(&file.message_id, &file.part_id)
            .is_none(),
        "gone with the bodies"
    );

    // Quiet, the whole conversation goes.
    harness.store.load_bodies(&thread).await.expect("opens");
    harness
        .store
        .part_data(
            &file.message_id,
            file.attachment_id.as_deref(),
            &file.part_id,
        )
        .await
        .expect("fetches");
    let everything = exo_store::Eviction {
        age: SignedDuration::ZERO,
        budget: 0,
    };
    harness.store.evict(&everything).await.expect("evicts");
    assert!(
        harness
            .store
            .thread(&thread)
            .await
            .expect("reads")
            .is_none()
    );
    assert!(
        harness
            .store
            .cached_part(&file.message_id, &file.part_id)
            .is_none(),
        "gone with the conversation"
    );
}
