//! The calendar's data off the UI thread: a task on the account's runtime lists the user's
//! calendars, reads the events of the days on screen from the store, fetches those days from
//! Google Calendar, sends the changes made to events, and publishes what it has for the UI to
//! read each frame.
//!
//! Exo has no server for Calendar to push changes through, so it polls: the days on screen every
//! minute while the window is focused, every five minutes otherwise, and the calendar lists every
//! ten: the mail account's and those of the calendar accounts signed in.
//!
//! Changes go into the store's calendar queue ([`CalendarChange`]), which shows them on the
//! calendar at once, keeps them across a quit and sends them once the connection allows. New
//! events, edits and deletions are held ten seconds first, so Undo can take one back before anyone
//! hears of it; Undo also takes back a change still waiting for the connection. After a panic the
//! task runs again (`recovery`), and finds its queue as it was.

use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    ops::Range,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use exo_store::{
    AccountCalendar, CalendarChange, CalendarOutcome, Connection, Outage, Store, StoredEvent,
};
use gmail::calendar::Event;
use jiff::{SignedDuration, Timestamp, civil::Date, tz::TimeZone};
use tokio::sync::{mpsc, oneshot, watch};

use crate::account::{Loader, Presence, Repaint};
use crate::recovery::{self, Restarts};

/// How long fetched days stay fresh before they're fetched again.
const FOCUSED: Duration = Duration::from_secs(60);
const UNFOCUSED: Duration = Duration::from_secs(300);
/// How long the calendar list stays fresh.
const LIST_FRESH: Duration = Duration::from_secs(600);
/// How soon to try again when Google can't be reached.
const RETRY: Duration = Duration::from_secs(15);
/// How long a change waits for Undo before it's sent.
const HOLD: SignedDuration = SignedDuration::from_secs(10);

/// Days of events, from some calendars, in a time zone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    /// The days.
    pub days: Range<Date>,
    /// Calendars whose events show.
    pub shown: Vec<String>,
    /// Calendars whose events are fetched (a superset of `shown`).
    pub synced: Vec<String>,
    /// Where days start and end.
    pub tz: TimeZone,
}

/// What the calendar has, as the task last read it.
#[derive(Clone, Debug, Default)]
pub struct Data {
    /// The calendars of every calendar account read, each once with the account it's read
    /// through, once listed (the store's copy, then Google's).
    pub calendars: Option<Vec<AccountCalendar>>,
    /// Calendar accounts just signed in whose list hasn't come yet.
    pub loading_accounts: BTreeSet<String>,
    /// The window `events` hold, once read.
    pub window: Option<Window>,
    /// The shown calendars' events in the window, with the changes still queued for Calendar.
    pub events: Vec<StoredEvent>,
    /// The `.ics` of the last thread asked about that has an invite, by thread.
    pub invite: Option<(String, Vec<u8>)>,
    /// A new window's events are being fetched.
    pub loading: bool,
    /// Free/busy as last asked for (my shown calendars, for Find Time): each calendar's busy
    /// spans, or `None` when Calendar won't say; and the time it covers.
    pub busy: BTreeMap<String, Option<Vec<(Timestamp, Timestamp)>>>,
    pub busy_range: Option<(Timestamp, Timestamp)>,
    /// Series' `recurrence` lines, by `{calendar}/{series}`, as asked for this session.
    pub series: BTreeMap<String, Vec<String>>,
    /// Guests' calendars as last read, by address.
    pub guests: BTreeMap<String, GuestCalendar>,
}

/// A guest's calendar as last read.
#[derive(Clone, Debug)]
pub struct GuestCalendar {
    /// The time it was last asked for.
    pub range: (Timestamp, Timestamp),
    /// Being read; what it had before stays meanwhile.
    pub loading: bool,
    pub events: GuestEvents,
}

/// What Calendar said about a guest's calendar.
#[derive(Clone, Debug)]
pub enum GuestEvents {
    /// Nothing yet.
    Pending,
    /// Their events (instances) over the range.
    Shown(Vec<Event>),
    /// Calendar won't show them.
    Hidden,
    /// Calendar couldn't be reached; asked again once the connection is back.
    Failed,
}

/// A change Calendar refused, so the calendar went back to how it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// An answer to an event.
    Respond,
    /// An answer to the invite with this UID, from conversation `thread`; `gone` when the event
    /// has been deleted or cancelled.
    RespondToInvite {
        uid: String,
        thread: Option<String>,
        gone: bool,
    },
    /// A new event, queued under `token`.
    Create {
        token: u64,
        calendar: String,
        event: Box<Event>,
    },
    /// An edit.
    Edit,
    /// A deletion.
    Delete,
}

