//! Rows first, bodies after: sync stores threads for their rows (`format=metadata`, with
//! messages that may hold files or invites in full), prefetches the inbox's bodies, and an
//! opened thread fetches what it lacks.

mod support;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use exo_store::{ListId, Mutation, Options, Quota, Search, ThreadId, ThreadSummary};
use gmail::Format;
use gmail_mock::Config;
use serde_json::{Value, json};
use support::Harness;

/// Every list a row shows in, with the fixtures' user labels.
async fn surfaces(harness: &Harness) -> Vec<ListId> {
    let mut lists = vec![
        ListId::Inbox,
        ListId::Important,
        ListId::Other,
        ListId::Done,
        ListId::Starred,
        ListId::Sent,
        ListId::Drafts,
        ListId::Spam,
        ListId::Trash,
        ListId::Muted,
        ListId::All,
    ];
    let labels = harness.store.labels().await.expect("labels");
    lists.extend(
        labels
            .into_iter()
            .filter(|label| label.id.starts_with("Label_"))
            .map(|label| ListId::Label(label.id)),
    );
    lists
}

/// Each surface's rows, in order.
async fn rows(harness: &Harness, lists: &[ListId]) -> BTreeMap<String, Vec<ThreadSummary>> {
    let mut rows = BTreeMap::new();
    for list in lists {
        rows.insert(format!("{list:?}"), harness.all(list).await);
    }
    rows
}

async fn units(harness: &Harness) -> u64 {
    harness
        .admin(reqwest::Method::GET, "/__mock/quota", Value::Null)
        .await["units"]
        .as_u64()
        .expect("units")
}

/// A thread stored for its rows alone, out of the inbox.
async fn archived_without_bodies(harness: &Harness) -> ThreadId {
    for thread in harness.all(&ListId::Done).await {
        if !harness
            .store
            .bodies_stored(&thread.id)
            .await
            .expect("reads")
        {
            return thread.id;
        }
    }
    panic!("every Done thread has its bodies");
}

#[tokio::test]
async fn rows_read_the_same_as_threads_fetched_in_full() {
    let harness = Harness::synced().await;
    let lists = surfaces(&harness).await;
    let before = rows(&harness, &lists).await;
    let contact = harness
        .store
        .recent_threads("emekao@gmail.com", 5)
        .await
        .expect("conversations");
    let queries = [
        "has:attachment",
        "from:emekao@gmail.com",
        "filename:pdf",
        "in:sent",
        "label:family",
    ];
    let mut found = Vec::new();
    for query in queries {
        let mut search = Search::new(query);
        found.push(
            harness
                .store
                .search(&mut search, 100, false, 15)
                .await
                .expect("search"),
        );
    }
    let stored: Vec<ThreadId> = before
        .values()
        .flatten()
        .map(|thread| thread.id.clone())
        .collect();
    let mut rows_only = 0;
    for thread in &stored {
        rows_only += usize::from(!harness.store.bodies_stored(thread).await.expect("reads"));
        harness.store.load_bodies(thread).await.expect("loads");
        assert!(harness.store.bodies_stored(thread).await.expect("reads"));
    }
    assert!(
        rows_only > 10,
        "most archived mail is stored as rows ({rows_only})"
    );

    assert_eq!(
        rows(&harness, &lists).await,
        before,
        "every list's rows, fetched in full"
    );
    assert_eq!(
        harness
            .store
            .recent_threads("emekao@gmail.com", 5)
            .await
            .expect("conversations"),
        contact
    );
    for (query, before) in queries.iter().zip(found) {
        let mut search = Search::new(query);
        let after = harness
            .store
            .search(&mut search, 100, false, 15)
            .await
            .expect("search");
        let ids = |page: &exo_store::SearchPage| {
            page.results
                .iter()
                .map(|result| result.thread.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&after), ids(&before), "{query}");
    }
}

