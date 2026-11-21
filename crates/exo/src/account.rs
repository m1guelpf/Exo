//! One Gmail account's mail, as the UI sees it.
//!
//! The UI is immediate-mode and synchronous; the store is async. An [`Account`] runs the store,
//! its sync and outbox loops and every Gmail call on a tokio runtime of its own. The UI sends it
//! requests (show a list, apply a mutation, undo) that return at once, and reads the latest
//! [`Snapshot`] of the list on screen each frame without waiting; each new snapshot asks egui to
//! repaint. Nothing in `ui()` blocks on the network or the database.
//!
//! Each page showing the account (a tab's) has its own [`Account`]: a handle on
//! the account, which they share, with a view of its own in the account's worker (its list,
//! conversation, search, filter and contact) and its own snapshots ([`Account::view`]). The
//! account closes when its last handle goes.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    panic::AssertUnwindSafe,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use exo_store::{
    Alias, Attachment, AttachmentSource, Connection, DeviceSettings, Draft, DraftAttachment,
    DraftFailure, Filter, GmailPage, Label, ListCount, ListId, Mutation, RECENT_THREADS,
    ScoredContact, Search, SendFailure, SentDraft, Settings, Store, StoredDraft, ThreadDetail,
    ThreadId, Undo,
};
use futures::FutureExt as _;
use gmail::TokenProvider;
use tokio::{
    runtime::Runtime,
    sync::{mpsc, watch},
};

use crate::mailbox::{Chips, RecentThread, ThreadSummary};
use crate::notifications::{Notifier, mail as mail_notifications};
use crate::power::{Power, PowerSource};
use crate::recovery::{self, Restarts};

/// How often to sync ([`sync_every`]): about every 25 s while the window is focused (for want of
/// push); every minute, `FAST`, while it shows the account otherwise and for an account off
/// screen on mains; every five minutes, `MEDIUM`, for one off screen on battery.
const FOCUSED: Duration = Duration::from_secs(25);
const FAST: Duration = Duration::from_secs(60);
const MEDIUM: Duration = Duration::from_secs(300);
/// While Gmail is unreachable, how soon to try again (the connection coming back also syncs at
/// once).
const OFFLINE_RETRY: Duration = Duration::from_secs(5);
/// A sync this long after the last one that reached Gmail follows a gap (asleep, offline): of
/// its new mail, only the newest notifies. Twice the slowest pace, off screen on battery.
const NOTIFY_GAP: jiff::SignedDuration = jiff::SignedDuration::from_mins(10);
/// While something is down, how often to check whether it's back: every 10 s while the window
/// shows and every minute while it's hidden.
const CHECK_VISIBLE: Duration = Duration::from_secs(10);
const CHECK_HIDDEN: Duration = Duration::from_secs(60);
/// Rows a list reads at first, and how many more each time it scrolls near its end.
const PAGE: usize = 100;
/// Undos kept for toasts; only the newest toast's can run, but a mutation and its toast can
/// cross in flight.
const UNDOS: usize = 8;
/// How long a remote image may take before the body gives up on it.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);
/// The longest the reminder scheduler sleeps before looking at the clock again: a Mac asleep
/// doesn't advance the timer it sleeps on.
const REMINDER_RECHECK: Duration = Duration::from_secs(60);
/// The most a remote image may weigh: past it the body gives up on the image rather than let a
/// sender fill memory (a 30 s download can be gigabytes).
const MAX_DOWNLOAD: usize = 25 << 20;
/// How long quitting waits for held sends and saved drafts to go out, and signing out for the
/// drafts waiting out their pace.
const QUIT_GRACE: Duration = Duration::from_secs(5);
/// How long signing out waits for the account's tasks to stop before its files go.
const STOP_GRACE: Duration = Duration::from_secs(2);
/// How long after the first sync the store first evicts what it no longer keeps
/// ([`Store::evict`]), out of the launch's way, and how often after.
const EVICT_AFTER: Duration = Duration::from_secs(60);
const EVICT_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// How many conversations stored for their rows alone each pre-render fetches the bodies of, and
/// how often one runs (100 rows every ten minutes), the first a minute after the first sync, out
/// of the launch's way as eviction is.
const PRERENDER: usize = 100;
const PRERENDER_AFTER: Duration = Duration::from_secs(60);
const PRERENDER_EVERY: Duration = Duration::from_secs(10 * 60);

/// Where `email`'s Gmail parts are kept: `~/Library/Caches/Exo/<account>/parts`, a cache macOS
/// may empty, which signing out and Clear Cache delete.
pub(crate) fn parts_cache(email: &str) -> Option<PathBuf> {
    std::env::home_dir().map(|home| home.join("Library/Caches/Exo").join(email).join("parts"))
}

/// Who an account is and how to reach its mail. Sign-in will produce these; until then `main`
/// builds them for a running gmail-mock.
#[derive(Clone)]
pub struct Credentials {
    /// The account's address, which also names its store.
    pub email: String,
    /// Where Gmail is.
    pub endpoints: gmail::Endpoints,
    /// Access tokens for it.
    pub tokens: Arc<dyn TokenProvider>,
}

/// How an account signs its calendar accounts in: Exo's sign-in, which keeps their grants, and
/// the browser it shows Google's consent in.
#[derive(Clone)]
pub struct CalendarAuth {
    pub auth: exo_auth::Auth,
    pub browser: crate::sign_in::Browser,
}

/// A calendar sign-in in flight. Dropping it abandons it, closing its loopback listener.
struct CalendarSignIn {
    task: tokio::task::JoinHandle<()>,
    outcome: tokio::sync::oneshot::Receiver<Result<String, exo_auth::Error>>,
}

impl Drop for CalendarSignIn {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Whether the window shows the account and is in front, which sets how often mail syncs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    /// On screen, the window focused.
    Focused,
    /// On screen, the window visible but not focused.
    Visible,
    /// On screen, the window minimized or hidden.
    Background,
    /// Another account is on screen: no tab shows this one.
    OffScreen,
}

/// Why an account couldn't be opened.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// Its runtime couldn't start.
    #[error("starting the account's runtime: {0}")]
    Runtime(#[from] std::io::Error),
    /// Its store couldn't be opened.
    #[error(transparent)]
    Store(#[from] exo_store::Error),
}

/// Names the undo of one [`Account::apply`] (a toast's Undo).
pub type UndoToken = u64;

/// The list on screen, as the async side last read it.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// The list.
    pub list: ListId,
    /// The filter its rows passed.
    pub filter: Option<Filter>,
    /// Its rows, newest first.
    pub threads: Vec<ThreadSummary>,
    /// How many threads the list has locally.
    pub count: Option<ListCount>,
    /// How many threads the Inbox has locally (the folder drawer's count).
    pub inbox: Option<ListCount>,
    /// How many threads the inbox's tabs have locally: Inbox, Important, Other and each enabled
    /// split.
    pub counts: crate::splits::Counts,
    /// Some thread is muted: the folder drawer lists Muted.
    pub muted: bool,
    /// More rows exist past `threads`, locally or in Gmail.
    pub more: bool,
    /// The list is still being fetched (the first sync, or a backfill of it).
    pub loading: bool,
    /// Every thread of the list is stored: Gmail has none older than the store has.
    pub complete: bool,
    /// The account's labels.
    pub labels: Vec<Label>,
    /// The contact pane's address, lower-cased, and their latest conversations in local mail
    /// (its Mail section), newest first.
    pub contact: Option<(String, Vec<RecentThread>)>,
    /// The open thread, with every message, when it's stored locally.
    pub thread: Option<Arc<ThreadDetail>>,
    /// Of its messages, those stored without their body, which show placeholders until they're
    /// in.
    pub bodiless: Vec<String>,
    /// Every draft compose has saved.
    pub drafts: Vec<StoredDraft>,
    /// The account's send-as aliases, with their signatures.
    pub aliases: Vec<Alias>,
    /// Everyone compose can suggest, once [`Account::load_address_book`] has read them.
    pub address_book: Option<Arc<Vec<ScoredContact>>>,
    /// The search being run, if any.
    pub search: Option<Arc<SearchResults>>,
    /// How many [`Account::apply`] calls the snapshot reflects.
    pub applied: u64,
    /// The connection to Gmail, as the offline bar shows it.
    pub connection: Connection,
    /// What's on its way to Gmail, as the footer counts it ([`Store::uploading`]), once read.
    pub uploading: Option<exo_store::Unsent>,
}

/// What a search has found.
#[derive(Clone, Debug)]
pub struct SearchResults {
    /// The query.
    pub query: String,
    /// The conversations, newest first.
    pub threads: Vec<ThreadSummary>,
    /// More can be loaded past them.
    pub more: bool,
    /// Both the local index and Gmail have answered.
    pub finished: bool,
    /// Gmail couldn't be reached, so there may be more than the local index found.
    pub offline: bool,
}

/// Something the UI should tell the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A mutation (or its undo) couldn't be applied or sent, and its effect is gone.
    Failed {
        /// What the user did.
        mutation: Mutation,
        /// Whether it was the undo that failed.
        undo: bool,
    },
    /// Gmail has a draft's latest version.
    DraftSaved(String),
    /// A held draft went out.
    DraftSent(SentDraft),
    /// Gmail refused to save a draft, or its send failed.
    DraftFailed(DraftFailure),
    /// A draft is waiting for the network.
    DraftsOffline,
    /// Drafts an earlier run saved without some of their files, by id: saving each again now
    /// that Gmail answers fetches them, and writes it to Gmail once it has them all.
    Incomplete(Vec<String>),
    /// Some of a draft's attachments couldn't be read or fetched, so its save left them off.
    AttachmentsFailed {
        /// The draft.
        id: String,
        /// How many.
        count: usize,
    },
    /// An undo send came back: whether the draft was still unsent (held, or failed meanwhile).
    SendTakenBack {
        /// The draft.
        id: String,
        /// It was still held.
        held: bool,
    },
    /// [`Account::create_label`] created it.
    LabelCreated(Label),
    /// [`Account::create_label`] couldn't.
    LabelNotCreated {
        /// Because Gmail was out of reach.
        offline: bool,
    },
    /// What [`Account::ask_unsent`] asked for: what Gmail doesn't have yet.
    Unsent(exo_store::Unsent),
    /// What [`Account::read_thread`] read: the thread, or `None` when it isn't stored.
    Thread {
        /// The thread asked for.
        id: String,
        /// It, with every message.
        detail: Option<Arc<ThreadDetail>>,
    },
}

/// Names one of the account's views ([`Account::view`]).
type ViewId = u64;

/// What a view asks of the worker.
enum ViewRequest {
    Show(ListId),
    Open(Option<ThreadId>),
    More,
    Contact(String),
    /// Run this search (or stop searching), excerpts this many words long.
    Search(Option<String>, i64),
    /// Run this search ahead of being asked for (instant search), excerpts this many words long.
    Preload(String, i64),
    /// Narrow the view's list with this filter, or show all of it.
    Filter(Option<Filter>),
    /// Let go of the list's sticky rows.
    Unstick,
    /// The view's page came on screen: the store's hooks for the list on screen follow it.
    Focus,
}

enum Request {
    /// For one view.
    View(ViewId, ViewRequest),
    /// A new view, publishing its snapshots on this channel.
    AddView(ViewId, watch::Sender<Arc<Snapshot>>),
    RemoveView(ViewId),
    Apply {
        /// The view it was applied from, whose rows it may hold.
        view: ViewId,
        threads: Vec<ThreadId>,
        mutation: Mutation,
        undo: Option<UndoToken>,
    },
    Undo(UndoToken),
    CreateLabel(String),
    /// Read a thread for [`Event::Thread`].
    ReadThread(ThreadId),
    /// Queue a one-click unsubscribe from this link and post it.
    Unsubscribe(String),
    /// Block a sender, or send them to Important or Other; undone with `undo`'s token.
    Sender {
        job: crate::senders::SenderJob,
        undo: UndoToken,
    },
    /// Delete the Gmail filters that block this address or domain.
    Unblock(String),
    /// From the account's own tasks.
    ContactFillFailed(String),
    /// From the sync loop: a round ended, and whether it reached Gmail.
    Synced {
        reached: bool,
    },
    /// From the account's own tasks: the files incomplete drafts lacked were fetched again, but
    /// for these (by uuid), which trying again won't get.
    FilesFetched(Vec<String>),
    Backfilled(ListId),
    /// From the account's own tasks: the open thread's bodies were fetched (or couldn't be).
    BodiesLoaded(ThreadId),
    /// Fetch this thread's bodies, if they aren't stored: the list's cursor rests on it.
    Prefetch(ThreadId),
    /// From the account's own tasks: bringing back snoozed threads with no reminder failed, to
    /// try again at the next sync.
    OrphansFailed,
    /// From the account's own tasks: a thread's bodies were prefetched (or couldn't be).
    Prefetched(ThreadId),
    Refresh,
    /// Boxed: the settings document is several times the other requests' size.
    SaveSettings(Box<Settings>),
    SaveDeviceSettings(DeviceSettings),
    /// Save a draft as `email` with its attachments' bytes added.
    SaveDraft {
        draft: Box<Draft>,
        email: Box<gmail::mime::Email>,
    },
    DiscardDraft(String),
    SendDraft {
        id: String,
        at: jiff::Timestamp,
    },
    /// Hold the draft for its Send later time.
    ScheduleDraft(String),
    CancelSend(String),
    /// Cancel the held send and take back the reminder it put on its thread.
    UndoSend(String),
    /// Give the message just sent as this draft a reminder, or take it off: the held draft's,
    /// else (it went) the thread's.
    RemindSent {
        draft: String,
        thread: Option<String>,
        reminder: Option<exo_store::DraftReminder>,
    },
    RecoverDraft(String),
    /// Write every draft Gmail lacks now, not at its pace: the account is being left.
    WriteDraftsNow,
    /// Write every draft Gmail lacks now, then count what it still doesn't have, for
    /// [`Event::Unsent`].
    Unsent,
    /// Read the address book: stored mail now, Google contacts in the background when they're
    /// missing or stale.
    LoadAddressBook,
    /// From the task that scored stored mail.
    MailContacts(Arc<Vec<ScoredContact>>),
    /// From the task that listed Google contacts and "other contacts".
    GoogleContacts(Arc<Vec<GoogleContact>>),
    /// Quitting: send every draft held for undo now, write the drafts saved on the way out, and
    /// say when done.
    SendHeldNow(tokio::sync::oneshot::Sender<()>),
    /// From the account's own tasks: a page of Gmail's matches for the search with this
    /// number, or `None` when Gmail couldn't be reached.
    SearchPage(u64, Option<GmailPage>),
}

/// One account as one page sees it: a handle on the account (its store and the tasks that keep
/// it in sync, seen from the UI thread) with a view of its own, whose snapshots carry the page's
/// list, conversation, search, filter and contact. Another page of the account has another
/// handle ([`Account::view`]).
pub struct Account {
    shared: Rc<Shared>,
    view: ViewId,
    snapshot: watch::Receiver<Arc<Snapshot>>,
    /// The account's events this handle hasn't read yet ([`Account::events`]).
    events: Rc<RefCell<Vec<Event>>>,
    /// The account's settings, as this handle last saw them.
    settings: Versioned,
}

/// The account's settings as last changed, numbered, so a handle can tell when another page
/// changed them since it last looked.
#[derive(Clone)]
struct Versioned {
    version: u64,
    settings: Arc<Settings>,
    device: Arc<DeviceSettings>,
}

/// The account's events, read by each of its handles ([`Account::events`]).
struct Events {
    receiver: std::sync::mpsc::Receiver<Event>,
    /// Each handle's events not read yet.
    readers: Vec<std::rc::Weak<RefCell<Vec<Event>>>>,
}

/// The calendar accounts signed in besides the mail account's own.
#[derive(Default)]
struct CalendarAccounts {
    /// Signs them in, once the app says ([`Account::sign_in_calendars_with`]).
    auth: Option<CalendarAuth>,
    /// The ones read, and their tokens, which say when Google refuses a grant.
    tokens: Vec<(String, exo_auth::Tokens)>,
    signing_in: Option<CalendarSignIn>,
}

