//! The SQLite schema (toasty models), opening a database file, and the key-value table.
//!
//! Provider truth (`threads`, `messages`, `attachments`, `bodies`, `labels`) is kept apart
//! from the local overlay (`outbox`); `list_ids` is materialized from both on every change, so
//! list reads are a single indexed range scan.
//!
//! Changing a model here changes the schema: files written before it need a step in
//! [`crate::migrations`], which says how.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use toasty::stmt::Value;
use toasty::{Db, Executor, Json};
use toasty_driver_sqlite::Sqlite;

use crate::error::{Error, Result};
use crate::ids::ListId;
use crate::ids::labels::{INBOX, SH_ALL};
use crate::migrations;
use crate::model::Header;

/// Where the history cursor is kept in `kv`.
pub(crate) const HISTORY_ID: &str = "history_id";
/// Where the account's identity is kept in `kv`.
pub(crate) const IDENTITY: &str = "identity";
/// Where the account's send-as aliases are kept in `kv`.
pub(crate) const ALIASES: &str = "aliases";
/// The draft messages Exo wrote, a JSON set in `kv`.
pub(crate) const EXO_DRAFTS: &str = "exo_drafts";
/// The held sends whose reminder is on their thread already, and which reminder, a JSON map in
/// `kv`: Gmail sending them leaves the thread's reminder be.
pub(crate) const GIVEN_REMINDERS: &str = "given_reminders";
/// Set in `kv` by a new store's first sync until it has brought back the threads Gmail has under
/// `[Exo]/Is Snoozed` that it has no reminder for ([`Store::return_orphaned_snoozes`]).
///
/// [`Store::return_orphaned_snoozes`]: crate::Store::return_orphaned_snoozes
pub(crate) const ORPHANED_SNOOZES: &str = "orphaned_snoozes";
/// The newest message date, epoch milliseconds, of the conversations eviction let go of in `kv`:
/// the pre-render fetches nothing that old back.
pub(crate) const EVICTED_BEFORE: &str = "evicted_before";

/// Thread-level provider facts.
#[derive(Debug, toasty::Model)]
#[table = "threads"]
pub(crate) struct ThreadRow {
    #[key]
    pub id: String,
    /// The newest `historyId` stored for this thread; a listed one above it means stale.
    pub history_id: u64,
}

/// A Gmail message as Gmail last reported it (drafts included).
#[derive(Debug, toasty::Model)]
#[table = "messages"]
pub(crate) struct MessageRow {
    #[key]
    pub id: String,
    #[index]
    pub thread_id: String,
    pub history_id: u64,
    /// `internalDate`, epoch milliseconds: the message's date for sorting.
    pub internal_date: i64,
    pub label_ids: Vec<String>,
    pub subject: String,
    /// Gmail's snippet, still HTML-escaped.
    pub snippet: String,
    /// `From` (`from` would clash with toasty's generated builders).
    #[column(type = text)]
    pub sender: Json<Option<Mailbox>>,
    /// The raw `From` value, for the importance rule's "via Google" test.
    pub sender_raw: String,
    #[column(type = text)]
    pub to: Json<Vec<Mailbox>>,
    #[column(type = text)]
    pub cc: Json<Vec<Mailbox>>,
    #[column(type = text)]
    pub bcc: Json<Vec<Mailbox>>,
    #[column(type = text)]
    pub reply_to: Json<Vec<Mailbox>>,
    pub rfc822_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub list_id: Option<String>,
    pub list_unsubscribe: Option<String>,
    pub list_unsubscribe_post: Option<String>,
    #[column(type = text)]
    pub headers: Json<Vec<Header>>,
    pub size: u32,
    /// A real (non-inline) attachment or a calendar invite.
    pub has_attachments: bool,
}

/// A name and address from a header.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Mailbox {
    pub name: Option<String>,
    pub email: String,
}

#[derive(Debug, toasty::Model)]
#[table = "attachments"]
#[key(message_id, part_id)]
pub(crate) struct AttachmentRow {
    pub message_id: String,
    pub part_id: String,
    pub attachment_id: Option<String>,
    pub filename: String,
    pub mime_type: String,
    pub size: u32,
    pub content_id: Option<String>,
    pub inline: bool,
}