impl Failure {
    /// What `change`, queued under `token`, failing (`gone`: its event has been deleted or
    /// cancelled) means for the UI.
    fn of(token: u64, change: CalendarChange, gone: bool) -> Self {
        match change {
            CalendarChange::Create { calendar, event } => Self::Create {
                token,
                calendar,
                event: Box::new(event),
            },
            CalendarChange::Edit { .. } | CalendarChange::Split { .. } => Self::Edit,
            CalendarChange::Delete { .. } | CalendarChange::Truncate { .. } => Self::Delete,
            CalendarChange::Respond { .. } => Self::Respond,
            CalendarChange::RespondToInvite { invite, thread, .. } => Self::RespondToInvite {
                uid: invite.ical_uid.unwrap_or_default(),
                thread,
                gone,
            },
        }
    }
}

enum Request {
    Show(Window),
    /// Queue `change` under `token`, held for Undo first when `held`.
    Queue {
        token: u64,
        change: Box<CalendarChange>,
        held: bool,
    },
    /// Take back the change queued under this token.
    Undo(u64),
    /// Read these calendars' free/busy over this time.
    Busy(Vec<String>, Timestamp, Timestamp),
    /// Send every queued change now, held ones too, then say so (quitting).
    SendNow(oneshot::Sender<()>),
    /// Read this thread's invite.
    Invite(String),
    /// The calendar accounts read changed: list them again, `added` (just signed in) first.
    Accounts {
        added: Option<String>,
    },
    /// Stop reading calendar account `email` (signed out), forgetting its list when `removed`.
    Forget {
        email: String,
        removed: bool,
    },
    /// Read this series' rule: its calendar and id.
    Series(String, String),
    /// Read these guests' calendars over this time.
    Guests(Vec<String>, Timestamp, Timestamp),
    /// From the task fetching it: a guest's calendar over a time, `None` when it couldn't be
    /// reached.
    GuestLoaded(String, (Timestamp, Timestamp), Option<Option<Vec<Event>>>),
    /// From the task fetching it: a thread's invite, if it has one.
    InviteLoaded(String, Option<Vec<u8>>),
    /// Panics the task, as a bug would.
    #[cfg(test)]
    Panic,
}

/// The changes of this session Undo can take back.
#[derive(Default)]
struct Tokens {
    /// Queued, and not going out right now.
    undoable: HashSet<u64>,
    /// Taken back, and not dropped from the queue yet.
    undone: HashSet<u64>,
}

/// What the UI and the task share: it outlives the task's runs.
struct Shared {
    data: watch::Sender<Arc<Data>>,
    busy: AtomicUsize,
    failures: Mutex<Vec<Failure>>,
    tokens: Mutex<Tokens>,
    /// The window on screen, as last asked for.
    window: Mutex<Option<Window>>,
    /// Series whose rule was asked for (`{calendar}/{series}`).
    series_asked: Mutex<HashSet<String>>,
    /// The time each guest's calendar was last asked for.
    guests_asked: Mutex<BTreeMap<String, (Timestamp, Timestamp)>>,
}

impl Shared {
    fn tokens(&self) -> MutexGuard<'_, Tokens> {
        self.tokens.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The UI's handle on the calendar task.
pub struct CalendarSync {
    requests: mpsc::UnboundedSender<Request>,
    data: watch::Receiver<Arc<Data>>,
    shared: Arc<Shared>,
    /// Which calendar accounts the store reads is said here, at once.
    store: Store,
}

impl CalendarSync {
    /// Starts the task on the current runtime, and again after a panic as `restarts` says,
    /// fetching invites with `loader`; `repaint` asks for a frame when data arrives.
    pub(crate) fn start(
        store: Store,
        loader: Loader,
        presence: watch::Receiver<Presence>,
        repaint: Repaint,
        restarts: &Arc<Restarts>,
    ) -> Self {
        let (requests, receiver) = mpsc::unbounded_channel();
        let (sender, data) = watch::channel(Arc::new(Data::default()));
        // The first run's opening counts as busy from now.
        let shared = Arc::new(Shared {
            data: sender,
            busy: AtomicUsize::new(1),
            failures: Mutex::default(),
            tokens: Mutex::default(),
            window: Mutex::default(),
            series_asked: Mutex::default(),
            guests_asked: Mutex::default(),
        });
        let receiver = Arc::new(tokio::sync::Mutex::new(receiver));
        let mut resumed = false;
        let handle = store.clone();
        recovery::spawn(restarts, {
            let (requests, shared) = (requests.clone(), shared.clone());
            move || {
                if std::mem::replace(&mut resumed, true) {
                    shared.busy.fetch_add(1, Ordering::SeqCst);
                }
                let task = Task {
                    store: store.clone(),
                    loader: loader.clone(),
                    requests: requests.clone(),
                    presence: presence.clone(),
                    connection: store.connection(),
                    repaint: repaint.clone(),
                    shared: shared.clone(),
                    window: None,
                    fetched: Vec::new(),
                    listed: None,
                    retry_at: None,
                };
                task.run(receiver.clone(), resumed)
            }
        });
        Self {
            requests,
            data,
            shared,
            store: handle,
        }
    }

