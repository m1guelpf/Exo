//! The store handle: opening an account, reads, and the shared plumbing the outbox and sync
//! use (transactions, change notification, recomputing list membership).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use gmail::{Client, Endpoints, TokenProvider};
use toasty::{Db, Executor, Transaction};
use tokio::sync::{Mutex, MutexGuard, watch};

use crate::api::{Api, Failure};
use crate::connection::{Connection, Debounce, Outage};
use crate::db::{
    self, AttachmentRow, BodyRow, EXO_DRAFTS, IDENTITY, LabelRow, ListCursor, ListEntry,
    MessageRow, OutboxRow, ParticipantEntry, ReminderRow,
};
use crate::error::{Error, Result};
use crate::ids::labels::{INBOX, UNREAD};
use crate::ids::{Filter, ListId, ThreadId};
use crate::model::{Cursor, Label, LabelKind, ListCount, RecentThread, ThreadDetail, ThreadPage};
use crate::quota::{Lane, Quota};
use crate::settings::{self, Settings};
use crate::split::Splits;
use crate::view::{Identity, Router, ThreadView};

/// An account's local mail store: a SQLite cache of Gmail, the sync engine that fills it, and
/// the outbox that sends the user's changes. Cheap to clone; clones share everything.
///
/// Nothing runs in the background. The app drives it:
/// - [`sync`](Self::sync) on a timer (about every 20–30 s while focused, 60 s while visible,
///   5 min in the background), on focus, on network recovery, after
///   [`flush`](Self::flush) sends something, and repeatedly while it reports backfilling;
/// - [`flush`](Self::flush) after [`apply`](Self::apply)/[`undo`](Self::undo) and whenever a
///   flush reports a retry time;
/// - [`load_bodies`](Self::load_bodies) when a thread opens whose bodies aren't stored
///   ([`bodies_stored`](Self::bodies_stored)): sync stores most mail for its rows alone;
///   [`reading`](Self::reading) first;
/// - [`evict`](Self::evict) every day, which lets old mail go;
/// - [`changes`](Self::changes) to know when to re-read lists and threads;
/// - [`check_connection`](Self::check_connection) every so often while
///   [`connection`](Self::connection) says something is down, and
///   [`network_changed`](Self::network_changed) when the OS says the network changed.
#[derive(Clone)]
pub struct Store {
    pub(crate) inner: Arc<Inner>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").finish_non_exhaustive()
    }
}

pub(crate) struct Inner {
    pub db: Db,
    pub api: Api,
    /// The account's address.
    pub account: String,
    /// The clients of its other calendar accounts that are signed in, by address
    /// ([`Store::add_calendar_account`]).
    pub calendar_clients: RwLock<HashMap<String, Client>>,
    /// The calendar account each person's calendar was last read through
    /// ([`Store::attendee_events`]).
    pub attendee_accounts: RwLock<HashMap<String, String>>,
    /// Serializes write transactions (SQLite has one writer; this keeps it from ever being
    /// contended or upgraded mid-transaction).
    write: Mutex<()>,
    /// One sync or backfill page at a time.
    pub sync: Mutex<()>,
    /// Labels and identity were refreshed since this store was opened (or rebuilt).
    pub bootstrapped: AtomicBool,
    /// One flush at a time, so no queue is sent twice at once.
    pub flush: Mutex<()>,
    /// Held across `drafts.send` and by undo send, so an undo either comes before the send or
    /// finds out it went.
    pub sending: Mutex<()>,
    pub identity: RwLock<Identity>,
    /// The account's splits, as last saved.
    splits: RwLock<Arc<Splits>>,
    /// The list on screen, whose background pass goes first in every round.
    pub on_screen: RwLock<Option<ListId>>,
    /// How many of its rows the app has loaded, all of which sync fetches the bodies of.
    pub loaded_rows: std::sync::atomic::AtomicU32,
    /// Where Gmail's parts are kept, once the app says ([`Store::cache_parts`]).
    pub parts: std::sync::OnceLock<crate::parts::PartsCache>,
    /// What works out the search index's visible text, once the app says
    /// ([`Store::index_text_with`]).
    pub index_text: std::sync::OnceLock<crate::IndexText>,
    /// The spans of the calendar read this session (epoch milliseconds), whose events eviction
    /// keeps: the calendar shows them without fetching them again.
    pub calendar_read: std::sync::Mutex<Vec<(i64, i64)>>,
    /// The conversation open on screen, whose bodies eviction leaves alone; a batch of eviction
    /// holds it ([`Store::reading`]).
    pub reading: Mutex<Option<String>>,
    /// Inbox threads whose bodies Gmail wouldn't give this session: they wait to be opened.
    pub skipped_bodies: std::sync::Mutex<HashSet<String>>,
    /// The outbox rows given up on this session, whose mutation never reached Gmail: undoing
    /// one has nothing to take back. Row ids are never reused (`AUTOINCREMENT`), and an
    /// [`Undo`](crate::Undo) lives no longer than the session.
    pub given_up: std::sync::Mutex<HashSet<u64>>,
    changes: watch::Sender<u64>,
    /// How far history is applied ([`Store::sync_progress`]).
    pub progress: watch::Sender<crate::SyncProgress>,
    /// The folder draft attachments' bytes are kept in.
    pub files: PathBuf,
    /// When each draft was last written to Gmail this session, which paces its next write.
    pub written: std::sync::Mutex<HashMap<String, jiff::Timestamp>>,
    /// What Gmail holds of each draft Exo wrote this session: the message the write made and its
    /// [`content`](crate::drafts::content), which a save Gmail already has matches.
    pub acked: std::sync::Mutex<HashMap<String, (String, u64)>>,
    /// How each draft's files stand with Gmail ([`Store::uploads`]).
    pub uploads: watch::Sender<crate::drafts::Uploads>,
    /// How to take back the reminder each held send put on its thread this session, which undo
    /// send does.
    pub given: std::sync::Mutex<HashMap<String, crate::outbox::Undo>>,
    /// One held send's reminder given or taken back at a time, so none is given twice.
    pub giving: Mutex<()>,
    /// [`Options::draft_pace`] and [`Options::draft_files_pace`].
    pub draft_pace: Duration,
    pub draft_files_pace: Duration,
}

