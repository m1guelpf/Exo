//! Clearing an account's cache (Clear Cache and Restart): its database is made again and its
//! mail downloaded again, keeping what Gmail doesn't have (the settings, drafts, held sends,
//! queued changes, reminders, unsubscribes to retry and the calendar's queued changes) when the
//! old file still reads.

mod support;

use std::path::{Path, PathBuf};

use exo_store::{
    Address, CalendarChange, Draft, DraftAction, DraftRecord, Error, ListId, Mutation, Settings,
    Store, Theme, Unusable,
};
use jiff::Timestamp;
use support::{ACCOUNT, Harness};

fn draft(id: &str, subject: &str) -> Draft {
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
        subject: subject.into(),
        body: format!("<div>{subject}</div>"),
        quoted_content: None,
        quoted_inlined: false,
        in_reply_to: None,
        in_reply_to_rfc822: None,
        references: Vec::new(),
        rfc822_id: format!("{id}@exo"),
        fingerprint: String::new(),
        attachments: Vec::new(),
        send_later: None,
        reminder: None,
    };
    let email = gmail::mime::Email {
        from: Some(gmail::mime::Mailbox::new("Maya Okafor", ACCOUNT)),
        to: vec![gmail::mime::Mailbox::new("Ann", "ann@example.com")],
        subject: subject.into(),
        html: Some(format!("<div>{subject}</div>")),
        message_id: Some(format!("{id}@exo")),
        ..gmail::mime::Email::default()
    };
    let raw = String::from_utf8(email.to_rfc822()).expect("MIME is ASCII");
    Draft {
        id: id.into(),
        thread: None,
        adopted: None,
        record,
        raw,
        complete: true,
        sending: false,
    }
}

fn database(dir: &Path) -> PathBuf {
    dir.join(format!("{ACCOUNT}.sqlite3"))
}

/// The files in `dir` named for the account's database.
fn files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("lists")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(&format!("{ACCOUNT}.sqlite3")))
        .collect();
    names.sort();
    names
}

async fn drafts(store: &Store) -> Vec<(String, Option<Timestamp>)> {
    let mut drafts: Vec<_> = store
        .drafts()
        .await
        .expect("reads")
        .into_iter()
        .map(|draft| (draft.record.subject, draft.send_at))
        .collect();
    drafts.sort();
    drafts
}

#[tokio::test]
async fn clearing_a_newer_exos_cache_keeps_what_gmail_doesnt_have() {
    let harness = Harness::synced_with(support::unpaced()).await;
    let held: Timestamp = "2030-01-01T00:00:00Z".parse().expect("a time");
    harness
        .store
        .save_draft(draft("draft00000000000001", "Held send"))
        .await
        .expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    harness
        .store
        .send_draft("draft00000000000001", held)
        .await
        .expect("holds");
    harness
        .store
        .save_draft(draft("draft00000000000002", "Only here"))
        .await
        .expect("saves");
    let thread = harness.all(&ListId::Inbox).await.remove(0).id;
    harness
        .store
        .apply(std::slice::from_ref(&thread), Mutation::Star)
        .await
        .expect("queues");
    let remind_at: Timestamp = "2030-02-01T13:00:00Z".parse().expect("a time");
    harness
        .store
        .apply(
            &[thread],
            Mutation::Remind {
                at: remind_at,
                keep_on_reply: false,
                snooze: true,
            },
        )
        .await
        .expect("reminds");
    harness
        .store
        .queue_unsubscribe("https://lists.example.com/unsubscribe/1")
        .await
        .expect("queues");
    let offline_event = gmail::calendar::Event {
        id: Some("e0offline0001".into()),
        summary: Some("Made offline".into()),
        ..gmail::calendar::Event::default()
    };
    harness
        .store
        .queue_calendar_change(
            1,
            CalendarChange::Create {
                calendar: ACCOUNT.into(),
                event: offline_event,
            },
            None,
        )
        .await
        .expect("queues");
    let mut settings: Settings = harness.store.settings().await.expect("reads");
    settings.theme = Theme::Carbon;
    harness.store.save_settings(&settings).await.expect("saves");
    let unsent = harness.store.unsent().await.expect("counts");
    let kept = drafts(&harness.store).await;
    assert_eq!(kept.len(), 2);

    let Harness {
        server, dir, store, ..
    } = harness;
    drop(store);
    // As a newer Exo leaves it: with a migration this one doesn't have.
    rusqlite::Connection::open(database(dir.path())).and_then(|db| db.execute_batch("INSERT INTO __toasty_migrations (id, name, applied_at) VALUES (99, '0002_from_a_newer_exo.sql', datetime('now'))")).expect("a newer migration");
    let error = support::try_open(&server, dir.path())
        .await
        .expect_err("isn't opened");
    assert!(matches!(error, Error::UnknownSchema { .. }), "{error}");
    assert_eq!(error.unusable(), Some(Unusable::Corrupt));

    Store::clear_cache(dir.path(), ACCOUNT)
        .await
        .expect("clears");
    assert_eq!(
        files(dir.path())
            .iter()
            .filter(|name| name.contains("cleared"))
            .count(),
        0,
        "the old files are gone: {:?}",
        files(dir.path())
    );
    let store = support::open(&server, dir.path()).await;
    assert_eq!(
        drafts(&store).await,
        kept,
        "the held send and the draft only on this device"
    );
    assert_eq!(
        store.unsent().await.expect("counts"),
        unsent,
        "and the star Gmail hasn't got"
    );
    assert_eq!(
        store.settings::<Settings>().await.expect("reads").theme,
        Theme::Carbon
    );
    assert_eq!(
        store.next_reminder().await.expect("reads"),
        Some(remind_at),
        "the reminder, whose schedule is only here"
    );
    assert_eq!(
        store.pending_unsubscribes().await.expect("reads").len(),
        1,
        "the unsubscribe still to retry"
    );
    assert_eq!(
        store.calendar_pending().await.expect("counts"),
        1,
        "the calendar's change Calendar hasn't got"
    );
    assert!(
        store
            .list(&ListId::Inbox, None, 50)
            .await
            .expect("lists")
            .threads
            .is_empty(),
        "the mail itself is gone"
    );
    let mut backfilling = true;
    for _ in 0..50 {
        backfilling = store.sync().await.expect("syncs").backfilling;
        if !backfilling {
            break;
        }
    }
    assert!(!backfilling, "the backfill finishes");
    assert!(
        !store
            .list(&ListId::Inbox, None, 50)
            .await
            .expect("lists")
            .threads
            .is_empty(),
        "and comes back from Gmail"
    );
}

