//! Every triage mutation, end to end: visible at once, then sent to Gmail.

mod support;

use std::collections::BTreeSet;

use exo_store::{LabelKind, ListId, Mutation, ThreadId, ThreadSummary};
use serde_json::json;
use support::Harness;

type Labels = Vec<(String, BTreeSet<String>)>;

/// An unstarred inbox thread (not a draft) that also has a message without `INBOX` (your
/// reply).
async fn inbox_thread_with_reply(harness: &Harness) -> ThreadSummary {
    for thread in harness.all(&ListId::Inbox).await {
        if thread.drafts > 0 || thread.muted || thread.starred {
            continue;
        }
        let labels = harness.gmail_labels(&thread.id).await;
        if labels.iter().any(|(_, l)| l.contains("INBOX"))
            && labels.iter().any(|(_, l)| !l.contains("INBOX"))
        {
            return thread;
        }
    }
    panic!("the fixture has an inbox thread with a reply");
}

async fn send(harness: &Harness, thread: &ThreadId, mutation: Mutation) {
    let undo = harness
        .store
        .apply(std::slice::from_ref(thread), mutation)
        .await
        .unwrap();
    assert!(!undo.is_empty());
    let report = harness.store.flush().await.unwrap();
    assert_eq!(report.completed, 1, "{report:?}");
    assert_eq!(harness.store.pending().await.unwrap(), 0);
}

fn without(labels: &Labels, label: &str) -> Labels {
    labels
        .iter()
        .map(|(id, l)| {
            (
                id.clone(),
                l.iter().filter(|x| *x != label).cloned().collect(),
            )
        })
        .collect()
}

#[tokio::test]
async fn done_and_not_done() {
    let harness = Harness::synced().await;
    let thread = inbox_thread_with_reply(&harness).await;
    let before = harness.gmail_labels(&thread.id).await;

    // Visible at once, before Gmail hears of it.
    harness
        .store
        .apply(std::slice::from_ref(&thread.id), Mutation::MarkDone)
        .await
        .unwrap();
    assert!(
        !harness
            .ids(&ListId::Inbox)
            .await
            .contains(thread.id.as_str())
    );
    assert!(
        harness
            .ids(&ListId::Done)
            .await
            .contains(thread.id.as_str())
    );
    assert_eq!(harness.gmail_labels(&thread.id).await, before);

    // INBOX comes off the messages that had it; nothing else changes.
    assert_eq!(harness.store.flush().await.unwrap().completed, 1);
    assert_eq!(
        harness.gmail_labels(&thread.id).await,
        without(&before, "INBOX")
    );
    harness.store.sync().await.unwrap();
    assert!(
        harness
            .ids(&ListId::Done)
            .await
            .contains(thread.id.as_str())
    );

    // Mark not Done adds INBOX to the whole thread (threads.modify), your reply included.
    send(&harness, &thread.id, Mutation::MarkNotDone).await;
    assert!(
        harness
            .gmail_labels(&thread.id)
            .await
            .iter()
            .all(|(_, l)| l.contains("INBOX"))
    );
    assert!(
        harness
            .ids(&ListId::Inbox)
            .await
            .contains(thread.id.as_str())
    );
}

#[tokio::test]
async fn star_marks_one_message_and_unstar_clears_all() {
    let harness = Harness::synced().await;
    let thread = inbox_thread_with_reply(&harness).await;
    assert!(!thread.starred);
    let detail = harness.store.thread(&thread.id).await.unwrap().unwrap();
    let latest_received = detail
        .messages
        .iter()
        .rev()
        .find(|m| !m.is_draft() && !m.from.as_ref().unwrap().is_me)
        .unwrap()
        .id
        .clone();

    send(&harness, &thread.id, Mutation::Star).await;
    let starred: Vec<String> = harness
        .gmail_labels(&thread.id)
        .await
        .into_iter()
        .filter(|(_, labels)| labels.contains("STARRED"))
        .map(|(id, _)| id)
        .collect();
    assert_eq!(starred, [latest_received]);
    assert!(
        harness
            .ids(&ListId::Starred)
            .await
            .contains(thread.id.as_str())
    );

    send(&harness, &thread.id, Mutation::Unstar).await;
    assert!(
        harness
            .gmail_labels(&thread.id)
            .await
            .iter()
            .all(|(_, l)| !l.contains("STARRED"))
    );
    assert!(
        !harness
            .ids(&ListId::Starred)
            .await
            .contains(thread.id.as_str())
    );
}