/// Tuning knobs. The defaults are what the app runs with: the backoff, Gmail's quota, the offline
/// debounce and the drafts' pace; tests shorten the backoff and the debounce, and turn the drafts'
/// pace off or shorten it.
#[derive(Debug, Clone)]
pub struct Options {
    /// First retry delay; later retries double it (capped at 32 s per call, 120 s in the
    /// outbox).
    pub backoff: Duration,
    /// The quota Gmail calls are paced under; `None` sends them as they come (for a server
    /// that enforces none, like gmail-mock unless asked to).
    pub quota: Option<Quota>,
    /// How long an outage lasts at least before [`Store::connection`] says it's over.
    pub debounce: Debounce,
    /// How soon after a draft is written to Gmail its next edits are, at the earliest: each save
    /// is kept on the device at once, and the latest goes once the pace allows.
    pub draft_pace: Duration,
    /// [`draft_pace`](Self::draft_pace) for a draft with files, which every write uploads again.
    pub draft_files_pace: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            backoff: Duration::from_secs(1),
            quota: Some(Quota::GMAIL),
            debounce: Debounce::default(),
            draft_pace: Duration::from_secs(5),
            draft_files_pace: Duration::from_secs(30),
        }
    }
}

impl Store {
    /// Opens `account`'s store, one SQLite file in `dir`, talking to Gmail at `endpoints`.
    /// Calls to Google are paced under Gmail's quota; anywhere else (gmail-mock) they aren't,
    /// since the mock enforces no quota unless asked to and tests shouldn't wait one out.
    ///
    /// # Errors
    /// When the directory or database can't be created or read.
    pub async fn open(
        dir: impl AsRef<Path>,
        account: &str,
        endpoints: Endpoints,
        tokens: impl TokenProvider + 'static,
    ) -> Result<Self> {
        let quota = (endpoints == Endpoints::google()).then_some(Quota::GMAIL);
        Self::open_with(
            dir,
            account,
            endpoints,
            tokens,
            Options {
                quota,
                ..Options::default()
            },
        )
        .await
    }

    /// [`open`](Self::open) with non-default [`Options`].
    ///
    /// # Errors
    /// When the directory or database can't be created or read.
    pub async fn open_with(
        dir: impl AsRef<Path>,
        account: &str,
        endpoints: Endpoints,
        tokens: impl TokenProvider + 'static,
        options: Options,
    ) -> Result<Self> {
        check_account(account)?;
        let mut db = db::open(&dir.as_ref().join(format!("{account}.sqlite3"))).await?;
        let identity = db::kv_get(&mut db, IDENTITY).await?.unwrap_or_default();
        let splits = load_splits(&mut db).await?;
        Ok(Self {
            inner: Arc::new(Inner {
                db,
                api: Api::new(
                    Client::new(endpoints, tokens),
                    options.backoff,
                    options.quota,
                    options.debounce,
                    dir.as_ref(),
                ),
                account: account.to_owned(),
                calendar_clients: RwLock::default(),
                attendee_accounts: RwLock::default(),
                write: Mutex::new(()),
                sync: Mutex::new(()),
                bootstrapped: AtomicBool::new(false),
                flush: Mutex::new(()),
                sending: Mutex::new(()),
                identity: RwLock::new(identity),
                splits: RwLock::new(Arc::new(splits)),
                on_screen: RwLock::new(None),
                loaded_rows: std::sync::atomic::AtomicU32::new(0),
                parts: std::sync::OnceLock::new(),
                index_text: std::sync::OnceLock::new(),
                calendar_read: std::sync::Mutex::default(),
                reading: Mutex::new(None),
                skipped_bodies: std::sync::Mutex::default(),
                given_up: std::sync::Mutex::default(),
                changes: watch::channel(0).0,
                progress: watch::Sender::new(crate::SyncProgress::default()),
                files: dir.as_ref().join(format!("{account}.files")),
                written: std::sync::Mutex::default(),
                acked: std::sync::Mutex::default(),
                uploads: watch::Sender::new(crate::drafts::Uploads::new()),
                given: std::sync::Mutex::default(),
                giving: Mutex::new(()),
                draft_pace: options.draft_pace,
                draft_files_pace: options.draft_files_pace,
            }),
        })
    }