/// A message's body parts as sent. Kept apart from `messages` so lists never load them, and so
/// they can be evicted without losing metadata.
#[derive(Debug, toasty::Model)]
#[table = "bodies"]
pub(crate) struct BodyRow {
    #[key]
    pub message_id: String,
    pub html: Option<String>,
    pub text: Option<String>,
    /// The text a reader sees, without the quoted reply and the signature the thread view hides
    /// behind the `…`: what the search index holds. `None` until it's worked out
    /// ([`crate::Store::index_text_with`]); the index has the whole text until then.
    pub visible: Option<String>,
}

/// A conversation with a body stored whose [`BodyRow::visible`] text isn't worked out yet,
/// queued when the body is stored ([`crate::Store::index_text_with`]).
#[derive(Debug, toasty::Model)]
#[table = "text_due"]
pub(crate) struct TextDue {
    #[key]
    pub thread_id: String,
}

#[derive(Debug, toasty::Model)]
#[table = "labels"]
pub(crate) struct LabelRow {
    #[key]
    pub id: String,
    pub name: String,
    pub system: bool,
    pub hidden: bool,
    pub text_color: Option<String>,
    pub background_color: Option<String>,
}

/// Materialized list membership: one row per list a thread is in, with its sort key there.
#[derive(Debug, toasty::Model)]
#[table = "list_ids"]
#[key(list_id, thread_id)]
#[index(list_id, sort)]
pub(crate) struct ListEntry {
    pub list_id: String,
    #[index]
    pub thread_id: String,
    pub sort: i64,
}

/// Who took part in each thread, for a contact's latest conversations: one row per address,
/// lower-cased, and thread, with the thread's date. Materialized with `list_ids`.
#[derive(Debug, toasty::Model)]
#[table = "participants"]
#[key(email, thread_id)]
#[index(email, sort)]
pub(crate) struct ParticipantEntry {
    pub email: String,
    #[index]
    pub thread_id: String,
    pub sort: i64,
}

/// A list's backfill cursor.
#[derive(Debug, toasty::Model)]
#[table = "lists"]
pub(crate) struct ListCursor {
    #[key]
    pub id: String,
    /// The next `threads.list` page.
    pub page_token: Option<String>,
    /// The oldest thread date the backfill has reached; only ever lowered.
    pub min_sort: Option<i64>,
    /// Gmail has no more pages: everything is covered.
    pub complete: bool,
    /// Threads listed so far, for the per-list cap.
    pub seen: u32,
    /// The pass lists only threads with mail before this date, epoch milliseconds (Gmail's
    /// `before:`): eviction moved the list's coverage forward to it, so the pass takes up where
    /// what's stored ends rather than listing it all again.
    pub before: Option<i64>,
}

/// One queued mutation.
#[derive(Debug, toasty::Model)]
#[table = "outbox"]
pub(crate) struct OutboxRow {
    /// Insertion order, which is the queue order.
    #[key]
    #[auto]
    pub id: u64,
    /// The thread id: one FIFO per thread.
    #[index]
    pub queue: String,
    /// Coalescing key: a new row replaces an unstarted row with the same key at the queue's
    /// tail.
    pub replace_key: Option<String>,
    #[column(type = text)]
    pub mutation: Json<crate::Mutation>,
    /// This row undoes `mutation`.
    pub undo: bool,
    /// The Gmail calls still to make, in order; each is removed once Gmail acknowledges it.
    #[column(type = text)]
    pub steps: Json<Vec<crate::mutation::Step>>,
    /// Failures that count toward giving up (offline and auth failures don't).
    pub attempts: u32,
    /// Consecutive failures of any kind, for the backoff.
    pub failures: u32,
    pub not_before: Option<jiff::Timestamp>,
    pub started_at: Option<jiff::Timestamp>,
}

