//! A contact's latest conversations: read from local mail, filled from Gmail's search.

mod support;

use std::collections::BTreeSet;

use exo_store::{ListId, Mutation, RECENT_THREADS, RecentThread};
use gmail::ListQuery;
use support::{ACCOUNT, Harness};

/// The search for a contact's conversations.
fn search(email: &str) -> ListQuery {
    ListQuery {
        q: Some(format!(
            "(from:{email} OR to:{email} OR cc:{email} OR bcc:{email}) AND -in:DRAFT"
        )),
        ..ListQuery::default()
    }
}

/// Of the contacts of `list`'s threads, the one whose conversations Gmail has the most of, and
/// how many.
async fn busiest_contact(harness: &Harness, list: &ListId) -> (String, usize) {
    let mut best = (String::new(), 0);
    for thread in harness.all(list).await {
        let Some(contact) = thread.contact else {
            continue;
        };
        let count = harness.gmail_ids(search(&contact.email)).await.len();
        if count > best.1 {
            best = (contact.email, count);
        }
    }
    best
}

fn ids(threads: &[RecentThread]) -> BTreeSet<String> {
    threads
        .iter()
        .map(|thread| thread.id.as_str().to_owned())
        .collect()
}

/// The newest of `email`'s conversations Gmail lists, newest first.
async fn newest_from_gmail(harness: &Harness, email: &str) -> Vec<String> {
    let page = harness
        .gmail
        .gmail()
        .threads(&ListQuery {
            max_results: Some(RECENT_THREADS),
            ..search(email)
        })
        .await
        .expect("threads.list");
    page.threads
        .into_iter()
        .filter_map(|thread| thread.id)
        .collect()
}

fn listed(threads: &[RecentThread]) -> Vec<String> {
    threads
        .iter()
        .map(|thread| thread.id.as_str().to_owned())
        .collect()
}

#[tokio::test]
async fn local_mail_lists_the_newest_and_gmail_fills_in_older_ones() {
    let harness = Harness::start().await;
    // Only the inbox is stored: the newest conversations, but not all five.
    harness.store.backfill(&ListId::Inbox, 100).await.unwrap();
    let (email, total) = busiest_contact(&harness, &ListId::Inbox).await;
    assert!(
        total > RECENT_THREADS as usize,
        "{email} has {total} conversations"
    );
    let gmail = newest_from_gmail(&harness, &email).await;

    let local = harness
        .store
        .recent_threads(&email, RECENT_THREADS as usize)
        .await
        .unwrap();
    assert!(
        !local.is_empty() && local.len() < gmail.len(),
        "{} of {}",
        local.len(),
        gmail.len()
    );
    assert_eq!(listed(&local), gmail[..local.len()]);

    harness.store.fetch_recent_threads(&email).await.unwrap();
    let recent = harness
        .store
        .recent_threads(&email, RECENT_THREADS as usize)
        .await
        .unwrap();
    assert_eq!(listed(&recent), gmail);
    assert!(
        recent.windows(2).all(|pair| pair[0].date >= pair[1].date),
        "newest first"
    );
    assert!(recent.iter().all(|thread| !thread.subject.is_empty()));
    // Addresses match whatever their case.
    let shouted = harness
        .store
        .recent_threads(&email.to_uppercase(), 2)
        .await
        .unwrap();
    assert_eq!(shouted, recent[..2]);
}

#[tokio::test]
async fn your_own_address_lists_nothing() {
    let harness = Harness::synced().await;
    assert!(
        harness
            .store
            .recent_threads(ACCOUNT, 5)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn trashing_a_conversation_takes_it_out_until_undone() {
    let harness = Harness::synced().await;
    let (email, _) = busiest_contact(&harness, &ListId::All).await;
    let recent = harness.store.recent_threads(&email, 5).await.unwrap();
    let newest = recent[0].id.clone();

    let undo = harness
        .store
        .apply(std::slice::from_ref(&newest), Mutation::Trash)
        .await
        .unwrap();
    let after = harness.store.recent_threads(&email, 5).await.unwrap();
    assert!(
        !ids(&after).contains(newest.as_str()),
        "pending trash applies at once"
    );

    harness.store.undo(undo).await.unwrap();
    assert_eq!(
        harness.store.recent_threads(&email, 5).await.unwrap(),
        recent
    );
}

#[tokio::test]
async fn gmail_fills_in_what_isnt_stored() {
    let synced = Harness::synced().await;
    let (email, _) = busiest_contact(&synced, &ListId::All).await;
    let expected = newest_from_gmail(&synced, &email).await;
    drop(synced);

    // A store that hasn't backfilled anything.
    let harness = Harness::start().await;
    assert!(
        harness
            .store
            .recent_threads(&email, 5)
            .await
            .unwrap()
            .is_empty()
    );
    harness.store.fetch_recent_threads(&email).await.unwrap();
    assert_eq!(
        listed(&harness.store.recent_threads(&email, 5).await.unwrap()),
        expected
    );
}

#[tokio::test]
async fn the_fill_retries_through_gmail_errors() {
    let harness = Harness::start().await;
    let email = "priya@brightmoor.co";
    harness
        .fault(
            serde_json::json!({ "path": "/gmail/v1/users/me/threads", "status": 503, "times": 2 }),
        )
        .await;
    harness.store.fetch_recent_threads(email).await.unwrap();
    assert_eq!(
        listed(&harness.store.recent_threads(email, 5).await.unwrap()),
        newest_from_gmail(&harness, email).await
    );
}
