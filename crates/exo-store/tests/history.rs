//! Incremental sync from Gmail history: new mail, changes made in other clients, the cursor
//! moving only after everything is applied, and recovery when history expires.

mod support;

use exo_store::{ListId, Mutation, ThreadId};
use gmail::{BatchModifyMessagesRequest, ModifyThreadRequest};
use serde_json::json;
use support::Harness;

#[tokio::test]
async fn delivered_mail_appears_after_sync() {
    let harness = Harness::synced().await;
    let delivered = harness
        .deliver(json!({
            "from": "Priya Raman <priya@brightmoor.co>",
            "subject": "Launch checklist",
            "text": "Can you look at the checklist before 3?",
        }))
        .await;
    let id = ThreadId::new(delivered.thread_id.unwrap());

    let report = harness.store.sync().await.unwrap();
    assert_eq!(report.changed, 1);
    let inbox = harness.all(&ListId::Inbox).await;
    let top = &inbox[0];
    assert_eq!(top.id, id, "newest first");
    assert_eq!(top.subject, "Launch checklist");
    assert!(top.unread);
    assert_eq!(top.participants[0].email, "priya@brightmoor.co");
    assert!(
        harness.ids(&ListId::Important).await.contains(id.as_str()),
        "CATEGORY_PERSONAL"
    );

    // A reply into a stored thread bumps it and adds the message.
    let old = inbox.last().unwrap().clone();
    harness
        .deliver(
            json!({ "from": "Ana <ana@example.com>", "threadId": old.id.as_str(), "text": "Bump" }),
        )
        .await;
    harness.store.sync().await.unwrap();
    let inbox = harness.all(&ListId::Inbox).await;
    assert_eq!(inbox[0].id, old.id);
    assert_eq!(inbox[0].messages, old.messages + 1);
}

#[tokio::test]
async fn changes_made_elsewhere_are_applied() {
    let harness = Harness::synced().await;
    let inbox = harness.all(&ListId::Inbox).await;
    let archived = inbox.iter().find(|t| t.drafts == 0).unwrap().id.clone();
    let starred = inbox
        .iter()
        .rev()
        .find(|t| !t.starred && t.drafts == 0)
        .unwrap()
        .id
        .clone();
    let projects = harness
        .store
        .labels()
        .await
        .unwrap()
        .into_iter()
        .find(|label| label.name == "Projects/Exo")
        .unwrap()
        .id;
    let gmail = harness.gmail.gmail();

    // Archived in Gmail web: INBOX removed from every message.
    let ids: Vec<String> = harness
        .gmail_labels(&archived)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    gmail
        .batch_modify_messages(&BatchModifyMessagesRequest {
            ids,
            remove_label_ids: vec!["INBOX".into()],
            ..BatchModifyMessagesRequest::default()
        })
        .await
        .unwrap();
    // Starred and labelled in Gmail web.
    gmail
        .modify_thread(
            starred.as_str(),
            &ModifyThreadRequest {
                add_label_ids: vec!["STARRED".into(), projects.clone()],
                remove_label_ids: Vec::new(),
            },
        )
        .await
        .unwrap();

    let report = harness.store.sync().await.unwrap();
    assert_eq!(report.changed, 2);
    assert!(
        !harness
            .ids(&ListId::Inbox)
            .await
            .contains(archived.as_str())
    );
    assert!(harness.ids(&ListId::Done).await.contains(archived.as_str()));
    assert!(
        harness
            .summary(&ListId::Inbox, &starred)
            .await
            .unwrap()
            .starred
    );
    assert!(
        harness
            .ids(&ListId::Starred)
            .await
            .contains(starred.as_str())
    );
    assert!(
        harness
            .ids(&ListId::Label(projects))
            .await
            .contains(starred.as_str())
    );

    // Nothing new: nothing changes.
    assert_eq!(harness.store.sync().await.unwrap().changed, 0);
}