/// A draft compose is writing, and how far its Gmail draft has caught up with it. Not provider
/// truth, like the outbox.
#[derive(Debug, toasty::Model)]
#[table = "drafts"]
pub(crate) struct DraftRow {
    /// Exo's draft id (`draft00…`), stable for the draft's life.
    #[key]
    pub id: String,
    /// The Gmail thread, once there is one.
    pub thread_id: Option<String>,
    /// The Gmail draft, once created.
    pub gmail_id: Option<String>,
    /// The Gmail message of the latest version Gmail has.
    pub message_id: Option<String>,
    #[column(type = text)]
    pub record: Json<crate::drafts::DraftRecord>,
    /// The RFC 822 message the Gmail draft should hold: what sending sends.
    pub raw: String,
    /// Bumped by every save.
    pub revision: u64,
    /// The revision Gmail has (0: none).
    pub pushed: u64,
    /// Discarded: its Gmail draft is to be deleted, and the row with it.
    pub discarded: bool,
    /// Held for sending until then (undo send).
    pub send_at: Option<jiff::Timestamp>,
    /// Consecutive failures, for the backoff.
    pub failures: u32,
    pub not_before: Option<jiff::Timestamp>,
    /// Its send failed, as [`SendFailure::code`](crate::SendFailure::code) has it (`""` for no
    /// reason).
    pub failure: Option<String>,
    /// When its send failed.
    pub failed_at: Option<jiff::Timestamp>,
}

/// A thread's reminder: when it brings the thread back and what cancels it. Not provider truth:
/// Gmail only has the hidden labels.
#[derive(Debug, Clone, toasty::Model)]
#[table = "reminders"]
pub(crate) struct ReminderRow {
    #[key]
    pub thread_id: String,
    /// When it brings the thread back.
    pub at: jiff::Timestamp,
    /// A reply doesn't cancel it (the picker's `regardless`).
    pub keep_on_reply: bool,
    /// The thread's messages when it was set: a reply is a message not among them.
    pub message_ids: Vec<String>,
    /// When it was set.
    pub created_at: jiff::Timestamp,
    /// When it brought the thread back, which then sorts there (the trigger message's date).
    pub fired_at: Option<jiff::Timestamp>,
    /// A reply cancelled it and the scheduler took its label off.
    pub aborted: bool,
}

/// A calendar on the Google Calendar list of one of the account's calendar accounts (the mail
/// account's own among them), as Calendar last listed it.
#[derive(Debug, toasty::Model)]
#[table = "calendars"]
#[key(account, id)]
pub(crate) struct CalendarRow {
    /// The calendar account whose list it's on.
    pub account: String,
    pub id: String,
    /// Its place in the list.
    pub position: u32,
    #[column(type = text)]
    pub entry: Json<gmail::calendar::CalendarListEntry>,
}

/// An event (or one instance of a recurring event) as Calendar last listed it.
#[derive(Debug, toasty::Model)]
#[table = "events"]
#[key(calendar_id, id)]
#[index(calendar_id, start)]
pub(crate) struct EventRow {
    pub calendar_id: String,
    pub id: String,
    /// Start and end, epoch milliseconds; an all-day event's are its dates' UTC midnights.
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    #[column(type = text)]
    pub event: Json<gmail::calendar::Event>,
}

/// A change to the calendar Calendar doesn't have yet. Not provider truth, like the outbox.
#[derive(Debug, toasty::Model)]
#[table = "calendar_changes"]
pub(crate) struct CalendarChangeRow {
    /// Insertion order, which is the queue order.
    #[key]
    #[auto]
    pub id: u64,
    /// What the caller queued it under, to take it back with.
    pub token: u64,
    /// The event and its series: one FIFO per event.
    pub queue: String,
    #[column(type = text)]
    pub change: Json<crate::CalendarChange>,
    /// Consecutive postponements, for the backoff.
    pub failures: u32,
    /// Held for Undo, or backing off, until then.
    pub not_before: Option<jiff::Timestamp>,
}

/// Small persistent values: the history cursor, the account's identity.
#[derive(Debug, toasty::Model)]
#[table = "kv"]
pub(crate) struct KvRow {
    #[key]
    pub key: String,
    pub value: String,
}

/// The user's settings, a JSON document per [`crate::settings::Document`]. Not a cache.
#[derive(Debug, toasty::Model)]
#[table = "settings"]
pub(crate) struct SettingsRow {
    #[key]
    pub key: String,
    pub value: String,
}