/// What every handle on an account shares: the account.
struct Shared {
    email: String,
    /// The folder draft attachments' bytes are kept in.
    files: PathBuf,
    /// Wakes the task that writes to Gmail.
    flush: Wake,
    /// Wakes the task that syncs.
    sync_now: Wake,
    /// Wakes the task that checks the connection, to check it now.
    network: Wake,
    /// Tells `network` when the OS's network changes.
    _reachability: Option<crate::reachability::Reachability>,
    /// `Option` so `Drop` and [`Account::discard`] can shut it down without waiting on its
    /// tasks.
    runtime: RefCell<Option<Runtime>>,
    requests: Requests,
    /// The store's revision.
    changes: watch::Receiver<u64>,
    /// The store revision the latest snapshot read.
    published: Arc<AtomicU64>,
    presence: watch::Sender<Presence>,
    /// The Mac's power source, as [`Account::follow_power`] says.
    power: watch::Sender<Power>,
    events: RefCell<Events>,
    loader: Loader,
    /// The UI's copy of the settings: changes apply here at once and are saved behind it.
    settings: RefCell<Versioned>,
    /// How many times [`Account::apply`] was called.
    applies: AtomicU64,
    /// The calendar's data.
    calendar: crate::calendar::CalendarSync,
    /// Bulk actions, queued and running.
    bulk: crate::bulk::Jobs,
    /// Where the account's notifications go, once the app says ([`Account::set_notifier`]).
    notifier: watch::Sender<Option<Notifier>>,
    /// Where one-click unsubscribes may post.
    reach: crate::unsubscribe::Reach,
    /// What time it is for the reminder scheduler: the app's clock, once it has one.
    clock: Arc<std::sync::RwLock<crate::Clock>>,
    /// Wakes the reminder scheduler, to look at the clock now.
    reminders: Wake,
    /// Asks for frames while the account is on screen.
    repaint: Repaint,
    /// The account's loops and worker, stopped before its runtime is (`recovery`).
    restarts: Arc<Restarts>,
    /// Whether its store still works.
    health: Arc<Health>,
    /// How far history is applied.
    progress: watch::Receiver<exo_store::SyncProgress>,
    /// How drafts' files stand with Gmail.
    uploads: watch::Receiver<exo_store::Uploads>,
    calendar_accounts: RefCell<CalendarAccounts>,
    /// The number the next view gets.
    next_view: Cell<ViewId>,
    /// The number the next undo gets, which the worker and the bulk jobs know it by.
    next_undo: Cell<UndoToken>,
}

/// Asks the window for a frame when an account's data changes, while the account is on screen.
/// An account off screen draws nothing, so what it syncs, sends and fetches costs the window no
/// frames.
#[derive(Clone)]
pub(crate) struct Repaint {
    ctx: egui::Context,
    on_screen: Arc<AtomicBool>,
}

impl Repaint {
    /// A frame, if the account is on screen.
    pub(crate) fn request(&self) {
        if self.on_screen.load(Ordering::Relaxed) {
            self.ctx.request_repaint();
        }
    }
}

/// Whether the account's store still works once open, read from its database's errors: a
/// corrupt file or a full disk stops the account at once, failures that may pass (I/O) after five
/// in a row, and the app shows the teardown a store that won't open gets (`teardown`). The
/// worker's reads, which follow every change and flush, and sync's writes find them.
pub(crate) struct Health {
    unusable: std::sync::Mutex<Option<exo_store::Unusable>>,
    /// Failures that may pass, in a row.
    passing: AtomicUsize,
    repaint: Repaint,
}

/// Failures that may pass, in a row, after which the store counts as corrupt.
const PASSING_FAILURES: usize = 5;

impl Health {
    fn new(repaint: Repaint) -> Self {
        Self {
            unusable: std::sync::Mutex::new(None),
            passing: AtomicUsize::new(0),
            repaint,
        }
    }

    /// The store answered.
    fn ok(&self) {
        self.passing.store(0, Ordering::Relaxed);
    }

    /// The store failed with `error`.
    fn failed(&self, error: &exo_store::Error) {
        let unusable = match error.failing() {
            Some(exo_store::Failing::Unusable(unusable)) => unusable,
            Some(exo_store::Failing::Passing)
                if self.passing.fetch_add(1, Ordering::Relaxed) + 1 >= PASSING_FAILURES =>
            {
                exo_store::Unusable::Corrupt
            }
            Some(exo_store::Failing::Passing) | None => return,
        };
        let mut known = self.unusable.lock().expect("health lock is never poisoned");
        if known.is_none() {
            log::error!("the account's store stopped working ({unusable:?}): {error}");
            *known = Some(unusable);
            self.repaint.request();
        }
    }

    fn unusable(&self) -> Option<exo_store::Unusable> {
        *self.unusable.lock().expect("health lock is never poisoned")
    }
}

/// The request channel, counting what's in flight so tests can wait for quiet.
#[derive(Clone)]
struct Requests {
    sender: mpsc::UnboundedSender<Request>,
    busy: Arc<AtomicUsize>,
}

impl Requests {
    fn post(&self, request: Request) {
        self.busy.fetch_add(1, Ordering::SeqCst);
        if self.sender.send(request).is_err() {
            self.handled();
        }
    }

    /// The worker is done with a posted request.
    fn handled(&self) {
        self.busy.fetch_sub(1, Ordering::SeqCst);
    }

    /// Counts a task as busy until the guard drops.
    fn busy(&self) -> Busy {
        self.busy.fetch_add(1, Ordering::SeqCst);
        Busy(self.busy.clone())
    }
}

struct Busy(Arc<AtomicUsize>);

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Wakes one of the account's loops. A wake counts as busy from the call, not from when the
/// loop gets round to it, so the account is never idle between asking for a sync or a flush and
/// its start.
#[derive(Clone)]
struct Wake {
    sender: mpsc::UnboundedSender<Busy>,
    requests: Requests,
}

impl Wake {
    /// A wake, and the loop's end of it.
    fn new(requests: &Requests) -> (Self, Wakes) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (
            Self {
                sender,
                requests: requests.clone(),
            },
            Wakes(Arc::new(tokio::sync::Mutex::new(receiver))),
        )
    }

    fn wake(&self) {
        // Fails only once the loop is gone, and the count goes with the message.
        let _ = self.sender.send(self.requests.busy());
    }
}

/// The woken loop's end of a [`Wake`], shared by the runs of a loop that restarts after a panic
/// (`recovery`).
#[derive(Clone)]
struct Wakes(Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<Busy>>>);

impl Wakes {
    /// Waits for a wake and takes every one made since, which one run of the loop answers: it
    /// counts as busy until the returned guard drops. `None` once nothing can wake it.
    async fn next(&mut self) -> Option<Busy> {
        let mut receiver = self.0.lock().await;
        let busy = receiver.recv().await?;
        while receiver.try_recv().is_ok() {}
        Some(busy)
    }
}

/// The default splits for the account `email` (`SplitInbox::defaults`): Team covers its domain,
/// unless that's a webmail one.
fn default_splits(email: &str) -> Vec<exo_store::SplitInbox> {
    let me = [email.to_owned()];
    exo_store::SplitInbox::defaults(
        crate::splits::team_domain(email).as_deref(),
        exo_store::Definitions {
            labels: &[],
            me: &me,
        },
    )
}

impl Account {
    /// Opens `credentials`' store in `dir` and starts syncing it; `ctx` repaints on new data while
    /// the account is on screen ([`Account::set_on_screen`]), which it is to start with.
    ///
    /// # Errors
    /// When the store can't be opened, or the runtime can't start.
    pub fn open(
        dir: &Path,
        credentials: Credentials,
        ctx: egui::Context,
    ) -> Result<Self, OpenError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("exo-account")
            .enable_all()
            .build()?;
        let Credentials {
            email,
            endpoints,
            tokens,
        } = credentials;
        let reach = crate::unsubscribe::Reach::of(&endpoints);
        // Remote content may come from this Mac only when Gmail does too: gmail-mock's images.
        let local_gmail = crate::remote_content::serves_gmail_locally(&endpoints);
        let store = runtime.block_on(Store::open(dir, &email, endpoints, tokens))?;
        // On gmail-mock, beside the store: a test's or a screenshot's cache is its own.
        let parts = if local_gmail {
            Some(dir.join(format!("{email}.parts")))
        } else {
            parts_cache(&email)
        };
        if let Some(parts) = parts {
            store.cache_parts(exo_store::PartsCache::new(parts));
        }
        // The search index holds what the thread view shows above the `…`.
        store.index_text_with(Arc::new(crate::body::visible_texts));
        // Read before the first frame, so it already has the account's theme.
        let (mut settings, device_settings): (Settings, DeviceSettings) =
            runtime.block_on(async {
                Ok::<_, exo_store::Error>((store.settings().await?, store.settings().await?))
            })?;
        if settings.set_up_splits(|| default_splits(&email)) {
            runtime.block_on(store.save_settings(&settings))?;
        }

        let (sender, receiver) = mpsc::unbounded_channel();
        let requests = Requests {
            sender,
            busy: Arc::new(AtomicUsize::new(0)),
        };
        let (snapshot_sender, snapshot) = watch::channel(Arc::new(Snapshot {
            list: ListId::Inbox,
            filter: None,
            threads: Vec::new(),
            count: None,
            inbox: None,
            counts: crate::splits::Counts::new(),
            muted: false,
            more: false,
            loading: true,
            complete: false,
            labels: Vec::new(),
            contact: None,
            thread: None,
            bodiless: Vec::new(),
            drafts: Vec::new(),
            aliases: Vec::new(),
            address_book: None,
            search: None,
            applied: 0,
            connection: Connection::default(),
            uploading: None,
        }));
        let (presence, presence_receiver) = watch::channel(Presence::Focused);
        let (power, power_receiver) = watch::channel(Power::Plugged);
        let (notifier, notifier_receiver) = watch::channel(None);
        let (event_sender, events) = std::sync::mpsc::channel();
        let (flush, flushes) = Wake::new(&requests);
        let (sync_now, syncs) = Wake::new(&requests);
        let (synced, first_sync) = watch::channel(false);
        let (reminders, reminder_wakes) = Wake::new(&requests);
        let clock = Arc::new(std::sync::RwLock::new(crate::Clock::system()));
        let (network, networks) = Wake::new(&requests);
        let changes = store.changes();
        let published = Arc::new(AtomicU64::new(*changes.borrow()));
        // The account's background, restarted after a panic (`recovery`).
        let restarts = Arc::new(Restarts::new(ctx.clone()));
        let repaint = Repaint {
            ctx,
            on_screen: Arc::new(AtomicBool::new(true)),
        };
        let health = Arc::new(Health::new(repaint.clone()));

        let guard = runtime.enter();
        let files = store.draft_files().to_owned();
        let progress = store.sync_progress();
        let uploads = store.uploads();
        let pruning = store.clone();
        tokio::spawn(async move {
            if let Err(error) = pruning.prune_draft_files().await {
                log::warn!("pruning draft files: {error}");
            }
        });
        sync_now.wake();
        // What an earlier run left unsent goes now: a send held when Exo crashed, or when
        // Google signed the account out.
        flush.wake();
        // A loop that runs again after a panic starts as at opening, woken: its timers went
        // with it.
        recovery::spawn(&restarts, {
            let (store, requests, presence, power, notify, sync_now, health) = (
                store.clone(),
                requests.clone(),
                presence_receiver.clone(),
                power_receiver,
                (email.clone(), notifier_receiver.clone()),
                sync_now.clone(),
                health.clone(),
            );
            move || {
                sync_now.wake();
                let pace = Pace {
                    presence: presence.clone(),
                    power: power.clone(),
                };
                sync_loop(
                    store.clone(),
                    requests.clone(),
                    pace,
                    syncs.clone(),
                    notify.clone(),
                    synced.clone(),
                    health.clone(),
                )
            }
        });
        recovery::spawn(&restarts, {
            let (store, first_sync) = (store.clone(), first_sync.clone());
            move || evict_loop(store.clone(), first_sync.clone())
        });
        recovery::spawn(&restarts, {
            let (store, requests, first_sync) =
                (store.clone(), requests.clone(), first_sync.clone());
            move || prerender_loop(store.clone(), requests.clone(), first_sync.clone())
        });
        recovery::spawn(&restarts, {
            let (store, clock, flush, notify) = (
                store.clone(),
                clock.clone(),
                flush.clone(),
                (email.clone(), notifier_receiver.clone()),
            );
            move || {
                reminder_loop(
                    store.clone(),
                    clock.clone(),
                    first_sync.clone(),
                    reminder_wakes.clone(),
                    flush.clone(),
                    notify.clone(),
                )
            }
        });
        recovery::spawn(&restarts, {
            let (store, email) = (store.clone(), email.clone());
            move || {
                crate::notifications::reminders::run(
                    store.clone(),
                    email.clone(),
                    notifier_receiver.clone(),
                )
            }
        });
        recovery::spawn(&restarts, {
            let (store, requests, sync_now, events, repaint, flush) = (
                store.clone(),
                requests.clone(),
                sync_now.clone(),
                event_sender.clone(),
                repaint.clone(),
                flush.clone(),
            );
            move || {
                flush.wake();
                flush_loop(
                    store.clone(),
                    requests.clone(),
                    flushes.clone(),
                    sync_now.clone(),
                    events.clone(),
                    repaint.clone(),
                    reach,
                )
            }
        });
        recovery::spawn(&restarts, {
            let (store, presence, sync_now, flush) = (
                store.clone(),
                presence_receiver.clone(),
                sync_now.clone(),
                flush.clone(),
            );
            move || {
                connection_loop(
                    store.clone(),
                    presence.clone(),
                    networks.clone(),
                    sync_now.clone(),
                    flush.clone(),
                )
            }
        });
        let reachability = crate::reachability::Reachability::watch({
            let network = network.clone();
            move || network.wake()
        });
        let loader = Loader {
            runtime: runtime.handle().clone(),
            restarts: restarts.clone(),
            store: store.clone(),
            http: crate::remote_content::Client::new(DOWNLOAD_TIMEOUT, local_gmail),
            requests: requests.clone(),
        };
        let calendar = crate::calendar::CalendarSync::start(
            store.clone(),
            loader.clone(),
            presence_receiver,
            repaint.clone(),
            &restarts,
        );
        let bulk = crate::bulk::Jobs::start(store.clone(), repaint.clone(), &restarts);
        let worker = Worker {
            address_book: None,
            mail_contacts: None,
            google_contacts: None,
            listing_google: false,
            incomplete: HashMap::new(),
            fetching_files: false,
            changes: changes.clone(),
            published: published.clone(),
            connection: store.connection(),
            progress: store.sync_progress(),
            uploads: store.uploads(),
            store,
            repaint: repaint.clone(),
            requests: requests.clone(),
            events: event_sender,
            views: HashMap::from([(0, ViewState::new(snapshot_sender))]),
            focused: 0,
            opening: HashSet::new(),
            prefetching: HashSet::new(),
            orphans: false,
            synced: false,
            resaved: false,
            backfilling: HashSet::new(),
            filled: HashSet::new(),
            undos: Vec::new(),
            sender_jobs: Vec::new(),
            applied: 0,
            flush: flush.clone(),
            searches: 0,
            restarts: restarts.clone(),
            health: health.clone(),
        };
        recovery::spawn_once(&restarts, worker.run(receiver));
        requests.post(Request::Refresh);
        drop(guard);

