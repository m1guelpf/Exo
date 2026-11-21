//! Bulk relabels and sender filters, against gmail-mock.

mod support;

use std::collections::BTreeSet;

use exo_store::{BulkTarget, ListCutoff, ListId, Mutation, Progress, Relabelled, Scope};
use gmail::ListQuery;
use support::Harness;

const SENDER: &str = "tom@brightmoor.co";

/// The mock's filters on [`SENDER`], as `(from, added labels)`.
async fn filters(harness: &Harness) -> Vec<(String, Vec<String>)> {
    let list = harness
        .gmail
        .gmail()
        .filters()
        .await
        .expect("filters.list")
        .filter;
    list.into_iter()
        .filter(|filter| {
            filter
                .criteria
                .as_ref()
                .and_then(|criteria| criteria.from.as_deref())
                == Some(SENDER)
        })
        .map(|filter| {
            (
                filter
                    .criteria
                    .and_then(|criteria| criteria.from)
                    .unwrap_or_default(),
                filter
                    .action
                    .map(|action| action.add_label_ids)
                    .unwrap_or_default(),
            )
        })
        .collect()
}

#[tokio::test]
async fn trashing_everything_from_a_sender_and_back() {
    let harness = Harness::synced().await;
    let query = format!("from:\"{SENDER}\"");
    let found = harness.store.find_threads(&query).await.unwrap();
    assert!(!found.is_empty());
    let in_inbox = harness
        .store
        .find_threads(&format!("{query} in:inbox"))
        .await
        .unwrap();
    let changed = harness
        .store
        .relabel(&found, &["TRASH".into()], &["INBOX".into(), "SPAM".into()])
        .await
        .unwrap();
    assert_eq!(changed.len(), found.len());
    assert!(
        harness.store.find_threads(&query).await.unwrap().is_empty(),
        "all in Trash"
    );
    let inbox = harness.ids(&ListId::Inbox).await;
    assert!(
        found.iter().all(|thread| !inbox.contains(thread.as_str())),
        "the store synced"
    );

    harness
        .store
        .relabel(&changed, &[], &["TRASH".into()])
        .await
        .unwrap();
    harness
        .store
        .relabel(&in_inbox, &["INBOX".into()], &[])
        .await
        .unwrap();
    let back = harness
        .gmail_ids(ListQuery {
            q: Some(query),
            ..ListQuery::default()
        })
        .await;
    assert_eq!(back.len(), found.len());
    let inbox = harness.ids(&ListId::Inbox).await;
    assert!(
        in_inbox
            .iter()
            .all(|thread| inbox.contains(thread.as_str())),
        "back in the inbox"
    );
}

#[tokio::test]
async fn a_sender_filter_replaces_the_one_for_the_other_category() {
    let harness = Harness::synced().await;
    let (important, replaced) = harness
        .store
        .create_sender_filter(SENDER, "CATEGORY_PERSONAL")
        .await
        .unwrap();
    assert!(replaced.is_none());
    let (other, replaced) = harness
        .store
        .create_sender_filter(SENDER, "CATEGORY_SOCIAL")
        .await
        .unwrap();
    assert_eq!(replaced.and_then(|filter| filter.id), important);
    assert_eq!(
        filters(&harness).await,
        [(SENDER.to_owned(), vec!["CATEGORY_SOCIAL".to_owned()])]
    );

    // Undo: the Important filter comes back.
    let replaced = gmail::Filter {
        id: None,
        criteria: Some(gmail::FilterCriteria {
            from: Some(SENDER.into()),
            ..Default::default()
        }),
        action: Some(gmail::FilterAction {
            add_label_ids: vec!["CATEGORY_PERSONAL".into()],
            ..Default::default()
        }),
    };
    harness
        .store
        .delete_sender_filter(other.as_deref(), Some(&replaced))
        .await
        .unwrap();
    assert_eq!(
        filters(&harness).await,
        [(SENDER.to_owned(), vec!["CATEGORY_PERSONAL".to_owned()])]
    );
}