    /// `account` has a store in `dir`: it has opened on this Mac since it last signed out.
    #[must_use]
    pub fn exists(dir: impl AsRef<Path>, account: &str) -> bool {
        check_account(account).is_ok() && dir.as_ref().join(format!("{account}.sqlite3")).exists()
    }

    /// Deletes `account`'s store in `dir`, its database, its drafts' attachments and any parts
    /// kept beside them, as signing out does. Close every [`Store`] on it first. A store that
    /// isn't there deletes fine.
    ///
    /// # Errors
    /// [`Error::InvalidAccount`], or [`Error::Delete`] for a file the OS wouldn't delete.
    pub fn delete(dir: impl AsRef<Path>, account: &str) -> Result<()> {
        check_account(account)?;
        let dir = dir.as_ref();
        let gone = |path: PathBuf, result: std::io::Result<()>| match result {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(Error::Delete {
                path,
                source: error,
            }),
            _ => Ok(()),
        };
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let file = dir.join(format!("{account}.sqlite3{suffix}"));
            gone(file.clone(), std::fs::remove_file(&file))?;
        }
        // A store kept beside its files keeps its parts there too (gmail-mock's, the app's).
        for folder in ["files", "parts"] {
            let folder = dir.join(format!("{account}.{folder}"));
            gone(folder.clone(), std::fs::remove_dir_all(&folder))?;
        }
        Ok(())
    }

    /// Clears `account`'s cache in `dir` (Clear Cache and Restart): its database is made again,
    /// empty, and the next sync downloads its mail again. What Gmail doesn't have is kept when
    /// the old database still reads: the settings, the drafts (held and scheduled sends and their
    /// files among them), the queued changes, the reminders and the unsubscribes to retry. Close
    /// every [`Store`] on it first.
    ///
    /// # Errors
    /// [`Error::InvalidAccount`], or when the old files can't be moved aside or the new database
    /// made.
    pub async fn clear_cache(dir: impl AsRef<Path>, account: &str) -> Result<()> {
        check_account(account)?;
        db::clear(&dir.as_ref().join(format!("{account}.sqlite3"))).await
    }

    /// The connection to Gmail as the offline bar shows it: what's down, as the last calls found
    /// it. It changes as calls fail and get through, not with [`changes`](Self::changes).
    #[must_use]
    pub fn connection(&self) -> watch::Receiver<Connection> {
        self.inner.api.connection()
    }

    /// Checks whether Gmail answers, with its profile (the app checks every 10 s, 60 s while
    /// hidden, while something is down); an answer brings back what has been down long enough.
    pub async fn check_connection(&self) {
        if let Err(failure) = self.inner.api.ping().await {
            log::debug!("checking Gmail: {}", failure.message());
        }
    }

    /// The OS says the network changed: checks whether Gmail answers, and when nothing does the
    /// connection is [`Outage::Offline`] until something does.
    pub async fn network_changed(&self) {
        if let Err(Failure::Offline(error)) = self.inner.api.ping().await {
            log::info!("the network changed and Gmail can't be reached: {error}");
            self.inner.api.down(Outage::Offline);
        }
    }

    /// Calls a Google API other than Gmail and Calendar (People's contacts) through the store's
    /// retry policy, `attempts` times at most, and tells the connection how it ended, as
    /// Calendar's calls do: no answer means the network is down.
    ///
    /// # Errors
    /// [`Error::Offline`] and the like, as for Gmail's calls.
    pub async fn google<'a, T: serde::de::DeserializeOwned + 'a>(
        &'a self,
        attempts: u32,
        make: impl FnMut(&'a Client) -> gmail::Call<'a, T>,
    ) -> Result<T> {
        Ok(self.inner.api.request(attempts, make).await?)
    }

    /// A revision counter that ticks after every change the UI can see (synced mail, applied
    /// or persisted mutations, undos). Re-read what's on screen when it changes.
    #[must_use]
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.inner.changes.subscribe()
    }

    /// Up to `limit` threads of `list` after `after`, newest first (threads of the same date by
    /// id, the higher first), with pending mutations applied. Only the range backfilled without
    /// gaps is returned; see [`ThreadPage::more_remote`].
    ///
    /// # Errors
    /// On database failures.
    pub async fn list(
        &self,
        list: &ListId,
        after: Option<&Cursor>,
        limit: usize,
    ) -> Result<ThreadPage> {
        self.list_filtered(list, None, &[], &[], after, limit).await
    }

    /// [`list`](Self::list) narrowed by `filter`, with sticky rows: only threads also in the
    /// filter's list, and those in `keep` (a thread marked read in an Unread view stays until the
    /// view changes) as long as they're still in `list`; and the threads in `held` even once
    /// they've left `list`, where they'd sort in it (a conversation unstarred in Starred stays
    /// until the view changes).
    ///
    /// # Errors
    /// On database failures.
    pub async fn list_filtered(
        &self,
        list: &ListId,
        filter: Option<Filter>,
        keep: &[ThreadId],
        held: &[ThreadId],
        after: Option<&Cursor>,
        limit: usize,
    ) -> Result<ThreadPage> {
        let mut db = self.inner.db.clone();
        let Some(bound) = db::coverage(&mut db, list).await? else {
            return Ok(ThreadPage {
                threads: Vec::new(),
                next: None,
                more_remote: true,
            });
        };
        let narrow = filter;
        let fields = ListEntry::fields();
        let mut filter_expr = fields
            .list_id()
            .in_list(list.members())
            .and(fields.sort().ge(bound));
        if let Some(after) = after {
            filter_expr = filter_expr.and(
                fields.sort().lt(after.sort).or(fields
                    .sort()
                    .eq(after.sort)
                    .and(fields.thread_id().lt(&after.thread))),
            );
        }
        let query = ListEntry::filter(filter_expr)
            .order_by((fields.sort().desc(), fields.thread_id().desc()));
        let mut entries = match narrow {
            None => query.limit(limit + 1).exec(&mut db).await?,
            Some(narrow) => {
                let passing: HashSet<String> = ListEntry::filter_by_list_id(narrow.key())
                    .select(fields.thread_id())
                    .exec(&mut db)
                    .await?
                    .into_iter()
                    .chain(keep.iter().map(|id| id.as_str().to_owned()))
                    .collect();
                let mut entries = query.exec(&mut db).await?;
                entries.retain(|entry| passing.contains(&entry.thread_id));
                entries.truncate(limit + 1);
                entries
            }
        };
        let router = self.inner.router();
        // While Gmail can't be reached, the sends waiting for it are listed in Scheduled too,
        // held there like `held`'s.
        let waiting: Vec<ThreadId> =
            if *list == ListId::Scheduled && !self.inner.api.connection().borrow().is_online() {
                crate::drafts::marks(&mut db)
                    .await?
                    .sending
                    .into_iter()
                    .map(ThreadId::new)
                    .collect()
            } else {
                Vec::new()
            };
        let held: Vec<&ThreadId> = held.iter().chain(&waiting).collect();
        if !held.is_empty() {
            let held: Vec<&str> = held.iter().map(|id| id.as_str()).collect();
            let listed: HashSet<String> = ListEntry::filter(
                fields
                    .list_id()
                    .in_list(list.members())
                    .and(fields.thread_id().in_list(held.as_slice())),
            )
            .select(fields.thread_id())
            .exec(&mut db)
            .await?
            .into_iter()
            .collect();
            let gone: Vec<String> = held
                .iter()
                .filter(|id| !listed.contains(**id))
                .map(|id| (*id).to_owned())
                .collect();
            let sort_key = list.members()[0];
            for (thread_id, view) in load_views(&mut db, &gone).await? {
                let sort = view.sort_for(sort_key, &router.me);
                let in_page = after.is_none_or(|after| {
                    sort < after.sort || (sort == after.sort && thread_id < after.thread)
                });
                if sort >= bound && in_page {
                    entries.push(ListEntry {
                        list_id: sort_key.to_owned(),
                        thread_id,
                        sort,
                    });
                }
            }
            entries.sort_by(|a, b| (b.sort, &b.thread_id).cmp(&(a.sort, &a.thread_id)));
            entries.truncate(limit + 1);
        }
        let next = if entries.len() > limit {
            entries.truncate(limit);
            entries.last().map(|entry| Cursor {
                sort: entry.sort,
                thread: entry.thread_id.clone(),
            })
        } else {
            None
        };
        let ids: Vec<String> = entries
            .iter()
            .map(|entry| entry.thread_id.clone())
            .collect();
        let mut views = load_views(&mut db, &ids).await?;
        let messages: Vec<String> = views
            .values()
            .flat_map(|view| view.messages.iter().map(|m| m.row.id.clone()))
            .collect();
        let attachments = AttachmentRow::filter(
            AttachmentRow::fields()
                .message_id()
                .in_list(messages.as_slice()),
        )
        .exec(&mut db)
        .await?;
        Ok(ThreadPage {
            threads: entries
                .iter()
                .filter_map(|entry| {
                    let view = views.get_mut(&entry.thread_id)?;
                    Some(view.summary(list, entry.sort, &router, &attachments))
                })
                .collect(),
            next,
            more_remote: bound != i64::MIN,
        })
    }

    /// How many local threads `list` has, and how many are unread.
    ///
    /// # Errors
    /// On database failures.
    pub async fn count(&self, list: &ListId) -> Result<ListCount> {
        let mut db = self.inner.db.clone();
        let fields = ListEntry::fields();
        let threads: Vec<String> = ListEntry::filter(fields.list_id().in_list(list.members()))
            .select(fields.thread_id())
            .exec(&mut db)
            .await?;
        let unread: HashSet<String> = ListEntry::filter_by_list_id(UNREAD)
            .select(fields.thread_id())
            .exec(&mut db)
            .await?
            .into_iter()
            .collect();
        Ok(ListCount {
            unread: threads.iter().filter(|id| unread.contains(*id)).count(),
            threads: threads.len(),
            exact: db::coverage(&mut db, list).await? == Some(i64::MIN),
        })
    }

    /// A thread with every message, body and attachment, pending mutations applied. `None`
    /// when it isn't stored locally.
    ///
    /// # Errors
    /// On database failures.
    pub async fn thread(&self, id: &ThreadId) -> Result<Option<ThreadDetail>> {
        let mut db = self.inner.db.clone();
        let Some(view) = load_view(&mut db, id.as_str()).await? else {
            return Ok(None);
        };
        // Drafts written elsewhere too: the thread shows them in compose.
        let ids: Vec<String> = view.all_messages().map(|m| m.row.id.clone()).collect();
        let attachments =
            AttachmentRow::filter(AttachmentRow::fields().message_id().in_list(ids.as_slice()))
                .exec(&mut db)
                .await?;
        let bodies = BodyRow::filter(BodyRow::fields().message_id().in_list(ids.as_slice()))
            .exec(&mut db)
            .await?;
        Ok(Some(view.detail(attachments, bodies, &self.inner.me())))
    }

    /// [`Store::thread`] without bodies or attachments: who each message is from and to, its
    /// labels and headers, for what needs no more (making a split from conversations).
    ///
    /// # Errors
    /// On database failures.
    pub async fn thread_headers(&self, id: &ThreadId) -> Result<Option<ThreadDetail>> {
        let mut db = self.inner.db.clone();
        let Some(view) = load_view(&mut db, id.as_str()).await? else {
            return Ok(None);
        };
        Ok(Some(view.detail(Vec::new(), Vec::new(), &self.inner.me())))
    }

    /// Records that Exo wrote the draft message `message`, so it routes and shows as Exo's own:
    /// a new message sits in the Inbox, and a reply's row reads `Draft to …` with the draft's
    /// text. Drafts written anywhere else only appear in Drafts. Gmail gives a draft a new
    /// message id every time it's saved, so compose records each one.
    ///
    /// # Errors
    /// On database failures.
    pub async fn record_draft(&self, message: &str) -> Result<()> {
        self.inner.record_draft(message).await
    }

    /// `email`'s latest conversations in local mail, newest first, at most `limit`: threads
    /// where they sent or received a message, outside drafts, Spam and Trash.
    /// [`fetch_recent_threads`](Self::fetch_recent_threads) brings in older ones from Gmail.
    ///
    /// # Errors
    /// On database failures.
    pub async fn recent_threads(&self, email: &str, limit: usize) -> Result<Vec<RecentThread>> {
        let mut db = self.inner.db.clone();
        let fields = ParticipantEntry::fields();
        let entries = ParticipantEntry::filter(fields.email().eq(email.to_lowercase()))
            .order_by((fields.sort().desc(), fields.thread_id().asc()))
            .limit(limit)
            .exec(&mut db)
            .await?;
        let ids: Vec<String> = entries
            .iter()
            .map(|entry| entry.thread_id.clone())
            .collect();
        let views = load_views(&mut db, &ids).await?;
        Ok(entries
            .into_iter()
            .filter_map(|entry| {
                let view = views.get(&entry.thread_id)?;
                Some(RecentThread {
                    subject: view.subject(),
                    id: ThreadId::new(entry.thread_id),
                    date: jiff::Timestamp::from_millisecond(entry.sort).unwrap_or_default(),
                })
            })
            .collect())
    }

    /// The account's send-as aliases, primary first, as of the last sync.
    ///
    /// # Errors
    /// On database failures.
    pub async fn aliases(&self) -> Result<Vec<crate::model::Alias>> {
        let mut db = self.inner.db.clone();
        Ok(db::kv_get(&mut db, db::ALIASES).await?.unwrap_or_default())
    }

    /// The account's Gmail labels, as of the last sync.
    ///
    /// # Errors
    /// On database failures.
    pub async fn labels(&self) -> Result<Vec<Label>> {
        let mut db = self.inner.db.clone();
        Ok(LabelRow::all()
            .exec(&mut db)
            .await?
            .into_iter()
            .map(|row| Label {
                id: row.id,
                name: row.name,
                kind: if row.system {
                    LabelKind::System
                } else {
                    LabelKind::User
                },
                text_color: row.text_color,
                background_color: row.background_color,
                hidden: row.hidden,
            })
            .collect())
    }

    /// Creates the user label `name` (shown in Gmail's label and message lists) and stores it, so
    /// [`labels`](Self::labels) has it at once. Gmail decides what a valid name is.
    ///
    /// # Errors
    /// [`Error::Offline`] without a network; [`Error::Gmail`] when Gmail refuses the name (taken,
    /// reserved or invalid).
    pub async fn create_label(&self, name: &str) -> Result<Label> {
        let request = gmail::Label {
            name: Some(name.to_owned()),
            label_list_visibility: Some(gmail::LabelListVisibility::LabelShow),
            message_list_visibility: Some(gmail::MessageListVisibility::Show),
            ..gmail::Label::default()
        };
        let created = self
            .inner
            .api
            .call(Lane::Act, 3, |gmail| gmail.create_label(&request))
            .await?;
        let label = Label {
            id: created.id.unwrap_or_default(),
            name: created.name.unwrap_or_else(|| name.to_owned()),
            kind: LabelKind::User,
            text_color: None,
            background_color: None,
            hidden: false,
        };
        let mut db = self.inner.db.clone();
        let mut write = self.inner.begin(&mut db).await?;
        LabelRow::create()
            .id(&label.id)
            .name(&label.name)
            .system(false)
            .hidden(false)
            .text_color(None::<String>)
            .background_color(None::<String>)
            .exec(write.exec())
            .await?;
        write.publish().await?;
        Ok(label)
    }

    /// The bytes of message `message`'s MIME part `part` (an attachment, or an inline image),
    /// fetched from Gmail with sync's retries. `attachment` is the attachment id the part had
    /// when it was stored: Gmail hands out a new one every time it returns the message, so
    /// when Gmail refuses a stored id (or there is none, because the part's data came inline)
    /// the part is read from the message as Gmail returns it now.
    ///
    /// Someone is waiting on these bytes (an image in a body, a file they clicked), so they're
    /// asked for ahead of sync, and a rate-limit pause that ends within the longest wait a
    /// call sleeps through, often one sync ran into while backfilling, is waited out, as for
    /// every Gmail call.
    ///
    /// # Errors
    /// [`Error::Offline`] without a network, [`Error::Throttled`] while Gmail rate limits the
    /// account for longer, [`Error::MissingPart`] when the message no longer has the part, and
    /// [`Error::Gmail`] for anything else Gmail refuses.
    pub async fn part_data(
        &self,
        message: &str,
        attachment: Option<&str>,
        part: &str,
    ) -> Result<Vec<u8>> {
        self.download_part(message, attachment, part, None).await
    }

    /// [`part_data`](Self::part_data), reporting how far Gmail's answer has come to `progress`
    /// as it arrives (a file being downloaded: its card's bar and its notification). A part kept
    /// already reports nothing.
    ///
    /// # Errors
    /// As [`part_data`](Self::part_data).
    pub async fn download_part(
        &self,
        message: &str,
        attachment: Option<&str>,
        part: &str,
        progress: Option<gmail::Progress>,
    ) -> Result<Vec<u8>> {
        if let Some(kept) = self.cached_part(message, part) {
            return Ok(kept);
        }
        let data = self
            .fetch_part(Lane::Act, message, attachment, part, progress)
            .await?;
        self.keep_part(message, part, &data);
        Ok(data)
    }

    /// [`part_data`](Self::part_data) from Gmail, in `lane`; a part read by its attachment id
    /// reports its transfer to `progress`.
    pub(crate) async fn fetch_part(
        &self,
        lane: Lane,
        message: &str,
        attachment: Option<&str>,
        part: &str,
        progress: Option<gmail::Progress>,
    ) -> Result<Vec<u8>> {
        let api = &self.inner.api;
        fn report<'a>(
            call: gmail::Call<'a, gmail::MessagePartBody>,
            progress: Option<&gmail::Progress>,
        ) -> gmail::Call<'a, gmail::MessagePartBody> {
            match progress {
                Some(progress) => call.on_progress(progress.clone()),
                None => call,
            }
        }
        if let Some(attachment) = attachment {
            match api
                .call(lane, 3, |gmail| {
                    report(gmail.attachment(message, attachment), progress.as_ref())
                })
                .await
            {
                Ok(body) => return Ok(body.data.unwrap_or_default()),
                Err(failure) if failure.is_permanent() => {
                    log::info!(
                        "Gmail refused part {part} of {message} by its stored id ({}); fetching the message",
                        failure.message()
                    );
                }
                Err(failure) => return Err(failure.into()),
            }
        }
        let missing = || Error::MissingPart {
            message: message.to_owned(),
            part: part.to_owned(),
        };
        let fetched = api
            .call(lane, 3, |gmail| {
                gmail.message(message, gmail::Format::Full, &[])
            })
            .await?;
        let body = fetched
            .payload
            .as_ref()
            .and_then(|payload| {
                payload
                    .walk()
                    .find(|candidate| candidate.part_id.as_deref() == Some(part))
            })
            .and_then(|found| found.body.clone())
            .ok_or_else(missing)?;
        match (body.data, body.attachment_id) {
            (Some(data), _) => Ok(data),
            (None, Some(fresh)) => Ok(api
                .call(lane, 3, |gmail| {
                    report(gmail.attachment(message, &fresh), progress.as_ref())
                })
                .await?
                .data
                .unwrap_or_default()),
            (None, None) => Err(missing()),
        }
    }
}