        let shared = Shared {
            email,
            files,
            flush,
            sync_now,
            network,
            _reachability: reachability,
            runtime: RefCell::new(Some(runtime)),
            requests,
            changes,
            published,
            presence,
            power,
            events: RefCell::new(Events {
                receiver: events,
                readers: Vec::new(),
            }),
            loader,
            settings: RefCell::new(Versioned {
                version: 0,
                settings: Arc::new(settings),
                device: Arc::new(device_settings),
            }),
            applies: AtomicU64::new(0),
            calendar,
            bulk,
            notifier,
            reach,
            clock,
            reminders,
            repaint,
            restarts,
            health,
            progress,
            uploads,
            calendar_accounts: RefCell::default(),
            next_view: Cell::new(1),
            next_undo: Cell::new(0),
        };
        Ok(Self::handle(Rc::new(shared), 0, snapshot))
    }

    /// A handle on `shared` for view `view`, whose snapshots come on `snapshot`.
    fn handle(shared: Rc<Shared>, view: ViewId, snapshot: watch::Receiver<Arc<Snapshot>>) -> Self {
        let events = Rc::new(RefCell::new(Vec::new()));
        shared
            .events
            .borrow_mut()
            .readers
            .push(Rc::downgrade(&events));
        let settings = shared.settings.borrow().clone();
        Self {
            shared,
            view,
            snapshot,
            events,
            settings,
        }
    }

    /// Another handle on the account, for another page: a view of its own, on the inbox with
    /// nothing open, whose first snapshot is this one's latest without its list, conversation,
    /// search and contact.
    pub fn view(&self) -> Self {
        let view = self
            .shared
            .next_view
            .replace(self.shared.next_view.get() + 1);
        let first = Snapshot {
            list: ListId::Inbox,
            filter: None,
            threads: Vec::new(),
            count: None,
            more: false,
            loading: true,
            complete: false,
            contact: None,
            thread: None,
            bodiless: Vec::new(),
            search: None,
            ..Snapshot::clone(&self.snapshot())
        };
        let (sender, snapshot) = watch::channel(Arc::new(first));
        self.shared.requests.post(Request::AddView(view, sender));
        Self::handle(self.shared.clone(), view, snapshot)
    }

    /// Posts `request` for this handle's view.
    fn ask(&self, request: ViewRequest) {
        self.shared.requests.post(Request::View(self.view, request));
    }

    /// Takes up the settings another page of the account changed since this one last looked: a
    /// page does as it starts a frame or comes on screen.
    pub fn catch_up(&mut self) {
        let latest = self.shared.settings.borrow();
        if latest.version != self.settings.version {
            self.settings = latest.clone();
        }
    }

    /// Makes this handle's settings the account's.
    fn share_settings(&mut self) {
        let mut latest = self.shared.settings.borrow_mut();
        self.settings.version = latest.version + 1;
        *latest = self.settings.clone();
    }

    /// A number for an undo ([`Account::apply`], [`Account::sender_job`], [`Account::bulk`]),
    /// the account's own: no other page's undo has it.
    pub fn undo_token(&self) -> UndoToken {
        self.shared
            .next_undo
            .replace(self.shared.next_undo.get() + 1)
    }

    /// Signs calendar accounts in with `auth` from now on, and reads the ones the settings list
    /// as signed in, with their grants (from the Keychain, the first time: this can block, as
    /// opening the mail's grant does). One whose grant is gone is listed as signed out; a grant
    /// the settings don't list any more is forgotten.
    pub fn sign_in_calendars_with(&mut self, auth: CalendarAuth) {
        let email = self.shared.email.clone();
        let held = auth
            .auth
            .accounts()
            .into_iter()
            .find(|account| account.email == email)
            .map(|account| account.calendars)
            .unwrap_or_default();
        for calendar in held
            .iter()
            .filter(|calendar| !self.settings().calendar_accounts.contains_key(*calendar))
        {
            if let Err(error) = auth.auth.forget_calendar(&email, calendar) {
                log::warn!("forgetting a calendar account's grant: {error}");
            }
        }
        let listed: Vec<String> = self
            .settings()
            .calendar_accounts
            .iter()
            .filter(|(calendar, account)| **calendar != email && account.authed)
            .map(|(calendar, _)| calendar.clone())
            .collect();
        let mut signed_out = Vec::new();
        let mut read = Vec::new();
        for calendar in listed {
            match auth.auth.calendar_tokens(&email, &calendar) {
                Ok(tokens) if !tokens.needs_sign_in() => {
                    self.shared
                        .calendar
                        .read_account(&calendar, tokens.clone(), false);
                    read.push((calendar, tokens));
                }
                Ok(_) => signed_out.push(calendar),
                Err(error) => {
                    log::warn!("reading a calendar account's grant: {error}");
                    signed_out.push(calendar);
                }
            }
        }
        self.update_settings(|settings| {
            for email in &signed_out {
                if let Some(account) = settings.calendar_accounts.get_mut(email) {
                    account.authed = false;
                }
            }
        });
        let mut calendars = self.calendar_accounts();
        calendars.tokens.extend(read);
        calendars.auth = Some(auth);
    }

    /// The calendar accounts, to change.
    fn calendar_accounts(&self) -> std::cell::RefMut<'_, CalendarAccounts> {
        self.shared.calendar_accounts.borrow_mut()
    }

    /// Starts signing a calendar account in: `hint` names the one asked for (`Sign in again`, an
    /// invite's). Abandons a sign-in in flight. Returns whether it started: a session without
    /// Exo's sign-in can't.
    pub fn sign_in_calendar(&mut self, hint: Option<&str>) -> bool {
        let mut calendars = self.calendar_accounts();
        let Some(CalendarAuth { auth, browser }) = calendars.auth.clone() else {
            return false;
        };
        let (owner, hint, ctx) = (
            self.shared.email.clone(),
            hint.map(str::to_owned),
            self.shared.repaint.ctx.clone(),
        );
        let (sender, outcome) = tokio::sync::oneshot::channel();
        let task = self.shared.loader.runtime.spawn(async move {
            let _ = sender.send(
                auth.sign_in_calendar(&owner, hint.as_deref(), |url| browser(url))
                    .await,
            );
            ctx.request_repaint();
        });
        calendars.signing_in = Some(CalendarSignIn { task, outcome });
        true
    }

    /// The outcome of the calendar sign-in in flight, once it has one. A calendar account
    /// signed in is listed (`authed`) and read, its calendars loading; the mail account's own
    /// address changes nothing.
    pub fn poll_calendar_sign_in(&mut self) -> Option<Result<String, exo_auth::Error>> {
        let mut calendars = self.calendar_accounts();
        let outcome = calendars.signing_in.as_mut()?.outcome.try_recv().ok()?;
        calendars.signing_in = None;
        let auth = calendars.auth.clone();
        drop(calendars);
        if let Ok(email) = &outcome
            && *email != self.shared.email
            && let Some(auth) = auth
        {
            match auth.auth.calendar_tokens(&self.shared.email, email) {
                Ok(tokens) => {
                    let mut calendars = self.calendar_accounts();
                    calendars.tokens.retain(|(known, _)| known != email);
                    calendars.tokens.push((email.clone(), tokens.clone()));
                    drop(calendars);
                    self.shared.calendar.read_account(email, tokens, true);
                    self.update_settings(|settings| {
                        settings
                            .calendar_accounts
                            .entry(email.clone())
                            .or_insert_with(|| exo_store::CalendarAccount {
                                email: email.clone(),
                                ..exo_store::CalendarAccount::default()
                            })
                            .authed = true;
                    });
                }
                Err(error) => return Some(Err(error)),
            }
        }
        Some(outcome)
    }

    /// Lists the calendar accounts whose grant Google refused as signed out (`authed: false`),
    /// and stops reading them, until they sign in again.
    pub fn check_calendar_grants(&mut self) {
        let refused: Vec<String> = {
            let mut calendars = self.calendar_accounts();
            let refused = calendars
                .tokens
                .iter()
                .filter(|(_, tokens)| tokens.needs_sign_in())
                .map(|(email, _)| email.clone())
                .collect::<Vec<_>>();
            calendars
                .tokens
                .retain(|(email, _)| !refused.contains(email));
            refused
        };
        if refused.is_empty() {
            return;
        }
        for email in &refused {
            log::warn!("a calendar account was signed out of Google");
            self.shared.calendar.forget_account(email, false);
        }
        self.update_settings(|settings| {
            for email in &refused {
                if let Some(account) = settings.calendar_accounts.get_mut(email) {
                    account.authed = false;
                }
            }
        });
    }

    /// Removes calendar account `email`: its grant is forgotten, it leaves the settings, and its
    /// calendars and the events only they had go. The mail account's own can't be.
    pub fn remove_calendar_account(&mut self, email: &str) {
        if email == self.shared.email {
            return;
        }
        let mut calendars = self.calendar_accounts();
        if let Some(auth) = &calendars.auth
            && let Err(error) = auth.auth.forget_calendar(&self.shared.email, email)
        {
            log::warn!("forgetting a calendar account's grant: {error}");
        }
        calendars.tokens.retain(|(known, _)| known != email);
        drop(calendars);
        self.shared.calendar.forget_account(email, true);
        self.update_settings(|settings| {
            settings.calendar_accounts.remove(email);
        });
    }

    /// Posts the account's notifications with `notifier` from now on: new mail, calendar
    /// reminders. With none, it posts none.
    pub fn set_notifier(&self, notifier: Option<Notifier>) {
        self.shared.notifier.send_replace(notifier);
    }

    /// Paces the account's syncs off screen by `source` from now on ([`sync_every`]); until then
    /// it's plugged in.
    pub fn follow_power(&self, source: &PowerSource) {
        let (own, mut power) = (self.shared.power.clone(), source.subscribe());
        self.shared.loader.runtime.spawn(async move {
            loop {
                let now = *power.borrow_and_update();
                own.send_if_modified(|current| std::mem::replace(current, now) != now);
                if power.changed().await.is_err() {
                    return;
                }
            }
        });
    }

    /// Has the reminder scheduler look at the clock now: the minute changed, maybe by more
    /// than the scheduler slept (a Mac asleep).
    pub fn check_reminders(&self) {
        self.shared.reminders.wake();
    }

    /// Dates reminders by `clock`, the app's (fixed in screenshots and tests).
    pub fn set_clock(&self, clock: crate::Clock) {
        *self
            .shared
            .clock
            .write()
            .expect("the clock lock is never poisoned") = clock;
    }

    /// The account's address.
    pub fn email(&self) -> &str {
        &self.shared.email
    }

    /// The latest snapshot of the list on screen.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot.borrow().clone()
    }

    /// Shows `list` (snapshots follow it from now on).
    pub fn show(&self, list: ListId) {
        self.ask(ViewRequest::Show(list));
    }

    /// Opens `thread` (snapshots carry it from now on), or closes the open one.
    pub fn open_thread(&self, thread: Option<&str>) {
        self.ask(ViewRequest::Open(thread.map(ThreadId::new)));
    }

    /// Fetches `thread`'s bodies ahead of its opening when they aren't stored: the list's cursor
    /// rests on it.
    pub fn prefetch(&self, thread: &str) {
        self.shared
            .requests
            .post(Request::Prefetch(ThreadId::new(thread)));
    }

    /// Fetches the bytes behind attachments and remote images.
    pub fn loader(&self) -> Loader {
        self.shared.loader.clone()
    }

    /// Reads further down the list on screen (or the search's results), fetching from Gmail past
    /// what's stored.
    pub fn more(&self) {
        self.ask(ViewRequest::More);
    }

    /// Runs `query` (snapshots carry its results from now on, excerpts `snippet_tokens` words
    /// long), or stops searching. Running the query already running keeps its results.
    pub fn search(&self, query: Option<&str>, snippet_tokens: i64) {
        self.ask(ViewRequest::Search(
            query.map(str::to_owned),
            snippet_tokens,
        ));
    }

    /// Runs `query` ahead of being asked for: Gmail's first page of its
    /// matches is fetched, without holding up what's on screen, so that running it with
    /// [`search`](Self::search) shows them at once. One at a time: a newer one replaces it.
    pub fn preload_search(&self, query: &str, snippet_tokens: i64) {
        self.ask(ViewRequest::Preload(query.to_owned(), snippet_tokens));
    }

    /// Narrows the list on screen with `filter` (snapshots follow it from now on), or shows all
    /// of it.
    pub fn filter(&self, filter: Option<Filter>) {
        self.ask(ViewRequest::Filter(filter));
    }

    /// Lets the list's rows that stay after an action took them out of it go
    /// (as entering search does).
    pub fn unstick(&self) {
        self.ask(ViewRequest::Unstick);
    }

    /// Applies `mutation` to `threads`; undo it with `undo`'s token while its toast shows.
    /// Returns the call's number: snapshots whose [`Snapshot::applied`] reaches it show its
    /// effect.
    pub fn apply(&self, threads: &[String], mutation: Mutation, undo: Option<UndoToken>) -> u64 {
        let threads = threads.iter().map(|id| ThreadId::new(id.clone())).collect();
        self.shared.requests.post(Request::Apply {
            view: self.view,
            threads,
            mutation,
            undo,
        });
        self.shared.applies.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Runs `job` (blocks a sender, sends one to Important or Other); [`Account::undo`] with
    /// `undo` undoes it, even while it runs.
    pub fn sender_job(&self, job: crate::senders::SenderJob, undo: UndoToken) {
        self.shared.requests.post(Request::Sender { job, undo });
    }

    /// Deletes the Gmail filters blocking `from`, an address or domain (Blocked Senders'
    /// Unblock); the block list is the caller's.
    pub fn unblock(&self, from: &str) {
        self.shared.requests.post(Request::Unblock(from.to_owned()));
    }

    /// Creates the label `name` in Gmail; [`Event::LabelCreated`] or
    /// [`Event::LabelNotCreated`] follows.
    pub fn create_label(&self, name: &str) {
        self.shared
            .requests
            .post(Request::CreateLabel(name.to_owned()));
    }

    /// Reads thread `id` from the store, for [`Event::Thread`]: what an action on a
    /// conversation not open needs of its messages.
    pub fn read_thread(&self, id: &str) {
        self.shared
            .requests
            .post(Request::ReadThread(ThreadId::new(id.to_owned())));
    }

    /// Unsubscribes with a one-click POST to `url` (RFC 8058), now or, offline, once Gmail can
    /// be reached; a quit meanwhile doesn't lose it.
    pub fn unsubscribe(&self, url: &url::Url) {
        self.shared
            .requests
            .post(Request::Unsubscribe(url.to_string()));
    }

    /// Where one-click unsubscribes may post.
    pub fn reach(&self) -> crate::unsubscribe::Reach {
        self.shared.reach
    }

    /// Undoes the [`Account::apply`], sender job or bulk action that carried `token`.
    pub fn undo(&self, token: UndoToken) {
        if !self.shared.bulk.undo(token) {
            self.shared.requests.post(Request::Undo(token));
        }
    }

    /// Queues a bulk action; [`Account::undo`] with `token` undoes it.
    pub fn bulk(&self, token: UndoToken, request: crate::bulk::Request) {
        self.shared.bulk.submit(token, request);
    }

    /// This session's bulk actions, oldest first.
    pub fn bulk_actions(&self) -> Vec<crate::bulk::Summary> {
        self.shared.bulk.summaries()
    }

    /// Puts `email`'s latest conversations in snapshots from now on (the contact pane's Mail
    /// section), read from local mail and topped up from Gmail once a session.
    pub fn show_contact(&self, email: &str) {
        self.ask(ViewRequest::Contact(email.to_lowercase()));
    }

    /// The network changed: checks now whether Gmail still answers, as the OS's network changing
    /// does ([`exo_store::Store::network_changed`]).
    pub fn network_changed(&self) {
        self.shared.network.wake();
    }

    /// Sets how often to sync; gaining focus syncs at once.
    pub fn set_presence(&self, presence: Presence) {
        let changed = self
            .shared
            .presence
            .send_if_modified(|current| std::mem::replace(current, presence) != presence);
        if changed && presence == Presence::Focused {
            self.shared.sync_now.wake();
        }
    }

    /// Whether the account's mail is on screen, this handle's page showing it: off screen, its
    /// new data asks for no frame. Coming on screen asks for one, for what came meanwhile, and
    /// the store's hooks for the list on screen follow this handle's view.
    pub fn set_on_screen(&self, on_screen: bool) {
        if !self
            .shared
            .repaint
            .on_screen
            .swap(on_screen, Ordering::Relaxed)
            && on_screen
        {
            self.shared.repaint.ctx.request_repaint();
        }
        if on_screen {
            self.ask(ViewRequest::Focus);
        }
    }

    /// What happened since this handle's last call that the user should hear about. Every
    /// handle hears every event, each page acting on its own.
    pub fn events(&self) -> Vec<Event> {
        let mut events = self.shared.events.borrow_mut();
        let Events { receiver, readers } = &mut *events;
        readers.retain(|reader| reader.strong_count() > 0);
        for event in receiver.try_iter() {
            for reader in readers.iter().filter_map(std::rc::Weak::upgrade) {
                reader.borrow_mut().push(event.clone());
            }
        }
        drop(events);
        self.events.take()
    }

    /// Nothing is in flight: no request, sync, flush, backfill or fetch is running or asked for,
    /// and the latest snapshot has read every change to the store. For screenshots and tests.
    pub fn is_idle(&self) -> bool {
        self.shared.requests.busy.load(Ordering::SeqCst) == 0
            && self.shared.published.load(Ordering::SeqCst) == *self.shared.changes.borrow()
            && self.shared.calendar.is_idle()
            && self.shared.bulk.is_idle()
    }

    /// How far Gmail's history is applied this session (the footer's `Updating: k of n`).
    pub fn sync_progress(&self) -> exo_store::SyncProgress {
        *self.shared.progress.borrow()
    }

    /// How draft `id`'s files stand with Gmail (compose's cards); none are up for a draft the
    /// store doesn't know of yet.
    pub fn upload(&self, id: &str) -> exo_store::Upload {
        self.shared
            .uploads
            .borrow()
            .get(id)
            .cloned()
            .unwrap_or_default()
    }

    /// Why the account's store stopped working since it opened, if it did: the app shows the
    /// teardown for it, as for a store that wouldn't open.
    pub fn unusable(&self) -> Option<exo_store::Unusable> {
        self.shared.health.unusable()
    }

    /// The calendar's data.
    pub fn calendar(&self) -> &crate::calendar::CalendarSync {
        &self.shared.calendar
    }

    /// The account's settings.
    pub fn settings(&self) -> &Settings {
        &self.settings.settings
    }

    /// Changes the account's settings: at once for the UI, every page of it, saved in the
    /// background.
    pub fn update_settings(&mut self, change: impl FnOnce(&mut Settings)) {
        self.catch_up();
        let mut settings = Settings::clone(&self.settings.settings);
        change(&mut settings);
        if settings != *self.settings.settings {
            self.shared
                .requests
                .post(Request::SaveSettings(Box::new(settings.clone())));
            self.settings.settings = Arc::new(settings);
            self.share_settings();
        }
    }

    /// Saves `draft` as `email`: locally at once, to Gmail once it has every file and its pace
    /// allows (at most every 5 s while it's typed in, 30 s with files).
    pub fn save_draft(&self, draft: Draft, email: gmail::mime::Email) {
        self.shared.requests.post(Request::SaveDraft {
            draft: Box::new(draft),
            email: Box::new(email),
        });
    }

    /// The folder draft attachments' bytes go in, each named by its uuid.
    pub fn draft_files(&self) -> &Path {
        &self.shared.files
    }

    /// Discards draft `id` (bring it back by saving it again).
    pub fn discard_draft(&self, id: &str) {
        self.shared
            .requests
            .post(Request::DiscardDraft(id.to_owned()));
    }

    /// Holds draft `id` for sending at `at`, or moves its hold there.
    pub fn send_draft(&self, id: &str, at: jiff::Timestamp) {
        self.shared.requests.post(Request::SendDraft {
            id: id.to_owned(),
            at,
        });
    }

    /// Holds draft `id` for its Send later time ([`exo_store::Store::schedule_draft`]).
    pub fn schedule_draft(&self, id: &str) {
        self.shared
            .requests
            .post(Request::ScheduleDraft(id.to_owned()));
    }

    /// Makes failed draft `id` a draft to edit again (`Edit Message`).
    pub fn recover_draft(&self, id: &str) {
        self.shared
            .requests
            .post(Request::RecoverDraft(id.to_owned()));
    }

    /// Writes every draft Gmail lacks to Gmail now, rather than at its pace, as switching away
    /// does, so Gmail web and the phone have them.
    pub fn write_drafts_now(&self) {
        self.shared.requests.post(Request::WriteDraftsNow);
    }

    /// Reads (again) everyone compose can suggest into snapshots: stored mail, scored, and
    /// Google contacts.
    pub fn load_address_book(&self) {
        self.shared.requests.post(Request::LoadAddressBook);
    }

    /// Takes back a held send; [`Event::SendTakenBack`] says whether it was still held.
    pub fn cancel_send(&self, id: &str) {
        self.shared
            .requests
            .post(Request::CancelSend(id.to_owned()));
    }

    /// Undo send: takes back a held send and the reminder it put on its thread, as
    /// [`cancel_send`](Self::cancel_send) answered.
    pub fn undo_send(&self, id: &str) {
        self.shared.requests.post(Request::UndoSend(id.to_owned()));
    }

    /// Gives the message just sent as draft `id` `reminder`, or takes it off (the follow-up
    /// hint's `Set reminder`, and its toast's Undo): while the send is held, the draft and its
    /// thread get it, as at Send; once it's gone, `thread` does, without being marked done.
    pub fn remind_sent(
        &self,
        id: &str,
        thread: Option<String>,
        reminder: Option<exo_store::DraftReminder>,
    ) {
        self.shared.requests.post(Request::RemindSent {
            draft: id.to_owned(),
            thread,
            reminder,
        });
    }

    /// Asks what Gmail doesn't have yet (signing out asks first), once the drafts waiting out
    /// their pace are written; [`Event::Unsent`] answers.
    pub fn ask_unsent(&self) {
        self.shared.requests.post(Request::Unsent);
    }

    /// Stops the account for good without sending what it holds, as signing out does: held
    /// sends, queued changes and the calendar's held changes stay unsent. Its tasks are stopped
    /// when this returns; its store closes once every handle on the account is dropped.
    pub fn discard(&mut self) {
        let runtime = self.shared.runtime.take();
        if let Some(runtime) = runtime {
            runtime.block_on(self.shared.restarts.stop(STOP_GRACE));
            runtime.shutdown_timeout(STOP_GRACE);
        }
    }

    /// This device's settings for the account.
    pub fn device_settings(&self) -> &DeviceSettings {
        &self.settings.device
    }

    /// Changes this device's settings for the account, as [`Account::update_settings`] does.
    pub fn update_device_settings(&mut self, change: impl FnOnce(&mut DeviceSettings)) {
        self.catch_up();
        let mut device = DeviceSettings::clone(&self.settings.device);
        change(&mut device);
        if device != *self.settings.device {
            self.shared
                .requests
                .post(Request::SaveDeviceSettings(device.clone()));
            self.settings.device = Arc::new(device);
            self.share_settings();
        }
    }
}