    /// Queues `change`, shown on the calendar at once and sent once Undo's hold is over.
    /// Returns its token, for [`CalendarSync::undo`].
    pub fn hold(&self, change: CalendarChange) -> u64 {
        let token = token();
        self.shared.tokens().undoable.insert(token);
        self.post(Request::Queue {
            token,
            change: Box::new(change),
            held: true,
        });
        token
    }

    /// Queues `change` to go at once, shown on the calendar meanwhile, with no Undo.
    pub fn queue(&self, change: CalendarChange) {
        self.post(Request::Queue {
            token: token(),
            change: Box::new(change),
            held: false,
        });
    }

    /// Takes back the change `token`. Returns whether it hadn't gone yet.
    pub fn undo(&self, token: u64) -> bool {
        let mut tokens = self.shared.tokens();
        if !tokens.undoable.remove(&token) {
            return false;
        }
        tokens.undone.insert(token);
        drop(tokens);
        self.post(Request::Undo(token));
        true
    }

    /// How many of this session's changes Undo could still take back: held, or waiting for the
    /// connection.
    pub fn held(&self) -> usize {
        self.shared.tokens().undoable.len()
    }

    /// Sends the queued changes now, held ones too, waiting at most `grace` (Exo is quitting).
    pub async fn send_now(&self, grace: Duration) {
        let (done, wait) = oneshot::channel();
        self.post(Request::SendNow(done));
        let _ = tokio::time::timeout(grace, wait).await;
    }

    /// What the calendar has now.
    pub fn data(&self) -> Arc<Data> {
        self.data.borrow().clone()
    }

    /// Reads (and keeps fresh) `window`'s events from now on.
    pub fn show(&self, window: Window) {
        *self
            .shared
            .window
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(window.clone());
        self.post(Request::Show(window));
    }

    /// Reads `calendars`' free/busy from `from` to `to` into the data.
    pub fn busy(&self, calendars: Vec<String>, from: Timestamp, to: Timestamp) {
        self.post(Request::Busy(calendars, from, to));
    }

    /// Reads `guests`' calendars from `from` to `to` into the data, each unless it was asked for
    /// over that time already.
    pub fn load_guests(&self, guests: &[String], from: Timestamp, to: Timestamp) {
        let mut asked = self
            .shared
            .guests_asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let wanted: Vec<String> = guests
            .iter()
            .filter(|email| {
                !asked
                    .get(*email)
                    .is_some_and(|(start, end)| *start <= from && *end >= to)
            })
            .cloned()
            .collect();
        if wanted.is_empty() {
            return;
        }
        for email in &wanted {
            asked.insert(email.clone(), (from, to));
        }
        drop(asked);
        self.post(Request::Guests(wanted, from, to));
    }