impl Inner {
    /// [`Store::record_draft`].
    pub async fn record_draft(&self, message: &str) -> Result<()> {
        let router = self.router();
        let mut db = self.db.clone();
        let mut write = self.begin(&mut db).await?;
        let mut drafts = exo_drafts(write.exec()).await?;
        if !drafts.insert(message.to_owned()) {
            return write.commit().await;
        }
        db::kv_set(write.exec(), EXO_DRAFTS, &drafts).await?;
        let stored = MessageRow::filter_by_id(message)
            .first()
            .exec(write.exec())
            .await?;
        if let Some(stored) = stored {
            reindex(write.exec(), &stored.thread_id, &router).await?;
        }
        write.publish().await
    }

    pub fn me(&self) -> Identity {
        self.identity
            .read()
            .expect("identity lock is never poisoned")
            .clone()
    }

    /// Who the user is and the account's splits, for routing threads.
    pub fn router(&self) -> Router {
        Router {
            me: self.me(),
            splits: self
                .splits
                .read()
                .expect("splits lock is never poisoned")
                .clone(),
        }
    }

    /// Reads the splits again (after settings or labels change). When `reroute` and the
    /// enabled splits changed, every inbox thread is routed again.
    pub async fn reload_splits(&self, reroute: bool) -> Result<()> {
        let mut db = self.db.clone();
        let splits = Arc::new(load_splits(&mut db).await?);
        let changed = {
            let current = self.splits.read().expect("splits lock is never poisoned");
            current.routed_ids() != splits.routed_ids()
        };
        if !(reroute && changed) {
            *self.splits.write().expect("splits lock is never poisoned") = splits;
            return Ok(());
        }
        // Swapped first, so whatever routes next already sees the new splits.
        *self.splits.write().expect("splits lock is never poisoned") = splits;
        let router = self.router();
        let threads: Vec<String> = ListEntry::filter_by_list_id(INBOX)
            .select(ListEntry::fields().thread_id())
            .exec(&mut db)
            .await?;
        let mut write = self.begin(&mut db).await?;
        for thread in &threads {
            reindex(write.exec(), thread, &router).await?;
        }
        // The Inbox, Important and Other searches subtract the splits, so a page token from the
        // old search can't continue the new one: their passes start again from the top, keeping
        // how far back they've covered.
        let inbox_lists = [ListId::Inbox, ListId::Important, ListId::Other];
        let keys: Vec<&str> = inbox_lists.iter().map(ListId::key).collect();
        ListCursor::filter(
            ListCursor::fields()
                .id()
                .in_list(keys)
                .and(ListCursor::fields().complete().eq(false)),
        )
        .update()
        .page_token(None::<String>)
        .seen(0)
        .exec(write.exec())
        .await?;
        write.publish().await
    }

