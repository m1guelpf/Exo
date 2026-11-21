//! Initial sync: the history cursor first, list backfill with the coverage rule, and
//! `threads.get` only for missing or stale threads.

mod support;

use std::collections::BTreeSet;

use exo_store::{ListId, ThreadId};
use gmail::ListQuery;
use serde_json::json;
use support::Harness;

/// Threads that are only a Gmail draft (a new message being written). Written outside Exo,
/// they're only in Drafts.
async fn draft_only(harness: &Harness) -> BTreeSet<String> {
    harness
        .all(&ListId::Drafts)
        .await
        .into_iter()
        .filter(|thread| thread.drafts == thread.messages)
        .map(|thread| thread.id.as_str().to_owned())
        .collect()
}

fn label(id: &str) -> ListQuery {
    ListQuery {
        label_ids: vec![id.to_owned()],
        ..ListQuery::default()
    }
}

#[tokio::test]
async fn initial_sync_fills_the_inbox() {
    let harness = Harness::start().await;
    assert!(
        harness
            .store
            .list(&ListId::Inbox, None, 50)
            .await
            .unwrap()
            .threads
            .is_empty()
    );

    let first = harness.store.sync().await.unwrap();
    assert!(!first.rebuilt);
    harness.sync_all().await;

    let inbox = harness.all(&ListId::Inbox).await;
    let gmail_inbox = harness.gmail_ids(label("INBOX")).await;
    assert_eq!(gmail_inbox.len(), 24, "fixture inbox");
    let drafts = draft_only(&harness).await;
    assert_eq!(
        drafts.len(),
        2,
        "fixture new-message drafts, in Drafts only"
    );
    assert_eq!(
        inbox
            .iter()
            .map(|t| t.id.as_str().to_owned())
            .collect::<BTreeSet<_>>(),
        gmail_inbox
    );
    // Newest first, and Important ∪ Other is the Inbox.
    assert!(inbox.windows(2).all(|pair| pair[0].date >= pair[1].date));
    let important = harness.ids(&ListId::Important).await;
    let other = harness.ids(&ListId::Other).await;
    assert!(important.is_disjoint(&other));
    assert_eq!(&important | &other, gmail_inbox);
    assert!(!important.is_empty() && !other.is_empty());

    let page = harness.store.list(&ListId::Inbox, None, 50).await.unwrap();
    assert!(!page.more_remote, "the inbox pass reached the end");
    let count = harness.store.count(&ListId::Inbox).await.unwrap();
    assert_eq!(count.threads, 24);
    assert!(count.exact);
    let unread = inbox.iter().filter(|thread| thread.unread).count();
    assert_eq!(count.unread, unread);
    assert!(unread > 0);
}

#[tokio::test]
async fn a_draft_exo_wrote_routes_as_a_message_being_written() {
    let harness = Harness::synced().await;
    let drafts = draft_only(&harness).await;
    let thread = ThreadId::new(drafts.first().unwrap().clone());
    assert!(!harness.ids(&ListId::Inbox).await.contains(thread.as_str()));
    let detail = harness.store.thread(&thread).await.unwrap().unwrap();
    harness
        .store
        .record_draft(&detail.messages[0].id)
        .await
        .unwrap();
    assert!(
        harness
            .ids(&ListId::Important)
            .await
            .contains(thread.as_str()),
        "a new message being written sits in Important"
    );
    let row = harness
        .all(&ListId::Inbox)
        .await
        .into_iter()
        .find(|row| row.id == thread)
        .unwrap();
    assert_eq!(row.drafts, 1);
}

#[tokio::test]
async fn summaries_carry_what_the_list_shows() {
    let harness = Harness::synced().await;
    let inbox = harness.all(&ListId::Inbox).await;
    for thread in &inbox {
        assert!(!thread.subject.is_empty(), "{thread:?}");
        assert!(!thread.participants.is_empty(), "{thread:?}");
        assert!(thread.messages >= 1);
        assert!(
            thread.labels.iter().any(|label| label == "INBOX"),
            "{thread:?}"
        );
        assert_eq!(thread.drafts, 0, "Gmail's drafts don't show: {thread:?}");
        assert!(!thread.snippet.contains("&#39;"), "snippets are unescaped");
    }
    // Senders other than me, unless I'm the only one.
    for thread in &inbox {
        let all_me = thread.participants.iter().all(|c| c.is_me);
        assert!(!all_me || thread.participants.len() == 1, "{thread:?}");
    }
    let detail = harness.store.thread(&inbox[0].id).await.unwrap().unwrap();
    assert_eq!(detail.messages.len(), inbox[0].messages);
    assert!(
        detail
            .messages
            .iter()
            .all(|m| m.body.html.is_some() || m.body.text.is_some())
    );
    assert!(
        detail
            .messages
            .iter()
            .all(|m| m.from.is_some() && !m.headers.is_empty())
    );
}