#[tokio::test]
async fn labels_made_elsewhere_are_picked_up() {
    let harness = Harness::synced().await;
    let thread = harness
        .all(&ListId::Inbox)
        .await
        .into_iter()
        .find(|t| t.drafts == 0)
        .unwrap()
        .id;
    let gmail = harness.gmail.gmail();
    let label = gmail
        .create_label(&gmail::Label {
            name: Some("Board".into()),
            ..gmail::Label::default()
        })
        .await
        .unwrap()
        .id
        .unwrap();
    gmail
        .modify_thread(
            thread.as_str(),
            &ModifyThreadRequest {
                add_label_ids: vec![label.clone()],
                remove_label_ids: Vec::new(),
            },
        )
        .await
        .unwrap();
    // Listing labels fails past every retry: the sync fails, and the next one tries again.
    harness
        .fault(json!({ "method": "GET", "path": "/gmail/v1/users/me/labels", "status": 500, "times": 5 }))
        .await;
    assert!(harness.store.sync().await.is_err());
    harness.store.sync().await.unwrap();
    let labels = harness.store.labels().await.unwrap();
    assert!(labels.iter().any(|l| l.id == label && l.name == "Board"));
    assert!(
        harness
            .ids(&ListId::Label(label))
            .await
            .contains(thread.as_str())
    );
}

#[tokio::test]
async fn deleted_mail_disappears() {
    let harness = Harness::synced().await;
    let inbox = harness.all(&ListId::Inbox).await;
    let doomed = inbox.iter().find(|t| t.drafts == 0).unwrap().id.clone();
    harness
        .gmail
        .gmail()
        .delete_thread(doomed.as_str())
        .await
        .unwrap();
    harness.store.sync().await.unwrap();
    assert!(!harness.ids(&ListId::Inbox).await.contains(doomed.as_str()));
    assert!(harness.store.thread(&doomed).await.unwrap().is_none());
}

#[tokio::test]
async fn the_cursor_moves_only_after_everything_is_stored() {
    let harness = Harness::synced().await;
    let thread = harness.all(&ListId::Inbox).await.last().unwrap().clone();
    harness
        .deliver(json!({ "from": "Ana <ana@example.com>", "threadId": thread.id.as_str(), "text": "Again" }))
        .await;
    // Fetching the new message fails past every retry: the sync fails and stores nothing.
    harness
        .fault(json!({ "method": "GET", "path": "/gmail/v1/users/me/messages/", "status": 500, "times": 5 }))
        .await;
    assert!(matches!(
        harness.store.sync().await,
        Err(exo_store::Error::Gmail(_))
    ));
    assert_eq!(
        harness
            .summary(&ListId::Inbox, &thread.id)
            .await
            .unwrap()
            .messages,
        thread.messages
    );
    // The next sync replays the same history.
    assert_eq!(harness.store.sync().await.unwrap().changed, 1);
    assert_eq!(
        harness
            .summary(&ListId::Inbox, &thread.id)
            .await
            .unwrap()
            .messages,
        thread.messages + 1
    );
}

#[tokio::test]
async fn unknown_threads_that_change_are_fetched() {
    let harness = Harness::synced().await;
    // An old thread the initial sync didn't store, starred in Gmail web.
    let all = harness.gmail_ids(gmail::ListQuery::default()).await;
    let mut old = None;
    for id in all {
        if harness
            .store
            .thread(&ThreadId::new(id.clone()))
            .await
            .unwrap()
            .is_none()
        {
            old = Some(id);
            break;
        }
    }
    let old = ThreadId::new(old.expect("the fixture is older than the backfill window"));
    harness
        .gmail
        .gmail()
        .modify_thread(
            old.as_str(),
            &ModifyThreadRequest {
                add_label_ids: vec!["STARRED".into()],
                remove_label_ids: Vec::new(),
            },
        )
        .await
        .unwrap();
    harness.store.sync().await.unwrap();
    let detail = harness
        .store
        .thread(&old)
        .await
        .unwrap()
        .expect("fetched whole");
    assert!(detail.labels.iter().any(|label| label == "STARRED"));
}

