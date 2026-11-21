//! Split inboxes: saving a split routes inbox threads into it and out of Important and Other,
//! and its list backfills from Gmail.

mod support;

use std::collections::BTreeSet;

use exo_store::{Definitions, ListId, Settings, SplitChanges, SplitInbox, ThreadId};
use gmail::ListQuery;
use support::{ACCOUNT, Harness};

fn query(q: &str) -> ListQuery {
    ListQuery {
        q: Some(q.to_owned()),
        ..ListQuery::default()
    }
}

/// The domain most inbox threads' contacts write from, so the split takes a few of them.
async fn busiest_domain(harness: &Harness) -> String {
    let mut domains: Vec<String> = harness
        .all(&ListId::Inbox)
        .await
        .iter()
        .filter_map(|thread| thread.contact.as_ref())
        .filter_map(|contact| {
            contact
                .email
                .rsplit_once('@')
                .map(|(_, domain)| domain.to_lowercase())
        })
        .collect();
    domains.sort();
    let mut best = (0, String::new());
    for domain in &domains {
        let count = domains.iter().filter(|other| *other == domain).count();
        if count > best.0 {
            best = (count, domain.clone());
        }
    }
    best.1
}

/// Inbox threads with a message (not a draft) from `domain`. A split matches `in:inbox` on the
/// thread and `from:` on each message, so a reply you sent from that domain counts, where
/// Gmail's search (one message matching everything) wouldn't.
async fn from_domain(harness: &Harness, domain: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for id in harness.gmail_ids(query("in:inbox")).await {
        let thread = harness
            .store
            .thread(&ThreadId::new(id.clone()))
            .await
            .unwrap()
            .expect("stored");
        let from = |message: &exo_store::MessageDetail| {
            message
                .from
                .as_ref()
                .is_some_and(|from| from.email.to_lowercase().ends_with(domain))
        };
        if thread
            .messages
            .iter()
            .any(|message| !message.labels.iter().any(|label| label == "DRAFT") && from(message))
        {
            found.insert(id);
        }
    }
    found
}

async fn save(harness: &Harness, splits: Vec<SplitInbox>) {
    let mut settings = Settings::default();
    for split in splits {
        settings.add_split(split);
    }
    harness.store.save_settings(&settings).await.expect("saves");
}

#[tokio::test]
async fn a_split_takes_its_threads_out_of_important_and_other() {
    let harness = Harness::synced().await;
    let labels = harness.store.labels().await.unwrap();
    let me = [ACCOUNT.to_owned()];
    let definitions = Definitions {
        labels: &labels,
        me: &me,
    };
    let inbox = harness.ids(&ListId::Inbox).await;
    let domain = busiest_domain(&harness).await;
    let split = SplitInbox::new("Busy", &format!("from:{domain}"), None, None, definitions);
    let list = ListId::Split(split.id.clone());

    save(&harness, vec![split.clone()]).await;
    let taken = harness.ids(&list).await;
    let expected = from_domain(&harness, &domain).await;
    assert!(!taken.is_empty());
    assert_eq!(
        taken, expected,
        "the split holds the inbox threads its definition matches"
    );
    assert_eq!(
        harness.ids(&ListId::Inbox).await,
        &inbox - &taken,
        "and the Inbox doesn't"
    );
    let count = harness.store.count(&list).await.unwrap();
    assert_eq!(count.threads, taken.len());

    // Left in Important and Other, the threads show in both places.
    let leave = split.update(
        &SplitChanges {
            leave_threads_in_important_other: Some(true),
            ..SplitChanges::default()
        },
        definitions,
    );
    assert_ne!(leave.id, split.id, "a routing change mints a new id");
    save(&harness, vec![leave.clone()]).await;
    assert_eq!(harness.ids(&ListId::Inbox).await, inbox);
    assert_eq!(harness.ids(&ListId::Split(leave.id.clone())).await, taken);
    assert!(
        harness.ids(&list).await.is_empty(),
        "the old id's list is gone"
    );

    // Turned off, it takes nothing.
    let off = leave.update(
        &SplitChanges {
            is_disabled: Some(true),
            ..SplitChanges::default()
        },
        definitions,
    );
    save(&harness, vec![off.clone()]).await;
    assert_eq!(harness.ids(&ListId::Inbox).await, inbox);
    assert!(harness.ids(&ListId::Split(off.id)).await.is_empty());
}

#[tokio::test]
async fn new_mail_and_triage_route_through_splits() {
    let harness = Harness::synced().await;
    let labels = harness.store.labels().await.unwrap();
    let me = [ACCOUNT.to_owned()];
    let definitions = Definitions {
        labels: &labels,
        me: &me,
    };
    let domain = busiest_domain(&harness).await;
    let split = SplitInbox::new("Busy", &format!("from:{domain}"), None, None, definitions);
    let list = ListId::Split(split.id.clone());
    save(&harness, vec![split]).await;

    // Marking a split's thread done takes it out of the split, as the row's slide says.
    let thread = harness
        .all(&list)
        .await
        .into_iter()
        .next()
        .expect("the split has threads");
    assert!(thread.removed_by.contains(&exo_store::Mutation::MarkDone));
    harness
        .store
        .apply(
            std::slice::from_ref(&thread.id),
            exo_store::Mutation::MarkDone,
        )
        .await
        .unwrap();
    assert!(!harness.ids(&list).await.contains(thread.id.as_str()));
    harness.store.flush().await.unwrap();
    harness.sync_all().await;
    assert!(!harness.ids(&list).await.contains(thread.id.as_str()));
}

#[tokio::test]
async fn a_split_list_backfills_with_its_gmail_search() {
    let harness = Harness::synced().await;
    let labels = harness.store.labels().await.unwrap();
    let me = [ACCOUNT.to_owned()];
    let domain = busiest_domain(&harness).await;
    let split = SplitInbox::new(
        "Busy",
        &format!("from:{domain}"),
        None,
        None,
        Definitions {
            labels: &labels,
            me: &me,
        },
    );
    let list = ListId::Split(split.id.clone());
    save(&harness, vec![split]).await;
    harness.sync_all().await;
    let page = harness.store.list(&list, None, 500).await.unwrap();
    assert!(!page.more_remote, "the split's own pass reached the end");
    let ids: BTreeSet<String> = page
        .threads
        .iter()
        .map(|thread| thread.id.as_str().to_owned())
        .collect();
    assert_eq!(ids, from_domain(&harness, &domain).await);
}