#[tokio::test]
async fn an_outlook_invite_shows_its_icon_unopened() {
    let harness = Harness::synced().await;
    // Exchange's invite: `text/calendar` as the third alternative, no file.
    let invite = gmail::mime::Email {
        from: Some(gmail::mime::Mailbox { name: Some("Dana Whitfield".into()), email: "dana@contoso.example".into() }),
        to: vec![gmail::mime::Mailbox { name: None, email: support::ACCOUNT.into() }],
        subject: "Quarterly review".into(),
        text: Some("When: Thursday 10:00".into()),
        html: Some("<p>When: Thursday 10:00</p>".into()),
        calendar: Some("BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:q-review\r\nSUMMARY:Quarterly review\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n".into()),
        headers: vec![("Content-class".into(), "urn:content-classes:calendarmessage".into())],
        ..gmail::mime::Email::default()
    };
    // Out of the inbox, so no prefetch fetches its body.
    let raw = gmail::wire::encode_base64url(&invite.to_rfc822());
    let delivered = harness
        .deliver(json!({ "raw": raw, "labelIds": ["CATEGORY_UPDATES"] }))
        .await;
    harness.sync_all().await;
    let thread = ThreadId::new(delivered.thread_id.expect("a thread"));
    let row = harness
        .summary(&ListId::All, &thread)
        .await
        .expect("listed");
    assert_eq!(
        row.attachments,
        exo_store::Attachments::CalendarInvite,
        "fetched in full for its row, as a message with files is"
    );
}

#[tokio::test]
async fn archived_mail_comes_without_bodies_until_opened() {
    let harness = Harness::synced().await;
    let thread = archived_without_bodies(&harness).await;
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
            .all(|message| message.body.html.is_none() && message.body.text.is_none())
    );
    assert!(
        detail
            .messages
            .iter()
            .all(|message| message.from.is_some() && !message.subject.is_empty()),
        "the row's headers are there"
    );

    let spent = units(&harness).await;
    harness.store.load_bodies(&thread).await.expect("loads");
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
            .all(|message| message.body.html.is_some() || message.body.text.is_some())
    );
    assert!(
        detail
            .messages
            .iter()
            .all(|message| message.headers.len() > 3),
        "every header, not just the row's"
    );
    let cost = units(&harness).await - spent;
    assert_eq!(
        cost,
        if detail.messages.len() == 1 { 60 } else { 120 },
        "one message alone, else the thread"
    );

    harness.store.load_bodies(&thread).await.expect("loads");
    assert_eq!(units(&harness).await - spent, cost, "nothing more to fetch");
}

#[tokio::test]
async fn the_inbox_gets_its_bodies_in_the_background() {
    let harness = Harness::synced().await;
    let inbox = harness.all(&ListId::Inbox).await;
    assert!(!inbox.is_empty());
    for thread in &inbox {
        assert!(
            harness
                .store
                .bodies_stored(&thread.id)
                .await
                .expect("reads"),
            "{}",
            thread.subject
        );
    }
}

#[tokio::test]
async fn a_body_arriving_reaches_the_search_index() {
    let harness = Harness::synced().await;
    let thread = archived_without_bodies(&harness).await;
    let fetched = harness
        .gmail
        .gmail()
        .thread(thread.as_str(), Format::Full, &[])
        .await
        .expect("threads.get");
    let text: String = fetched
        .messages
        .iter()
        .filter_map(|message| message.payload.as_ref())
        .flat_map(|payload| payload.walk())
        .filter(|part| part.mime_type.as_deref() == Some("text/plain"))
        .filter_map(|part| part.body.as_ref()?.data.as_deref())
        .map(|data| String::from_utf8_lossy(data).into_owned())
        .collect();
    let finds = |word: String| {
        let (store, thread) = (harness.store.clone(), thread.clone());
        async move {
            let mut search = Search::new(&word);
            let page = store
                .search(&mut search, 100, false, 15)
                .await
                .expect("search");
            page.results.iter().any(|result| result.thread.id == thread)
        }
    };
    // The longest word of the body the row's headers don't hold.
    let mut words: Vec<&str> = text
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|word| word.len() >= 6)
        .collect();
    words.sort_by_key(|word| std::cmp::Reverse(word.len()));
    let mut word = None;
    for candidate in words {
        if !finds(candidate.to_owned()).await {
            word = Some(candidate.to_owned());
            break;
        }
    }
    let word = word.expect("a word only the body has");
    harness.store.load_bodies(&thread).await.expect("loads");
    assert!(
        finds(word.clone()).await,
        "{word:?}: indexed once the body is in"
    );
}