/// A cutoff past every fixture message.
fn far_future() -> jiff::Timestamp {
    "2100-01-01T00:00:00Z".parse().unwrap()
}

/// The inbox's rows, newest first.
async fn inbox(harness: &Harness) -> Vec<exo_store::ThreadSummary> {
    harness.all(&ListId::Inbox).await
}

#[tokio::test]
async fn a_select_all_takes_what_is_older_and_leaves_the_unselected_rows() {
    let harness = Harness::synced().await;
    let rows = inbox(&harness).await;
    assert!(rows.len() > 6);
    let all: BTreeSet<String> = rows.iter().map(|row| row.id.as_str().to_owned()).collect();
    // ⌘A on the fourth row with the second unselected, and the first selected by hand.
    let target = BulkTarget {
        list: Some(ListCutoff {
            scope: Scope::List(ListId::Inbox),
            filter: None,
            keep: Vec::new(),
            before: rows[3].date,
        }),
        include: [0, 2, 3].map(|at| rows[at].id.clone()).to_vec(),
        exclude: vec![rows[1].id.clone()],
    };
    let progress = Progress::default();
    let mut done = Relabelled::default();
    harness
        .store
        .bulk(&target, &Mutation::MarkDone, &progress, &mut done)
        .await
        .unwrap();
    assert_eq!(
        harness.ids(&ListId::Inbox).await,
        BTreeSet::from([rows[1].id.as_str().to_owned()])
    );
    assert_eq!(done.len(), rows.len() - 1);
    assert_eq!(
        (progress.done(), progress.total()),
        (done.len(), done.len())
    );
    assert_eq!(
        harness
            .gmail_ids(ListQuery {
                q: Some("in:inbox".into()),
                ..ListQuery::default()
            })
            .await
            .len(),
        1,
        "Gmail has it"
    );

    harness
        .store
        .undo_bulk(&done, &Progress::default())
        .await
        .unwrap();
    assert_eq!(harness.ids(&ListId::Inbox).await, all);
}

#[tokio::test]
async fn undoing_a_bulk_action_leaves_what_already_had_it() {
    let harness = Harness::synced().await;
    let receipts = harness
        .store
        .labels()
        .await
        .unwrap()
        .into_iter()
        .find(|label| label.name == "Receipts")
        .unwrap()
        .id;
    let labelled = || {
        harness.gmail_ids(ListQuery {
            q: Some("-in:inbox".into()),
            label_ids: vec![receipts.clone()],
            ..ListQuery::default()
        })
    };
    let done_list = || {
        harness.gmail_ids(ListQuery {
            q: Some("-in:inbox".into()),
            ..ListQuery::default()
        })
    };
    let before = labelled().await;
    let everything = done_list().await;
    assert!(!before.is_empty() && before.len() < everything.len());
    // Done reaches back past what the store has: most of these threads aren't stored. Gmail's
    // `-in:inbox` leaves out the trashed rows Done also shows, so a bulk action can't reach them.
    let stored: BTreeSet<String> = harness
        .ids(&ListId::Done)
        .await
        .intersection(&everything)
        .cloned()
        .collect();
    assert!(stored.len() < everything.len());
    // Threads Gmail's query finds that the store has outside Done (drafts, sent-only mail)
    // aren't rows of Done, so they're left alone.
    let mut elsewhere = Vec::new();
    for thread in everything.difference(&stored) {
        if harness
            .store
            .thread(&exo_store::ThreadId::new(thread.clone()))
            .await
            .unwrap()
            .is_some()
            && !before.contains(thread)
        {
            elsewhere.push(thread.clone());
        }
    }
    assert!(!elsewhere.is_empty());

    let target = BulkTarget {
        list: Some(ListCutoff {
            scope: Scope::List(ListId::Done),
            filter: None,
            keep: Vec::new(),
            before: far_future(),
        }),
        ..BulkTarget::default()
    };
    let mut done = Relabelled::default();
    harness
        .store
        .bulk(
            &target,
            &Mutation::AddLabel(receipts.clone()),
            &Progress::default(),
            &mut done,
        )
        .await
        .unwrap();
    let after = labelled().await;
    assert!(stored.is_subset(&after), "every row of Done");
    assert!(after.len() > stored.len(), "and the threads not stored");
    assert!(elsewhere.iter().all(|thread| !after.contains(thread)));
    assert_eq!(
        done.len(),
        after.len() - before.len(),
        "only the threads without it changed"
    );

    harness
        .store
        .undo_bulk(&done, &Progress::default())
        .await
        .unwrap();
    assert_eq!(labelled().await, before);
}