    /// The changes Calendar refused since the last call.
    pub fn failures(&self) -> Vec<Failure> {
        std::mem::take(
            &mut *self
                .shared
                .failures
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    /// Reads series `id`'s rule on `calendar` into the data (`series`), once a session.
    pub fn load_series(&self, calendar: &str, id: &str) {
        let key = format!("{calendar}/{id}");
        if self
            .shared
            .series_asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key)
        {
            self.post(Request::Series(calendar.to_owned(), id.to_owned()));
        }
    }

    /// Reads the invite of thread `id` into the data, if it carries one.
    pub fn load_invite(&self, id: &str) {
        self.post(Request::Invite(id.to_owned()));
    }

    /// Reads calendar account `email`'s calendars with `tokens` from now on: it signed in
    /// (`added`), which shows `Loading calendars` until its list comes, or the account opened.
    pub fn read_account(
        &self,
        email: &str,
        tokens: impl gmail::TokenProvider + 'static,
        added: bool,
    ) {
        self.store.add_calendar_account(email, tokens);
        if added {
            self.shared.data.send_modify(|data| {
                Arc::make_mut(data)
                    .loading_accounts
                    .insert(email.to_owned());
            });
        }
        self.post(Request::Accounts {
            added: added.then(|| email.to_owned()),
        });
    }

    /// Stops reading calendar account `email`: Google signed it out, or it was `removed`, which
    /// forgets its list and the events only it had.
    pub fn forget_account(&self, email: &str, removed: bool) {
        self.post(Request::Forget {
            email: email.to_owned(),
            removed,
        });
    }

    fn post(&self, request: Request) {
        self.shared.busy.fetch_add(1, Ordering::SeqCst);
        if self.requests.send(request).is_err() {
            self.shared.busy.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Nothing is being read or fetched.
    pub fn is_idle(&self) -> bool {
        self.shared.busy.load(Ordering::SeqCst) == 0
    }
}

/// A token for a change: random, so this session's never meet one an earlier session queued.
fn token() -> u64 {
    // SQLite's integers are signed.
    getrandom::u64().unwrap_or_else(|_| Timestamp::now().as_nanosecond().unsigned_abs() as u64) >> 1
}

struct Task {
    store: Store,
    loader: Loader,
    /// The task's own requests, for invites fetched off the task.
    requests: mpsc::UnboundedSender<Request>,
    presence: watch::Receiver<Presence>,
    /// The connection, whose return sends what's queued.
    connection: watch::Receiver<Connection>,
    repaint: Repaint,
    shared: Arc<Shared>,
    /// The window on screen.
    window: Option<Window>,
    /// Calendar and days fetched, and when.
    fetched: Vec<(String, Range<Date>, Instant)>,
    /// When the calendar list was last fetched.
    listed: Option<Instant>,
    /// When the next queued change is due: its hold is over, or its backoff.
    retry_at: Option<Timestamp>,
}

/// Counts as busy until dropped.
struct Busy(Arc<Shared>);

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.busy.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Task {
    /// Handles requests until the UI's handle is gone. A run after a panic (`resumed`) takes up
    /// the window on screen again; every run sends what's queued.
    async fn run(
        mut self,
        requests: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<Request>>>,
        resumed: bool,
    ) {
        let mut requests = requests.lock().await;
        // `start`, or the restart, counted this.
        {
            let _busy = Busy(self.shared.clone());
            match self.store.calendars().await {
                Ok(stored) if !stored.is_empty() => self.publish_calendars(stored),
                Ok(_) => {}
                Err(error) => log::error!("reading calendars: {error}"),
            }
            self.list().await;
            let window = self
                .shared
                .window
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            if resumed && let Some(window) = window {
                self.window = Some(window);
                self.fetch(false).await;
                self.read().await;
            }
            self.flush(false).await;
        }
        let mut watching = true;
        loop {
            let wait = self.next_poll();
            let retry = self.retry_at.map(|at| {
                Duration::try_from(at.duration_since(Timestamp::now())).unwrap_or_default()
            });
            tokio::select! {
                request = requests.recv() => {
                    let Some(request) = request else { return };
                    let _busy = Busy(self.shared.clone());
                    self.handle(request).await;
                }
                () = tokio::time::sleep(retry.unwrap_or_default()), if retry.is_some() => {
                    let _busy = self.busy();
                    self.flush(false).await;
                }
                changed = self.connection.changed(), if watching => match changed {
                    Ok(()) => {
                        let _busy = self.busy();
                        self.retry_guests();
                        self.flush(false).await;
                    }
                    Err(_) => watching = false,
                },
                () = tokio::time::sleep(wait) => {
                    let _busy = self.busy();
                    if self.listed.is_none_or(|at| at.elapsed() >= LIST_FRESH) {
                        self.list().await;
                    }
                    if self.fetch(true).await {
                        self.read().await;
                    }
                }
            }
        }
    }

    /// Counts the task as busy until the guard drops.
    fn busy(&self) -> Busy {
        self.shared.busy.fetch_add(1, Ordering::SeqCst);
        Busy(self.shared.clone())
    }

    async fn handle(&mut self, request: Request) {
        match request {
            Request::Show(window) => {
                self.window = Some(window);
                self.set_loading(true);
                self.read().await;
                if self.fetch(false).await {
                    self.read().await;
                }
                self.set_loading(false);
            }
            Request::Queue {
                token,
                change,
                held,
            } => {
                // Undo took it back before it was queued.
                if !self.shared.tokens().undone.remove(&token) {
                    let hold = held.then(|| Timestamp::now() + HOLD);
                    if let Err(error) = self.store.queue_calendar_change(token, *change, hold).await
                    {
                        log::error!("queueing a calendar change: {error}");
                    }
                    self.read().await;
                }
                self.flush(false).await;
            }
            Request::Undo(token) => {
                if let Err(error) = self.store.unqueue_calendar_change(token).await {
                    log::error!("taking back a calendar change: {error}");
                }
                self.shared.tokens().undone.remove(&token);
                self.read().await;
            }
            Request::Busy(calendars, from, to) => {
                match self.store.free_busy(&calendars, from, to).await {
                    Ok(found) => {
                        let mut data = Data::clone(&self.shared.data.borrow());
                        data.busy = found.into_iter().collect();
                        data.busy_range = Some((from, to));
                        self.shared.data.send_replace(Arc::new(data));
                        self.repaint.request();
                    }
                    Err(error) => log::warn!("reading free/busy: {error}"),
                }
            }
            Request::SendNow(done) => {
                self.flush(true).await;
                let _ = done.send(());
            }
            Request::Invite(thread) => self.load_invite(thread).await,
            Request::Accounts { added } => {
                if let Some(email) = &added {
                    self.list_account(email).await;
                    self.shared.data.send_modify(|data| {
                        Arc::make_mut(data).loading_accounts.remove(email);
                    });
                }
                self.list().await;
                if self.fetch(false).await {
                    self.read().await;
                }
                self.flush(false).await;
            }
            Request::Forget { email, removed } => {
                if let Err(error) = self.store.remove_calendar_account(&email, removed).await {
                    log::error!("forgetting a calendar account's calendars: {error}");
                }
                match self.store.calendars().await {
                    Ok(calendars) => self.publish_calendars(calendars),
                    Err(error) => log::error!("reading calendars: {error}"),
                }
                self.read().await;
            }
            Request::Series(calendar, id) => match self.store.series_rules(&calendar, &id).await {
                Ok(rules) => {
                    let mut data = Data::clone(&self.shared.data.borrow());
                    data.series.insert(format!("{calendar}/{id}"), rules);
                    self.shared.data.send_replace(Arc::new(data));
                    self.repaint.request();
                }
                Err(error) => {
                    // Asked again next time the details open.
                    self.shared
                        .series_asked
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&format!("{calendar}/{id}"));
                    log::warn!("reading a series' rule: {error}");
                }
            },
            Request::Guests(guests, from, to) => self.load_guests(guests, (from, to)),
            Request::GuestLoaded(email, range, found) => {
                let mut data = Data::clone(&self.shared.data.borrow());
                // A later ask's answer stands.
                if data
                    .guests
                    .get(&email)
                    .is_some_and(|calendar| calendar.range == range)
                {
                    let events = match found {
                        Some(Some(events)) => GuestEvents::Shown(events),
                        Some(None) => GuestEvents::Hidden,
                        None => GuestEvents::Failed,
                    };
                    data.guests.insert(
                        email,
                        GuestCalendar {
                            range,
                            loading: false,
                            events,
                        },
                    );
                    self.shared.data.send_replace(Arc::new(data));
                    self.repaint.request();
                }
            }
            Request::InviteLoaded(thread, ics) => {
                let mut data = Data::clone(&self.shared.data.borrow());
                data.invite = ics.map(|ics| (thread, ics));
                self.shared.data.send_replace(Arc::new(data));
                self.repaint.request();
            }
            #[cfg(test)]
            Request::Panic => panic!("a test panic in the calendar's task"),
        }
    }

    /// How long the days on screen stay fresh.
    fn freshness(&self) -> Duration {
        if *self.presence.borrow() == Presence::Focused {
            FOCUSED
        } else {
            UNFOCUSED
        }
    }

    /// When to look at the days on screen again.
    fn next_poll(&self) -> Duration {
        let fresh = self.freshness();
        let oldest = self.fetched.iter().map(|(_, _, at)| *at).min();
        match (self.window.as_ref(), oldest) {
            (Some(_), Some(at)) => fresh
                .saturating_sub(at.elapsed())
                .max(Duration::from_secs(1)),
            (Some(_), None) => RETRY,
            (None, _) => LIST_FRESH,
        }
    }

    /// Finds thread `id`'s invite in the store and fetches its `.ics` off the task; the bytes
    /// come back as a request.
    async fn load_invite(&self, id: String) {
        let detail = match self
            .store
            .thread(&exo_store::ThreadId::new(id.clone()))
            .await
        {
            Ok(detail) => detail,
            Err(error) => return log::error!("reading thread {id}: {error}"),
        };
        let attachment = detail
            .as_ref()
            .and_then(|detail| {
                detail
                    .messages
                    .iter()
                    .find_map(crate::thread::invite::attachment)
            })
            .cloned();
        let shared = self.shared.clone();
        shared.busy.fetch_add(1, Ordering::SeqCst);
        let Some(attachment) = attachment else {
            if self.requests.send(Request::InviteLoaded(id, None)).is_err() {
                shared.busy.fetch_sub(1, Ordering::SeqCst);
            }
            return;
        };
        let requests = self.requests.clone();
        self.loader.attachment(&attachment, move |bytes| {
            if requests
                .send(Request::InviteLoaded(id, bytes.ok()))
                .is_err()
            {
                shared.busy.fetch_sub(1, Ordering::SeqCst);
            }
        });
    }

    /// Marks `guests`' calendars as loading over `range` and reads each off the task, at once and
    /// side by side; with the connection down, they fail.
    fn load_guests(&self, guests: Vec<String>, range: (Timestamp, Timestamp)) {
        let down = self.down();
        let mut data = Data::clone(&self.shared.data.borrow());
        for email in &guests {
            let events = data
                .guests
                .remove(email)
                .map_or(GuestEvents::Pending, |calendar| calendar.events);
            let events = if down { GuestEvents::Failed } else { events };
            data.guests.insert(
                email.clone(),
                GuestCalendar {
                    range,
                    loading: !down,
                    events,
                },
            );
        }
        self.shared.data.send_replace(Arc::new(data));
        self.repaint.request();
        if down {
            return;
        }
        for email in guests {
            let (store, requests, shared) = (
                self.store.clone(),
                self.requests.clone(),
                self.shared.clone(),
            );
            shared.busy.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let found = match store.attendee_events(&email, range.0, range.1).await {
                    Ok(found) => Some(found),
                    Err(error) => {
                        log::warn!("reading a guest's calendar: {error}");
                        None
                    }
                };
                if requests
                    .send(Request::GuestLoaded(email, range, found))
                    .is_err()
                {
                    shared.busy.fetch_sub(1, Ordering::SeqCst);
                }
            });
        }
    }