    /// Tells subscribers to re-read.
    pub fn notify(&self) {
        self.changes.send_modify(|revision| *revision += 1);
    }

    /// Starts a write transaction on `db`, a clone of the store's handle.
    pub async fn begin<'a>(&'a self, db: &'a mut Db) -> Result<Write<'a>> {
        let lock = self.write.lock().await;
        let tx = db.transaction().await?;
        Ok(Write {
            tx,
            _lock: lock,
            inner: self,
        })
    }
}

/// A write transaction. It holds the store's write lock until it ends (SQLite has one writer;
/// taking turns in-process means it's never contended), and rolls back if dropped.
pub(crate) struct Write<'a> {
    tx: Transaction<'a>,
    _lock: MutexGuard<'a, ()>,
    inner: &'a Inner,
}

impl Write<'_> {
    pub fn exec(&mut self) -> &mut dyn Executor {
        &mut self.tx
    }

    /// Commits a change the UI doesn't show (cursors, retry bookkeeping).
    pub async fn commit(self) -> Result<()> {
        self.tx.commit().await?;
        Ok(())
    }

    /// Commits, then tells subscribers to re-read.
    pub async fn publish(self) -> Result<()> {
        let inner = self.inner;
        self.commit().await?;
        inner.notify();
        Ok(())
    }
}

