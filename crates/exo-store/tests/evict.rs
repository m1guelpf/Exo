//! Eviction (`Store::evict`): what a store accumulates, what goes, what never does, and the
//! file giving the space back.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use exo_store::{
    Address, Draft, DraftAction, DraftRecord, Eviction, ListId, Mutation, Search, SendLater,
    Settings, Theme, ThreadId,
};
use gmail::{Format, ListQuery};
use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};
use support::{ACCOUNT, Harness, options};

/// Eviction that lets go of everything it may.
const EVERYTHING: Eviction = Eviction {
    age: SignedDuration::ZERO,
    budget: 0,
};

/// Eviction by `budget` alone.
fn by_budget(budget: u64) -> Eviction {
    Eviction {
        age: SignedDuration::from_hours(24 * 365 * 100),
        budget,
    }
}

/// The store's database file in `dir`.
fn database(dir: &Path) -> PathBuf {
    dir.join(format!("{ACCOUNT}.sqlite3"))
}

/// A connection reading the store in `dir` beside it.
fn reader(dir: &Path) -> rusqlite::Connection {
    rusqlite::Connection::open_with_flags(database(dir), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .expect("opens the store")
}

/// `sql`'s rows on the store in `dir`, each value as text, read beside the store.
fn rows(dir: &Path, sql: &str) -> Vec<Vec<String>> {
    let db = reader(dir);
    let mut statement = db
        .prepare(sql)
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
    let columns = statement.column_count();
    statement
        .query_map([], |row| {
            (0..columns)
                .map(|at| {
                    Ok(match row.get_ref(at)? {
                        rusqlite::types::ValueRef::Null => "NULL".to_owned(),
                        rusqlite::types::ValueRef::Integer(value) => value.to_string(),
                        rusqlite::types::ValueRef::Real(value) => value.to_string(),
                        rusqlite::types::ValueRef::Text(text)
                        | rusqlite::types::ValueRef::Blob(text) => {
                            String::from_utf8_lossy(text).into_owned()
                        }
                    })
                })
                .collect()
        })
        .expect("reads")
        .map(|row| row.expect("a row"))
        .collect()
}

/// The conversations with a body stored, with their newest message's date and their bodies'
/// bytes.
fn bodied(dir: &Path) -> BTreeMap<String, (i64, u64)> {
    let db = reader(dir);
    let mut statement = db
        .prepare("SELECT m.thread_id, (SELECT max(internal_date) FROM messages n WHERE n.thread_id = m.thread_id), sum(coalesce(octet_length(b.html), 0) + coalesce(octet_length(b.text), 0)) FROM bodies b JOIN messages m ON m.id = b.message_id GROUP BY m.thread_id")
        .expect("prepares");
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                (
                    row.get(1)?,
                    u64::try_from(row.get::<_, i64>(2)?).unwrap_or_default(),
                ),
            ))
        })
        .expect("reads")
        .map(|row| row.expect("a row"))
        .collect()
}

/// The threads of lists `lists`.
fn listed(dir: &Path, lists: &[&str]) -> BTreeSet<String> {
    let lists = lists
        .iter()
        .map(|list| format!("'{list}'"))
        .collect::<Vec<_>>()
        .join(", ");
    rows(
        dir,
        &format!("SELECT DISTINCT thread_id FROM list_ids WHERE list_id IN ({lists})"),
    )
    .into_iter()
    .map(|row| row[0].clone())
    .collect()
}

/// Every table's rows but the mail cache's (what eviction lets go: conversations, their bodies,
/// lists and search index), sorted.
fn kept_tables(dir: &Path) -> BTreeMap<String, Vec<Vec<String>>> {
    const MAIL: &str = "'threads', 'messages', 'attachments', 'bodies', 'list_ids', 'participants', 'lists', 'text_due'";
    let tables = rows(
        dir,
        &format!(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'search_fts%' AND name NOT IN ({MAIL})"
        ),
    );
    tables
        .into_iter()
        .map(|row| {
            let table = row[0].clone();
            // But for the mark eviction leaves of what it let go of (`evicted_before`).
            let own = if table == "kv" {
                " WHERE key <> 'evicted_before'"
            } else {
                ""
            };
            let mut contents = rows(dir, &format!("SELECT * FROM \"{table}\"{own}"));
            contents.sort();
            (table, contents)
        })
        .collect()
}

/// The conversations stored.
fn stored(dir: &Path) -> BTreeSet<String> {
    rows(dir, "SELECT id FROM threads")
        .into_iter()
        .map(|row| row[0].clone())
        .collect()
}

/// Calls of Gmail method `method` the mock has seen.
async fn calls(harness: &Harness, method: &str) -> u64 {
    harness
        .admin(reqwest::Method::GET, "/__mock/quota", json!({}))
        .await["methods"][method]["calls"]
        .as_u64()
        .unwrap_or_default()
}

/// Backfills `list` as scrolling past its end does until it shows `rows` threads. Returns how
/// many pages Gmail listed.
async fn scroll_back(harness: &Harness, list: &ListId, rows: usize) -> usize {
    let before = calls(harness, "threads.list").await;
    while harness.all(list).await.len() < rows {
        assert!(
            harness.store.backfill(list, 10).await.expect("backfills"),
            "Gmail has more"
        );
    }
    usize::try_from(calls(harness, "threads.list").await - before).unwrap_or(usize::MAX)
}

/// Eviction of whatever has been quiet for a day.
const DAY: Eviction = Eviction {
    age: SignedDuration::from_hours(24),
    budget: u64::MAX,
};

/// Gmail quota units the mock has seen spent.
async fn units(harness: &Harness) -> u64 {
    harness
        .admin(reqwest::Method::GET, "/__mock/quota", json!({}))
        .await["units"]
        .as_u64()
        .expect("units")
}

/// A new message to Ann, as compose saves it, on `thread` if a reply.
fn draft(id: &str, thread: Option<&ThreadId>, send_later: Option<SendLater>) -> Draft {
    let email = gmail::mime::Email {
        from: Some(gmail::mime::Mailbox::new("Maya Okafor", ACCOUNT)),
        to: vec![gmail::mime::Mailbox::new("Ann", "ann@example.com")],
        subject: "Plan".into(),
        html: Some("<div>Here it is</div>".into()),
        text: Some("Here it is".into()),
        message_id: Some(format!("{id}@exo")),
        ..gmail::mime::Email::default()
    };
    let record = DraftRecord {
        action: DraftAction::Compose,
        from: Some(Address {
            name: Some("Maya Okafor".into()),
            email: ACCOUNT.into(),
        }),
        to: vec![Address {
            name: Some("Ann".into()),
            email: "ann@example.com".into(),
        }],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: "Plan".into(),
        body: "<div>Here it is</div>".into(),
        quoted_content: None,
        quoted_inlined: false,
        in_reply_to: None,
        in_reply_to_rfc822: None,
        references: Vec::new(),
        rfc822_id: format!("{id}@exo"),
        fingerprint: String::new(),
        attachments: Vec::new(),
        send_later,
        reminder: None,
    };
    Draft {
        id: id.into(),
        thread: thread.cloned(),
        adopted: None,
        record,
        raw: String::from_utf8(email.to_rfc822()).expect("ASCII"),
        complete: true,
        sending: false,
    }
}