#[tokio::test]
async fn a_filtered_select_all_only_takes_what_the_filter_passes() {
    let harness = Harness::synced().await;
    let unread = harness
        .gmail_ids(ListQuery {
            q: Some("in:inbox is:unread".into()),
            ..ListQuery::default()
        })
        .await;
    let read: BTreeSet<String> = harness
        .gmail_ids(ListQuery {
            q: Some("in:inbox".into()),
            ..ListQuery::default()
        })
        .await
        .difference(&unread)
        .cloned()
        .collect();
    assert!(!unread.is_empty() && !read.is_empty());
    let target = BulkTarget {
        list: Some(ListCutoff {
            scope: Scope::List(ListId::Inbox),
            filter: Some(exo_store::Filter::Unread),
            keep: Vec::new(),
            before: far_future(),
        }),
        ..BulkTarget::default()
    };
    let mut done = Relabelled::default();
    harness
        .store
        .bulk(
            &target,
            &Mutation::MarkDone,
            &Progress::default(),
            &mut done,
        )
        .await
        .unwrap();
    let inbox = harness
        .gmail_ids(ListQuery {
            q: Some("in:inbox".into()),
            ..ListQuery::default()
        })
        .await;
    assert!(unread.iter().all(|thread| !inbox.contains(thread)));
    assert!(read.iter().all(|thread| inbox.contains(thread)));
}

#[tokio::test]
async fn a_stopped_bulk_action_records_what_it_did() {
    let harness = Harness::synced().await;
    let all = harness.ids(&ListId::Inbox).await;
    let target = BulkTarget {
        list: Some(ListCutoff {
            scope: Scope::List(ListId::Inbox),
            filter: None,
            keep: Vec::new(),
            before: far_future(),
        }),
        ..BulkTarget::default()
    };
    let progress = Progress::default();
    progress.stop();
    let mut done = Relabelled::default();
    harness
        .store
        .bulk(&target, &Mutation::MarkDone, &progress, &mut done)
        .await
        .unwrap();
    assert!(done.is_empty());
    assert_eq!(harness.ids(&ListId::Inbox).await, all);
}

#[tokio::test]
async fn a_select_all_of_search_results_takes_what_gmail_finds_for_the_search() {
    let harness = Harness::synced().await;
    let from = |q: &str| {
        harness.gmail_ids(ListQuery {
            q: Some(format!("from:{SENDER} {q}")),
            ..ListQuery::default()
        })
    };
    let inbox = || {
        harness.gmail_ids(ListQuery {
            q: Some("in:inbox".into()),
            ..ListQuery::default()
        })
    };
    let theirs = from("in:inbox").await;
    assert!(!theirs.is_empty());
    let others: BTreeSet<String> = inbox().await.difference(&theirs).cloned().collect();
    let target = BulkTarget {
        list: Some(ListCutoff {
            scope: Scope::Search(format!("from:{SENDER}")),
            filter: None,
            keep: Vec::new(),
            before: far_future(),
        }),
        ..BulkTarget::default()
    };
    let mut done = Relabelled::default();
    harness
        .store
        .bulk(
            &target,
            &Mutation::MarkDone,
            &Progress::default(),
            &mut done,
        )
        .await
        .unwrap();
    assert_eq!(inbox().await, others, "theirs are done, nothing else");
}