/// A thread as the user sees it: stored messages plus queued mutations. `None` when no
/// message is stored.
pub(crate) async fn load_view(exec: &mut dyn Executor, thread: &str) -> Result<Option<ThreadView>> {
    Ok(load_views(exec, &[thread.to_owned()]).await?.remove(thread))
}

/// [`load_view`] for many threads at once.
pub(crate) async fn load_views(
    exec: &mut dyn Executor,
    threads: &[String],
) -> Result<HashMap<String, ThreadView>> {
    if threads.is_empty() {
        return Ok(HashMap::new());
    }
    let mut rows: HashMap<String, Vec<MessageRow>> = HashMap::new();
    for row in MessageRow::filter(MessageRow::fields().thread_id().in_list(threads))
        .exec(exec)
        .await?
    {
        rows.entry(row.thread_id.clone()).or_default().push(row);
    }
    let mut ops: HashMap<String, Vec<OutboxRow>> = HashMap::new();
    for op in OutboxRow::filter(OutboxRow::fields().queue().in_list(threads))
        .order_by(OutboxRow::fields().id().asc())
        .exec(exec)
        .await?
    {
        ops.entry(op.queue.clone()).or_default().push(op);
    }
    let mut reminders: HashMap<String, ReminderRow> =
        ReminderRow::filter(ReminderRow::fields().thread_id().in_list(threads))
            .exec(exec)
            .await?
            .into_iter()
            .map(|row| (row.thread_id.clone(), row))
            .collect();
    let exo_drafts = exo_drafts(exec).await?;
    let marks = crate::drafts::marks(exec).await?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, mut rows)| {
            // A discarded or sending draft is gone from reads before Gmail hears about it.
            rows.retain(|row| !marks.hidden.contains(&row.id));
            if rows.is_empty() {
                return None;
            }
            let ops = ops.remove(&id).unwrap_or_default();
            let mut view = ThreadView::new(id.clone(), rows, &ops, &exo_drafts);
            for message in &mut view.messages {
                message.failed = marks.failed.get(&message.row.id).copied();
                message.scheduled = marks.scheduled.get(&message.row.id).copied();
            }
            view.sending = marks.sending.contains(&id);
            view.sending_snippet = marks.sending_snippets.get(&id).cloned();
            view.scheduled = marks.scheduled_threads.get(&id).copied();
            view.reminder = reminders.remove(&id);
            Some((id, view))
        })
        .collect())
}