/// Opens (fetches the bodies of) the first `n` conversations of Done without a draft. Returns
/// them, with the Gmail quota units each open spent.
async fn open_done(harness: &Harness, n: usize) -> Vec<(ThreadId, u64)> {
    let drafted = listed(harness.dir.path(), &["DRAFT"]);
    let done: Vec<ThreadId> = harness
        .all(&ListId::Done)
        .await
        .into_iter()
        .filter(|thread| !drafted.contains(thread.id.as_str()))
        .take(n)
        .map(|thread| thread.id)
        .collect();
    let mut opened = Vec::new();
    for id in done {
        let spent = units(harness).await;
        harness.store.load_bodies(&id).await.expect("fetches");
        let spent = units(harness).await - spent;
        assert!(harness.store.bodies_stored(&id).await.expect("reads"));
        opened.push((id, spent));
    }
    opened
}

#[tokio::test]
async fn quiet_conversations_go_and_scrolling_back_fetches_them_again() {
    let harness = Harness::synced().await;
    let (store, dir) = (&harness.store, harness.dir.path());
    let opened = open_done(&harness, 6).await;
    // Today's mail; the fixture's is days old.
    harness
        .admin(
            reqwest::Method::POST,
            "/__mock/clock",
            json!({ "now": Timestamp::now().to_string() }),
        )
        .await;
    let fresh = harness.deliver(json!({ "from": "Ann <ann@example.com>", "subject": "Fresh", "text": "From today", "labelIds": ["CATEGORY_PERSONAL"] })).await;
    let fresh = ThreadId::new(fresh.thread_id.expect("a thread"));
    harness.sync_all().await;
    store.load_bodies(&fresh).await.expect("fetches");
    let mut before = Vec::new();
    for (id, _) in &opened {
        before.push(store.thread(id).await.expect("reads").expect("stored"));
    }
    let inbox = listed(dir, &["INBOX"]);
    let ids = |threads: Vec<exo_store::ThreadSummary>| {
        threads
            .into_iter()
            .map(|thread| thread.id.as_str().to_owned())
            .collect::<Vec<_>>()
    };
    let (all, done) = (
        ids(harness.all(&ListId::All).await),
        ids(harness.all(&ListId::Done).await),
    );
    let all_stored = stored(dir);

    let evicted = store.evict(&by_budget(u64::MAX)).await.expect("evicts");
    assert_eq!(
        (evicted.rows, evicted.threads),
        (0, 0),
        "nothing is older than a century"
    );
    let evicted = store.evict(&DAY).await.expect("evicts");
    let left = stored(dir);
    let gone: BTreeSet<&String> = all_stored.difference(&left).collect();
    assert_eq!(evicted.rows, gone.len());
    assert!(
        opened
            .iter()
            .all(|(id, _)| gone.contains(&id.as_str().to_owned())),
        "the conversations opened a while ago went"
    );
    assert!(
        left.contains(fresh.as_str()) && store.bodies_stored(&fresh).await.expect("reads"),
        "today's stays, bodies and all"
    );
    assert!(
        inbox.iter().all(|id| left.contains(id)),
        "so does the inbox, whose passes list it"
    );

    // Each list shows what's left above the conversations that went, without a gap.
    let shown = ids(harness.all(&ListId::All).await);
    assert!(
        store
            .list(&ListId::All, None, 1000)
            .await
            .expect("lists")
            .more_remote,
        "Gmail has more"
    );
    assert_eq!(
        shown,
        all[..shown.len()],
        "the list as it was, down to where it now ends"
    );
    assert!(shown.iter().all(|id| left.contains(id)) && gone.iter().all(|id| !shown.contains(id)));

    // Scrolled back past it, All Mail fetches what went again, starting where what's stored ends.
    let pages = scroll_back(&harness, &ListId::All, all.len()).await;
    assert_eq!(
        ids(harness.all(&ListId::All).await)[..all.len()],
        all,
        "the list as it was"
    );
    assert!(
        pages <= gone.len() / 10 + 2,
        "{pages} pages of ten for {} conversations",
        gone.len()
    );

    // Done has no pass of its own: it starts where All Mail's ends too. Trash comes back with
    // All Mail's pass, which lists it, and not with Done's (`-in:INBOX`).
    store.evict(&DAY).await.expect("evicts");
    let listed = harness
        .gmail_ids(ListQuery {
            q: Some("-in:INBOX".into()),
            ..ListQuery::default()
        })
        .await;
    let done: Vec<String> = done
        .into_iter()
        .filter(|id| !gone.contains(id) || listed.contains(id))
        .collect();
    let pages = scroll_back(&harness, &ListId::Done, done.len()).await;
    assert_eq!(
        ids(harness.all(&ListId::Done).await)[..done.len()],
        done,
        "the list as it was"
    );
    let gone_from_done = done.iter().filter(|id| gone.contains(id)).count();
    assert!(
        pages <= gone_from_done / 10 + 2,
        "{pages} pages of ten for {gone_from_done} conversations"
    );

    // Fetched again for their rows, the conversations opened before open as cold ones, for the
    // quota of any (`messages.get` in full for one message, 60 units, `threads.get` for more,
    // 120), and are as they were.
    for ((id, cold), before) in opened.iter().zip(before) {
        assert!(!store.bodies_stored(id).await.expect("reads"));
        let spent = units(&harness).await;
        store.load_bodies(id).await.expect("fetches again");
        let spent = units(&harness).await - spent;
        assert_eq!(spent, *cold, "{id}");
        assert_eq!(
            spent,
            if before.messages.len() == 1 { 60 } else { 120 },
            "{id}"
        );
        assert_eq!(
            store.thread(id).await.expect("reads").expect("stored"),
            before
        );
    }
}