impl Drop for Account {
    /// Its view goes; the account closes with its last handle ([`Shared`]'s drop).
    fn drop(&mut self) {
        self.shared.requests.post(Request::RemoveView(self.view));
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        // Queued mutations are in the store's outbox and go out on the next launch. Held sends
        // go now rather than wait for one, for a few seconds at most, and so do
        // the drafts the app saved on its way out. So do the calendar's changes still held for
        // Undo; what Calendar can't take now stays queued for the next launch.
        let Some(runtime) = self.runtime.get_mut().take() else {
            return;
        };
        if self.calendar.held() > 0 {
            runtime.block_on(self.calendar.send_now(QUIT_GRACE));
        }
        // Asked even when no snapshot shows a held send: the worker handles requests in turn,
        // so a Send still queued is held, then sent.
        let (requests, flush) = (self.requests.clone(), self.flush.clone());
        let _ = runtime.block_on(async move {
            let (done, wait) = tokio::sync::oneshot::channel();
            requests.post(Request::SendHeldNow(done));
            flush.wake();
            tokio::time::timeout(QUIT_GRACE, wait).await
        });
        runtime.block_on(self.restarts.stop(STOP_GRACE));
        runtime.shutdown_background();
    }
}

/// A sender job's progress, shared with the task running it.
#[derive(Default)]
struct SenderJobState {
    /// What it changed, once it's done.
    done: Option<crate::senders::Done>,
    /// Undo was asked for: the task undoes it as soon as it's done.
    undo: bool,
}

/// The task that owns what's on screen: it handles requests in order and publishes snapshots.
struct Worker {
    store: Store,
    /// What compose suggests from: mail's contacts with Google's merged in.
    address_book: Option<Arc<Vec<ScoredContact>>>,
    /// Stored mail's contacts, as last scored.
    mail_contacts: Option<Arc<Vec<ScoredContact>>>,
    /// Google contacts and when they were listed, once they have been.
    google_contacts: Option<(Arc<Vec<GoogleContact>>, std::time::Instant)>,
    /// A Google listing is under way.
    listing_google: bool,
    /// Drafts whose last save left attachments off, with that save (the message without its
    /// files) to make again once they can be had: `None` when retrying won't get them.
    incomplete: HashMap<String, Option<Box<(Draft, gmail::mime::Email)>>>,
    /// Their files are being fetched again, off this task.
    fetching_files: bool,
    repaint: Repaint,
    requests: Requests,
    changes: watch::Receiver<u64>,
    /// The store revision the latest snapshot read.
    published: Arc<AtomicU64>,
    /// The connection to Gmail, which snapshots carry.
    connection: watch::Receiver<Connection>,
    /// How far history is applied, which the footer reads.
    progress: watch::Receiver<exo_store::SyncProgress>,
    /// How drafts' files stand with Gmail, which compose's cards show.
    uploads: watch::Receiver<exo_store::Uploads>,
    events: std::sync::mpsc::Sender<Event>,
    /// The account's views, one a page: what each one's snapshots carry.
    views: HashMap<ViewId, ViewState>,
    /// The view on screen, which the store's hooks for the list on screen follow.
    focused: ViewId,
    /// Open threads whose bodies are being fetched.
    opening: HashSet<ThreadId>,
    /// Threads whose bodies are being prefetched.
    prefetching: HashSet<ThreadId>,
    /// Snoozed threads with no reminder in this store were brought back, or are being.
    orphans: bool,
    /// The first round of sync finished, so an empty inbox really is empty.
    synced: bool,
    /// A sync round reached Gmail this session, so the drafts an earlier run left without their
    /// files were handed back to be saved again ([`Event::Incomplete`]).
    resaved: bool,
    backfilling: HashSet<ListId>,
    /// Addresses whose conversations were asked of Gmail this session (or are being).
    filled: HashSet<String>,
    undos: Vec<(UndoToken, Undo)>,
    /// Sender jobs by undo token: what each changed once it's done, and whether undo was asked
    /// for before then.
    sender_jobs: Vec<(UndoToken, Arc<std::sync::Mutex<SenderJobState>>)>,
    /// How many apply requests were handled.
    applied: u64,
    flush: Wake,
    /// The number the next search gets.
    searches: u64,
    /// The account's background's failures (`recovery`).
    restarts: Arc<Restarts>,
    /// Whether the store still works.
    health: Arc<Health>,
}

/// What one of the account's views shows (a page's), which its snapshots follow.
struct ViewState {
    snapshot: watch::Sender<Arc<Snapshot>>,
    list: ListId,
    /// The thread open, if one is.
    open: Option<ThreadId>,
    /// How many of the list's rows it reads.
    limit: usize,
    /// Whose conversations snapshots carry.
    contact: Option<String>,
    /// The search being run.
    search: Option<Searching>,
    /// The search run ahead of being asked for, which snapshots don't show.
    preload: Option<Searching>,
    /// The search whose preload's Gmail page is on its way, maybe one since replaced: a
    /// preload asks Gmail only once none is.
    preloading: Option<u64>,
    /// The list's filter.
    filter: Option<Filter>,
    /// Threads the filtered list has shown, which stay until the list or filter changes
    /// (`stick`, for Unread and Starred).
    sticky: Vec<ThreadId>,
    /// Threads starred or unstarred in Starred, which stay even once out of it until the list
    /// changes or an action dismisses them.
    held: Vec<ThreadId>,
}

impl ViewState {
    /// A view on the inbox with nothing open, publishing on `snapshot`.
    fn new(snapshot: watch::Sender<Arc<Snapshot>>) -> Self {
        Self {
            snapshot,
            list: ListId::Inbox,
            open: None,
            limit: PAGE,
            contact: None,
            search: None,
            preload: None,
            preloading: None,
            filter: None,
            sticky: Vec::new(),
            held: Vec::new(),
        }
    }

    /// Whether search `id`'s Gmail page is this view's: its search's, its preload's, or one it
    /// waits on.
    fn searches(&self, id: u64) -> bool {
        self.preloading == Some(id)
            || [&self.search, &self.preload]
                .into_iter()
                .flatten()
                .any(|search| search.id == id)
    }
}

/// A search the worker runs: the store's session, how many results to read, and where Gmail's
/// part of it is.
struct Searching {
    /// Which search it is, for Gmail's pages to find it.
    id: u64,
    session: Search,
    raw: String,
    limit: usize,
    snippet_tokens: i64,
    /// A Gmail page is being fetched.
    fetching: bool,
    /// Gmail couldn't be reached.
    offline: bool,
}

impl Searching {
    /// A new search for `raw`, numbered from `searches`, the account's count.
    fn new(searches: &mut u64, raw: &str, session: Search, snippet_tokens: i64) -> Self {
        *searches += 1;
        Self {
            id: *searches,
            session,
            raw: raw.to_owned(),
            limit: PAGE,
            snippet_tokens,
            fetching: false,
            offline: false,
        }
    }
}

/// Whether two searches are the same list: the same query, one trailing space aside.
fn same_search(a: &str, b: &str) -> bool {
    a.strip_suffix(' ').unwrap_or(a) == b.strip_suffix(' ').unwrap_or(b)
}

/// How many words a search excerpt holds at a window width.
pub fn snippet_tokens(window_width: f32) -> i64 {
    match window_width {
        width if width <= 1200.0 => 10,
        width if width <= 1700.0 => 15,
        width if width <= 2200.0 => 26,
        _ => 35,
    }
}

impl Worker {
    /// Keeps the rows `mutation` stars or unstars in Starred where they are in `view`, the view
    /// it was applied from (starring sticks them and doesn't dismiss them), and lets go of held
    /// rows anything else but marking read or unread acts on: those dismiss a row that's no
    /// longer starred, and unstick it.
    fn hold(&mut self, view: ViewId, threads: &[ThreadId], mutation: &Mutation) {
        match mutation {
            Mutation::Star | Mutation::Unstar => {
                let Some(view) = self
                    .views
                    .get_mut(&view)
                    .filter(|view| view.list == ListId::Starred && view.search.is_none())
                else {
                    return;
                };
                for thread in threads {
                    if !view.held.contains(thread) {
                        view.held.push(thread.clone());
                    }
                }
            }
            Mutation::MarkRead | Mutation::MarkUnread => {}
            _ => {
                for view in self.views.values_mut() {
                    view.held.retain(|id| !threads.contains(id));
                }
            }
        }
    }

    /// Publishes the address book from what's been read so far.
    fn merge_address_book(&mut self) {
        let Some(mail) = &self.mail_contacts else {
            return;
        };
        let book = match &self.google_contacts {
            Some((google, _)) => merge_contacts(mail, google),
            None => mail.to_vec(),
        };
        self.address_book = Some(Arc::new(book));
    }

    async fn run(mut self, mut requests: mpsc::UnboundedReceiver<Request>) {
        loop {
            tokio::select! {
                biased;
                request = requests.recv() => {
                    let Some(request) = request else { return };
                    // The contact pane's person changing only needs their conversations, which
                    // go out at once rather than with a re-read of everything.
                    let reread = !matches!(request, Request::View(_, ViewRequest::Contact(_)));
                    // A request that panics is dropped; those after it are answered (`recovery`).
                    let handled = AssertUnwindSafe(async {
                        self.handle(request).await;
                        if reread {
                            self.publish().await;
                        }
                    })
                    .catch_unwind()
                    .await;
                    self.requests.handled();
                    if handled.is_err() && !self.recovered().await {
                        return;
                    }
                }
                Ok(()) = self.changes.changed() => {
                    let _busy = self.requests.busy();
                    if AssertUnwindSafe(self.publish()).catch_unwind().await.is_err() && !self.recovered().await {
                        return;
                    }
                }
                Ok(()) = self.connection.changed() => {
                    let connection = *self.connection.borrow_and_update();
                    for view in self.views.values() {
                        let mut snapshot = Snapshot::clone(&view.snapshot.borrow());
                        snapshot.connection = connection;
                        view.snapshot.send_replace(Arc::new(snapshot));
                    }
                    self.repaint.request();
                }
                // The footer's `Updating: k of n`.
                Ok(()) = self.progress.changed() => self.repaint.request(),
                // Compose's cards' upload bars.
                Ok(()) = self.uploads.changed() => self.repaint.request(),
            }
        }
    }

    /// After a panic: waits as [`Restarts::failed`] says and returns `true`, or `false` once Exo
    /// gives up.
    async fn recovered(&self) -> bool {
        let Some(wait) = self.restarts.failed() else {
            return false;
        };
        tokio::time::sleep(wait).await;
        true
    }