/// The draft messages Exo wrote, which route and show as its own.
async fn exo_drafts(exec: &mut dyn Executor) -> Result<BTreeSet<String>> {
    Ok(db::kv_get(exec, EXO_DRAFTS).await?.unwrap_or_default())
}

/// The account's splits as saved, with the labels they can name.
async fn load_splits(exec: &mut dyn Executor) -> Result<Splits> {
    let settings: Settings = settings::read(exec).await?;
    let labels = LabelRow::all().exec(exec).await?;
    Ok(Splits::new(
        settings.splits().cloned().collect(),
        labels
            .iter()
            .map(|label| (label.id.as_str(), label.name.as_str())),
    ))
}

/// Recomputes a thread's list membership and participants from its messages and queued
/// mutations.
pub(crate) async fn reindex(exec: &mut dyn Executor, thread: &str, router: &Router) -> Result<()> {
    ListEntry::filter_by_thread_id(thread)
        .delete()
        .exec(exec)
        .await?;
    ParticipantEntry::filter_by_thread_id(thread)
        .delete()
        .exec(exec)
        .await?;
    let Some(view) = load_view(exec, thread).await? else {
        return crate::search::unindex(exec, thread).await;
    };
    let messages: Vec<&str> = view.messages.iter().map(|m| m.row.id.as_str()).collect();
    let attachments = AttachmentRow::filter(
        AttachmentRow::fields()
            .message_id()
            .in_list(messages.as_slice()),
    )
    .exec(exec)
    .await?;
    let me = &router.me;
    let lists = router.list_ids(&view, &attachments);
    let mut entries = ListEntry::create_many();
    for list in &lists {
        let sort = view.sort_for(list, me);
        entries = entries.with_item(|entry| entry.list_id(*list).thread_id(thread).sort(sort));
    }
    entries.exec(exec).await?;
    let (emails, sort) = view.participants(me);
    if !emails.is_empty() {
        let mut participants = ParticipantEntry::create_many();
        for email in &emails {
            participants =
                participants.with_item(|entry| entry.email(email).thread_id(thread).sort(sort));
        }
        participants.exec(exec).await?;
    }
    crate::search::index(exec, &view, &lists, &attachments).await
}