#[tokio::test]
async fn a_quiet_conversation_something_keeps_loses_its_bodies_and_keeps_its_rows() {
    let harness = Harness::synced().await;
    let (store, dir) = (&harness.store, harness.dir.path());
    let [starred, reminded]: [ThreadId; 2] = open_done(&harness, 2)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect::<Vec<_>>()
        .try_into()
        .expect("two");
    // A change Gmail hasn't had, and a reminder.
    store
        .apply(std::slice::from_ref(&starred), Mutation::Star)
        .await
        .expect("stars");
    let at = Timestamp::now() + SignedDuration::from_hours(48);
    store
        .apply(
            std::slice::from_ref(&reminded),
            Mutation::Remind {
                at,
                keep_on_reply: false,
                snooze: true,
            },
        )
        .await
        .expect("reminds");
    let rows_of = |id: &ThreadId| {
        rows(
            dir,
            &format!(
                "SELECT id, label_ids, snippet FROM messages WHERE thread_id = '{}' ORDER BY id",
                id.as_str()
            ),
        )
    };
    let before = [&starred, &reminded].map(rows_of);

    // On screen, Done's loaded rows stay, bodies and all, as sync fetches them.
    let done = harness.all(&ListId::Done).await;
    store.show(&ListId::Done);
    store.loaded(done.len());
    store.evict(&DAY).await.expect("evicts");
    let left = stored(dir);
    assert!(done.iter().all(|thread| left.contains(thread.id.as_str())));
    assert!(
        store.bodies_stored(&starred).await.expect("reads")
            && store.bodies_stored(&reminded).await.expect("reads")
    );

    // Off screen, the rest of Done goes, but for the conversations with a change or a reminder:
    // they keep their rows and need no bodies.
    store.show(&ListId::Inbox);
    store.evict(&DAY).await.expect("evicts");
    let left = stored(dir);
    assert!(done.iter().any(|thread| !left.contains(thread.id.as_str())));
    for (id, rows) in [&starred, &reminded].into_iter().zip(&before) {
        assert!(left.contains(id.as_str()), "{id} stays");
        assert_eq!(&rows_of(id), rows, "{id}'s rows as they were");
        assert!(
            !store.bodies_stored(id).await.expect("reads"),
            "{id}'s bodies went"
        );
    }
}

#[tokio::test]
async fn past_the_budget_the_oldest_conversations_go_first() {
    let harness = Harness::synced().await;
    let store = &harness.store;
    open_done(&harness, 40).await;
    let dir = harness.dir.path();
    let before = bodied(dir);
    let total: u64 = before.values().map(|(_, bytes)| bytes).sum();
    assert_eq!(
        store
            .evict(&by_budget(total))
            .await
            .expect("evicts")
            .threads,
        0,
        "they fit"
    );

    // The inbox's and the drafts' stay whatever the budget: half of the rest goes.
    let kept = listed(
        dir,
        &["INBOX", "SH_IMPORTANT", "SH_OTHER", "DRAFT", "SH_SCHEDULED"],
    );
    let kept_bytes: u64 = before
        .iter()
        .filter(|(id, _)| kept.contains(*id))
        .map(|(_, (_, bytes))| bytes)
        .sum();
    let budget = kept_bytes + (total - kept_bytes) / 2;
    let evicted = store.evict(&by_budget(budget)).await.expect("evicts");
    let after = bodied(dir);
    let left: u64 = after.values().map(|(_, bytes)| bytes).sum();
    let gone: Vec<&String> = before
        .keys()
        .filter(|id| !after.contains_key(*id))
        .collect();
    assert_eq!((evicted.threads, evicted.bytes), (gone.len(), total - left));
    assert!(gone.iter().all(|id| !kept.contains(*id)));
    assert!(
        left <= budget / 10 * 9,
        "down to nine tenths of the budget: {left} of {budget}"
    );
    let newest_gone = gone
        .iter()
        .map(|id| before[*id].0)
        .max()
        .expect("some went");
    let oldest_left = after
        .iter()
        .filter(|(id, _)| !kept.contains(*id))
        .map(|(_, (newest, _))| *newest)
        .min();
    assert!(
        oldest_left.is_none_or(|oldest| newest_gone <= oldest),
        "the oldest went first"
    );
    let last = gone
        .iter()
        .max_by_key(|id| before[**id].0)
        .expect("some went");
    assert!(
        left + before[*last].1 > budget / 10 * 9,
        "and no more than that"
    );
    assert_eq!(
        store
            .evict(&by_budget(budget))
            .await
            .expect("evicts")
            .threads,
        0,
        "within the budget, nothing more goes"
    );
}

#[tokio::test]
async fn eviction_never_touches_drafts_sends_the_outbox_or_local_state() {
    let harness = Harness::synced().await;
    let store = &harness.store;
    let dir = harness.dir.path();
    let [open, replied, starred, reminded]: [ThreadId; 4] = open_done(&harness, 4)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect::<Vec<_>>()
        .try_into()
        .expect("four");
    // The conversation on screen, a send held for undo and one scheduled for later, and a
    // reply being drafted on another.
    store.reading(Some(&open)).await;
    store
        .save_draft(draft("draft00000000000002", None, None))
        .await
        .expect("saves");
    store
        .send_draft(
            "draft00000000000002",
            Timestamp::now() + SignedDuration::from_hours(1),
        )
        .await
        .expect("holds");
    let at = Timestamp::now() + SignedDuration::from_hours(48);
    store
        .save_draft(draft(
            "draft00000000000003",
            None,
            Some(SendLater {
                at,
                abort_on_reply: false,
                current_message_ids: Vec::new(),
            }),
        ))
        .await
        .expect("saves");
    store
        .schedule_draft("draft00000000000003")
        .await
        .expect("schedules");
    store.push_drafts().await.expect("pushes");
    store
        .save_draft(draft("draft00000000000001", Some(&replied), None))
        .await
        .expect("saves");
    // A change Gmail hasn't had, a reminder, an unsubscribe and settings.
    store
        .apply(std::slice::from_ref(&starred), Mutation::Star)
        .await
        .expect("stars");
    store
        .apply(
            std::slice::from_ref(&reminded),
            Mutation::Remind {
                at,
                keep_on_reply: false,
                snooze: true,
            },
        )
        .await
        .expect("reminds");
    store
        .queue_unsubscribe("https://news.example/unsubscribe")
        .await
        .expect("queues");
    let mut settings = Settings::default();
    settings.theme = Theme::Carbon;
    store.save_settings(&settings).await.expect("saves");
    let tables = kept_tables(dir);
    let (inbox, sends) = (
        listed(dir, &["INBOX"]),
        listed(dir, &["DRAFT", "SH_SCHEDULED"]),
    );
    let before = bodied(dir);

    let evicted = store.evict(&EVERYTHING).await.expect("evicts");
    assert!(evicted.rows > 0 && evicted.threads > 0, "{evicted:?}");
    assert_eq!(
        kept_tables(dir),
        tables,
        "every row Gmail doesn't have is kept: drafts, the outbox, reminders, settings, kv, the calendar"
    );
    let left = stored(dir);
    assert!(
        [&open, &replied, &starred, &reminded]
            .iter()
            .all(|id| left.contains(id.as_str())),
        "conversations open, drafted, changed or reminded stay"
    );
    assert!(
        inbox.iter().chain(&sends).all(|id| left.contains(id)),
        "so do the inbox's and the sends'"
    );
    let after = bodied(dir);
    for (id, why) in [(&open, "open"), (&replied, "has a draft")] {
        assert!(
            after.contains_key(id.as_str()),
            "{id}'s bodies stay: it {why}"
        );
    }
    assert!(
        before
            .keys()
            .filter(|id| inbox.contains(*id))
            .all(|id| after.contains_key(id)),
        "so do the inbox's"
    );
    assert!(
        !after.contains_key(starred.as_str()) && !after.contains_key(reminded.as_str()),
        "a queued change or a reminder needs no body"
    );
    assert_eq!(store.pending_unsubscribes().await.expect("reads").len(), 1);

    // Closed, it goes too.
    store.reading(None).await;
    store.evict(&EVERYTHING).await.expect("evicts");
    assert!(store.thread(&open).await.expect("reads").is_none());
}