    async fn handle(&mut self, request: Request) {
        match request {
            Request::View(view, request) => self.handle_view(view, request).await,
            Request::AddView(view, snapshot) => {
                self.views.insert(view, ViewState::new(snapshot));
            }
            Request::RemoveView(view) => {
                self.views.remove(&view);
            }
            Request::BodiesLoaded(thread) => {
                self.opening.remove(&thread);
            }
            Request::Prefetch(thread) => {
                if self.opening.contains(&thread) || !self.prefetching.insert(thread.clone()) {
                    return;
                }
                let (store, requests, busy) = (
                    self.store.clone(),
                    self.requests.clone(),
                    self.requests.busy(),
                );
                self.restarts
                    .spawn_task(&tokio::runtime::Handle::current(), async move {
                        if let Err(error) = store.prefetch(&thread).await {
                            log::debug!(
                                "prefetching the bodies of thread {}: {error}",
                                thread.as_str()
                            );
                        }
                        requests.post(Request::Prefetched(thread));
                        drop(busy);
                    });
            }
            Request::Prefetched(thread) => {
                self.prefetching.remove(&thread);
            }
            Request::OrphansFailed => self.orphans = false,
            Request::SearchPage(id, page) => {
                let Some(view) = self.views.values_mut().find(|view| view.searches(id)) else {
                    return;
                };
                if view.preloading == Some(id) {
                    view.preloading = None;
                }
                if let Some(preload) = view.preload.as_mut().filter(|preload| preload.id == id) {
                    // A search run ahead stops at Gmail's first page until it's asked for.
                    preload.fetching = false;
                    match page {
                        Some(page) => preload.session.add_gmail(page),
                        None => preload.offline = true,
                    }
                    return;
                }
                Self::fetch_preload_page(&self.store, &self.requests, &self.restarts, view);
                let Some(search) = view.search.as_mut().filter(|search| search.id == id) else {
                    return;
                };
                search.fetching = false;
                match page {
                    Some(page) => {
                        search.offline = false;
                        search.session.add_gmail(page);
                        if search.session.gmail_hits() < search.limit {
                            Self::fetch_page(&self.store, &self.requests, &self.restarts, search);
                        }
                    }
                    None => search.offline = true,
                }
            }
            Request::Apply {
                view,
                threads,
                mutation,
                undo,
            } => {
                self.hold(view, &threads, &mutation);
                let result = self.store.apply(&threads, mutation.clone()).await;
                self.applied += 1;
                match result {
                    Ok(applied) => {
                        if let Some(token) = undo.filter(|_| !applied.is_empty()) {
                            self.undos.push((token, applied));
                            let excess = self.undos.len().saturating_sub(UNDOS);
                            self.undos.drain(..excess);
                        }
                        self.flush.wake();
                    }
                    Err(error) => {
                        log::error!("applying {mutation:?}: {error}");
                        let _ = self.events.send(Event::Failed {
                            mutation,
                            undo: false,
                        });
                    }
                }
            }
            Request::CreateLabel(name) => {
                let event = match self.store.create_label(&name).await {
                    Ok(label) => Event::LabelCreated(label),
                    Err(error) => {
                        log::warn!("creating a label: {error}");
                        Event::LabelNotCreated {
                            offline: matches!(error, exo_store::Error::Offline(_)),
                        }
                    }
                };
                let _ = self.events.send(event);
            }
            Request::ReadThread(id) => {
                let detail = match self.store.thread(&id).await {
                    Ok(detail) => detail.map(Arc::new),
                    Err(error) => {
                        log::error!("reading thread {id:?}: {error}");
                        None
                    }
                };
                let _ = self.events.send(Event::Thread {
                    id: id.as_str().to_owned(),
                    detail,
                });
                self.repaint.request();
            }
            Request::Unsubscribe(url) => match self.store.queue_unsubscribe(&url).await {
                Ok(()) => self.flush.wake(),
                Err(error) => log::error!("queueing an unsubscribe: {error}"),
            },
            Request::Sender { job, undo } => {
                let state = Arc::new(std::sync::Mutex::new(SenderJobState::default()));
                self.sender_jobs.push((undo, state.clone()));
                let excess = self.sender_jobs.len().saturating_sub(UNDOS);
                self.sender_jobs.drain(..excess);
                let (store, busy) = (self.store.clone(), self.requests.busy());
                self.restarts
                    .spawn_task(&tokio::runtime::Handle::current(), async move {
                        let _busy = busy;
                        let done = match crate::senders::run(&store, &job).await {
                            Ok(done) => done,
                            Err(error) => return log::error!("a sender job: {error}"),
                        };
                        // Undone while it ran: undo it now.
                        let done = {
                            let mut state =
                                state.lock().expect("sender job lock is never poisoned");
                            if !state.undo {
                                state.done = Some(done);
                                return;
                            }
                            done
                        };
                        if let Err(error) = crate::senders::undo(&store, done).await {
                            log::error!("undoing a sender job: {error}");
                        }
                    });
            }
            Request::Unblock(from) => {
                let (store, busy) = (self.store.clone(), self.requests.busy());
                self.restarts
                    .spawn_task(&tokio::runtime::Handle::current(), async move {
                        let _busy = busy;
                        if let Err(error) = store.delete_block_filters(&from).await {
                            log::error!("unblocking a sender: {error}");
                        }
                    });
            }
            Request::Undo(token) => {
                if let Some(at) = self
                    .sender_jobs
                    .iter()
                    .position(|(candidate, _)| *candidate == token)
                {
                    let (_, state) = self.sender_jobs.remove(at);
                    let done = {
                        let mut state = state.lock().expect("sender job lock is never poisoned");
                        state.undo = true;
                        state.done.take()
                    };
                    if let Some(done) = done {
                        let (store, busy) = (self.store.clone(), self.requests.busy());
                        self.restarts
                            .spawn_task(&tokio::runtime::Handle::current(), async move {
                                let _busy = busy;
                                if let Err(error) = crate::senders::undo(&store, done).await {
                                    log::error!("undoing a sender job: {error}");
                                }
                            });
                    }
                }
                let Some(at) = self
                    .undos
                    .iter()
                    .position(|(candidate, _)| *candidate == token)
                else {
                    return;
                };
                let (_, undo) = self.undos.remove(at);
                match self.store.undo(undo).await {
                    Ok(()) => self.flush.wake(),
                    Err(error) => log::error!("undoing: {error}"),
                }
            }
            // Asked again the next time the pane shows them.
            Request::ContactFillFailed(email) => {
                self.filled.remove(&email);
            }
            Request::Synced { reached } => {
                if !self.synced {
                    self.synced = true;
                    let lists: Vec<(ListId, usize)> = self
                        .views
                        .values()
                        .map(|view| (view.list.clone(), view.limit))
                        .collect();
                    for (list, limit) in lists {
                        self.backfill_if_short(&list, limit).await;
                    }
                }
                if reached {
                    self.fetch_missing_files();
                    self.resave_incomplete().await;
                    self.return_orphaned_snoozes();
                    // Opened while Gmail couldn't be reached: their bodies now.
                    let open: HashSet<ThreadId> = self
                        .views
                        .values()
                        .filter_map(|view| view.open.clone())
                        .collect();
                    for thread in open {
                        self.load_bodies(thread).await;
                    }
                }
            }
            Request::FilesFetched(gone) => {
                self.fetching_files = false;
                for retry in self.incomplete.values_mut() {
                    if retry.as_ref().is_some_and(|retry| {
                        retry
                            .0
                            .record
                            .attachments
                            .iter()
                            .any(|attachment| gone.contains(&attachment.uuid))
                    }) {
                        *retry = None;
                    }
                }
                self.complete_drafts().await;
            }
            Request::Backfilled(list) => {
                self.backfilling.remove(&list);
            }
            Request::Refresh => {}
            Request::SaveSettings(settings) => {
                if let Err(error) = self.store.save_settings(&*settings).await {
                    log::error!("saving settings: {error}");
                }
            }
            Request::SaveDeviceSettings(settings) => {
                if let Err(error) = self.store.save_settings(&settings).await {
                    log::error!("saving device settings: {error}");
                }
            }
            Request::SaveDraft { draft, email } => {
                let id = draft.id.clone();
                let missing = self.save_draft(*draft, *email).await;
                if missing > 0 {
                    let _ = self
                        .events
                        .send(Event::AttachmentsFailed { id, count: missing });
                    self.repaint.request();
                }
            }
            Request::DiscardDraft(id) => {
                // Saving it again would bring it back.
                self.incomplete.remove(&id);
                match self.store.discard_draft(&id).await {
                    Ok(()) => self.flush.wake(),
                    Err(error) => log::error!("discarding draft {id}: {error}"),
                }
            }
            Request::SendDraft { id, .. } if self.incomplete.contains_key(&id) => {
                // Sending would leave files off; the user gets the draft back instead.
                let reason = SendFailure::Other;
                let _ = self
                    .events
                    .send(Event::DraftFailed(DraftFailure::Send { id, reason }));
                self.repaint.request();
            }
            Request::SendDraft { id, at } => match self.store.send_draft(&id, at).await {
                Ok(()) => self.flush.wake(),
                Err(error) => log::error!("sending draft {id}: {error}"),
            },
            Request::ScheduleDraft(id) if self.incomplete.contains_key(&id) => {
                let reason = SendFailure::Other;
                let _ = self
                    .events
                    .send(Event::DraftFailed(DraftFailure::Send { id, reason }));
                self.repaint.request();
            }
            Request::ScheduleDraft(id) => match self.store.schedule_draft(&id).await {
                Ok(()) => self.flush.wake(),
                Err(error) => log::error!("scheduling draft {id}: {error}"),
            },
            Request::RecoverDraft(id) => {
                if let Err(error) = self.store.recover_draft(&id).await {
                    log::error!("recovering draft {id}: {error}");
                }
            }
            Request::RemindSent {
                draft,
                thread,
                reminder,
            } => {
                let held = self
                    .store
                    .remind_draft(&draft, reminder)
                    .await
                    .unwrap_or_else(|error| {
                        log::error!("giving draft {draft} a reminder: {error}");
                        false
                    });
                if held {
                    // Its thread's label changed.
                    self.flush.wake();
                }
                if let Some(thread) = thread.filter(|_| !held) {
                    let mutation = match reminder {
                        Some(reminder) => Mutation::Remind {
                            at: reminder.at,
                            keep_on_reply: reminder.keep_on_reply,
                            snooze: false,
                        },
                        None => Mutation::CancelReminder { to_inbox: false },
                    };
                    match self.store.apply(&[ThreadId::new(thread)], mutation).await {
                        Ok(_) => self.flush.wake(),
                        Err(error) => log::error!("reminding a sent thread: {error}"),
                    }
                }
            }
            Request::CancelSend(id) => {
                let held = self.store.cancel_send(&id).await.unwrap_or_else(|error| {
                    log::error!("taking back draft {id}: {error}");
                    false
                });
                let _ = self.events.send(Event::SendTakenBack { id, held });
                self.repaint.request();
            }
            Request::UndoSend(id) => {
                let held = self.store.undo_send(&id).await.unwrap_or_else(|error| {
                    log::error!("taking back draft {id}: {error}");
                    false
                });
                // Its reminder's label comes off.
                self.flush.wake();
                let _ = self.events.send(Event::SendTakenBack { id, held });
                self.repaint.request();
            }
            Request::WriteDraftsNow => {
                let (store, events, flush, busy) = (
                    self.store.clone(),
                    self.events.clone(),
                    self.flush.clone(),
                    self.requests.busy(),
                );
                self.restarts
                    .spawn_task(&tokio::runtime::Handle::current(), async move {
                        write_drafts_now(&store, &events, &flush).await;
                        drop(busy);
                    });
            }
            Request::Unsent => {
                // Signing out: drafts that only wait out their pace go first, so what's counted is
                // what Gmail can't take. Past the grace the write goes on, and what it hasn't
                // written yet counts.
                let (store, events, flush, repaint, busy) = (
                    self.store.clone(),
                    self.events.clone(),
                    self.flush.clone(),
                    self.repaint.clone(),
                    self.requests.busy(),
                );
                self.restarts
                    .spawn_task(&tokio::runtime::Handle::current(), async move {
                        let write = tokio::spawn({
                            let (store, events) = (store.clone(), events.clone());
                            async move { write_drafts_now(&store, &events, &flush).await }
                        });
                        let _ = tokio::time::timeout(QUIT_GRACE, write).await;
                        let unsent = store.unsent().await.unwrap_or_else(|error| {
                            log::error!("counting unsent changes: {error}");
                            exo_store::Unsent::default()
                        });
                        let _ = events.send(Event::Unsent(unsent));
                        repaint.request();
                        drop(busy);
                    });
            }
            Request::LoadAddressBook => {
                let (store, requests, busy) = (
                    self.store.clone(),
                    self.requests.clone(),
                    self.requests.busy(),
                );
                self.restarts
                    .spawn_task(&tokio::runtime::Handle::current(), async move {
                        match store.address_book().await {
                            Ok(book) => requests.post(Request::MailContacts(Arc::new(book))),
                            Err(error) => log::error!("reading the address book: {error}"),
                        }
                        drop(busy);
                    });
                // Google is slow and changes little: listed once a session, again when stale,
                // and never waited for (nor counted as busy, so the UI's quiet doesn't hang on it).
                let stale = self
                    .google_contacts
                    .as_ref()
                    .is_none_or(|(_, at)| at.elapsed() > GOOGLE_CONTACTS_FRESH);
                if stale && !self.listing_google {
                    self.listing_google = true;
                    let (store, requests) = (self.store.clone(), self.requests.clone());
                    self.restarts
                        .spawn_task(&tokio::runtime::Handle::current(), async move {
                            let contacts = google_contacts(&store).await;
                            requests.post(Request::GoogleContacts(Arc::new(contacts)));
                        });
                }
            }
            Request::MailContacts(contacts) => {
                self.mail_contacts = Some(contacts);
                self.merge_address_book();
            }
            Request::GoogleContacts(contacts) => {
                self.listing_google = false;
                self.google_contacts = Some((contacts, std::time::Instant::now()));
                self.merge_address_book();
            }
            Request::SendHeldNow(done) => {
                match self.store.drafts().await {
                    Ok(drafts) => {
                        let now = jiff::Timestamp::now();
                        // Undo send's holds, not scheduled sends: those wait for their time, or
                        // the next launch.
                        for draft in drafts.iter().filter(|draft| {
                            draft.send_at.is_some_and(|at| at > now)
                                && draft.record.send_later.is_none()
                        }) {
                            if let Err(error) = self.store.send_draft(&draft.id, now).await {
                                log::error!("sending draft {} before quitting: {error}", draft.id);
                            }
                        }
                    }
                    Err(error) => log::error!("reading drafts before quitting: {error}"),
                }
                // The held sends, and the drafts the app saved on its way out, backoffs or not.
                if let Err(error) = self.store.push_drafts_now().await {
                    log::error!("writing drafts before quitting: {error}");
                }
                let _ = done.send(());
            }
        }
    }

    /// Handles `request` for view `id`.
    async fn handle_view(&mut self, id: ViewId, request: ViewRequest) {
        let focused = id == self.focused;
        let Some(view) = self.views.get_mut(&id) else {
            return;
        };
        match request {
            ViewRequest::Focus => {
                self.focused = id;
                self.store.show(&view.list);
                self.store.loaded(view.limit);
                self.store.reading(view.open.as_ref()).await;
            }
            ViewRequest::Show(list) => {
                if focused {
                    self.store.show(&list);
                }
                if list != view.list {
                    view.list = list;
                    view.limit = PAGE;
                    view.sticky.clear();
                    view.held.clear();
                }
                if focused {
                    self.store.loaded(view.limit);
                }
                let (list, limit) = (view.list.clone(), view.limit);
                self.backfill_if_short(&list, limit).await;
            }
            ViewRequest::Filter(filter) => {
                if filter != view.filter {
                    view.filter = filter;
                    view.sticky.clear();
                    view.held.clear();
                }
            }
            ViewRequest::Unstick => {
                view.sticky.clear();
                view.held.clear();
            }
            ViewRequest::Open(thread) => {
                view.open = thread.clone();
                if focused {
                    self.store.reading(thread.as_ref()).await;
                }
                if let Some(thread) = thread {
                    self.load_bodies(thread).await;
                }
            }
            ViewRequest::More => {
                if let Some(search) = &mut view.search {
                    search.limit += PAGE;
                    Self::fetch_page(&self.store, &self.requests, &self.restarts, search);
                    return;
                }
                view.limit += PAGE;
                if focused {
                    self.store.loaded(view.limit);
                }
                let (list, limit) = (view.list.clone(), view.limit);
                self.backfill_if_short(&list, limit).await;
            }
            ViewRequest::Search(None, _) => {
                view.search = None;
                view.preload = None;
            }
            ViewRequest::Search(Some(raw), snippet_tokens) => {
                if view.search.as_ref().is_some_and(|search| search.raw == raw) {
                    return;
                }
                // The search run ahead for it shows what it has at once; any other is dropped.
                let search = match view
                    .preload
                    .take()
                    .filter(|preload| same_search(&preload.raw, &raw))
                {
                    Some(mut preload) => {
                        preload.session.ask();
                        Searching {
                            raw,
                            snippet_tokens,
                            ..preload
                        }
                    }
                    None => {
                        Searching::new(&mut self.searches, &raw, Search::new(&raw), snippet_tokens)
                    }
                };
                let fetch = search.session.gmail_hits() < search.limit;
                let search = view.search.insert(search);
                if fetch {
                    Self::fetch_page(&self.store, &self.requests, &self.restarts, search);
                }
            }
            ViewRequest::Preload(raw, snippet_tokens) => {
                let asked = |search: &Option<Searching>| {
                    search
                        .as_ref()
                        .is_some_and(|search| same_search(&search.raw, &raw))
                };
                if asked(&view.search) || asked(&view.preload) {
                    return;
                }
                // Like a search run for the screen, it reads the local index as it starts: what
                // that finds shows as found there, not as Gmail's hits.
                let mut preload = Searching::new(
                    &mut self.searches,
                    &raw,
                    Search::preload(&raw),
                    snippet_tokens,
                );
                if let Err(error) = self
                    .store
                    .search(&mut preload.session, preload.limit, true, snippet_tokens)
                    .await
                {
                    log::warn!("searching ahead: {error}");
                }
                view.preload = Some(preload);
                Self::fetch_preload_page(&self.store, &self.requests, &self.restarts, view);
            }
            ViewRequest::Contact(email) => self.show_contact(id, email).await,
        }
    }

    /// Asks Gmail for `view`'s preloaded search's first page, once no other preload's is on its
    /// way: run ahead as the user types, they'd otherwise pile up behind Gmail's pace.
    fn fetch_preload_page(
        store: &Store,
        requests: &Requests,
        restarts: &Restarts,
        view: &mut ViewState,
    ) {
        let Some(preload) = view
            .preload
            .as_mut()
            .filter(|preload| !preload.offline && preload.session.next_page().is_none())
        else {
            return;
        };
        if view.preloading.is_none() && Self::fetch_page(store, requests, restarts, preload) {
            view.preloading = Some(preload.id);
        }
    }

    /// Fetches `search`'s next page in the background, unless one is on its way or Gmail has
    /// no more; whether it did.
    fn fetch_page(
        store: &Store,
        requests: &Requests,
        restarts: &Restarts,
        search: &mut Searching,
    ) -> bool {
        if search.fetching || search.session.gmail_done() || search.session.query().is_empty() {
            return false;
        }
        search.fetching = true;
        let (store, requests, busy, session, id) = (
            store.clone(),
            requests.clone(),
            requests.busy(),
            search.session.clone(),
            search.id,
        );
        restarts.spawn_task(&tokio::runtime::Handle::current(), async move {
            let page = match store.search_gmail(&session).await {
                Ok(page) => Some(page),
                Err(error) => {
                    log::warn!("searching Gmail: {error}");
                    // Only an unreachable Gmail leaves the results partial; anything else ends
                    // Gmail's part of the search.
                    if matches!(
                        error,
                        exo_store::Error::Offline(_) | exo_store::Error::Throttled { .. }
                    ) {
                        None
                    } else {
                        Some(GmailPage::default())
                    }
                }
            };
            requests.post(Request::SearchPage(id, page));
            drop(busy);
        });
        true
    }

