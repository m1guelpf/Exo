//! Search: the local index, Gmail's search on top, and suggestions from local mail.

mod support;

use exo_store::{Highlighted, ListId, Mutation, Search, SearchPage};
use gmail::ListQuery;
use support::Harness;

/// The window the first sync stores (All Mail's 31 days).
const WINDOW_MS: i64 = 31 * 24 * 60 * 60 * 1000;

async fn search(harness: &Harness, query: &str) -> SearchPage {
    let mut search = Search::new(query);
    harness
        .store
        .search(&mut search, 100, true, 15)
        .await
        .expect("search")
}

fn subjects(page: &SearchPage) -> Vec<&str> {
    page.results
        .iter()
        .map(|result| result.thread.subject.as_str())
        .collect()
}

fn bold(text: &Highlighted) -> Vec<&str> {
    text.bold
        .iter()
        .map(|range| &text.text[range.clone()])
        .collect()
}

#[tokio::test]
async fn the_index_finds_words_in_bodies_and_shows_them_in_bold() {
    let harness = Harness::synced().await;
    // Emeka's mother: "…can you make your sweet potato thing again?"
    let page = search(&harness, "Potato").await;
    let thanksgiving = page
        .results
        .iter()
        .find(|result| result.thread.subject.starts_with("Thanksgiving"))
        .expect("found locally");
    let snippet = thanksgiving.snippet.as_ref().expect("an excerpt");
    assert_eq!(bold(snippet), ["potato"], "{snippet:?}");
    assert!(thanksgiving.subject.is_none(), "the subject has no match");
    assert!(
        thanksgiving.thread.messages > 1,
        "the row counts its messages"
    );
    assert!(
        !thanksgiving.thread.removed_by.contains(&Mutation::MarkDone),
        "a plain search keeps done threads"
    );
    assert!(
        thanksgiving.thread.removed_by.contains(&Mutation::Trash),
        "but not trashed ones"
    );

    let page = search(&harness, "subject:thanksgiving").await;
    let subject = page.results[0]
        .subject
        .as_ref()
        .expect("a highlighted subject");
    assert_eq!(bold(subject), ["Thanksgiving"]);
}

#[tokio::test]
async fn phrases_are_checked_as_typed_and_highlighted_whole() {
    let harness = Harness::synced().await;
    let page = search(&harness, "\"sweet potato\"").await;
    let thanksgiving = page
        .results
        .iter()
        .find(|result| result.thread.subject.starts_with("Thanksgiving"))
        .expect("found locally");
    assert_eq!(
        bold(thanksgiving.snippet.as_ref().expect("an excerpt")),
        ["sweet potato"],
        "the phrase in bold, not its words"
    );
    // The index stems `potatoes` to `potato`: "sweet potato thing" matches it.
    let page = search(&harness, "\"potatoes thing\"").await;
    assert!(
        !subjects(&page)
            .iter()
            .any(|subject| subject.starts_with("Thanksgiving")),
        "{:?}",
        subjects(&page)
    );
}

#[tokio::test]
async fn operators_narrow_the_index() {
    let harness = Harness::synced().await;
    let from = search(&harness, "from:emekao@gmail.com").await;
    assert!(
        subjects(&from).contains(&"Thanksgiving 🦃"),
        "{:?}",
        subjects(&from)
    );
    assert_eq!(
        subjects(&search(&harness, "potato label:family").await),
        ["Thanksgiving 🦃"]
    );
    let not_family = search(&harness, "potato -label:family").await;
    assert!(not_family.results.is_empty(), "{:?}", subjects(&not_family));
    assert!(search(&harness, "potato in:spam").await.results.is_empty());
    let inbox = search(&harness, "in:inbox").await;
    assert!(!inbox.results.is_empty());
    assert!(
        inbox
            .results
            .iter()
            .all(|result| result.thread.removed_by.contains(&Mutation::MarkDone)),
        "Done takes a thread out of in:inbox"
    );
    let local = harness.ids(&ListId::Inbox).await;
    assert!(
        inbox
            .results
            .iter()
            .all(|result| local.contains(result.thread.id.as_str()))
    );
    let attachments = search(&harness, "has:attachment").await;
    assert!(
        !attachments.results.is_empty()
            && attachments
                .results
                .iter()
                .all(|result| result.thread.attachments != exo_store::Attachments::None)
    );
    let none = search(&harness, "zzqx").await;
    assert!(none.results.is_empty());
    let dates = search(&harness, "before:2000/01/01").await;
    assert!(dates.results.is_empty());
    let results = search(&harness, "-in:inbox").await.results;
    assert!(
        results
            .windows(2)
            .all(|pair| pair[0].thread.date >= pair[1].thread.date),
        "newest first"
    );
}