#[tokio::test]
async fn an_evicted_conversation_is_found_locally_by_its_rows_and_online_by_gmail() {
    let harness = Harness::synced().await;
    let store = &harness.store;
    let delivered = harness.deliver(json!({ "from": "Ann <ann@example.com>", "subject": "Harbor survey", "html": "<p>The quokka census is done.</p>", "text": "The quokka census is done.", "labelIds": ["CATEGORY_UPDATES"] })).await;
    let thread = ThreadId::new(delivered.thread_id.expect("a thread"));
    harness.sync_all().await;
    store.load_bodies(&thread).await.expect("fetches");
    let local = async |query: &str| -> Vec<ThreadId> {
        let page = store
            .search(&mut Search::new(query), 10, false, 15)
            .await
            .expect("searches");
        page.results
            .into_iter()
            .map(|result| result.thread.id)
            .collect()
    };
    assert!(local("quokka").await.contains(&thread), "found by its body");

    // Past the budget, its bodies go and its rows stay.
    store.evict(&by_budget(0)).await.expect("evicts");
    assert!(
        !local("quokka").await.contains(&thread),
        "its text left the index with its body"
    );
    assert!(
        local("harbor survey").await.contains(&thread),
        "its subject didn't"
    );
    assert!(
        local("from:ann@example.com").await.contains(&thread),
        "nor its people"
    );

    // Quiet, it goes whole: offline nothing finds it, online Gmail's search does, merged with
    // the local results.
    store.evict(&EVERYTHING).await.expect("evicts");
    assert!(
        local("harbor survey").await.is_empty() && local("from:ann@example.com").await.is_empty()
    );
    let mut search = Search::new("quokka");
    let page = store.search_gmail(&search).await.expect("searches Gmail");
    search.add_gmail(page);
    let found = store
        .search(&mut search, 10, true, 15)
        .await
        .expect("searches");
    assert!(
        found
            .results
            .iter()
            .any(|result| result.thread.id == thread),
        "Gmail finds it"
    );
}

#[tokio::test]
async fn old_events_go_but_for_the_days_the_calendar_read() {
    let harness = Harness::start().await;
    let (store, dir) = (&harness.store, harness.dir.path());
    let tz = jiff::tz::TimeZone::get("America/New_York").expect("a zone");
    let at = |day: jiff::civil::Date, hour: i8| gmail::calendar::EventDateTime {
        date_time: Some(
            day.at(hour, 0, 0, 0)
                .to_zoned(tz.clone())
                .expect("exists")
                .strftime("%Y-%m-%dT%H:%M:%S%:z")
                .to_string(),
        ),
        ..gmail::calendar::EventDateTime::default()
    };
    let day = |day: jiff::civil::Date| day..day.tomorrow().expect("a day");
    // Three days long past with an event each, and this week.
    let (read, fetched, changed) = (
        jiff::civil::date(2026, 3, 3),
        jiff::civil::date(2026, 3, 4),
        jiff::civil::date(2026, 3, 5),
    );
    for (n, date) in [read, fetched, changed].into_iter().enumerate() {
        let event = gmail::calendar::Event {
            summary: Some(format!("Long ago {n}")),
            start: Some(at(date, 9)),
            end: Some(at(date, 10)),
            ..gmail::calendar::Event::default()
        };
        harness
            .gmail
            .calendar()
            .insert_event(ACCOUNT, &event, None)
            .await
            .expect("inserts");
        store
            .sync_events(ACCOUNT, day(date), &tz)
            .await
            .expect("syncs");
    }
    let week = jiff::civil::date(2026, 11, 15)..jiff::civil::date(2026, 11, 22);
    store
        .sync_events(ACCOUNT, week.clone(), &tz)
        .await
        .expect("syncs");
    let on = |range: std::ops::Range<jiff::civil::Date>| {
        let tz = tz.clone();
        async move {
            store
                .events(&[ACCOUNT.to_owned()], range, &tz)
                .await
                .expect("reads")
                .len()
        }
    };
    let this_week = rows(
        dir,
        "SELECT count(*) FROM events WHERE \"end\" > 1793000000000",
    )[0][0]
        .clone();
    // The calendar showed the first of them this session.
    assert!(on(day(read)).await > 0);

    // A change to an event waits to go: nothing goes, as it shows over what it changes.
    let lunch = gmail::calendar::Event {
        id: Some("e0lunch0001".into()),
        summary: Some("Lunch".into()),
        start: Some(at(jiff::civil::date(2026, 11, 20), 12)),
        end: Some(at(jiff::civil::date(2026, 11, 20), 13)),
        ..gmail::calendar::Event::default()
    };
    store
        .queue_calendar_change(
            1,
            exo_store::CalendarChange::Create {
                calendar: ACCOUNT.into(),
                event: lunch,
            },
            None,
        )
        .await
        .expect("queues");
    assert_eq!(
        store
            .evict(&Eviction::default())
            .await
            .expect("evicts")
            .events,
        0
    );
    store.unqueue_calendar_change(1).await.expect("takes back");

    let evicted = store.evict(&Eviction::default()).await.expect("evicts");
    assert_eq!(
        evicted.events, 2,
        "the days long past the calendar didn't read"
    );
    assert!(on(day(read)).await > 0, "the day it read stays");
    let gone = |date: jiff::civil::Date| {
        rows(
            dir,
            &format!(
                "SELECT count(*) FROM events WHERE start >= {} AND start < {}",
                date.to_zoned(tz.clone())
                    .expect("exists")
                    .timestamp()
                    .as_millisecond(),
                date.tomorrow()
                    .expect("a day")
                    .to_zoned(tz.clone())
                    .expect("exists")
                    .timestamp()
                    .as_millisecond()
            ),
        )[0][0]
            == "0"
    };
    assert!(gone(fetched) && gone(changed));
    assert_eq!(
        rows(
            dir,
            "SELECT count(*) FROM events WHERE \"end\" > 1793000000000"
        )[0][0],
        this_week,
        "this week's stay"
    );

    // Shown again, a day long past is fetched again, as a day the calendar hasn't fetched.
    store
        .sync_events(ACCOUNT, day(fetched), &tz)
        .await
        .expect("syncs");
    assert_eq!(on(day(fetched)).await, 1);
}