    /// Puts `email`'s latest conversations in snapshots from now on, read from local mail at
    /// once, and asks Gmail for older ones the first time this session (they arrive as a store
    /// change).
    async fn show_contact(&mut self, view: ViewId, email: String) {
        if self.filled.insert(email.clone()) {
            let (store, requests, busy, email) = (
                self.store.clone(),
                self.requests.clone(),
                self.requests.busy(),
                email.clone(),
            );
            self.restarts
                .spawn_task(&tokio::runtime::Handle::current(), async move {
                    if let Err(error) = store.fetch_recent_threads(&email).await {
                        log::warn!("loading a contact's conversations: {error}");
                        requests.post(Request::ContactFillFailed(email));
                    }
                    drop(busy);
                });
        }
        let threads = match self
            .store
            .recent_threads(&email, RECENT_THREADS as usize)
            .await
        {
            Ok(threads) => threads.into_iter().map(RecentThread::new).collect(),
            Err(error) => return log::error!("reading a contact's conversations: {error}"),
        };
        let Some(view) = self.views.get_mut(&view) else {
            return;
        };
        view.contact = Some(email.clone());
        let mut snapshot = Snapshot::clone(&view.snapshot.borrow());
        snapshot.contact = Some((email, threads));
        view.snapshot.send_replace(Arc::new(snapshot));
        self.repaint.request();
    }

    /// Saves `draft` as `email` with its attachments' bytes added, and returns how many couldn't
    /// be had. A draft missing some is kept on the device, not written to Gmail, and
    /// saved again once a sync round that reaches Gmail has fetched them
    /// ([`fetch_missing_files`](Self::fetch_missing_files)), if what kept them away passes.
    async fn save_draft(&mut self, mut draft: Draft, email: gmail::mime::Email) -> usize {
        let mut full = email.clone();
        let missing = attach(&self.store, &mut full, &draft.record.attachments).await;
        if missing.files == 0 {
            self.incomplete.remove(&draft.id);
        } else {
            let again = missing.passing.then(|| Box::new((draft.clone(), email)));
            self.incomplete.insert(draft.id.clone(), again);
        }
        draft.raw = String::from_utf8_lossy(&full.to_rfc822()).into_owned();
        draft.complete = missing.files == 0;
        match self.store.save_draft(draft).await {
            Ok(()) => self.flush.wake(),
            Err(error) => log::error!("saving a draft: {error}"),
        }
        missing.files
    }

    /// Fetches the files incomplete drafts lack into the draft files, in a task of its own (a
    /// fetch can wait out Gmail's rate limiting), then has them saved again
    /// ([`complete_drafts`](Self::complete_drafts)).
    fn fetch_missing_files(&mut self) {
        if self.fetching_files || !self.incomplete.values().any(Option::is_some) {
            return;
        }
        let files = self.store.draft_files().to_owned();
        let wanted: Vec<DraftAttachment> = self
            .incomplete
            .values()
            .flatten()
            .flat_map(|retry| shown(&retry.1, &retry.0.record.attachments))
            .filter(|attachment| !files.join(&attachment.uuid).exists())
            .cloned()
            .collect();
        self.fetching_files = true;
        let (store, requests, busy) = (
            self.store.clone(),
            self.requests.clone(),
            self.requests.busy(),
        );
        self.restarts
            .spawn_task(&tokio::runtime::Handle::current(), async move {
                let mut gone = Vec::new();
                for attachment in wanted {
                    if matches!(file_bytes(&store, &attachment).await, Err(false)) {
                        gone.push(attachment.uuid);
                    }
                }
                requests.post(Request::FilesFetched(gone));
                drop(busy);
            });
    }

    /// The first time a sync round reaches Gmail, hands the drafts an earlier run saved without
    /// some of their files back to the UI to save again (its record makes their message), so
    /// they don't wait for their next edit.
    async fn resave_incomplete(&mut self) {
        if std::mem::replace(&mut self.resaved, true) {
            return;
        }
        let drafts = match self.store.drafts().await {
            Ok(drafts) => drafts,
            Err(error) => return log::error!("reading drafts: {error}"),
        };
        let incomplete: Vec<String> = drafts
            .into_iter()
            .filter(|draft| {
                !draft.complete
                    && draft.failure.is_none()
                    && !self.incomplete.contains_key(&draft.id)
            })
            .map(|draft| draft.id)
            .collect();
        if !incomplete.is_empty() {
            let _ = self.events.send(Event::Incomplete(incomplete));
            self.repaint.request();
        }
    }

    /// Once a session, has the store bring back the snoozed threads it has no reminder for (a new
    /// store's), off this task; a round that fails tries again at the next sync.
    fn return_orphaned_snoozes(&mut self) {
        if std::mem::replace(&mut self.orphans, true) {
            return;
        }
        let (store, requests, flush, busy) = (
            self.store.clone(),
            self.requests.clone(),
            self.flush.clone(),
            self.requests.busy(),
        );
        self.restarts
            .spawn_task(&tokio::runtime::Handle::current(), async move {
                match store.return_orphaned_snoozes().await {
                    Ok(returned) if !returned.is_empty() => flush.wake(),
                    Ok(_) => {}
                    Err(error) => {
                        log::warn!("bringing back snoozed threads: {error}");
                        requests.post(Request::OrphansFailed);
                    }
                }
                drop(busy);
            });
    }

    /// Saves again the incomplete drafts whose files are all in the draft files now: they go to
    /// Gmail, and an account held off screen for them can close.
    async fn complete_drafts(&mut self) {
        let files = self.store.draft_files().to_owned();
        let ready: Vec<(Draft, gmail::mime::Email)> = self
            .incomplete
            .values()
            .flatten()
            .filter(|retry| {
                shown(&retry.1, &retry.0.record.attachments)
                    .iter()
                    .all(|attachment| files.join(&attachment.uuid).exists())
            })
            .map(|retry| (**retry).clone())
            .collect();
        for (draft, email) in ready {
            log::info!("draft {} has its files now", draft.id);
            self.save_draft(draft, email).await;
        }
    }

    /// Fetches an open thread's bodies when they aren't stored (it was stored for its row
    /// alone), unless they're on their way: snapshots carry it meanwhile, its messages without
    /// them as placeholders.
    async fn load_bodies(&mut self, thread: ThreadId) {
        if self.opening.contains(&thread) {
            return;
        }
        match self.store.bodies_stored(&thread).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => return log::error!("reading thread {}: {error}", thread.as_str()),
        }
        self.opening.insert(thread.clone());
        let (store, requests, busy) = (
            self.store.clone(),
            self.requests.clone(),
            self.requests.busy(),
        );
        self.restarts
            .spawn_task(&tokio::runtime::Handle::current(), async move {
                if let Err(error) = store.load_bodies(&thread).await {
                    log::warn!("fetching the bodies of thread {}: {error}", thread.as_str());
                }
                requests.post(Request::BodiesLoaded(thread));
                drop(busy);
            });
    }

    /// Fetches more of `list`, a view's, from Gmail when what's stored doesn't fill `limit`.
    async fn backfill_if_short(&mut self, list: &ListId, limit: usize) {
        // Until the first round of sync, its own backfill fills the inbox lists.
        if !self.synced || self.backfilling.contains(list) {
            return;
        }
        let page = match self.store.list(list, None, limit).await {
            Ok(page) => page,
            Err(error) => return log::error!("reading {list:?}: {error}"),
        };
        if page.next.is_some() || !page.more_remote {
            return;
        }
        let list = list.clone();
        let wanted = limit.saturating_sub(page.threads.len()).max(1);
        self.backfilling.insert(list.clone());
        let (store, requests, busy) = (
            self.store.clone(),
            self.requests.clone(),
            self.requests.busy(),
        );
        self.restarts
            .spawn_task(&tokio::runtime::Handle::current(), async move {
                if let Err(error) = store.backfill(&list, wanted).await {
                    log::warn!("backfilling {list:?}: {error}");
                }
                requests.post(Request::Backfilled(list));
                drop(busy);
            });
    }

    /// Reads what each view shows and publishes it: what's the account's once, then each view's
    /// list, conversation, contact and search.
    async fn publish(&mut self) {
        self.published
            .store(*self.changes.borrow_and_update(), Ordering::SeqCst);
        let store = &self.store;
        let account = async {
            let labels = store.labels().await?;
            let inbox = store.count(&ListId::Inbox).await?;
            let settings: Settings = store.settings().await?;
            let mut counts = crate::splits::Counts::new();
            for list in [ListId::Important, ListId::Other].into_iter().chain(
                settings
                    .enabled_splits()
                    .map(|split| ListId::Split(split.id.clone())),
            ) {
                counts.insert(list.clone(), store.count(&list).await?);
            }
            counts.insert(ListId::Inbox, inbox);
            let muted = !store
                .list(&ListId::Muted, None, 1)
                .await?
                .threads
                .is_empty();
            let drafts = store.drafts().await?;
            let uploading = store.uploading().await?;
            let aliases = store.aliases().await?;
            Ok::<_, exo_store::Error>((labels, inbox, counts, muted, drafts, uploading, aliases))
        };
        let (labels, inbox, counts, muted, drafts, uploading, aliases) = match account.await {
            Ok(read) => read,
            Err(error) => {
                self.health.failed(&error);
                return log::error!("reading the account: {error}");
            }
        };
        let chips = Chips::new(&labels);
        for view in self.views.values_mut() {
            let read = async {
                let page = store
                    .list_filtered(
                        &view.list,
                        view.filter,
                        &view.sticky,
                        &view.held,
                        None,
                        view.limit,
                    )
                    .await?;
                let count = store.count(&view.list).await?;
                let (thread, bodiless) = match &view.open {
                    Some(id) => (store.thread(id).await?, store.bodiless(id).await?),
                    None => (None, Vec::new()),
                };
                let contact = match &view.contact {
                    Some(email) => Some((
                        email.clone(),
                        store.recent_threads(email, RECENT_THREADS as usize).await?,
                    )),
                    None => None,
                };
                let search = match &mut view.search {
                    Some(search) => Some((
                        search.raw.clone(),
                        store
                            .search(
                                &mut search.session,
                                search.limit,
                                !search.offline,
                                search.snippet_tokens,
                            )
                            .await?,
                        search.fetching,
                        search.offline,
                        search.session.gmail_done(),
                    )),
                    None => None,
                };
                Ok::<_, exo_store::Error>((page, count, (thread, bodiless), contact, search))
            };
            let (page, count, (thread, bodiless), contact, search) = match read.await {
                Ok(read) => read,
                Err(error) => {
                    self.health.failed(&error);
                    return log::error!("reading {:?}: {error}", view.list);
                }
            };
            if matches!(view.filter, Some(Filter::Unread | Filter::Starred)) {
                for thread in &page.threads {
                    if !view.sticky.contains(&thread.id) {
                        view.sticky.push(thread.id.clone());
                    }
                }
            }
            let search = search.map(|(query, found, fetching, offline, gmail_done)| {
                Arc::new(SearchResults {
                    query,
                    more: found.more_local || !(gmail_done || offline),
                    finished: !fetching && (gmail_done || offline),
                    offline,
                    threads: found
                        .results
                        .into_iter()
                        .map(|result| {
                            ThreadSummary::search_result(
                                result,
                                &found.labels,
                                &found.names,
                                &chips,
                            )
                        })
                        .collect(),
                })
            });
            let loading =
                self.backfilling.contains(&view.list) || (!self.synced && page.threads.is_empty());
            let snapshot = Snapshot {
                threads: page
                    .threads
                    .into_iter()
                    .map(|thread| ThreadSummary::new(thread, &view.list, &chips))
                    .collect(),
                list: view.list.clone(),
                filter: view.filter,
                count: Some(count),
                inbox: Some(inbox),
                counts: counts.clone(),
                muted,
                more: page.next.is_some() || page.more_remote,
                loading,
                complete: !page.more_remote,
                labels: labels.clone(),
                contact: contact.map(|(email, threads)| {
                    (email, threads.into_iter().map(RecentThread::new).collect())
                }),
                thread: thread.map(Arc::new),
                bodiless,
                drafts: drafts.clone(),
                aliases: aliases.clone(),
                address_book: self.address_book.clone(),
                search,
                applied: self.applied,
                connection: *self.connection.borrow(),
                uploading: Some(uploading),
            };
            view.snapshot.send_replace(Arc::new(snapshot));
        }
        self.health.ok();
        self.repaint.request();
    }
}

/// How much of a part `size` bytes long has arrived, in percent: of the whole answer when Gmail
/// says how long it is, else of the base64 JSON it sends for that size (about 1.33 × the size).
fn percent(transfer: gmail::Transfer, size: u64) -> f32 {
    let total = transfer.total.unwrap_or(4 * size.div_ceil(3) + 32).max(1);
    (100.0 * transfer.done as f32 / total as f32).min(100.0)
}

/// Loads bytes for the UI without blocking it: attachments through Gmail, remote images straight
/// from the sender's server. Cheap to clone; what it has in flight keeps the
/// account from being [idle](Account::is_idle).
#[derive(Clone)]
pub struct Loader {
    runtime: tokio::runtime::Handle,
    /// Stops what it started with the account's loops.
    restarts: Arc<Restarts>,
    store: Store,
    http: crate::remote_content::Client,
    requests: Requests,
}

impl Loader {
    /// Fetches `attachment`'s bytes, retrying the way sync does, and hands them (or why they
    /// couldn't be had) to `done`.
    pub fn attachment(
        &self,
        attachment: &Attachment,
        done: impl FnOnce(Result<Vec<u8>, exo_store::Error>) + Send + 'static,
    ) {
        self.fetch(attachment, None, done);
    }

    /// [`attachment`](Self::attachment), telling `progress` the percentage of Gmail's answer
    /// that has arrived as it comes. A kept part reports nothing.
    pub fn download(
        &self,
        attachment: &Attachment,
        progress: impl Fn(f32) + Send + Sync + 'static,
        done: impl FnOnce(Result<Vec<u8>, exo_store::Error>) + Send + 'static,
    ) {
        let size = u64::from(attachment.size);
        self.fetch(
            attachment,
            Some(Arc::new(move |transfer| progress(percent(transfer, size)))),
            done,
        );
    }

    fn fetch(
        &self,
        attachment: &Attachment,
        progress: Option<gmail::Progress>,
        done: impl FnOnce(Result<Vec<u8>, exo_store::Error>) + Send + 'static,
    ) {
        let (store, attachment, busy) =
            (self.store.clone(), attachment.clone(), self.requests.busy());
        self.restarts.spawn_task(&self.runtime, async move {
            let bytes = store
                .download_part(
                    &attachment.message_id,
                    attachment.attachment_id.as_deref(),
                    &attachment.part_id,
                    progress,
                )
                .await;
            if let Err(error) = &bytes {
                log::warn!(
                    "loading an attachment of {}: {error}",
                    attachment.message_id
                );
            }
            done(bytes);
            drop(busy);
        });
    }

    /// Part `part` of message `message` (an attachment, an inline image) if the account keeps
    /// it, read now without asking Gmail: a draft's or a message's inline image drawn at once,
    /// and offline. [`attachment`](Self::attachment) reads the same cache first.
    pub fn cached_part(&self, message: &str, part: &str) -> Option<Vec<u8>> {
        self.store.cached_part(message, part)
    }

    /// Runs `task` on the account's store off the UI thread and hands what it comes to to
    /// `done`.
    pub(crate) fn run<T: Send + 'static, F: std::future::Future<Output = T> + Send + 'static>(
        &self,
        task: impl FnOnce(Store) -> F + Send + 'static,
        done: impl FnOnce(T) + Send + 'static,
    ) {
        let (store, busy) = (self.store.clone(), self.requests.busy());
        self.restarts.spawn_task(&self.runtime, async move {
            done(task(store).await);
            drop(busy);
        });
    }