/// Opens the database at `path`, making it, or bringing a file an older Exo wrote up to this
/// build's schema ([`crate::migrations`]).
pub(crate) async fn open(path: &Path) -> Result<Db> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|source| Error::Io {
            path: dir.to_owned(),
            source,
        })?;
    }
    let mut db = connect(path).await?;
    migrations::migrate(&mut db, path).await?;
    // Without it eviction only frees pages for the file to reuse: a file that can't be rebuilt
    // now (a disk too full for the copy) opens anyway, and the next open tries again.
    if let Err(error) = crate::evict::reclaimable(&db).await {
        log::warn!("the store keeps the space eviction frees for now: {error}");
    }
    Ok(db)
}

/// Makes the database at `path` again, empty, keeping what Gmail doesn't have when the old one
/// still reads: its settings, drafts and queued changes, in the columns both schemas have. What
/// can't be kept is dropped, logged.
pub(crate) async fn clear(path: &Path) -> Result<()> {
    let old = PathBuf::from(format!("{}.cleared", path.display()));
    if old.exists() {
        // A clear that stopped halfway: its old file still holds what to keep.
        remove(path)?;
    } else {
        for suffix in FILES {
            let (from, to) = (
                format!("{}{suffix}", path.display()),
                format!("{}{suffix}", old.display()),
            );
            match std::fs::rename(&from, &to) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(Error::Io {
                        path: from.into(),
                        source,
                    });
                }
            }
        }
    }
    let db = open(path).await?;
    if old.exists() {
        keep(&db, &old).await;
    }
    drop(db);
    remove(&old)
}

/// What Clear Cache and Restart copies from the old database: what Gmail doesn't have. The
/// settings, the drafts (held and scheduled sends among them), the queued changes, the
/// reminders' schedule and the calendar's queued changes. `kept_or_refilled` in
/// `tests/clear_cache.rs` lists the rest.
const KEPT_TABLES: [&str; 5] = [
    "settings",
    "drafts",
    "outbox",
    "reminders",
    "calendar_changes",
];
/// The `kv` entries kept with them, Exo's own records: which held sends gave their reminder, and
/// the unsubscribes still to retry.
const KEPT_KEYS: [&str; 2] = [GIVEN_REMINDERS, crate::unsubscribes::KEY];