#[tokio::test]
async fn the_file_gives_the_space_back() {
    let harness = Harness::synced().await;
    let store = &harness.store;
    let dir = harness.dir.path();
    assert_eq!(
        rows(dir, "PRAGMA auto_vacuum"),
        [["2"]],
        "a new file is incremental"
    );
    // Two hundred conversations of 40 KB, opened.
    let paragraph = "Shipments cleared customs at the harbor this morning. ".repeat(740);
    let mut threads = Vec::new();
    for n in 0..200 {
        let delivered = harness.deliver(json!({ "from": "Ops <ops@example.com>", "subject": format!("Report {n}"), "html": format!("<p>{paragraph}</p>"), "labelIds": ["CATEGORY_UPDATES"] })).await;
        threads.push(ThreadId::new(delivered.thread_id.expect("a thread")));
    }
    harness.sync_all().await;
    for id in &threads {
        store.load_bodies(id).await.expect("fetches");
    }
    let size = |suffix: &str| {
        std::fs::metadata(format!("{}{suffix}", database(dir).display()))
            .map_or(0, |metadata| metadata.len())
    };
    store
        .evict(&by_budget(u64::MAX))
        .await
        .expect("checkpoints");
    let (file, wal) = (size(""), size("-wal"));
    let index = || {
        rows(
            dir,
            "SELECT sum(pgsize) FROM dbstat WHERE name LIKE 'search_fts%'",
        )[0][0]
            .parse::<u64>()
            .expect("bytes")
    };
    let indexed = index();

    let evicted = store.evict(&EVERYTHING).await.expect("evicts");
    assert!(evicted.bytes >= 200 * 40_000, "{evicted:?}");
    assert!(
        evicted.reclaimed.abs_diff(file - size("")) <= 4 * 4096,
        "it gave back what it says: {} of {}",
        evicted.reclaimed,
        file - size("")
    );
    assert!(
        file - size("") >= evicted.bytes,
        "the file gave back their bytes and their index's: {} of {file}",
        size("")
    );
    assert_eq!((wal, size("-wal")), (0, 0), "the WAL is emptied");
    assert!(
        index() < indexed / 4,
        "the search index let their text go: {} of {indexed}",
        index()
    );
    assert_eq!(rows(dir, "PRAGMA freelist_count"), [["0"]]);
    assert_eq!(rows(dir, "PRAGMA integrity_check"), [["ok"]]);
}

#[tokio::test]
async fn a_cleared_store_gives_space_back_and_eviction_keeps_what_clearing_kept() {
    let harness = Harness::synced().await;
    let [replied, reminded]: [ThreadId; 2] = open_done(&harness, 2)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect::<Vec<_>>()
        .try_into()
        .expect("two");
    let store = &harness.store;
    store
        .save_draft(draft("draft00000000000002", None, None))
        .await
        .expect("saves");
    store
        .send_draft(
            "draft00000000000002",
            Timestamp::now() + SignedDuration::from_hours(1),
        )
        .await
        .expect("holds");
    store
        .save_draft(draft("draft00000000000001", Some(&replied), None))
        .await
        .expect("saves");
    let at: Timestamp = "2030-02-01T13:00:00Z".parse().expect("a time");
    store
        .apply(
            std::slice::from_ref(&reminded),
            Mutation::Remind {
                at,
                keep_on_reply: false,
                snooze: true,
            },
        )
        .await
        .expect("reminds");
    store
        .queue_unsubscribe("https://news.example/unsubscribe")
        .await
        .expect("queues");
    let Harness {
        server, dir, store, ..
    } = harness;
    drop(store);

    exo_store::Store::clear_cache(dir.path(), ACCOUNT)
        .await
        .expect("clears");
    assert_eq!(
        rows(dir.path(), "PRAGMA auto_vacuum"),
        [["2"]],
        "the file Clear Cache makes gives space back"
    );
    let store = support::open(&server, dir.path()).await;
    let cleared = kept_tables(dir.path());
    // Its first pass, before it syncs, has nothing to evict and leaves what clearing kept.
    assert_eq!(store.evict(&EVERYTHING).await.expect("evicts").threads, 0);
    assert_eq!(kept_tables(dir.path()), cleared);

    // Synced again, a pass keeps the drafted conversation's bodies and what Gmail doesn't have.
    while store.sync().await.expect("syncs").backfilling {}
    store.load_bodies(&replied).await.expect("fetches");
    store.load_bodies(&reminded).await.expect("fetches");
    let synced = kept_tables(dir.path());
    assert!(store.evict(&EVERYTHING).await.expect("evicts").threads > 0);
    assert_eq!(kept_tables(dir.path()), synced);
    let after = bodied(dir.path());
    assert!(
        after.contains_key(replied.as_str()) && !after.contains_key(reminded.as_str()),
        "the reply's draft keeps its conversation's bodies"
    );
    assert!(
        stored(dir.path()).contains(reminded.as_str()),
        "the reminder keeps its conversation"
    );
    assert_eq!(store.next_reminder().await.expect("reads"), Some(at));
    assert_eq!(store.pending_unsubscribes().await.expect("reads").len(), 1);
}