    /// The connection is back: guests' calendars that couldn't be read are asked for again.
    fn retry_guests(&self) {
        if self.down() {
            return;
        }
        let data = self.shared.data.borrow();
        let failed = data
            .guests
            .iter()
            .filter(|(_, calendar)| matches!(calendar.events, GuestEvents::Failed));
        let mut asked = self
            .shared
            .guests_asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let before = asked.len();
        for (email, _) in failed {
            asked.remove(email);
        }
        // The UI asks again on its next frame.
        if asked.len() < before {
            self.repaint.request();
        }
    }

    /// Whether Calendar is out of reach too: the network or Google's sign-in is down, or nothing
    /// of Google's answers. Gmail's own limits and a disabled Gmail don't hold the calendar.
    fn down(&self) -> bool {
        matches!(
            self.connection.borrow().outage,
            Some(Outage::Offline | Outage::Auth | Outage::Gmail)
        )
    }

    /// Sends each queued change whose turn it is (held ones too when `all`), in turn, unless
    /// the connection is down, whose return sends them. A change Calendar refused says so, and
    /// the calendar shows Calendar's copy again.
    async fn flush(&mut self, all: bool) {
        let mut changed = Vec::new();
        let mut progressed = true;
        while progressed {
            progressed = false;
            if self.down() {
                self.retry_at = None;
                break;
            }
            let (due, next) = match self.store.due_calendar_changes(all).await {
                Ok(found) => found,
                Err(error) => {
                    log::error!("reading the calendar's queue: {error}");
                    break;
                }
            };
            self.retry_at = next;
            for queued in due {
                // Going out, it can't be taken back. One taken back since it was read waits
                // for Undo's request, which drops it.
                let undoable = {
                    let mut tokens = self.shared.tokens();
                    if tokens.undone.contains(&queued.token) {
                        continue;
                    }
                    tokens.undoable.remove(&queued.token)
                };
                let calendar = queued.change.calendar().to_owned();
                match self.store.send_calendar_change(&queued).await {
                    Ok(CalendarOutcome::Sent) => changed.push(calendar),
                    Ok(CalendarOutcome::Refused { gone, .. }) => {
                        self.fail(Failure::of(queued.token, queued.change, gone));
                        changed.push(calendar);
                    }
                    outcome => {
                        if let Err(error) = outcome {
                            log::error!("sending a calendar change: {error}");
                        }
                        if undoable {
                            self.shared.tokens().undoable.insert(queued.token);
                        }
                        continue;
                    }
                }
                progressed = true;
            }
        }
        if changed.is_empty() {
            return;
        }
        // A series' instances, or what a refusal left, come with a fetch.
        self.fetched.retain(|(id, ..)| !changed.contains(id));
        self.fetch(false).await;
        self.read().await;
    }