/// Copies [`KEPT_TABLES`] and [`KEPT_KEYS`] of the database at `old` into `db`.
async fn keep(db: &Db, old: &Path) {
    // An attached database is the connection's, so one connection does it all.
    let mut connection = match db.connection().await {
        Ok(connection) => connection,
        Err(error) => return log::error!("keeping nothing of the old database: {error}"),
    };
    if let Err(error) = toasty::sql::statement("ATTACH DATABASE ?1 AS old")
        .bind(old.display().to_string())
        .exec(&mut connection)
        .await
    {
        return log::error!("keeping nothing of the old database: {error}");
    }
    for table in KEPT_TABLES {
        let columns = toasty::sql::query(r#"SELECT group_concat('"' || new.name || '"', ', ') FROM pragma_table_info(?1, 'main') new JOIN pragma_table_info(?1, 'old') old ON old.name = new.name"#)
            .bind(table)
            .exec(&mut connection)
            .await;
        let columns = match columns.as_deref() {
            Ok([Value::Record(row)]) => match row.fields.first() {
                Some(Value::String(columns)) => columns.clone(),
                _ => continue,
            },
            Ok(_) => continue,
            Err(error) => {
                log::error!("dropping the {table} the old database can't hand over: {error}");
                continue;
            }
        };
        match toasty::sql::statement(format!(
            r#"INSERT INTO main."{table}" ({columns}) SELECT {columns} FROM old."{table}""#
        ))
        .exec(&mut connection)
        .await
        {
            Ok(rows) => log::info!("kept {rows} rows of {table}"),
            Err(error) => {
                log::error!("dropping the {table} the old database can't hand over: {error}")
            }
        }
    }
    let keys = KEPT_KEYS.map(|key| format!("'{key}'")).join(", ");
    match toasty::sql::statement(format!(r#"INSERT INTO main."kv" ("key", "value") SELECT "key", "value" FROM old."kv" WHERE "key" IN ({keys})"#)).exec(&mut connection).await {
        Ok(rows) => log::info!("kept {rows} rows of kv"),
        Err(error) => log::error!("dropping the kv the old database can't hand over: {error}"),
    }
    if let Err(error) = toasty::sql::statement("DETACH DATABASE old")
        .exec(&mut connection)
        .await
    {
        log::warn!("detaching the old database: {error}");
    }
}

/// The files SQLite keeps for a database: the database itself, then its WAL, shared memory and
/// journal, each named for it with a suffix.
const FILES: [&str; 4] = ["", "-wal", "-shm", "-journal"];

/// Deletes the database at `path` and the files beside it.
fn remove(path: &Path) -> Result<()> {
    for suffix in FILES {
        let file = PathBuf::from(format!("{}{suffix}", path.display()));
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(Error::Io { path: file, source }),
        }
    }
    Ok(())
}

/// Connects to the file at `path` as it is, neither created nor upgraded.
pub(crate) async fn connect(path: &Path) -> Result<Db> {
    let mut db = Db::builder()
        .models(toasty::models!(crate::*))
        .build(Sqlite::open(path))
        .await?;
    // WAL lets the UI read while sync writes. The mode is stored in the file, so every pooled
    // connection gets it.
    toasty::sql::query("PRAGMA journal_mode = WAL")
        .exec(&mut db)
        .await?;
    Ok(db)
}

/// Runs a query returning one integer.
pub(crate) async fn query_i64(db: &mut dyn Executor, sql: &str) -> Result<i64> {
    let rows = toasty::sql::query(sql).exec(db).await?;
    Ok(match rows.first() {
        Some(Value::Record(row)) => match row.fields.first() {
            Some(Value::I64(value)) => *value,
            _ => 0,
        },
        _ => 0,
    })
}

/// A query's first column, as text.
pub(crate) fn strings(rows: Vec<Value>) -> impl Iterator<Item = String> {
    rows.into_iter().filter_map(|row| match row {
        Value::Record(row) => row.fields.into_iter().next().and_then(|value| match value {
            Value::String(text) => Some(text),
            _ => None,
        }),
        _ => None,
    })
}

/// Reads a JSON value from `kv`.
pub(crate) async fn kv_get<T: DeserializeOwned>(
    exec: &mut dyn Executor,
    key: &str,
) -> Result<Option<T>> {
    let row = KvRow::filter_by_key(key).first().exec(exec).await?;
    Ok(row.and_then(|row| match serde_json::from_str(&row.value) {
        Ok(value) => Some(value),
        Err(error) => {
            log::warn!("ignoring unreadable kv {key:?}: {error}");
            None
        }
    }))
}

/// Writes a JSON value to `kv`.
pub(crate) async fn kv_set<T: Serialize>(
    exec: &mut dyn Executor,
    key: &str,
    value: &T,
) -> Result<()> {
    let value = serde_json::to_string(value).expect("kv values serialize");
    KvRow::upsert_by_key(key).value(value).exec(exec).await?;
    Ok(())
}

/// The lowest list date `list` may show without a gap: as far back as the list's own backfill or
/// All Mail's reaches, whichever is further. `i64::MIN` once either is complete, since then every
/// thread is stored; `None` before either has started. A split only ever holds inbox threads, so
/// the Inbox's backfill covers it too.
pub(crate) async fn coverage(exec: &mut dyn Executor, list: &ListId) -> Result<Option<i64>> {
    // Every scheduled send and reminder is Exo's own, so stored.
    if matches!(list, ListId::Scheduled | ListId::Reminders) {
        return Ok(Some(i64::MIN));
    }
    let mut ids = vec![list.key(), SH_ALL];
    if matches!(list, ListId::Split(_)) {
        ids.push(INBOX);
    }
    let cursors = ListCursor::filter(ListCursor::fields().id().in_list(ids))
        .exec(exec)
        .await?;
    Ok(cursors
        .iter()
        .map(|cursor| {
            if cursor.complete {
                i64::MIN
            } else {
                cursor.min_sort.unwrap_or(i64::MAX)
            }
        })
        .min())
}