    /// Fetches `url` (http or https) from the public internet (`crate::remote_content`) and hands
    /// the body to `done` (`None` on any failure, or past [`MAX_DOWNLOAD`]).
    pub fn url(&self, url: &str, done: impl FnOnce(Option<Vec<u8>>) + Send + 'static) {
        let (http, url, busy) = (self.http.clone(), url.to_owned(), self.requests.busy());
        self.restarts.spawn_task(&self.runtime, async move {
            let bytes = async {
                let mut response = http.get(&url).await?.error_for_status()?;
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await? {
                    if bytes.len() + chunk.len() > MAX_DOWNLOAD {
                        log::debug!("loading remote content: over {MAX_DOWNLOAD} bytes");
                        return Ok(None);
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok(Some(bytes))
            };
            done(
                bytes
                    .await
                    .unwrap_or_else(|error: crate::remote_content::Error| {
                        log::debug!("loading remote content: {error}");
                        None
                    }),
            );
            drop(busy);
        });
    }
}

/// The files [`attach`] couldn't put on a message.
struct Missing {
    files: usize,
    /// Each was a Gmail part Gmail couldn't hand over just then (out of reach, rate limiting,
    /// failing), so a later try may get it; not a file gone from disk or a part gone from Gmail.
    passing: bool,
}

/// Puts the bytes of a draft's `attachments` on `email` ([`shown`] ones), fetching Gmail
/// parts into the draft files the first time. Says what couldn't be had.
async fn attach(
    store: &Store,
    email: &mut gmail::mime::Email,
    attachments: &[DraftAttachment],
) -> Missing {
    let mut missing = Missing {
        files: 0,
        passing: true,
    };
    for attachment in shown(email, attachments) {
        match file_bytes(store, attachment).await {
            Ok(data) => email.attachments.push(gmail::mime::Attachment {
                filename: attachment.name.clone(),
                content_type: attachment.mime_type.clone(),
                data,
                content_id: attachment.content_id.clone(),
            }),
            Err(passing) => {
                missing.files += 1;
                missing.passing &= passing;
            }
        }
    }
    missing
}

/// Of a draft's `attachments`, those that go on `email`: all but the inline images its HTML no
/// longer shows.
fn shown<'a>(
    email: &gmail::mime::Email,
    attachments: &'a [DraftAttachment],
) -> Vec<&'a DraftAttachment> {
    let html = email.html.as_deref().unwrap_or_default();
    attachments
        .iter()
        .filter(|attachment| {
            attachment
                .content_id
                .as_ref()
                .is_none_or(|cid| html.contains(&format!("cid:{cid}")))
        })
        .collect()
}

/// The bytes of `attachment`, from the draft files, or from Gmail into them the first time.
/// Failing, whether a later try may get them ([`passes`]).
async fn file_bytes(store: &Store, attachment: &DraftAttachment) -> Result<Vec<u8>, bool> {
    let path = store.draft_files().join(&attachment.uuid);
    match (std::fs::read(&path), &attachment.source) {
        (Ok(data), _) => Ok(data),
        (
            Err(error),
            AttachmentSource::Gmail {
                message_id,
                attachment_id,
                part_id,
            },
        ) if error.kind() == std::io::ErrorKind::NotFound => {
            match store
                .part_data(message_id, attachment_id.as_deref(), part_id)
                .await
            {
                Ok(data) => {
                    let kept = path
                        .parent()
                        .map_or(Ok(()), std::fs::create_dir_all)
                        .and_then(|()| std::fs::write(&path, &data));
                    if let Err(error) = kept {
                        log::warn!("keeping a draft's attachment: {error}");
                    }
                    Ok(data)
                }
                Err(error) => {
                    log::warn!("fetching an attachment of {message_id}: {error}");
                    Err(passes(&error))
                }
            }
        }
        (Err(error), _) => {
            log::warn!("reading a draft's attachment: {error}");
            Err(false)
        }
    }
}

/// Gmail couldn't hand a part over for a reason that passes: it was out of reach, rate limiting
/// the account or signing it out, or failing (5xx). Not when it said the part is gone.
fn passes(error: &exo_store::Error) -> bool {
    match error {
        exo_store::Error::Offline(_)
        | exo_store::Error::Throttled { .. }
        | exo_store::Error::Unauthorized(_) => true,
        exo_store::Error::Gmail(error) => error.status().is_none_or(|status| status >= 500),
        _ => false,
    }
}

/// How long after a sync the next one is due. While the window shows the account: about 25 s
/// focused, else a minute on either power source, hidden or minimized too: a hidden window still
/// shows the account. Off screen: a minute on mains, five on battery.
fn sync_every(presence: Presence, power: Power) -> Duration {
    match (presence, power) {
        (Presence::Focused, _) => FOCUSED,
        (Presence::Visible | Presence::Background, _) | (Presence::OffScreen, Power::Plugged) => {
            FAST
        }
        (Presence::OffScreen, Power::Battery) => MEDIUM,
    }
}

/// How often to check whether a lost connection is back: every 10 s while the window shows the
/// account, and every minute while it's hidden or the account is off screen.
fn check_every(presence: Presence) -> Duration {
    match presence {
        Presence::Focused | Presence::Visible => CHECK_VISIBLE,
        Presence::Background | Presence::OffScreen => CHECK_HIDDEN,
    }
}

/// What sets how often the account syncs: the window, and off screen the Mac's power source.
struct Pace {
    presence: watch::Receiver<Presence>,
    power: watch::Receiver<Power>,
}

impl Pace {
    /// Waits until the next sync after one that ended at `ended` is due, at the pace as it is
    /// then. A change of pace counts from that sync: plugging in five minutes after a sync on
    /// battery syncs at once.
    async fn due(&mut self, ended: tokio::time::Instant) {
        loop {
            let every = sync_every(
                *self.presence.borrow_and_update(),
                *self.power.borrow_and_update(),
            );
            tokio::select! {
                () = tokio::time::sleep_until(ended + every) => return,
                Ok(()) = self.presence.changed() => {}
                Ok(()) = self.power.changed() => {}
            }
        }
    }
}

/// Syncs at the [`Pace`], at once when focused or asked, and continuously while the store
/// backfills. The new mail of each round after the first is posted to the notifier of the
/// account `notify` names. `synced` turns true once the first round has been tried.
async fn sync_loop(
    store: Store,
    requests: Requests,
    mut pace: Pace,
    mut now: Wakes,
    notify: (String, watch::Receiver<Option<Notifier>>),
    synced: watch::Sender<bool>,
    health: Arc<Health>,
) {
    // Opening the account asks for the first sync.
    let Some(mut busy) = now.next().await else {
        return;
    };
    // When a sync last reached Gmail; none yet, so the first catches up quietly.
    let mut reached: Option<jiff::Timestamp> = None;
    loop {
        let wait = {
            let _busy = busy;
            let result = loop {
                let result = store.sync().await;
                synced.send_replace(true);
                // Every list has its first page after one round: an empty one is empty, and the
                // list on screen can backfill on its own while the rounds go on.
                requests.post(Request::Synced {
                    reached: result.is_ok(),
                });
                if let Ok(report) = &result {
                    let synced = jiff::Timestamp::now();
                    if let Some(last) = reached.replace(synced) {
                        notify_arrivals(
                            &store,
                            &notify,
                            &report.arrived,
                            synced.duration_since(last) > NOTIFY_GAP,
                        )
                        .await;
                    }
                }
                match result {
                    Ok(report) if report.backfilling => {}
                    other => break other,
                }
            };
            match result {
                Ok(_) => None,
                Err(exo_store::Error::Throttled { until }) => {
                    let left = until.duration_since(jiff::Timestamp::now());
                    Some(Duration::try_from(left).unwrap_or(OFFLINE_RETRY))
                }
                // The offline bar says so; a line every retry would fill the log (`logging`).
                Err(exo_store::Error::Offline(_)) => Some(OFFLINE_RETRY),
                Err(error) => {
                    health.failed(&error);
                    log::warn!("sync: {error}");
                    Some(OFFLINE_RETRY)
                }
            }
        };
        let ended = tokio::time::Instant::now();
        let due = async {
            match wait {
                Some(wait) => tokio::time::sleep_until(ended + wait).await,
                None => pace.due(ended).await,
            }
        };
        busy = tokio::select! {
            () = due => requests.busy(),
            woken = now.next() => match woken {
                Some(busy) => busy,
                None => return,
            },
        };
    }
}

/// Evicts the mail and events the store no longer keeps: [`EVICT_AFTER`] the
/// first sync, then every [`EVICT_EVERY`].
async fn evict_loop(store: Store, mut first_sync: watch::Receiver<bool>) {
    if first_sync.wait_for(|synced| *synced).await.is_err() {
        return;
    }
    tokio::time::sleep(EVICT_AFTER).await;
    loop {
        if let Err(error) = store.evict(&exo_store::Eviction::default()).await {
            log::warn!("evicting: {error}");
        }
        tokio::time::sleep(EVICT_EVERY).await;
    }
}

/// Fetches the bodies of [`PRERENDER`] conversations stored for their rows alone
/// [`PRERENDER_AFTER`] the first sync (not at once, while the launch is backfilling the lists),
/// then every [`PRERENDER_EVERY`], as eviction keeps them, their parts, and the search index's
/// text of every body that came in. Offline, the fetching stops and waits for the next pass.
async fn prerender_loop(store: Store, requests: Requests, mut first_sync: watch::Receiver<bool>) {
    if first_sync.wait_for(|synced| *synced).await.is_err() {
        return;
    }
    tokio::time::sleep(PRERENDER_AFTER).await;
    loop {
        {
            let _busy = requests.busy();
            match store
                .prerender(&exo_store::Eviction::default(), PRERENDER)
                .await
            {
                Ok(_) | Err(exo_store::Error::Offline(_) | exo_store::Error::Throttled { .. }) => {}
                Err(error) => log::warn!("pre-rendering: {error}"),
            }
            // Then their images and the inbox's files.
            match store.prefetch_parts(PRERENDER).await {
                Ok(_) | Err(exo_store::Error::Offline(_) | exo_store::Error::Throttled { .. }) => {}
                Err(error) => log::warn!("prefetching attachments: {error}"),
            }
            // And what the renders show, for the search index.
            if let Err(error) = store.index_texts().await {
                log::warn!("indexing visible text: {error}");
            }
        }
        tokio::time::sleep(PRERENDER_EVERY).await;
    }
}

/// Notifies about `thread`, which a reminder just brought back, in the account `notify` names:
/// its newest message even though it's been read, unless the thread is muted or out of the
/// inbox, notifications are off, or it's the user's own.
async fn notify_returned(
    store: &Store,
    (account, notifier): &(String, watch::Receiver<Option<Notifier>>),
    thread: &ThreadId,
) {
    let notifier = notifier.borrow().clone();
    let Some(notifier) = notifier else { return };
    let found = async {
        Ok::<_, exo_store::Error>((
            store.latest_arrival(thread.as_str()).await?,
            store.settings::<Settings>().await?,
            store.settings::<DeviceSettings>().await?,
        ))
    };
    match found.await {
        Ok((Some(arrival), settings, device))
            if mail_notifications::notifies(&arrival, &settings, &device, true) =>
        {
            notifier.post(vec![mail_notifications::notice(&arrival, account)])
        }
        Ok(_) => {}
        Err(error) => log::warn!("notifying about a reminder: {error}"),
    }
}

/// Posts the notifications a sync's new mail calls for in the account `notify` names, after a
/// `gap` only the newest's ([`mail_notifications::notifying`]), when it has a notifier.
async fn notify_arrivals(
    store: &Store,
    (account, notifier): &(String, watch::Receiver<Option<Notifier>>),
    arrived: &[exo_store::Arrival],
    gap: bool,
) {
    let notifier = notifier.borrow().clone();
    let Some(notifier) = notifier.filter(|_| !arrived.is_empty()) else {
        return;
    };
    let settings = async {
        Ok::<_, exo_store::Error>((
            store.settings::<Settings>().await?,
            store.settings::<DeviceSettings>().await?,
        ))
    };
    let (settings, device) = match settings.await {
        Ok(settings) => settings,
        Err(error) => return log::warn!("reading notification settings: {error}"),
    };
    notifier.post(
        mail_notifications::notifying(arrived, &settings, &device, gap)
            .into_iter()
            .map(|arrival| mail_notifications::notice(arrival, account))
            .collect(),
    );
}

/// Brings threads back when their reminders come due, on the device: at once for
/// those due while Exo was closed, and then as each comes due, as long as Exo runs, with a
/// notification for each to the notifier of the account `notify` names. It waits for the first
/// sync, so a reply that came meanwhile cancels an "if no reply" reminder before it would fire,
/// and looks again whenever the store changes.
async fn reminder_loop(
    store: Store,
    clock: Arc<std::sync::RwLock<crate::Clock>>,
    mut first_sync: watch::Receiver<bool>,
    mut wakes: Wakes,
    flush: Wake,
    notify: (String, watch::Receiver<Option<Notifier>>),
) {
    if first_sync.wait_for(|synced| *synced).await.is_err() {
        return;
    }
    let mut changes = store.changes();
    // A wake counts as busy until the look it asked for is done.
    let mut busy: Option<Busy> = None;
    loop {
        let now = clock
            .read()
            .expect("the clock lock is never poisoned")
            .now()
            .timestamp();
        match store.return_reminders(now).await {
            Ok(returned) if !returned.is_empty() => {
                flush.wake();
                for thread in &returned {
                    notify_returned(&store, &notify, thread).await;
                }
            }
            Ok(_) | Err(exo_store::Error::NotSynced(_)) => {}
            Err(error) => log::warn!("bringing reminders back: {error}"),
        }
        let next = store.next_reminder().await.unwrap_or_else(|error| {
            log::warn!("reading reminders: {error}");
            None
        });
        let wait = next.map_or(REMINDER_RECHECK, |at| {
            Duration::try_from(at.duration_since(now))
                .unwrap_or_default()
                .min(REMINDER_RECHECK)
        });
        drop(busy.take());
        busy = tokio::select! {
            changed = changes.changed() => {
                if changed.is_err() {
                    return;
                }
                None
            }
            woken = wakes.next() => match woken {
                Some(woken) => Some(woken),
                None => return,
            },
            () = tokio::time::sleep(wait) => None,
        };
    }
}

/// While the connection is down, checks every so often whether it's back; checks at once when
/// the OS says the network changed; and when it comes back, wakes sync and the outbox.
async fn connection_loop(
    store: Store,
    presence: watch::Receiver<Presence>,
    mut changes: Wakes,
    sync_now: Wake,
    flush: Wake,
) {
    let mut connection = store.connection();
    loop {
        let down = !connection.borrow_and_update().is_online();
        let every = check_every(*presence.borrow());
        tokio::select! {
            changed = connection.changed() => {
                if changed.is_err() {
                    return;
                }
                if down && connection.borrow().is_online() {
                    sync_now.wake();
                    flush.wake();
                }
            }
            woken = changes.next() => {
                let Some(busy) = woken else { return };
                store.network_changed().await;
                drop(busy);
            }
            () = tokio::time::sleep(every), if down => store.check_connection().await,
        }
    }
}

/// How long a listing of Google contacts is used before it's listed again.
const GOOGLE_CONTACTS_FRESH: Duration = Duration::from_secs(60 * 60);

/// A Google contact or "other contact": an address and its display name.
#[derive(Debug)]
struct GoogleContact {
    email: String,
    name: Option<String>,
}

/// Google contacts and "other contacts" (at most five pages of each), best-effort: what a
/// listing that fails has so far. A listing that gets no answer shows the offline bar.
async fn google_contacts(store: &Store) -> Vec<GoogleContact> {
    const FIELDS: &str = "names,emailAddresses";
    let mut people = Vec::new();
    for other in [false, true] {
        let mut query = gmail::people::PeopleQuery {
            page_size: Some(1000),
            ..gmail::people::PeopleQuery::default()
        };
        for _ in 0..5 {
            let page = if other {
                store
                    .google(1, |client| client.people().other_contacts(FIELDS, &query))
                    .await
                    .map(|page| (page.other_contacts, page.next_page_token))
            } else {
                store
                    .google(1, |client| client.people().connections(FIELDS, &query))
                    .await
                    .map(|page| (page.connections, page.next_page_token))
            };
            match page {
                Ok((page, next)) => {
                    people.extend(page);
                    match next {
                        Some(token) => query.page_token = Some(token),
                        None => break,
                    }
                }
                Err(error) => {
                    log::warn!(
                        "listing Google {}: {error}",
                        if other { "other contacts" } else { "contacts" }
                    );
                    break;
                }
            }
        }
    }
    people
        .into_iter()
        .flat_map(|person| {
            let name = person
                .names
                .first()
                .and_then(|name| name.display_name.clone())
                .filter(|name| !name.is_empty());
            person
                .email_addresses
                .into_iter()
                .filter_map(|address| address.value)
                .map(move |email| GoogleContact {
                    email,
                    name: name.clone(),
                })
        })
        .collect()
}

/// Mail's contacts with Google's added (unscored, unless mail scored them) and lending their
/// names to addresses mail has none for.
fn merge_contacts(mail: &[ScoredContact], google: &[GoogleContact]) -> Vec<ScoredContact> {
    let mut book = mail.to_vec();
    let mut known: std::collections::HashMap<String, usize> = book
        .iter()
        .enumerate()
        .map(|(at, contact)| (contact.email.to_lowercase(), at))
        .collect();
    for contact in google {
        match known.get(&contact.email.to_lowercase()) {
            Some(&at) => {
                if book[at].name.is_none() {
                    book[at].name.clone_from(&contact.name);
                }
            }
            None => {
                known.insert(contact.email.to_lowercase(), book.len());
                book.push(ScoredContact {
                    email: contact.email.clone(),
                    name: contact.name.clone(),
                    score: 0.0,
                });
            }
        }
    }
    book
}