    fn fail(&self, failure: Failure) {
        self.shared
            .failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(failure);
        self.repaint.request();
    }

    /// Lists every calendar account's calendars from Google, and publishes them.
    async fn list(&mut self) {
        let mut listed = false;
        for account in self.store.calendar_accounts() {
            listed |= self.list_account(&account).await;
        }
        if listed {
            self.listed = Some(Instant::now());
        }
        match self.store.calendars().await {
            Ok(calendars) => self.publish_calendars(calendars),
            Err(error) => log::error!("reading calendars: {error}"),
        }
    }

    /// Lists `account`'s calendars from Google. Returns whether Google answered.
    async fn list_account(&self, account: &str) -> bool {
        match self.store.sync_calendars(account).await {
            Ok(_) => true,
            Err(error) => {
                log::warn!("listing calendars: {error}");
                false
            }
        }
    }

    fn set_loading(&self, loading: bool) {
        let mut data = Data::clone(&self.shared.data.borrow());
        data.loading = loading;
        self.shared.data.send_replace(Arc::new(data));
        self.repaint.request();
    }

    fn publish_calendars(&self, calendars: Vec<AccountCalendar>) {
        let mut data = Data::clone(&self.shared.data.borrow());
        data.calendars = Some(calendars);
        self.shared.data.send_replace(Arc::new(data));
        self.repaint.request();
    }