#[tokio::test]
async fn the_index_follows_triage() {
    let harness = Harness::synced().await;
    let before = search(&harness, "in:inbox").await;
    let first = before.results[0].thread.id.clone();
    harness
        .store
        .apply(std::slice::from_ref(&first), Mutation::MarkDone)
        .await
        .expect("applies");
    let after = search(&harness, "in:inbox").await;
    assert!(after.results.iter().all(|result| result.thread.id != first));
    assert!(
        search(&harness, "in:done")
            .await
            .results
            .iter()
            .any(|result| result.thread.id == first)
    );
}

#[tokio::test]
async fn gmail_finds_what_the_window_left_out() {
    let harness = Harness::synced().await;
    let query = "from:newsletter@freightsignals.com";
    let mut session = Search::new(query);
    let local = harness
        .store
        .search(&mut session, 200, true, 15)
        .await
        .expect("search");
    let gmail = harness
        .gmail_ids(ListQuery {
            q: Some(query.into()),
            ..ListQuery::default()
        })
        .await;
    assert!(
        local.results.len() < gmail.len(),
        "{} local of {}",
        local.results.len(),
        gmail.len()
    );
    let cutoff = jiff::Timestamp::now().as_millisecond() - WINDOW_MS;
    while !session.gmail_done() {
        let page = harness.store.search_gmail(&session).await.expect("gmail");
        session.add_gmail(page);
    }
    let merged = harness
        .store
        .search(&mut session, 200, true, 15)
        .await
        .expect("search");
    let found: std::collections::BTreeSet<String> = merged
        .results
        .iter()
        .map(|result| result.thread.id.as_str().to_owned())
        .collect();
    assert_eq!(found, gmail);
    assert!(
        merged
            .results
            .iter()
            .any(|result| result.thread.date.as_millisecond() < cutoff),
        "older than the window"
    );
    assert!(
        merged
            .results
            .windows(2)
            .all(|pair| pair[0].thread.date >= pair[1].thread.date)
    );
}

#[tokio::test]
async fn hostile_queries_are_only_searched_for() {
    let harness = Harness::synced().await;
    let inbox = harness.ids(&ListId::Inbox).await;
    for query in [
        "'); DROP TABLE threads; --",
        "\" OR 1=1 --",
        "NEAR(a b",
        "\"unterminated",
        "subject:\"a\"\"b\"",
        "from:\"x\" AND OR NOT",
        "label:\"); DELETE FROM search_fts; --",
        "* ^ :",
        "from:%_\\",
        "a:b:c in:\"",
        "after:'; --",
    ] {
        let page = search(&harness, query).await;
        assert!(page.results.len() <= 100, "{query}");
        let mut session = Search::new(query);
        harness
            .store
            .search_gmail(&session)
            .await
            .expect("Gmail is asked, not SQLite");
        session.add_gmail(exo_store::GmailPage::default());
    }
    assert_eq!(
        harness.ids(&ListId::Inbox).await,
        inbox,
        "nothing was dropped"
    );
    assert!(
        !search(&harness, "potato").await.results.is_empty(),
        "the index is intact"
    );
}