#[tokio::test]
async fn a_file_from_before_eviction_is_rebuilt_once_to_give_space_back() {
    let server = gmail_mock::MockServer::start(gmail_mock::Config::default())
        .await
        .expect("the mock starts");
    let dir = support::TempDir::new();
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/store-0001.sqlite3"
        ),
        database(dir.path()),
    )
    .expect("copies the fixture");
    assert_eq!(rows(dir.path(), "PRAGMA auto_vacuum"), [["0"]]);
    let counts = "SELECT (SELECT count(*) FROM messages), (SELECT count(*) FROM bodies), (SELECT count(*) FROM drafts), (SELECT count(*) FROM outbox)";
    let before = rows(dir.path(), counts);

    let store = support::open(&server, dir.path()).await;
    assert_eq!(rows(dir.path(), "PRAGMA auto_vacuum"), [["2"]]);
    assert_eq!(rows(dir.path(), counts), before, "with everything in it");
    assert_eq!(rows(dir.path(), "PRAGMA integrity_check"), [["ok"]]);
    drop(store);
    let modified = std::fs::metadata(database(dir.path()))
        .and_then(|metadata| metadata.modified())
        .expect("a time");
    drop(support::open(&server, dir.path()).await);
    assert_eq!(
        std::fs::metadata(database(dir.path()))
            .and_then(|metadata| metadata.modified())
            .expect("a time"),
        modified,
        "once"
    );
}

/// Where a store's bytes are, from SQLite's own accounting (`dbstat`), grouped by what they hold.
#[derive(Debug, Default)]
struct Usage {
    /// The database file and its WAL.
    file: u64,
    wal: u64,
    /// Pages in use, by group: bodies, messages, search, lists, attachments, calendar, other.
    groups: BTreeMap<&'static str, u64>,
    free: u64,
    messages: i64,
    bodies: i64,
    threads: i64,
    events: i64,
    /// Average `headers` bytes of a message with its body stored, and of one without.
    headers: (f64, f64),
    /// Average body bytes (HTML and text).
    body: f64,
}

impl Usage {
    fn of(dir: &Path) -> Self {
        let size = |suffix: &str| {
            std::fs::metadata(format!("{}{suffix}", database(dir).display()))
                .map_or(0, |metadata| metadata.len())
        };
        let db = reader(dir);
        let mut usage = Self {
            file: size(""),
            wal: size("-wal"),
            ..Self::default()
        };
        let mut statement = db
            .prepare("SELECT name, sum(pgsize) FROM dbstat GROUP BY name")
            .expect("dbstat");
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .expect("reads");
        for row in rows {
            let (name, bytes) = row.expect("a row");
            let group = match name.as_str() {
                name if name.contains("bodies") => "bodies",
                name if name.contains("messages") => "messages",
                name if name.starts_with("search_fts") => "search",
                name if name.contains("list_ids")
                    || name.contains("participants")
                    || name.contains("threads")
                    || name == "lists" =>
                {
                    "lists"
                }
                name if name.contains("attachments") => "attachments",
                name if name.contains("events") || name.contains("calendars") => "calendar",
                _ => "other",
            };
            *usage.groups.entry(group).or_default() += u64::try_from(bytes).unwrap_or_default();
        }
        let one = |sql: &str| {
            db.query_row(sql, [], |row| row.get::<_, i64>(0))
                .expect(sql)
        };
        let real = |sql: &str| {
            db.query_row(sql, [], |row| row.get::<_, Option<f64>>(0))
                .expect(sql)
                .unwrap_or_default()
        };
        usage.free = u64::try_from(one("PRAGMA freelist_count") * one("PRAGMA page_size"))
            .unwrap_or_default();
        usage.messages = one("SELECT count(*) FROM messages");
        usage.bodies = one("SELECT count(*) FROM bodies");
        usage.threads = one("SELECT count(*) FROM threads");
        usage.events = one("SELECT count(*) FROM events");
        usage.headers = (
            real(
                "SELECT avg(octet_length(headers)) FROM messages WHERE id IN (SELECT message_id FROM bodies)",
            ),
            real(
                "SELECT avg(octet_length(headers)) FROM messages WHERE id NOT IN (SELECT message_id FROM bodies)",
            ),
        );
        usage.body = real(
            "SELECT avg(coalesce(octet_length(html), 0) + coalesce(octet_length(text), 0)) FROM bodies",
        );
        usage
    }

    fn header() -> String {
        format!(
            "{:>5} {:>8} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>6} {:>6} {:>8} {:>7} {:>7} {:>9} {:>9}",
            "day",
            "file",
            "wal",
            "bodies",
            "msgs",
            "search",
            "lists",
            "attach",
            "free",
            "other",
            "messages",
            "bodied",
            "threads",
            "avg body",
            "hdr b/r"
        )
    }

    fn row(&self, day: &str) -> String {
        let mb = |bytes: u64| format!("{:.1}", bytes as f64 / 1e6);
        let group = |name| mb(self.groups.get(name).copied().unwrap_or_default());
        format!(
            "{day:>5} {:>8} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>6} {:>6} {:>8} {:>7} {:>7} {:>9.0} {:>4.0}/{:<4.0}",
            mb(self.file),
            mb(self.wal),
            group("bodies"),
            group("messages"),
            group("search"),
            group("lists"),
            group("attachments"),
            mb(self.free),
            group("other"),
            self.messages,
            self.bodies,
            self.threads,
            self.body,
            self.headers.0,
            self.headers.1,
        )
    }
}

/// The fixture's bodies, by kind, to deliver again as a year of new mail.
struct Templates {
    personal: Vec<(String, String)>,
    newsletters: Vec<(String, String)>,
}

impl Templates {
    /// The fixture's bodies, each HTML part padded with `pad` bytes of markup no one sees:
    /// `(personal, bulk)`, for mail of real sizes.
    async fn of(harness: &Harness, pad: (usize, usize)) -> Self {
        let gmail = harness.gmail.gmail();
        let mut query = ListQuery {
            max_results: Some(500),
            ..ListQuery::default()
        };
        let mut ids = Vec::new();
        loop {
            let page = gmail.messages(&query).await.expect("messages.list");
            ids.extend(page.messages.into_iter().filter_map(|message| message.id));
            match page.next_page_token {
                Some(token) => query.page_token = Some(token),
                None => break,
            }
        }
        let (mut personal, mut newsletters) = (Vec::new(), Vec::new());
        for id in ids {
            let message = gmail
                .message(&id, Format::Full, &[])
                .await
                .expect("messages.get");
            let Some(payload) = &message.payload else {
                continue;
            };
            let part = |mime: &str| {
                payload
                    .walk()
                    .find(|part| part.mime_type.as_deref() == Some(mime))
                    .and_then(|part| part.body.as_ref()?.data.clone())
                    .map(|data| String::from_utf8_lossy(&data).into_owned())
            };
            let (Some(html), Some(text)) = (part("text/html"), part("text/plain")) else {
                continue;
            };
            if payload.header("List-Id").is_some() {
                newsletters.push((padded(&html, pad.1), text))
            } else {
                personal.push((padded(&html, pad.0), text))
            }
        }
        Self {
            personal,
            newsletters,
        }
    }
}