/// Refuses account names that would leave the store's folder (`../x`) or hide its files.
fn check_account(account: &str) -> Result<()> {
    if account.is_empty() || account.starts_with('.') || account.contains(['/', '\\', '\0']) {
        return Err(Error::InvalidAccount(account.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::split::{Definitions, SplitInbox};

    #[tokio::test]
    async fn changing_the_splits_restarts_the_inbox_passes() {
        let dir =
            std::env::temp_dir().join(format!("exo-store-split-cursors-{}", std::process::id()));
        let store = Store::open(
            &dir,
            "maya@brightmoor.co",
            gmail::Endpoints::google(),
            gmail::StaticToken("unused".into()),
        )
        .await
        .expect("opens");
        let mut db = store.inner.db.clone();
        for (id, complete) in [(INBOX, false), ("SH_OTHER", true), ("SH_ALL", false)] {
            ListCursor::create()
                .id(id)
                .page_token(Some("page-3".to_owned()))
                .min_sort(Some(42))
                .complete(complete)
                .seen(150)
                .exec(&mut db)
                .await
                .expect("writes");
        }
        let split = SplitInbox::new(
            "GitHub",
            "from:github.com",
            None,
            None,
            Definitions {
                labels: &[],
                me: &[],
            },
        );
        let mut settings = Settings::default();
        settings.add_split(split);
        store.save_settings(&settings).await.expect("saves");
        let cursor = |id: &'static str| {
            let mut db = store.inner.db.clone();
            async move {
                ListCursor::filter_by_id(id)
                    .first()
                    .exec(&mut db)
                    .await
                    .expect("reads")
                    .expect("kept")
            }
        };
        let inbox = cursor(INBOX).await;
        assert_eq!(
            (inbox.page_token, inbox.min_sort, inbox.seen),
            (None, Some(42), 0),
            "from the top, keeping its coverage"
        );
        assert_eq!(
            cursor("SH_OTHER").await.page_token.as_deref(),
            Some("page-3"),
            "a finished pass stays finished"
        );
        assert_eq!(
            cursor("SH_ALL").await.page_token.as_deref(),
            Some("page-3"),
            "All Mail doesn't subtract splits"
        );
        drop(store);
        let _ = std::fs::remove_dir_all(dir);
    }
}