#[tokio::test]
async fn filters_narrow_a_list_and_keep_sticky_rows() {
    use exo_store::{Filter, ThreadId};
    let harness = Harness::synced().await;
    let inbox = harness
        .store
        .list(&ListId::Inbox, None, 500)
        .await
        .expect("list")
        .threads;
    let unread = harness
        .store
        .list_filtered(&ListId::Inbox, Some(Filter::Unread), &[], &[], None, 500)
        .await
        .expect("list")
        .threads;
    assert!(!unread.is_empty() && unread.len() < inbox.len());
    assert!(unread.iter().all(|thread| thread.unread));
    assert_eq!(
        unread.len(),
        inbox.iter().filter(|thread| thread.unread).count()
    );
    let starred = harness
        .store
        .list_filtered(&ListId::Inbox, Some(Filter::Starred), &[], &[], None, 500)
        .await
        .expect("list")
        .threads;
    assert!(starred.iter().all(|thread| thread.starred));
    // Paging through a filtered list sees each thread once.
    let first = harness
        .store
        .list_filtered(&ListId::Inbox, Some(Filter::Unread), &[], &[], None, 3)
        .await
        .expect("list");
    let rest = harness
        .store
        .list_filtered(
            &ListId::Inbox,
            Some(Filter::Unread),
            &[],
            &[],
            first.next.as_ref(),
            500,
        )
        .await
        .expect("list");
    let paged: Vec<&ThreadId> = first
        .threads
        .iter()
        .chain(&rest.threads)
        .map(|thread| &thread.id)
        .collect();
    assert_eq!(
        paged,
        unread.iter().map(|thread| &thread.id).collect::<Vec<_>>()
    );

    let read = unread[0].id.clone();
    harness
        .store
        .apply(std::slice::from_ref(&read), Mutation::MarkRead)
        .await
        .expect("applies");
    let gone = harness
        .store
        .list_filtered(&ListId::Inbox, Some(Filter::Unread), &[], &[], None, 500)
        .await
        .expect("list")
        .threads;
    assert!(gone.iter().all(|thread| thread.id != read));
    let kept = harness
        .store
        .list_filtered(
            &ListId::Inbox,
            Some(Filter::Unread),
            std::slice::from_ref(&read),
            &[],
            None,
            500,
        )
        .await
        .expect("list")
        .threads;
    assert!(
        kept.iter().any(|thread| thread.id == read),
        "a sticky row stays"
    );

    // A conversation unstarred in Starred stays where it was while it's held, even paging.
    let starred = harness
        .store
        .list(&ListId::Starred, None, 500)
        .await
        .expect("list")
        .threads;
    assert!(starred.len() > 2, "the fixtures star a few");
    let unstarred = starred[1].id.clone();
    harness
        .store
        .apply(std::slice::from_ref(&unstarred), Mutation::Unstar)
        .await
        .expect("applies");
    let ids = |threads: &[exo_store::ThreadSummary]| {
        threads
            .iter()
            .map(|thread| thread.id.clone())
            .collect::<Vec<_>>()
    };
    assert!(
        !ids(&harness
            .store
            .list(&ListId::Starred, None, 500)
            .await
            .expect("list")
            .threads)
        .contains(&unstarred)
    );
    let held = std::slice::from_ref(&unstarred);
    let kept = harness
        .store
        .list_filtered(&ListId::Starred, None, &[], held, None, 500)
        .await
        .expect("list")
        .threads;
    assert_eq!(ids(&kept), ids(&starred), "in its place");
    assert!(!kept[1].starred, "showing it unstarred");
    let first = harness
        .store
        .list_filtered(&ListId::Starred, None, &[], held, None, 1)
        .await
        .expect("list");
    let second = harness
        .store
        .list_filtered(&ListId::Starred, None, &[], held, first.next.as_ref(), 1)
        .await
        .expect("list");
    let rest = harness
        .store
        .list_filtered(&ListId::Starred, None, &[], held, second.next.as_ref(), 500)
        .await
        .expect("list");
    assert_eq!(
        [first.threads, second.threads, rest.threads]
            .concat()
            .iter()
            .map(|thread| thread.id.clone())
            .collect::<Vec<_>>(),
        ids(&starred)
    );

    // Awaiting a reply: Sent threads whose last message is the user's.
    let no_reply = harness
        .store
        .list_filtered(&ListId::All, Some(Filter::NoReply), &[], &[], None, 500)
        .await
        .expect("list")
        .threads;
    assert!(!no_reply.is_empty());
    let sent = harness.ids(&ListId::Sent).await;
    assert!(
        no_reply
            .iter()
            .all(|thread| sent.contains(thread.id.as_str()))
    );
}