#[tokio::test]
async fn lists_show_only_their_backfilled_range() {
    let harness = Harness::synced().await;
    // Starred has no pass of its own yet: only All Mail's 31 days cover it.
    let starred_gmail = harness.gmail_ids(label("STARRED")).await;
    let starred_local = harness.ids(&ListId::Starred).await;
    assert!(starred_local.is_subset(&starred_gmail));
    let page = harness
        .store
        .list(&ListId::Starred, None, 100)
        .await
        .unwrap();
    assert!(page.more_remote);
    let oldest_all = harness.all(&ListId::All).await.last().unwrap().date;
    assert!(page.threads.iter().all(|thread| thread.date >= oldest_all));

    // Backfilling the list itself covers it completely.
    while harness.store.backfill(&ListId::Starred, 10).await.unwrap() {}
    assert_eq!(harness.ids(&ListId::Starred).await, starred_gmail);
    assert!(
        !harness
            .store
            .list(&ListId::Starred, None, 1)
            .await
            .unwrap()
            .more_remote
    );

    // Done is every thread without an inbox message, other than new-message drafts.
    while harness.store.backfill(&ListId::Done, 100).await.unwrap() {}
    let everything = harness.gmail_ids(ListQuery::default()).await;
    let expected =
        &(&everything - &harness.gmail_ids(label("INBOX")).await) - &draft_only(&harness).await;
    let done_local = harness.ids(&ListId::Done).await;
    assert!(
        expected.is_subset(&done_local),
        "{:?}",
        &expected - &done_local
    );
    assert!(done_local.is_disjoint(&harness.ids(&ListId::Inbox).await));
}

#[tokio::test]
async fn a_finished_all_mail_backfill_covers_every_list() {
    let harness = Harness::synced().await;
    while harness.store.backfill(&ListId::All, 100).await.unwrap() {}

    // Every thread is stored, so Starred is complete without a pass of its own.
    let page = harness
        .store
        .list(&ListId::Starred, None, 100)
        .await
        .unwrap();
    assert!(!page.more_remote);
    assert!(harness.store.count(&ListId::Starred).await.unwrap().exact);
    assert_eq!(
        harness.ids(&ListId::Starred).await,
        harness.gmail_ids(label("STARRED")).await
    );
}

#[tokio::test]
async fn backfill_fetches_only_missing_or_stale_threads() {
    let harness = Harness::synced().await;
    // All Mail backfilled to the end, so a list's own pass starts from the top rather than where
    // All Mail's ends.
    while harness.store.backfill(&ListId::All, 500).await.unwrap() {}
    // User labels whose threads the initial sync already stored, though their own passes
    // never ran.
    let mut covered = Vec::new();
    for label in harness.store.labels().await.unwrap() {
        if !label.id.starts_with("Label_") {
            continue;
        }
        let ids = harness.gmail_ids(self::label(&label.id)).await;
        let mut all_stored = !ids.is_empty();
        for id in &ids {
            all_stored &= harness
                .store
                .thread(&ThreadId::new(id.clone()))
                .await
                .unwrap()
                .is_some();
        }
        if all_stored {
            covered.push((ListId::Label(label.id), ids));
        }
    }
    let [(fresh, fresh_ids), (stale, stale_ids), ..] = covered.as_slice() else {
        panic!("the fixture has two labels covered by the initial sync");
    };
    let fail_threads_get = json!({ "method": "GET", "path": "/gmail/v1/users/me/threads/", "status": 400, "times": 1000 });

    // Nothing to fetch: listing works while every threads.get fails.
    harness.fault(fail_threads_get.clone()).await;
    assert!(!harness.store.backfill(fresh, 50).await.unwrap());
    assert_eq!(&harness.ids(fresh).await, fresh_ids);

    // A thread that changed since it was stored is stale: listing it needs threads.get.
    let changed = stale_ids.iter().next().unwrap().clone();
    harness
        .deliver(json!({ "from": "Ana <ana@example.com>", "threadId": changed, "text": "One more thing" }))
        .await;
    assert!(harness.store.backfill(stale, 50).await.is_err());
    harness.clear_faults().await;
    let before = harness
        .summary(stale, &ThreadId::new(changed.clone()))
        .await
        .unwrap()
        .messages;
    assert!(!harness.store.backfill(stale, 50).await.unwrap());
    let after = harness
        .summary(stale, &ThreadId::new(changed))
        .await
        .unwrap()
        .messages;
    assert_eq!(after, before + 1);
}

#[tokio::test]
async fn reading_an_unknown_thread_is_none() {
    let harness = Harness::start().await;
    assert!(
        harness
            .store
            .thread(&ThreadId::new("ffff"))
            .await
            .unwrap()
            .is_none()
    );
}