#[tokio::test]
async fn get_me_to_zero_keeps_unread_and_starred_conversations() {
    let harness = Harness::synced().await;
    let inbox = ListId::Label("INBOX".into());
    let rows = harness.all(&inbox).await;
    let cutoff = rows[rows.len() / 2].date;
    let kept: BTreeSet<String> = rows
        .iter()
        .filter(|row| row.date > cutoff || row.unread || row.starred)
        .map(|row| row.id.as_str().to_owned())
        .collect();
    assert!(kept.len() < rows.len() && rows.iter().any(|row| row.date <= cutoff && row.unread));
    let keep = vec![exo_store::Filter::Unread, exo_store::Filter::Starred];
    let target = BulkTarget {
        list: Some(ListCutoff {
            scope: Scope::List(inbox.clone()),
            filter: None,
            keep,
            before: cutoff,
        }),
        ..BulkTarget::default()
    };
    let mut done = Relabelled::default();
    harness
        .store
        .bulk(
            &target,
            &Mutation::MarkDone,
            &Progress::default(),
            &mut done,
        )
        .await
        .unwrap();
    assert_eq!(harness.ids(&inbox).await, kept);
    assert_eq!(
        harness
            .gmail_ids(ListQuery {
                q: Some("in:inbox".into()),
                ..ListQuery::default()
            })
            .await,
        kept,
        "Gmail has it"
    );
}