#[tokio::test]
async fn clearing_a_file_that_isnt_a_database_starts_afresh() {
    let server = gmail_mock::MockServer::start(gmail_mock::Config::default())
        .await
        .expect("the mock starts");
    let dir = support::TempDir::new();
    std::fs::write(database(dir.path()), vec![0x5a; 8192]).expect("writes");
    let error = support::try_open(&server, dir.path())
        .await
        .expect_err("isn't opened");
    assert_eq!(error.unusable(), Some(Unusable::Corrupt), "{error}");

    Store::clear_cache(dir.path(), ACCOUNT)
        .await
        .expect("clears");
    let store = support::open(&server, dir.path()).await;
    assert!(drafts(&store).await.is_empty(), "nothing could be kept");
    assert_eq!(
        store.settings::<Settings>().await.expect("reads").theme,
        Settings::default().theme
    );
    assert!(!store.sync().await.expect("syncs").rebuilt);
}

/// Every table is either kept by Clear Cache and Restart (`db::KEPT_TABLES`, and `kv`'s own
/// records) or one Gmail fills again. A new table fails this until it's one or the other.
#[tokio::test]
async fn every_table_is_kept_or_refilled() {
    let harness = Harness::synced_with(support::unpaced()).await;
    let Harness { dir, store, .. } = harness;
    drop(store);
    let db = rusqlite::Connection::open(database(dir.path())).expect("opens");
    let mut statement = db.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '__toasty%' AND name NOT LIKE 'search_fts%'").expect("prepares");
    let mut tables: Vec<String> = statement
        .query_map([], |row| row.get(0))
        .expect("queries")
        .collect::<Result<_, _>>()
        .expect("reads");
    tables.sort();
    let kept = [
        "calendar_changes",
        "drafts",
        "kv",
        "outbox",
        "reminders",
        "settings",
    ];
    let refilled = [
        "attachments",
        "bodies",
        "calendars",
        "events",
        "labels",
        "list_ids",
        "lists",
        "messages",
        "participants",
        "text_due",
        "threads",
    ];
    let mut known: Vec<String> = kept
        .iter()
        .chain(&refilled)
        .map(|table| (*table).to_owned())
        .collect();
    known.sort();
    assert_eq!(
        tables, known,
        "a new table: keep it in Clear Cache and Restart (db::KEPT_TABLES) if Gmail doesn't have it, or list it here as refilled"
    );
}