/// `html` with `bytes` more of the inline-styled table markup email HTML is mostly made of,
/// hidden, so it adds no text.
fn padded(html: &str, bytes: usize) -> String {
    const CELL: &str = r#"<td style="padding:0 0 16px;font-family:-apple-system,Helvetica,Arial,sans-serif;font-size:16px;line-height:24px;color:#374151"></td>"#;
    format!(
        "{html}<div style=\"display:none\"><table><tr>{}</tr></table></div>",
        CELL.repeat(bytes / CELL.len())
    )
}

/// A tiny deterministic generator (xorshift), so every run delivers the same year.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % u64::try_from(n.max(1)).unwrap_or(1)).unwrap_or_default()
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// A year of a busy inbox on gmail-mock with the fixture's bodies, then the default eviction.
///
/// `cargo test --release -p exo-store --test evict -- --ignored --nocapture busy_year`
#[tokio::test(flavor = "multi_thread")]
#[ignore = "a measurement, minutes in release"]
async fn a_busy_year() {
    year((0, 0)).await;
}

/// [`a_busy_year`] with bodies of real mail's sizes: personal mail about 15 KB of HTML and
/// text, newsletters and notifications about 40 KB.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "a measurement, minutes in release"]
async fn a_busy_year_of_real_sized_mail() {
    year((13_000, 37_000)).await;
}

/// A year of a busy inbox on gmail-mock, delivered a day at a time with a sync after each, as
/// Exo running every day would see it: per day 25 new conversations and 25 replies in the inbox,
/// 20 newsletters, 50 notifications that skip the inbox (20 of them follow-ups), and 15 sent
/// replies; the inbox is cleared each day (Mark Done), and 3 conversations older than a month
/// are opened. Prints where the bytes are every 30 days, then what rebuilding a file of that
/// size for `auto_vacuum` takes, what the default eviction does, and what opening an evicted
/// conversation costs. `pad` pads the bodies ([`Templates::of`]).
async fn year(pad: (usize, usize)) {
    const DAYS: i64 = 365;
    let start = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(24 * (DAYS + 1));
    let tz = jiff::tz::TimeZone::get("America/New_York").expect("a zone");
    let config = gmail_mock::Config {
        now: start.to_zoned(tz),
        ..gmail_mock::Config::default()
    };
    let harness = Harness::start_with(config, options()).await;
    harness.sync_all().await;
    let templates = Templates::of(&harness, pad).await;
    let size = |pool: &[(String, String)]| {
        pool.iter()
            .map(|(html, text)| html.len() + text.len())
            .sum::<usize>()
            / pool.len().max(1)
    };
    println!(
        "templates: {} personal ({} B), {} newsletters ({} B)",
        templates.personal.len(),
        size(&templates.personal),
        templates.newsletters.len(),
        size(&templates.newsletters)
    );
    println!("{}", Usage::header());
    println!("{}", Usage::of(harness.dir.path()).row("0"));

    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let http = reqwest::Client::new();
    let deliver = async |body: Value| -> gmail::Message {
        let url = harness.server.url().join("/__mock/deliver").expect("a URL");
        let response = http.post(url).json(&body).send().await.expect("delivers");
        assert!(response.status().is_success(), "{}", response.status());
        response.json().await.expect("a message")
    };
    // Conversations by the day they started, to reply into and to open.
    let mut personal: Vec<(i64, String)> = Vec::new();
    let mut notifications: Vec<(i64, String)> = Vec::new();
    let clock = Instant::now();
    for day in 1..=DAYS {
        let now = start + jiff::SignedDuration::from_hours(24 * day);
        harness
            .admin(
                reqwest::Method::POST,
                "/__mock/clock",
                json!({ "now": now.to_string() }),
            )
            .await;
        for n in 0..25 {
            let (html, text) = rng.pick(&templates.personal);
            let message = deliver(json!({ "from": format!("Colleague {} <c{}@partner{}.example>", n, rng.below(400), rng.below(40)), "subject": format!("Day {day} topic {n}"), "html": html, "text": text })).await;
            personal.push((day, message.thread_id.expect("a thread")));
        }
        let recent: Vec<String> = personal
            .iter()
            .filter(|(started, _)| day - started < 14)
            .map(|(_, id)| id.clone())
            .collect();
        for _ in 0..25 {
            let (html, text) = rng.pick(&templates.personal);
            deliver(json!({ "from": format!("Colleague <c{}@partner.example>", rng.below(400)), "html": html, "text": text, "threadId": rng.pick(&recent) })).await;
        }
        for _ in 0..15 {
            let (html, text) = rng.pick(&templates.personal);
            deliver(json!({ "from": "Maya Okafor <maya@brightmoor.co>", "to": ["someone@partner.example"], "html": html, "text": text, "threadId": rng.pick(&recent), "labelIds": ["SENT"] })).await;
        }
        for n in 0..20 {
            let (html, text) = rng.pick(&templates.newsletters);
            let labels = if n % 2 == 0 {
                ["INBOX", "UNREAD", "CATEGORY_UPDATES"]
            } else {
                ["INBOX", "UNREAD", "CATEGORY_PROMOTIONS"]
            };
            deliver(json!({ "from": format!("Brand {} <news@brand{}.example>", n, rng.below(60)), "subject": format!("Issue {day}.{n}"), "html": html, "text": text, "labelIds": labels })).await;
        }
        for n in 0..30 {
            let (html, text) = rng.pick(&templates.newsletters);
            let message = deliver(json!({ "from": "GitHub <notifications@github.example>", "subject": format!("[repo] Issue {day}.{n}"), "html": html, "text": text, "labelIds": ["CATEGORY_UPDATES"] })).await;
            notifications.push((day, message.thread_id.expect("a thread")));
        }
        let open: Vec<String> = notifications
            .iter()
            .filter(|(started, _)| day - started < 30)
            .map(|(_, id)| id.clone())
            .collect();
        for _ in 0..20 {
            let (html, text) = rng.pick(&templates.newsletters);
            deliver(json!({ "from": "GitHub <notifications@github.example>", "html": html, "text": text, "threadId": rng.pick(&open), "labelIds": ["CATEGORY_UPDATES"] })).await;
        }
        harness.sync_all().await;
        // Inbox zero: everything but today's is done.
        let inbox = harness
            .store
            .list(&ListId::Inbox, None, 2000)
            .await
            .expect("lists");
        let today = now.as_millisecond() - 12 * 3_600_000;
        let done: Vec<ThreadId> = inbox
            .threads
            .into_iter()
            .filter(|thread| thread.date.as_millisecond() < today)
            .map(|thread| thread.id)
            .collect();
        if !done.is_empty() {
            harness
                .store
                .apply(&done, Mutation::MarkDone)
                .await
                .expect("marks done");
            harness.store.flush().await.expect("flushes");
        }
        let old: Vec<&String> = personal
            .iter()
            .chain(&notifications)
            .filter(|(started, _)| day - started > 30)
            .map(|(_, id)| id)
            .collect();
        for _ in 0..3.min(old.len()) {
            harness
                .store
                .load_bodies(&ThreadId::new(rng.pick(&old).as_str()))
                .await
                .expect("opens");
        }
        if day % 30 == 0 || day == DAYS {
            println!("{}", Usage::of(harness.dir.path()).row(&day.to_string()));
        }
    }
    println!("simulated in {:.0?}", clock.elapsed());

    let dir = harness.dir.path();
    rebuild(dir);
    let clock = Instant::now();
    let evicted = harness
        .store
        .evict(&Eviction::default())
        .await
        .expect("evicts");
    println!("evicted {evicted:?} in {:.1?}", clock.elapsed());
    println!("{}", Usage::of(dir).row("after"));
    let clock = Instant::now();
    let again = harness
        .store
        .evict(&Eviction::default())
        .await
        .expect("evicts");
    println!("again: {again:?} in {:.1?}", clock.elapsed());

    // Scrolling All Mail back past where it now ends fetches what went, from where it ends, and
    // opening such a conversation is opening a cold one.
    let shown = harness.all(&ListId::All).await.len();
    let (units_before, clock) = (units(&harness).await, Instant::now());
    harness
        .store
        .backfill(&ListId::All, 500)
        .await
        .expect("backfills");
    let back = harness.all(&ListId::All).await;
    println!(
        "scrolled All Mail back {} conversations past its {shown}: {} units in {:.1?}",
        back.len() - shown,
        units(&harness).await - units_before,
        clock.elapsed()
    );
    let old: Vec<ThreadId> = back[shown..]
        .iter()
        .take(20)
        .map(|thread| thread.id.clone())
        .collect();
    let (units_before, clock) = (units(&harness).await, Instant::now());
    for id in &old {
        assert!(
            !harness.store.bodies_stored(id).await.expect("reads"),
            "fetched back for its rows"
        );
        harness.store.load_bodies(id).await.expect("opens");
    }
    println!(
        "opened {} of them: {} units and {:.0?} each",
        old.len(),
        (units(&harness).await - units_before) / old.len() as u64,
        clock.elapsed() / u32::try_from(old.len()).unwrap_or(1)
    );
}