    /// Fetches the window's days of each synced calendar that hasn't been (or, when
    /// `refresh`, was fetched longer ago than the freshness allows). Returns whether anything
    /// came.
    async fn fetch(&mut self, refresh: bool) -> bool {
        let Some(window) = self.window.clone() else {
            return false;
        };
        let fresh = self.freshness();
        let mut fetched_any = false;
        for calendar in &window.synced {
            let covered = self.fetched.iter().any(|(id, days, at)| {
                id == calendar
                    && days.start <= window.days.start
                    && days.end >= window.days.end
                    && (!refresh || at.elapsed() < fresh)
            });
            if covered {
                continue;
            }
            match self
                .store
                .sync_events(calendar, window.days.clone(), &window.tz)
                .await
            {
                Ok(()) => {
                    self.fetched.retain(|(id, days, _)| {
                        !(id == calendar
                            && days.start >= window.days.start
                            && days.end <= window.days.end)
                    });
                    self.fetched
                        .push((calendar.clone(), window.days.clone(), Instant::now()));
                    fetched_any = true;
                }
                Err(error) => log::warn!("fetching a calendar's events: {error}"),
            }
        }
        fetched_any
    }

    /// Reads the window's events from the store, the queued changes made to them, and
    /// publishes them.
    async fn read(&self) {
        let Some(window) = &self.window else { return };
        match self
            .store
            .events(&window.shown, window.days.clone(), &window.tz)
            .await
        {
            Ok(events) => {
                let mut data = Data::clone(&self.shared.data.borrow());
                data.window = Some(window.clone());
                data.events = events;
                self.shared.data.send_replace(Arc::new(data));
                self.repaint.request();
            }
            Err(error) => log::error!("reading events: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use exo_store::Cut;
    use gmail::calendar::EventDateTime;

    use super::*;

    const ME: &str = "maya@brightmoor.co";

    /// An account on a new gmail-mock, its store in a folder of its own.
    fn account(
        name: &str,
    ) -> (
        tokio::runtime::Runtime,
        gmail_mock::MockServer,
        gmail::Client,
        std::path::PathBuf,
        crate::account::Account,
    ) {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let server = runtime
            .block_on(gmail_mock::MockServer::start(gmail_mock::Config::default()))
            .expect("mock starts");
        let tokens = Arc::new(gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()));
        let gmail = gmail::Client::new(server.endpoints(), tokens.as_ref().clone());
        let dir = std::env::temp_dir().join(format!("exo-calendar-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let credentials = crate::account::Credentials {
            email: ME.into(),
            endpoints: server.endpoints(),
            tokens,
        };
        let account = crate::account::Account::open(&dir, credentials, egui::Context::default())
            .expect("opens");
        (runtime, server, gmail, dir, account)
    }

    /// Waits until the task is idle and `done` holds, for a few seconds at most.
    fn until(calendar: &CalendarSync, done: impl Fn(&Data) -> bool) -> bool {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(10) {
            if calendar.is_idle() && done(&calendar.data()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn this_and_following_ends_an_all_day_series_on_the_day_before() {
        let (runtime, _server, gmail, dir, account) = account("truncate-all-day");
        let day = |date: &str| EventDateTime {
            date: Some(date.into()),
            ..EventDateTime::default()
        };
        let series = Event {
            summary: Some("Recycling out".into()),
            start: Some(day("2026-11-03")),
            end: Some(day("2026-11-04")),
            recurrence: vec!["RRULE:FREQ=WEEKLY".into()],
            ..Event::default()
        };
        let series = runtime
            .block_on(
                gmail
                    .calendar()
                    .insert_event(ME, &series, None)
                    .into_future(),
            )
            .expect("inserts")
            .id
            .expect("an id");
        // Deleting the third, on the 17th, and the ones after.
        account.calendar().hold(CalendarChange::Truncate {
            calendar: ME.into(),
            series: series.clone(),
            from: Cut::Day(Date::constant(2026, 11, 17)),
            notify: false,
        });
        // Quitting sends what's held.
        drop(account);
        let instances = runtime
            .block_on(
                gmail
                    .calendar()
                    .instances(ME, &series, None, Some("2027-01-01T00:00:00Z"))
                    .into_future(),
            )
            .expect("instances");
        let days: Vec<String> = instances
            .items
            .iter()
            .filter_map(|event| event.start.as_ref()?.date.clone())
            .collect();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(days, ["2026-11-03", "2026-11-10"]);
    }

    #[test]
    fn the_task_runs_again_after_a_panic_and_keeps_its_window_and_held_changes() {
        let (runtime, _server, gmail, dir, account) = account("calendar-panic");
        let calendar = account.calendar();
        let tz = TimeZone::get("America/New_York").expect("zone");
        let friday = Date::constant(2026, 11, 20);
        calendar.show(Window {
            days: friday..friday.tomorrow().expect("a day"),
            shown: vec![ME.into()],
            synced: vec![ME.into()],
            tz: tz.clone(),
        });
        let at = |hour: i8| EventDateTime {
            date_time: Some(
                friday
                    .at(hour, 0, 0, 0)
                    .to_zoned(tz.clone())
                    .expect("exists")
                    .strftime("%Y-%m-%dT%H:%M:%S%:z")
                    .to_string(),
            ),
            ..EventDateTime::default()
        };
        let lunch = Event {
            id: Some("e0lunch0009".into()),
            summary: Some("Lunch".into()),
            start: Some(at(12)),
            end: Some(at(13)),
            ..Event::default()
        };
        let token = calendar.hold(CalendarChange::Create {
            calendar: ME.into(),
            event: lunch,
        });
        let has_lunch = |data: &Data| {
            data.events
                .iter()
                .any(|stored| stored.event.summary.as_deref() == Some("Lunch"))
        };
        assert!(until(calendar, has_lunch), "held, it shows");

        calendar.post(Request::Panic);
        // The task that runs again has the window, and Undo still takes the change back.
        assert!(calendar.undo(token));
        assert!(
            until(calendar, |data| !has_lunch(data)),
            "the run after the panic took it back"
        );
        drop(account);
        let query = gmail::calendar::EventsQuery {
            q: Some("Lunch".into()),
            ..Default::default()
        };
        let found = runtime
            .block_on(gmail.calendar().events(ME, &query).into_future())
            .expect("lists");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(found.items.is_empty(), "Calendar never heard of it");
    }
}