#[tokio::test]
async fn unblocking_deletes_only_the_block_filter() {
    let harness = Harness::synced().await;
    let (_, _) = harness
        .store
        .create_sender_filter(SENDER, "TRASH")
        .await
        .unwrap();
    // A filter of the user's own on the same sender, which isn't a block.
    let own = gmail::Filter {
        id: None,
        criteria: Some(gmail::FilterCriteria {
            from: Some(SENDER.into()),
            ..Default::default()
        }),
        action: Some(gmail::FilterAction {
            add_label_ids: vec!["TRASH".into()],
            remove_label_ids: vec!["INBOX".into()],
            ..Default::default()
        }),
    };
    harness.gmail.gmail().create_filter(&own).await.unwrap();
    assert_eq!(harness.store.delete_block_filters(SENDER).await.unwrap(), 1);
    assert_eq!(
        filters(&harness).await,
        [(SENDER.to_owned(), vec!["TRASH".to_owned()])],
        "the user's own filter stays"
    );
    assert_eq!(
        harness
            .store
            .delete_block_filters("someone@else.com")
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn get_me_to_zero_leaves_conversations_with_mail_since_its_cutoff() {
    let harness = Harness::synced().await;
    let inbox = ListId::Label("INBOX".into());
    let rows = harness.all(&inbox).await;
    let cutoff = rows[rows.len() / 2].date;
    // A conversation the store doesn't have: a read message from before the cutoff, then a
    // reply since, both in the inbox.
    let now = harness
        .admin(reqwest::Method::GET, "/__mock/clock", serde_json::json!({}))
        .await["now"]
        .as_str()
        .expect("the mock's time")
        .to_owned();
    let before = cutoff
        .checked_sub(jiff::SignedDuration::from_hours(24))
        .expect("in range");
    harness
        .admin(
            reqwest::Method::POST,
            "/__mock/clock",
            serde_json::json!({"now": before.to_string()}),
        )
        .await;
    let old = harness.deliver(serde_json::json!({"from": "Ines Duarte <ines@calloway.io>", "subject": "Quarterly numbers", "text": "Here they are", "labelIds": ["INBOX"]})).await;
    harness
        .admin(
            reqwest::Method::POST,
            "/__mock/clock",
            serde_json::json!({"now": now}),
        )
        .await;
    let thread = old.thread_id.expect("a thread");
    harness.deliver(serde_json::json!({"from": "Ines Duarte <ines@calloway.io>", "threadId": thread, "text": "One correction"})).await;
    // Get Me To Zero before the cutoff, keeping unread conversations.
    let target = BulkTarget {
        list: Some(ListCutoff {
            scope: Scope::List(inbox),
            filter: None,
            keep: vec![exo_store::Filter::Unread],
            before: cutoff,
        }),
        ..BulkTarget::default()
    };
    let mut done = Relabelled::default();
    harness
        .store
        .bulk(
            &target,
            &Mutation::MarkDone,
            &Progress::default(),
            &mut done,
        )
        .await
        .unwrap();
    let in_inbox = harness
        .gmail_ids(ListQuery {
            q: Some("in:inbox".into()),
            ..ListQuery::default()
        })
        .await;
    assert!(
        in_inbox.contains(&thread),
        "the reply since the cutoff (unread) wasn't archived unseen"
    );
}

#[tokio::test]
async fn a_sender_filter_gmail_refuses_leaves_the_one_it_would_replace() {
    let harness = Harness::synced().await;
    harness
        .store
        .create_sender_filter(SENDER, "CATEGORY_PERSONAL")
        .await
        .unwrap();
    // Moving the sender to Other: Gmail won't make the new filter.
    harness.fault(serde_json::json!({"method": "POST", "path": "/gmail/v1/users/me/settings/filters", "status": 400, "message": "Invalid filter", "reason": "invalidArgument"})).await;
    assert!(
        harness
            .store
            .create_sender_filter(SENDER, "CATEGORY_SOCIAL")
            .await
            .is_err()
    );
    assert_eq!(
        filters(&harness).await,
        [(SENDER.to_owned(), vec!["CATEGORY_PERSONAL".to_owned()])],
        "the Important filter stays: nothing replaces it, and the failed job has nothing to undo"
    );
}

#[tokio::test]
async fn a_sender_filter_whose_answer_was_lost_is_the_one_undo_deletes() {
    let harness = Harness::synced().await;
    // Gmail makes the filter but the answer is lost, so the retry is refused as a duplicate.
    harness.fault(serde_json::json!({"method": "POST", "path": "/gmail/v1/users/me/settings/filters", "status": 503, "after": true})).await;
    let (id, _) = harness
        .store
        .create_sender_filter(SENDER, "TRASH")
        .await
        .expect("the filter Gmail made counts");
    assert_eq!(
        filters(&harness).await,
        [(SENDER.to_owned(), vec!["TRASH".to_owned()])]
    );
    harness
        .store
        .delete_sender_filter(id.as_deref(), None)
        .await
        .unwrap();
    assert!(filters(&harness).await.is_empty(), "undo deletes it");
}

#[tokio::test]
async fn a_sender_filter_gmail_already_has_stays_after_undo() {
    let harness = Harness::synced().await;
    // The same block, made in Gmail web.
    let block = gmail::Filter {
        id: None,
        criteria: Some(gmail::FilterCriteria {
            from: Some(SENDER.into()),
            ..Default::default()
        }),
        action: Some(gmail::FilterAction {
            add_label_ids: vec!["TRASH".into()],
            ..Default::default()
        }),
    };
    harness.gmail.gmail().create_filter(&block).await.unwrap();
    let (id, _) = harness
        .store
        .create_sender_filter(SENDER, "TRASH")
        .await
        .expect("blocking goes on");
    assert_eq!(id, None, "nothing made");
    harness
        .store
        .delete_sender_filter(id.as_deref(), None)
        .await
        .unwrap();
    assert_eq!(
        filters(&harness).await,
        [(SENDER.to_owned(), vec!["TRASH".to_owned()])],
        "undo leaves the user's own"
    );
}