#[tokio::test]
async fn a_triage_refreshes_the_thread_for_its_rows() {
    let harness = Harness::synced().await;
    let thread = harness
        .all(&ListId::Inbox)
        .await
        .into_iter()
        .find(|thread| !thread.starred)
        .expect("an unstarred thread");
    let spent = units(&harness).await;
    harness
        .store
        .apply(std::slice::from_ref(&thread.id), Mutation::Star)
        .await
        .expect("applies");
    assert_eq!(harness.store.flush().await.expect("flushes").completed, 1);
    // `messages.modify` and the refresh's `threads.get` metadata, nothing in full.
    assert_eq!(units(&harness).await - spent, 5 + 40);
    assert!(
        harness
            .store
            .bodies_stored(&thread.id)
            .await
            .expect("reads"),
        "the bodies stay"
    );
    assert!(
        harness
            .summary(&ListId::Inbox, &thread.id)
            .await
            .expect("listed")
            .starred
    );
}

#[tokio::test]
async fn new_mail_brings_its_body_when_it_lands_in_the_inbox() {
    let harness = Harness::synced().await;
    let deliver = |subject: &str, labels: &[&str]| json!({ "from": "Dana Ruiz <dana@harborlight.example>", "subject": subject, "text": "Shipping manifests attached below.", "labelIds": labels });
    let inbox = harness
        .deliver(deliver(
            "Manifests for Q4",
            &["INBOX", "UNREAD", "CATEGORY_PERSONAL"],
        ))
        .await;
    let archived = harness
        .deliver(deliver("Manifests, filed", &["CATEGORY_UPDATES"]))
        .await;
    harness.store.sync().await.expect("syncs");
    let id = |message: &gmail::Message| ThreadId::new(message.thread_id.clone().expect("a thread"));
    assert!(harness.summary(&ListId::Inbox, &id(&inbox)).await.is_some());
    assert!(
        harness
            .store
            .bodies_stored(&id(&inbox))
            .await
            .expect("reads"),
        "prefetched in the same round"
    );
    assert!(
        harness
            .summary(&ListId::All, &id(&archived))
            .await
            .is_some(),
        "its row is stored"
    );
    assert!(
        !harness
            .store
            .bodies_stored(&id(&archived))
            .await
            .expect("reads"),
        "its body waits to be opened"
    );
}