#[tokio::test]
async fn expired_history_rebuilds_and_keeps_the_outbox() {
    let harness = Harness::synced().await;
    let inbox_before = harness.ids(&ListId::Inbox).await;
    let target = harness
        .all(&ListId::Inbox)
        .await
        .into_iter()
        .find(|t| t.drafts == 0)
        .unwrap()
        .id;
    harness
        .store
        .apply(std::slice::from_ref(&target), Mutation::MarkDone)
        .await
        .unwrap();

    // Mail arrives, then history from before it expires.
    harness
        .deliver(json!({ "from": "Ana <ana@example.com>", "subject": "While away", "text": "Hi" }))
        .await;
    harness
        .admin(reqwest::Method::POST, "/__mock/history/expire", json!({}))
        .await;
    let report = harness.store.sync().await.unwrap();
    assert!(report.rebuilt);
    harness.sync_all().await;

    // The cache is back, and the queued Done still shows and still goes out.
    let inbox = harness.ids(&ListId::Inbox).await;
    assert!(!inbox.contains(target.as_str()));
    assert_eq!(inbox.len(), inbox_before.len(), "one Done, one delivered");
    assert_eq!(harness.store.pending().await.unwrap(), 1);
    assert_eq!(harness.store.flush().await.unwrap().completed, 1);
    let labels = harness.gmail_labels(&target).await;
    assert!(labels.iter().all(|(_, labels)| !labels.contains("INBOX")));

    // And history works again from the new cursor.
    let delivered = harness
        .deliver(json!({ "from": "Ana <ana@example.com>", "subject": "After the rebuild", "text": "Hi" }))
        .await;
    harness.store.sync().await.unwrap();
    assert!(
        harness
            .ids(&ListId::Inbox)
            .await
            .contains(&delivered.thread_id.unwrap())
    );
}

#[tokio::test]
async fn history_never_overwrites_a_newer_copy() {
    let mut harness = Harness::synced().await;
    let gate = harness.reopen_gated().await;
    harness.store.sync().await.unwrap();
    let thread = harness
        .all(&ListId::Inbox)
        .await
        .into_iter()
        .find(|t| t.drafts == 0 && !t.starred && !t.muted)
        .unwrap();

    // Elsewhere, the thread is starred and you reply to it.
    harness
        .gmail
        .gmail()
        .modify_thread(
            thread.id.as_str(),
            &ModifyThreadRequest {
                add_label_ids: vec!["STARRED".into()],
                remove_label_ids: Vec::new(),
            },
        )
        .await
        .unwrap();
    harness
        .deliver(json!({ "from": support::ACCOUNT, "to": ["ana@example.com"], "threadId": thread.id.as_str(), "labelIds": ["SENT"], "text": "On it" }))
        .await;

    // A sync reads that history, then waits on fetching your reply...
    gate.hold_after(1);
    let syncing = tokio::spawn({
        let store = harness.store.clone();
        async move { store.sync().await }
    });
    gate.held().await;
    // ...while you mark the thread Done and Gmail acknowledges it.
    harness
        .store
        .apply(std::slice::from_ref(&thread.id), Mutation::MarkDone)
        .await
        .unwrap();
    assert_eq!(harness.store.flush().await.unwrap().completed, 1);
    gate.release();
    syncing.await.unwrap().unwrap();

    // The star from history lands; the older INBOX it carried doesn't.
    assert!(
        !harness
            .ids(&ListId::Inbox)
            .await
            .contains(thread.id.as_str())
    );
    let done = harness.summary(&ListId::Done, &thread.id).await.unwrap();
    assert!(done.starred);
    assert_eq!(done.messages, thread.messages + 1);
}