#[tokio::test]
async fn read_and_unread() {
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
    assert!(
        !harness
            .summary(&ListId::Inbox, &thread.id)
            .await
            .unwrap()
            .unread
    );
    assert_eq!(harness.store.flush().await.unwrap().completed, 1);
    assert!(
        harness
            .gmail_labels(&thread.id)
            .await
            .iter()
            .all(|(_, l)| !l.contains("UNREAD"))
    );

    // Unread goes on the latest message only.
    send(&harness, &thread.id, Mutation::MarkUnread).await;
    let labels = harness.gmail_labels(&thread.id).await;
    let unread: Vec<bool> = labels.iter().map(|(_, l)| l.contains("UNREAD")).collect();
    assert_eq!(unread.iter().filter(|u| **u).count(), 1);
    assert!(unread.last().unwrap());
    assert!(
        harness
            .summary(&ListId::Inbox, &thread.id)
            .await
            .unwrap()
            .unread
    );
}

#[tokio::test]
async fn trash_and_untrash() {
    let harness = Harness::synced().await;
    let thread = inbox_thread_with_reply(&harness).await;

    harness
        .store
        .apply(std::slice::from_ref(&thread.id), Mutation::Trash)
        .await
        .unwrap();
    assert!(
        harness
            .ids(&ListId::Trash)
            .await
            .contains(thread.id.as_str())
    );
    assert!(
        !harness
            .ids(&ListId::Inbox)
            .await
            .contains(thread.id.as_str())
    );
    assert_eq!(harness.store.flush().await.unwrap().completed, 1);
    for (_, labels) in harness.gmail_labels(&thread.id).await {
        assert!(labels.contains("TRASH") && !labels.contains("INBOX") && !labels.contains("SPAM"));
    }

    send(&harness, &thread.id, Mutation::Untrash).await;
    for (_, labels) in harness.gmail_labels(&thread.id).await {
        assert!(labels.contains("INBOX") && !labels.contains("TRASH"));
    }
    assert!(
        harness
            .ids(&ListId::Inbox)
            .await
            .contains(thread.id.as_str())
    );
}

#[tokio::test]
async fn spam_and_not_spam() {
    let harness = Harness::synced().await;
    let thread = inbox_thread_with_reply(&harness).await;

    harness
        .store
        .apply(std::slice::from_ref(&thread.id), Mutation::Spam)
        .await
        .unwrap();
    // Every message is spam now, so the thread is only in Spam.
    assert!(
        harness
            .ids(&ListId::Spam)
            .await
            .contains(thread.id.as_str())
    );
    assert!(
        !harness
            .ids(&ListId::Done)
            .await
            .contains(thread.id.as_str())
    );
    assert_eq!(harness.store.flush().await.unwrap().completed, 1);
    for (_, labels) in harness.gmail_labels(&thread.id).await {
        assert!(labels.contains("SPAM") && !labels.contains("INBOX"));
    }

    send(&harness, &thread.id, Mutation::NotSpam).await;
    for (_, labels) in harness.gmail_labels(&thread.id).await {
        assert!(labels.contains("INBOX") && !labels.contains("SPAM"));
    }
}

#[tokio::test]
async fn labels_go_on_the_whole_thread() {
    let harness = Harness::synced().await;
    let thread = inbox_thread_with_reply(&harness).await;
    let travel = harness
        .store
        .labels()
        .await
        .unwrap()
        .into_iter()
        .find(|label| label.name == "Travel")
        .unwrap();

    send(&harness, &thread.id, Mutation::AddLabel(travel.id.clone())).await;
    assert!(
        harness
            .gmail_labels(&thread.id)
            .await
            .iter()
            .all(|(_, l)| l.contains(&travel.id))
    );
    let list = ListId::Label(travel.id.clone());
    assert!(harness.ids(&list).await.contains(thread.id.as_str()));

    send(
        &harness,
        &thread.id,
        Mutation::RemoveLabel(travel.id.clone()),
    )
    .await;
    assert!(
        harness
            .gmail_labels(&thread.id)
            .await
            .iter()
            .all(|(_, l)| !l.contains(&travel.id))
    );
    assert!(!harness.ids(&list).await.contains(thread.id.as_str()));
}