#[tokio::test]
async fn opening_a_thread_goes_ahead_of_the_backfill() {
    let per_minute = 6_000;
    let config = Config {
        quota: Some(per_minute),
        ..Config::default()
    };
    let options = Options {
        quota: Some(Quota { per_minute }),
        ..support::options()
    };
    let harness = Harness::start_with(config, options).await;
    let store = harness.store.clone();
    let sync =
        tokio::spawn(
            async move { while store.sync().await.is_ok_and(|report| report.backfilling) {} },
        );
    let thread = loop {
        let mut found = None;
        for thread in harness.all(&ListId::Done).await {
            if !harness
                .store
                .bodies_stored(&thread.id)
                .await
                .expect("reads")
            {
                found = Some(thread.id);
                break;
            }
        }
        if let Some(thread) = found {
            break thread;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let asked = Instant::now();
    harness.store.load_bodies(&thread).await.expect("loads");
    assert!(
        asked.elapsed() < Duration::from_millis(500),
        "opening waited {:?}",
        asked.elapsed()
    );
    assert!(!sync.is_finished(), "the backfill was still going");
    sync.abort();
}

#[tokio::test]
async fn the_first_hundred_of_each_inbox_list_get_their_bodies() {
    let harness = Harness::start().await;
    for at in 0..110 {
        let message = json!({ "from": format!("Notices <notices{at}@tracklight.example>"), "subject": format!("Shipment {at} cleared customs"), "text": "Your shipment cleared customs.", "labelIds": ["INBOX", "CATEGORY_UPDATES"] });
        harness.deliver(message).await;
    }
    harness.sync_all().await;
    let other = harness.all(&ListId::Other).await;
    assert!(other.len() > 110, "{}", other.len());
    let mut stored = Vec::new();
    for thread in &other {
        stored.push(
            harness
                .store
                .bodies_stored(&thread.id)
                .await
                .expect("reads"),
        );
    }
    assert!(
        stored[..100].iter().all(|stored| *stored),
        "the first hundred are ready to open"
    );
    assert!(
        !stored[100..].iter().all(|stored| *stored),
        "further down they wait to be opened"
    );

    let first: Vec<ThreadId> = other[..20].iter().map(|thread| thread.id.clone()).collect();
    harness
        .store
        .apply(&first, Mutation::MarkDone)
        .await
        .expect("applies");
    harness.store.flush().await.expect("flushes");
    harness.sync_all().await;
    let other = harness.all(&ListId::Other).await;
    assert!(other.len() <= 100, "{}", other.len());
    for thread in &other {
        assert!(
            harness
                .store
                .bodies_stored(&thread.id)
                .await
                .expect("reads"),
            "{} moved up",
            thread.subject
        );
    }
}

#[tokio::test]
async fn bodies_gmail_refuses_wait_to_be_opened() {
    let harness = Harness::synced().await;
    let message = json!({ "from": "Dana Ruiz <dana@harborlight.example>", "subject": "Dock schedule", "text": "Dock 4 from Monday.", "labelIds": ["INBOX", "UNREAD", "CATEGORY_PERSONAL"] });
    let delivered = harness.deliver(message).await;
    let path = format!(
        "/gmail/v1/users/me/messages/{}",
        delivered.id.clone().expect("an id")
    );
    harness
        .fault(json!({ "method": "GET", "path": path, "status": 400, "times": 100 }))
        .await;
    assert!(
        !harness.store.sync().await.expect("syncs").backfilling,
        "the refused thread doesn't keep the round going"
    );
    let thread = ThreadId::new(delivered.thread_id.clone().expect("a thread"));
    assert!(!harness.store.bodies_stored(&thread).await.expect("reads"));
    let spent = units(&harness).await;
    assert!(!harness.store.sync().await.expect("syncs").backfilling);
    assert_eq!(
        units(&harness).await - spent,
        2,
        "a history check, and no new try"
    );

    harness.clear_faults().await;
    harness
        .store
        .load_bodies(&thread)
        .await
        .expect("opening fetches them");
    assert!(harness.store.bodies_stored(&thread).await.expect("reads"));
}

#[tokio::test]
async fn every_row_the_list_on_screen_has_loaded_gets_its_bodies() {
    let harness = Harness::start().await;
    for at in 0..120 {
        let message = json!({ "from": format!("Notices <notices{at}@tracklight.example>"), "subject": format!("Shipment {at} cleared customs"), "text": "Your shipment cleared customs.", "labelIds": ["CATEGORY_UPDATES"] });
        harness.deliver(message).await;
    }
    harness.store.show(&ListId::Done);
    harness.sync_all().await;
    let done = harness.all(&ListId::Done).await;
    assert!(done.len() > 120, "{}", done.len());
    assert!(
        bodied(&harness, &done[100..120]).await < 20,
        "past the first hundred, not yet"
    );
    // The app has read 200 rows of it: all of them.
    harness.store.loaded(200);
    harness.sync_all().await;
    assert_eq!(bodied(&harness, &done[..120]).await, 120);
}

/// How many of `threads` have their bodies stored.
async fn bodied(harness: &Harness, threads: &[ThreadSummary]) -> usize {
    let mut bodied = 0;
    for thread in threads {
        bodied += usize::from(
            harness
                .store
                .bodies_stored(&thread.id)
                .await
                .expect("reads"),
        );
    }
    bodied
}