/// Tells the UI what writing drafts to Gmail did.
fn events_for_drafts(
    events: &std::sync::mpsc::Sender<Event>,
    saved: Vec<String>,
    sent: Vec<SentDraft>,
    failed: Vec<DraftFailure>,
    offline: bool,
) {
    for id in saved {
        let _ = events.send(Event::DraftSaved(id));
    }
    for sent in sent {
        let _ = events.send(Event::DraftSent(sent));
    }
    for failure in failed {
        log::warn!("Gmail refused a draft: {failure:?}");
        let _ = events.send(Event::DraftFailed(failure));
    }
    if offline {
        let _ = events.send(Event::DraftsOffline);
    }
}

/// Writes every draft Gmail lacks now, its pace and backoff notwithstanding, and
/// tells the UI what that did; the push loop, woken after, takes the retries over. Off the
/// worker, which a slow upload would otherwise hold up.
async fn write_drafts_now(store: &Store, events: &std::sync::mpsc::Sender<Event>, flush: &Wake) {
    match store.push_drafts_now().await {
        Ok(report) => events_for_drafts(
            events,
            report.saved,
            report.sent,
            report.failed,
            report.offline,
        ),
        Err(error) => log::error!("writing drafts now: {error}"),
    }
    flush.wake();
}

/// Waits until the clock reads `at`. A sleeping Mac stops a timer's clock (`Instant` doesn't
/// advance), so a send scheduled for the morning would go as late as the Mac slept: the clock is
/// read again at least every minute.
async fn sleep_until(at: jiff::Timestamp) {
    const CHECK: Duration = Duration::from_secs(60);
    while let Ok(left) = Duration::try_from(at.duration_since(jiff::Timestamp::now())) {
        if left.is_zero() {
            return;
        }
        tokio::time::sleep(left.min(CHECK)).await;
    }
}

/// Sends queued mutations and drafts whenever one is applied, undone or saved, and again when
/// a failed one or a held send is due.
async fn flush_loop(
    store: Store,
    requests: Requests,
    mut wake: Wakes,
    sync_now: Wake,
    events: std::sync::mpsc::Sender<Event>,
    repaint: Repaint,
    reach: crate::unsubscribe::Reach,
) {
    let mut retry: Option<jiff::Timestamp> = None;
    loop {
        let _busy = tokio::select! {
            woken = wake.next() => match woken {
                Some(busy) => busy,
                None => return,
            },
            () = sleep_until(retry.unwrap_or_default()), if retry.is_some() => requests.busy(),
        };
        retry = None;
        match store.flush().await {
            Ok(report) => {
                if report.completed > 0 {
                    sync_now.wake();
                }
                for failed in report.failed {
                    log::warn!("giving up on {failed:?}");
                    let _ = events.send(Event::Failed {
                        mutation: failed.mutation,
                        undo: failed.undo,
                    });
                }
                retry = report.retry_at;
            }
            Err(error) => log::error!("flushing: {error}"),
        }
        match store.push_drafts().await {
            Ok(report) => {
                let notable = !report.saved.is_empty()
                    || !report.sent.is_empty()
                    || !report.failed.is_empty()
                    || report.offline;
                // A held send's reminder queued its label: flush again at once.
                let reminded = report.reminded.then(jiff::Timestamp::now);
                events_for_drafts(
                    &events,
                    report.saved,
                    report.sent,
                    report.failed,
                    report.offline,
                );
                if notable {
                    repaint.request();
                }
                retry = [retry, reminded, report.retry_at]
                    .into_iter()
                    .flatten()
                    .min();
            }
            Err(error) => log::error!("writing drafts: {error}"),
        }
        if let Some(at) = crate::unsubscribe::push(&store, reach).await {
            retry = Some(retry.map_or(at, |retry| retry.min(at)));
        }
        requests.post(Request::Refresh);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    /// Waits for `account` to be idle.
    fn settle(account: &Account) {
        let started = Instant::now();
        while !account.is_idle() {
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "the account never went idle"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn database_failure(code: std::ffi::c_int) -> exo_store::Error {
        exo_store::Error::Database(toasty::Error::driver_operation_failed(
            rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None),
        ))
    }

    #[test]
    fn a_store_stops_on_corruption_a_full_disk_or_five_io_failures_in_a_row() {
        let health = || {
            Health::new(Repaint {
                ctx: egui::Context::default(),
                on_screen: Arc::new(AtomicBool::new(false)),
            })
        };
        let io = database_failure(rusqlite::ffi::SQLITE_IOERR);
        let passing = health();
        for _ in 0..4 {
            passing.failed(&io);
        }
        passing.ok();
        for _ in 0..4 {
            passing.failed(&io);
        }
        assert_eq!(
            passing.unusable(),
            None,
            "an answer between them starts the count over"
        );
        passing.failed(&io);
        assert_eq!(
            passing.unusable(),
            Some(exo_store::Unusable::Corrupt),
            "five in a row"
        );
        let corrupt = health();
        corrupt.failed(&database_failure(rusqlite::ffi::SQLITE_CORRUPT));
        assert_eq!(
            corrupt.unusable(),
            Some(exo_store::Unusable::Corrupt),
            "at once"
        );
        let full = health();
        full.failed(&database_failure(rusqlite::ffi::SQLITE_FULL));
        full.failed(&database_failure(rusqlite::ffi::SQLITE_CORRUPT));
        assert_eq!(
            full.unusable(),
            Some(exo_store::Unusable::DiskFull),
            "the first reason stays"
        );
        let busy = health();
        busy.failed(&database_failure(rusqlite::ffi::SQLITE_BUSY));
        busy.failed(&exo_store::Error::NotSynced("labels"));
        assert_eq!(busy.unusable(), None);
    }

    #[test]
    fn a_wake_is_busy_until_the_loop_is_done_with_it() {
        let requests = Requests {
            sender: mpsc::unbounded_channel().0,
            busy: Arc::default(),
        };
        let busy = || requests.busy.load(Ordering::SeqCst);
        let (wake, mut wakes) = Wake::new(&requests);
        wake.wake();
        wake.wake();
        assert_eq!(busy(), 2, "busy from the call, before the loop runs");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let woken = runtime.block_on(wakes.next()).expect("woken");
        assert_eq!(busy(), 1, "one run answers both");
        drop(woken);
        assert_eq!(busy(), 0);
        drop(wakes);
        wake.wake();
        assert_eq!(busy(), 0, "a wake for a loop that's gone isn't counted");
    }

    #[test]
    fn regaining_focus_is_busy_until_its_sync_is_on_screen() {
        let runtime = Runtime::new().expect("runtime");
        let server = runtime
            .block_on(gmail_mock::MockServer::start(gmail_mock::Config::default()))
            .expect("mock starts");
        let dir = std::env::temp_dir().join(format!("exo-account-focus-{}", std::process::id()));
        let tokens = Arc::new(gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()));
        let credentials = Credentials {
            email: "maya@brightmoor.co".into(),
            endpoints: server.endpoints(),
            tokens: tokens.clone(),
        };
        let account = Account::open(&dir, credentials, egui::Context::default()).expect("opens");
        settle(&account);
        let id = account
            .snapshot()
            .threads
            .first()
            .expect("the inbox has mail")
            .id
            .clone();
        let gmail = gmail::Client::new(server.endpoints(), tokens);
        let archive = gmail::ModifyThreadRequest {
            add_label_ids: Vec::new(),
            remove_label_ids: vec!["INBOX".into()],
        };
        runtime
            .block_on(async { gmail.gmail().modify_thread(&id, &archive).await })
            .expect("archived elsewhere");

        account.set_presence(Presence::Visible);
        account.set_presence(Presence::Focused);
        assert!(
            !account.is_idle(),
            "busy from the moment focus asks for a sync"
        );
        settle(&account);
        assert!(
            !account
                .snapshot()
                .threads
                .iter()
                .any(|thread| thread.id == id),
            "the sync took it out of the list"
        );
        drop(account);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_reminder_coming_back_notifies_its_thread_though_its_read() {
        let runtime = Runtime::new().expect("runtime");
        let server = runtime
            .block_on(gmail_mock::MockServer::start(gmail_mock::Config::default()))
            .expect("mock starts");
        let dir = std::env::temp_dir().join(format!(
            "exo-account-reminder-notify-{}",
            std::process::id()
        ));
        let credentials = Credentials {
            email: "maya@brightmoor.co".into(),
            endpoints: server.endpoints(),
            tokens: Arc::new(gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into())),
        };
        let account = Account::open(&dir, credentials, egui::Context::default()).expect("opens");
        let notifier = Notifier::recording(crate::desktop::Launcher::default());
        account.set_notifier(Some(notifier.clone()));
        let now = jiff::Zoned::now();
        account.set_clock(crate::Clock::fixed(now.clone()));
        // Only Important's mail notifies while Important and Other are on.
        account.show(ListId::Important);
        settle(&account);
        let mail = || {
            notifier
                .posted()
                .into_iter()
                .filter(|notice| notice.key.starts_with("EMAIL-"))
                .collect::<Vec<_>>()
        };
        // Its newest message someone else's: the user's own never notify.
        let theirs = |thread: &ThreadSummary| {
            runtime
                .block_on(account.shared.loader.store.latest_arrival(&thread.id))
                .expect("reads")
                .is_some_and(|arrival| arrival.from.is_some_and(|from| !from.is_me))
        };
        let thread = account
            .snapshot()
            .threads
            .iter()
            .find(|thread| !thread.unread && thread.drafts == 0 && theirs(thread))
            .cloned()
            .expect("a read Important thread");
        let at = now
            .timestamp()
            .checked_add(jiff::SignedDuration::from_mins(1))
            .expect("a time");
        account.apply(
            std::slice::from_ref(&thread.id),
            Mutation::Remind {
                at,
                keep_on_reply: true,
                snooze: true,
            },
            None,
        );
        settle(&account);
        assert!(
            !account
                .snapshot()
                .threads
                .iter()
                .any(|row| row.id == thread.id),
            "snoozed"
        );
        let before = mail().len();

        account.set_clock(crate::Clock::fixed(
            now.checked_add(jiff::SignedDuration::from_mins(2))
                .expect("a time"),
        ));
        account.check_reminders();
        settle(&account);
        assert!(
            account
                .snapshot()
                .threads
                .iter()
                .any(|row| row.id == thread.id),
            "back in the inbox"
        );
        let posted = mail();
        let [notice] = &posted[before..] else {
            panic!("one notification: {posted:?}")
        };
        let newest = runtime
            .block_on(account.shared.loader.store.latest_arrival(&thread.id))
            .expect("reads")
            .expect("stored");
        assert_eq!(
            notice.key,
            format!("EMAIL-{}", newest.message),
            "for the thread's newest message"
        );
        drop(account);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn mail_that_arrives_after_the_first_sync_notifies_once() {
        let runtime = Runtime::new().expect("runtime");
        let server = runtime
            .block_on(gmail_mock::MockServer::start(gmail_mock::Config::default()))
            .expect("mock starts");
        let dir = std::env::temp_dir().join(format!("exo-account-notify-{}", std::process::id()));
        let credentials = Credentials {
            email: "maya@brightmoor.co".into(),
            endpoints: server.endpoints(),
            tokens: Arc::new(gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into())),
        };
        let account = Account::open(&dir, credentials, egui::Context::default()).expect("opens");
        let notifier = Notifier::recording(crate::desktop::Launcher::default());
        account.set_notifier(Some(notifier.clone()));
        settle(&account);
        let sync = || {
            account.set_presence(Presence::Visible);
            account.set_presence(Presence::Focused);
            settle(&account);
        };
        // The mock's calendar is near today, so reminders may come too; only mail counts here.
        let mail = || {
            notifier
                .posted()
                .into_iter()
                .filter(|notice| notice.key.starts_with("EMAIL-"))
                .collect::<Vec<_>>()
        };
        assert!(mail().is_empty(), "the first sync catches up quietly");

        let deliver = |body: serde_json::Value| {
            runtime.block_on(async {
                reqwest::Client::new()
                    .post(server.url().join("/__mock/deliver").expect("path"))
                    .json(&body)
                    .send()
                    .await
                    .expect("delivered")
                    .json::<serde_json::Value>()
                    .await
                    .expect("a message")
            })
        };
        let delivered = deliver(
            serde_json::json!({ "from": "Priya Raman <priya@brightmoor.co>", "subject": "Launch checklist", "text": "Before 3?" }),
        );
        deliver(
            serde_json::json!({ "from": "deals@shop.example", "subject": "Sale", "labelIds": ["INBOX", "UNREAD", "CATEGORY_PROMOTIONS"] }),
        );
        sync();
        let posted = mail();
        let [notice] = posted.as_slice() else {
            panic!("one notification: {posted:?}")
        };
        assert_eq!(
            notice.key,
            format!("EMAIL-{}", delivered["id"].as_str().expect("an id"))
        );
        assert_eq!(
            (notice.title.as_str(), notice.body.as_str()),
            ("Priya Raman", "Launch checklist")
        );
        sync();
        assert_eq!(mail().len(), 1, "once");
        drop(account);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// How long after a sync that ended now the next is due, the pace starting at `start` and
    /// becoming `then.1` after `then.0`.
    async fn due(
        start: (Presence, Power),
        then: Option<(Duration, (Presence, Power))>,
    ) -> Duration {
        let (presence_sender, presence) = watch::channel(start.0);
        let (power_sender, power) = watch::channel(start.1);
        let ended = tokio::time::Instant::now();
        if let Some((after, (presence, power))) = then {
            tokio::spawn(async move {
                tokio::time::sleep(after).await;
                presence_sender.send_replace(presence);
                power_sender.send_replace(power);
                std::future::pending::<()>().await;
            });
        }
        Pace { presence, power }.due(ended).await;
        ended.elapsed()
    }

    #[tokio::test(start_paused = true)]
    async fn the_pace_follows_the_window_and_off_screen_the_power_source() {
        let secs = Duration::from_secs;
        let (on_mains, on_battery) = (
            (Presence::OffScreen, Power::Plugged),
            (Presence::OffScreen, Power::Battery),
        );
        assert_eq!(
            due((Presence::Focused, Power::Battery), None).await,
            secs(25)
        );
        assert_eq!(
            due((Presence::Visible, Power::Battery), None).await,
            secs(60)
        );
        assert_eq!(
            due((Presence::Background, Power::Battery), None).await,
            secs(60),
            "a hidden window's account syncs as a shown one's"
        );
        assert_eq!(due(on_mains, None).await, secs(60));
        assert_eq!(due(on_battery, None).await, secs(300));
        assert_eq!(
            due(on_battery, Some((secs(30), on_mains))).await,
            secs(60),
            "plugged in, due a minute after the last sync"
        );
        assert_eq!(
            due(on_battery, Some((secs(90), on_mains))).await,
            secs(90),
            "and at once when that's past"
        );
        assert_eq!(due(on_mains, Some((secs(30), on_battery))).await, secs(300));
        assert_eq!(
            due(
                on_battery,
                Some((secs(10), (Presence::Focused, Power::Battery)))
            )
            .await,
            secs(25),
            "coming on screen"
        );
        assert_eq!(
            (
                check_every(Presence::Visible),
                check_every(Presence::Background),
                check_every(Presence::OffScreen)
            ),
            (secs(10), secs(60), secs(60))
        );
    }

    #[test]
    fn an_account_off_screen_syncs_every_minute_on_mains_and_every_five_on_battery() {
        let runtime = Runtime::new().expect("runtime");
        let server = runtime
            .block_on(gmail_mock::MockServer::start(gmail_mock::Config::default()))
            .expect("mock starts");
        let dir = std::env::temp_dir().join(format!("exo-account-power-{}", std::process::id()));
        let credentials = Credentials {
            email: "maya@brightmoor.co".into(),
            endpoints: server.endpoints(),
            tokens: Arc::new(gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into())),
        };
        let account = Account::open(&dir, credentials, egui::Context::default()).expect("opens");
        let power = PowerSource::fixed(Power::Battery);
        account.follow_power(&power);
        account.set_on_screen(false);
        account.set_presence(Presence::OffScreen);
        settle(&account);
        let quota = server.url().join("/__mock/quota").expect("path");
        // What sync's look for new mail spent: the pre-render's pass a minute in isn't sync.
        let units = || {
            runtime.block_on(async {
                reqwest::get(quota.clone())
                    .await
                    .expect("answers")
                    .json::<serde_json::Value>()
                    .await
                    .expect("json")["methods"]["history.list"]["units"]
                    .as_u64()
                    .unwrap_or_default()
            })
        };
        let synced = units();

        std::thread::sleep(Duration::from_secs(62));
        assert_eq!(units(), synced, "on battery, no sync a minute later");
        // Plugged in, the sync due a minute after the last goes at once: one `history.list`.
        power.set(Power::Plugged);
        let plugged = Instant::now();
        while units() == synced {
            assert!(
                plugged.elapsed() < Duration::from_secs(5),
                "no sync once plugged in"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        settle(&account);
        assert_eq!(units() - synced, 2);
        drop(account);
        let _ = std::fs::remove_dir_all(dir);
    }
}