#[tokio::test]
async fn muted_threads_stay_out_of_the_inbox() {
    let harness = Harness::synced().await;
    let delivered = harness
        .deliver(
            json!({ "from": "Ana <ana@example.com>", "subject": "Weekly digest", "text": "News" }),
        )
        .await;
    harness.store.sync().await.unwrap();
    let thread = ThreadId::new(delivered.thread_id.unwrap());
    let muted = harness
        .store
        .labels()
        .await
        .unwrap()
        .into_iter()
        .find(|label| label.name == "[Exo]/Muted")
        .unwrap();
    assert!(muted.hidden);

    send(&harness, &thread, Mutation::Mute).await;
    for (_, labels) in harness.gmail_labels(&thread).await {
        assert!(labels.contains(&muted.id) && !labels.contains("INBOX"));
    }
    assert!(harness.ids(&ListId::Muted).await.contains(thread.as_str()));

    // A reply lands in Gmail's inbox, but stays out of Exo's: muting is Exo's rule, not Gmail's.
    harness
        .deliver(
            json!({ "from": "Ana <ana@example.com>", "threadId": thread.as_str(), "text": "Ping" }),
        )
        .await;
    harness.store.sync().await.unwrap();
    assert!(
        harness
            .gmail_labels(&thread)
            .await
            .iter()
            .any(|(_, l)| l.contains("INBOX"))
    );
    assert!(!harness.ids(&ListId::Inbox).await.contains(thread.as_str()));
    assert!(
        harness
            .summary(&ListId::Muted, &thread)
            .await
            .unwrap()
            .muted
    );

    send(&harness, &thread, Mutation::Unmute).await;
    for (_, labels) in harness.gmail_labels(&thread).await {
        assert!(labels.contains("INBOX") && !labels.contains(&muted.id));
    }
    assert!(harness.ids(&ListId::Inbox).await.contains(thread.as_str()));
}

