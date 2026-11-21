//! The search index's text (`Store::index_text_with`, `Store::index_texts`): each message's
//! visible text, worked out by the app's function (here a stand-in for exo-mail's pipeline that
//! cuts a body at its quote), once its body is stored and the one it replies to is too.

mod support;

use std::sync::{Arc, Mutex};

use exo_store::{IndexText, ListId, Search, TextSource, ThreadId};
use serde_json::json;
use support::Harness;

/// What the stand-in was asked: each message, and the one it was read against.
type Asked = Arc<Mutex<Vec<(String, Option<String>)>>>;

/// A stand-in for exo-mail's pipeline: a body's text up to its quote (`On … wrote:`), each
/// message read against the one it replies to. It records what it was asked.
fn cut_at_quotes(asked: &Asked) -> IndexText {
    let asked = asked.clone();
    Arc::new(move |sources: &[TextSource]| {
        let mut asked = asked.lock().expect("lock");
        sources
            .iter()
            .map(|source| {
                asked.push((
                    source.id.clone(),
                    source.replying_to.map(|at| sources[at].id.clone()),
                ));
                let text = source.body.text.clone().unwrap_or_default();
                text.split("On Monday")
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect()
    })
}

async fn found(harness: &Harness, query: &str) -> Vec<ThreadId> {
    let page = harness
        .store
        .search(&mut Search::new(query), 50, false, 15)
        .await
        .expect("searches");
    page.results
        .into_iter()
        .map(|result| result.thread.id)
        .collect()
}

/// The message rows Gmail has of `thread`, oldest first.
async fn messages(harness: &Harness, thread: &ThreadId) -> Vec<String> {
    harness
        .gmail
        .gmail()
        .thread(thread.as_str(), gmail::Format::Minimal, &[])
        .await
        .expect("threads.get")
        .messages
        .into_iter()
        .filter_map(|message| message.id)
        .collect()
}

/// Moves the mock's clock a minute on, so the next message is dated after the last.
async fn a_minute_later(harness: &Harness) {
    let now: jiff::Timestamp = harness
        .admin(reqwest::Method::GET, "/__mock/clock", json!({}))
        .await["now"]
        .as_str()
        .expect("a time")
        .parse()
        .expect("a time");
    harness
        .admin(
            reqwest::Method::POST,
            "/__mock/clock",
            json!({ "now": (now + jiff::SignedDuration::from_mins(1)).to_string() }),
        )
        .await;
}

/// A conversation in the inbox (so sync fetches its bodies): Ann asks, Maya replies quoting her.
async fn conversation(harness: &Harness) -> ThreadId {
    let first = harness.deliver(json!({ "from": "Ann <ann@example.com>", "subject": "Lunch", "text": "Shall we try the boathouse on Friday?" })).await;
    let thread = first.thread_id.expect("a thread");
    a_minute_later(harness).await;
    harness.deliver(json!({ "from": "Ann <ann@example.com>", "threadId": thread, "text": "Noon works too.\n\nOn Monday Ann wrote:\n> Shall we try the boathouse? The walrus exhibit opens then." })).await;
    ThreadId::new(thread)
}

#[tokio::test]
async fn the_index_holds_what_shows_above_the_quote_once_worked_out() {
    let harness = Harness::synced().await;
    let asked = Asked::default();
    harness.store.index_text_with(cut_at_quotes(&asked));
    let thread = conversation(&harness).await;
    harness.sync_all().await;
    assert!(
        harness.store.bodies_stored(&thread).await.expect("reads"),
        "sync fetched the inbox's bodies"
    );
    assert!(
        found(&harness, "walrus").await.contains(&thread),
        "until the pass, the whole text"
    );
    assert!(harness.store.index_texts().await.expect("indexes") > 0);

    assert!(
        found(&harness, "noon").await.contains(&thread),
        "the reply's own words"
    );
    assert!(
        found(&harness, "boathouse").await.contains(&thread),
        "the first message's"
    );
    assert!(
        !found(&harness, "walrus").await.contains(&thread),
        "but not what the reply quotes"
    );
    let [first, reply] = messages(&harness, &thread)
        .await
        .try_into()
        .expect("two messages");
    let asked = asked.lock().expect("lock").clone();
    assert!(
        asked.contains(&(reply.clone(), Some(first.clone()))),
        "the reply is read against the message it answers: {asked:?}"
    );
    assert!(asked.contains(&(first, None)));

    // Local results stop where All Mail's backfill does online; offline they're all there. The
    // excerpt is the reply's, not its quote's.
    let page = harness
        .store
        .search(&mut Search::new("boathouse"), 50, false, 15)
        .await
        .expect("searches");
    let excerpt = page
        .results
        .iter()
        .find(|result| result.thread.id == thread)
        .and_then(|result| result.snippet.clone())
        .expect("an excerpt");
    assert!(
        excerpt
            .text
            .contains("Shall we try the boathouse on Friday"),
        "{excerpt:?}"
    );
}

#[tokio::test]
async fn without_the_function_the_index_holds_the_whole_text() {
    let harness = Harness::synced().await;
    let thread = conversation(&harness).await;
    harness.sync_all().await;
    assert_eq!(
        harness.store.index_texts().await.expect("indexes"),
        0,
        "nothing to work it out with"
    );
    assert!(
        found(&harness, "walrus").await.contains(&thread),
        "the quote is searchable"
    );

    // Given it later, the bodies stored since are worked out at the next pass.
    harness
        .store
        .index_text_with(cut_at_quotes(&Asked::default()));
    harness.store.index_texts().await.expect("indexes");
    assert!(!found(&harness, "walrus").await.contains(&thread));
}

#[tokio::test]
async fn a_reply_whose_quoted_message_has_no_body_waits_for_it() {
    let harness = Harness::synced().await;
    let asked = Asked::default();
    harness.store.index_text_with(cut_at_quotes(&asked));
    // Out of the inbox, sync stores the first message for its row alone.
    let first = harness.deliver(json!({ "from": "Ann <ann@example.com>", "subject": "Report", "text": "The boathouse report is attached.", "labelIds": ["CATEGORY_UPDATES"] })).await;
    let thread = ThreadId::new(first.thread_id.clone().expect("a thread"));
    harness.sync_all().await;
    harness.store.index_texts().await.expect("indexes");
    assert!(!harness.store.bodies_stored(&thread).await.expect("reads"));
    // The reply comes with its body, as new mail in a stored conversation does.
    a_minute_later(&harness).await;
    let reply = harness.deliver(json!({ "from": "Ann <ann@example.com>", "threadId": thread.as_str(), "text": "Noon works too.\n\nOn Monday Ann wrote:\n> The walrus report.", "labelIds": ["CATEGORY_UPDATES"] })).await;
    harness.sync_all().await;
    harness.store.index_texts().await.expect("indexes");
    assert_eq!(
        harness.store.bodiless(&thread).await.expect("reads"),
        [first.id.clone().expect("an id")]
    );
    let reply = reply.id.expect("an id");
    assert!(
        !asked
            .lock()
            .expect("lock")
            .iter()
            .any(|(id, _)| *id == reply),
        "not worked out without what it replies to"
    );
    assert!(
        found(&harness, "walrus").await.contains(&thread),
        "the whole text meanwhile"
    );

    // Opened, the first message's body comes, and the next pass works the reply out against it.
    harness.store.load_bodies(&thread).await.expect("fetches");
    harness.store.index_texts().await.expect("indexes");
    assert!(!found(&harness, "walrus").await.contains(&thread));
    assert!(
        found(&harness, "noon").await.contains(&thread)
            && found(&harness, "boathouse").await.contains(&thread)
    );
    assert_eq!(
        messages(&harness, &thread).await,
        [first.id.clone().expect("an id"), reply.clone()]
    );
    assert!(asked.lock().expect("lock").contains(&(reply, first.id)));
}

#[tokio::test]
async fn a_function_that_panics_leaves_the_whole_text_and_isnt_asked_again() {
    let harness = Harness::synced().await;
    let calls = Arc::new(Mutex::new(0));
    let counted = calls.clone();
    harness
        .store
        .index_text_with(Arc::new(move |_: &[TextSource]| {
            *counted.lock().expect("lock") += 1;
            panic!("a hostile body");
        }));
    let thread = conversation(&harness).await;
    harness.sync_all().await;
    harness.store.index_texts().await.expect("indexes");
    let asked = *calls.lock().expect("lock");
    assert!(asked > 0);
    assert!(
        found(&harness, "walrus").await.contains(&thread),
        "the whole text"
    );
    assert_eq!(
        harness.store.index_texts().await.expect("indexes"),
        0,
        "nothing is queued"
    );
    assert_eq!(
        *calls.lock().expect("lock"),
        asked,
        "nothing is asked twice"
    );
    assert!(!harness.all(&ListId::Inbox).await.is_empty());
}