/// Times what an older Exo's file of the store's size goes through once: a copy without
/// `auto_vacuum` rebuilt with it.
fn rebuild(dir: &Path) {
    let copy = dir.join("rebuild.sqlite3");
    {
        let db = rusqlite::Connection::open(database(dir)).expect("opens the store");
        db.execute_batch(&format!("VACUUM INTO '{}'", copy.display()))
            .expect("copies");
    }
    let db = rusqlite::Connection::open(&copy).expect("opens the copy");
    db.execute_batch("PRAGMA auto_vacuum = NONE; VACUUM; PRAGMA journal_mode = WAL;")
        .expect("makes it an older Exo's");
    let size = std::fs::metadata(&copy).map_or(0, |metadata| metadata.len());
    let clock = Instant::now();
    db.execute_batch("PRAGMA auto_vacuum = INCREMENTAL; VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("rebuilds");
    let took = clock.elapsed();
    println!(
        "rebuilding {:.1} MB for auto_vacuum took {took:.2?} ({:.0} MB/s)",
        size as f64 / 1e6,
        size as f64 / 1e6 / took.as_secs_f64()
    );
    drop(db);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", copy.display()));
    }
}

/// The conversations stored for their rows alone, with their newest message's date.
fn unrendered(dir: &Path) -> BTreeMap<String, i64> {
    rows(dir, "SELECT thread_id, max(internal_date) FROM messages WHERE thread_id IN (SELECT m.thread_id FROM messages m WHERE NOT EXISTS (SELECT 1 FROM bodies b WHERE b.message_id = m.id)) GROUP BY thread_id")
        .into_iter()
        .map(|row| (row[0].clone(), row[1].parse().expect("a date")))
        .collect()
}

#[tokio::test]
async fn the_pre_render_fetches_the_newest_conversations_eviction_keeps() {
    let harness = Harness::synced().await;
    let (store, dir) = (&harness.store, harness.dir.path());
    let before = unrendered(dir);
    assert!(
        before.len() > 10,
        "most archived mail is stored for its rows ({})",
        before.len()
    );
    let everything = by_budget(u64::MAX);
    assert_eq!(
        store.prerender(&everything, 5).await.expect("pre-renders"),
        5
    );
    let after = unrendered(dir);
    let rendered: Vec<&String> = before
        .keys()
        .filter(|id| !after.contains_key(*id))
        .collect();
    assert_eq!(rendered.len(), 5);
    let oldest_rendered = rendered.iter().map(|id| before[*id]).min().expect("some");
    assert!(
        after.values().all(|newest| *newest <= oldest_rendered),
        "newest first"
    );

    // Nothing eviction would let go of: none quieter than its age, none past its budget.
    assert_eq!(
        store
            .prerender(
                &Eviction {
                    age: SignedDuration::ZERO,
                    budget: u64::MAX
                },
                100
            )
            .await
            .expect("pre-renders"),
        0,
        "too old"
    );
    assert_eq!(
        store
            .prerender(&by_budget(0), 100)
            .await
            .expect("pre-renders"),
        0,
        "no room"
    );
    assert_eq!(unrendered(dir), after);
}

#[tokio::test]
async fn the_pre_render_fetches_nothing_eviction_let_go_of() {
    let harness = Harness::synced().await;
    let (store, dir) = (&harness.store, harness.dir.path());
    open_done(&harness, 40).await;
    let total: u64 = bodied(dir).values().map(|(_, bytes)| bytes).sum();
    let before = bodied(dir);
    assert!(
        store
            .evict(&by_budget(total / 2))
            .await
            .expect("evicts")
            .threads
            > 0
    );
    let gone: BTreeSet<String> = before
        .keys()
        .filter(|id| !bodied(dir).contains_key(*id))
        .cloned()
        .collect();
    let newest_gone = gone.iter().map(|id| before[id].0).max().expect("some went");
    // Room again (a bigger budget, say), the conversations that went stay as they are.
    store
        .prerender(&by_budget(u64::MAX), 1000)
        .await
        .expect("pre-renders");
    let rendered = bodied(dir);
    assert!(
        gone.iter().all(|id| !rendered.contains_key(id)),
        "none of the evicted came back"
    );
    assert!(
        unrendered(dir)
            .values()
            .all(|newest| *newest <= newest_gone),
        "nor anything older"
    );
}