#[tokio::test]
async fn threads_already_there_are_left_alone() {
    let harness = Harness::synced().await;
    let thread = inbox_thread_with_reply(&harness).await;
    let done = harness.ids(&ListId::Done).await.into_iter().next().unwrap();
    let undo = harness
        .store
        .apply(
            &[thread.id.clone(), ThreadId::new(done)],
            Mutation::MarkDone,
        )
        .await
        .unwrap();
    assert!(undo.threads().eq([thread.id.clone()]));
    assert!(
        harness
            .store
            .apply(std::slice::from_ref(&thread.id), Mutation::MarkDone)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(harness.store.pending().await.unwrap(), 1);

    assert!(matches!(
        harness
            .store
            .apply(&[ThreadId::new("ffff")], Mutation::Star)
            .await,
        Err(exo_store::Error::UnknownThread(_))
    ));
}

#[tokio::test]
async fn each_thread_queue_runs_in_order() {
    let harness = Harness::synced().await;
    let thread = inbox_thread_with_reply(&harness).await;
    harness
        .store
        .apply(std::slice::from_ref(&thread.id), Mutation::Star)
        .await
        .unwrap();
    harness
        .store
        .apply(std::slice::from_ref(&thread.id), Mutation::Unstar)
        .await
        .unwrap();
    harness
        .store
        .apply(std::slice::from_ref(&thread.id), Mutation::MarkDone)
        .await
        .unwrap();
    assert_eq!(harness.store.pending().await.unwrap(), 3);
    assert_eq!(harness.store.flush().await.unwrap().completed, 3);
    for (_, labels) in harness.gmail_labels(&thread.id).await {
        assert!(!labels.contains("STARRED") && !labels.contains("INBOX"));
    }
}

/// A row's `removed_by` names exactly the mutations that take it out of its list, so the app can
/// slide it out on the keypress and the store then agrees.
#[tokio::test]
async fn removed_by_is_what_takes_a_row_out_of_its_list() {
    use Mutation::*;
    let harness = Harness::synced().await;
    let user = harness
        .store
        .labels()
        .await
        .unwrap()
        .into_iter()
        .filter(|label| label.kind == LabelKind::User)
        .map(|label| label.id)
        .collect::<Vec<_>>();
    let inbox = harness.all(&ListId::Inbox).await;
    let label = inbox
        .iter()
        .flat_map(|thread| &thread.labels)
        .find(|id| user.contains(id))
        .unwrap()
        .clone();
    // Something to look at in Muted.
    let muted = inbox_thread_with_reply(&harness).await;
    harness.store.apply(&[muted.id], Mute).await.unwrap();
    let lists = [
        ListId::Inbox,
        ListId::Done,
        ListId::Starred,
        ListId::Drafts,
        ListId::Spam,
        ListId::Trash,
        ListId::Muted,
        ListId::Sent,
        ListId::Label(label.clone()),
    ];
    for list in lists {
        harness.store.backfill(&list, 20).await.unwrap();
        let page = harness.store.list(&list, None, 4).await.unwrap();
        assert!(!page.threads.is_empty(), "{list:?} has threads");
        for thread in page.threads {
            let mutations = [
                MarkDone,
                MarkNotDone,
                Star,
                Unstar,
                MarkRead,
                MarkUnread,
                Trash,
                Untrash,
                Spam,
                NotSpam,
                Mute,
                Unmute,
                AddLabel(label.clone()),
                RemoveLabel(label.clone()),
            ];
            for mutation in mutations {
                let undo = harness
                    .store
                    .apply(std::slice::from_ref(&thread.id), mutation.clone())
                    .await
                    .unwrap();
                let listed = harness.ids(&list).await.contains(thread.id.as_str());
                harness.store.undo(undo).await.unwrap();
                assert_eq!(
                    thread.removed_by.contains(&mutation),
                    !listed,
                    "{mutation:?} in {list:?}: {}",
                    thread.subject
                );
            }
        }
    }
}

#[tokio::test]
async fn create_a_label_then_move_a_thread_into_it() {
    let harness = Harness::synced().await;
    let label = harness.store.create_label("Receipts/2026").await.unwrap();
    assert_eq!(label.name, "Receipts/2026");
    assert!(
        harness
            .store
            .labels()
            .await
            .unwrap()
            .iter()
            .any(|stored| stored.id == label.id && stored.kind == LabelKind::User)
    );
    // Gmail refuses a name that's taken; nothing is stored.
    assert!(harness.store.create_label("Receipts/2026").await.is_err());

    let thread = inbox_thread_with_reply(&harness).await;
    let mutation = Mutation::Move {
        from: Some("INBOX".into()),
        to: label.id.clone(),
    };
    send(&harness, &thread.id, mutation).await;
    let labels = harness.gmail_labels(&thread.id).await;
    assert!(
        labels
            .iter()
            .all(|(_, l)| l.contains(&label.id) && !l.contains("INBOX"))
    );
    assert!(
        !harness
            .ids(&ListId::Inbox)
            .await
            .contains(thread.id.as_str())
    );
    assert!(
        harness
            .ids(&ListId::Label(label.id))
            .await
            .contains(thread.id.as_str())
    );
}

#[tokio::test]
async fn removing_every_label_marks_done() {
    let harness = Harness::synced().await;
    let thread = inbox_thread_with_reply(&harness).await;
    let labels = harness.store.labels().await.unwrap();
    let user = labels
        .iter()
        .find(|label| label.kind == LabelKind::User && !label.hidden)
        .unwrap();
    send(&harness, &thread.id, Mutation::AddLabel(user.id.clone())).await;
    send(
        &harness,
        &thread.id,
        Mutation::RemoveLabels(vec![user.id.clone(), "INBOX".into()]),
    )
    .await;
    for (_, labels) in harness.gmail_labels(&thread.id).await {
        assert!(!labels.contains(&user.id) && !labels.contains("INBOX"));
    }
}
