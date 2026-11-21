//! Shared test helpers: a mock Gmail, a store pointed at it, and a second client that plays
//! "Gmail web" to check what the store sent and to change things behind its back.

#![allow(dead_code)] // Each test binary uses a different subset of these helpers.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use exo_store::{ListId, Options, Store, ThreadId, ThreadSummary};
use gmail::{Format, ListQuery, TokenFuture, TokenProvider};
use gmail_mock::{Config, DEV_ACCESS_TOKEN, MockServer};
use serde_json::{Value, json};
use tokio::sync::Notify;

pub const ACCOUNT: &str = "maya@brightmoor.co";

/// A directory removed on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "exo-store-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub struct Harness {
    pub server: MockServer,
    pub dir: TempDir,
    pub store: Store,
    /// Gmail as another client sees it.
    pub gmail: gmail::Client,
}

/// The debounce [`options`] gives outages: the default 10 s and 60 s, in the same ratio but
/// short enough for tests.
pub const DEBOUNCE: exo_store::Debounce = exo_store::Debounce {
    down: Duration::from_millis(300),
    bounce: Duration::from_millis(1800),
};

/// A short backoff, no pacing under a quota (the mock enforces none unless a test asks it to),
/// a short offline debounce, and drafts written at their real pace.
pub fn options() -> Options {
    Options {
        backoff: Duration::from_millis(5),
        quota: None,
        debounce: DEBOUNCE,
        ..Options::default()
    }
}

/// [`options`] with every push writing every draft, for tests of what one write does that
/// write the same draft twice in a row.
pub fn unpaced() -> Options {
    Options {
        draft_pace: Duration::ZERO,
        draft_files_pace: Duration::ZERO,
        ..options()
    }
}

impl Harness {
    /// A fresh mock and an empty store.
    pub async fn start() -> Self {
        Self::start_with(Config::default(), options()).await
    }

    /// A fresh mock built from `config`, and an empty store opened with `options`.
    pub async fn start_with(config: Config, options: Options) -> Self {
        let server = MockServer::start(config).await.expect("mock starts");
        let dir = TempDir::new();
        let store = Store::open_with(
            dir.path(),
            ACCOUNT,
            server.endpoints(),
            gmail::StaticToken(DEV_ACCESS_TOKEN.into()),
            options,
        )
        .await
        .expect("store opens");
        let gmail = gmail::Client::new(
            server.endpoints(),
            gmail::StaticToken(DEV_ACCESS_TOKEN.into()),
        );
        Self {
            server,
            dir,
            store,
            gmail,
        }
    }

    /// A fresh mock and a store that has finished its initial sync.
    pub async fn synced() -> Self {
        Self::synced_with(options()).await
    }

    /// [`synced`](Self::synced), the store opened with `options`.
    pub async fn synced_with(options: Options) -> Self {
        let harness = Self::start_with(Config::default(), options).await;
        harness.sync_all().await;
        harness
    }

    /// Syncs until the background backfill is done.
    pub async fn sync_all(&self) {
        for _ in 0..50 {
            if !self.store.sync().await.expect("sync").backfilling {
                return;
            }
        }
        panic!("backfill never finished");
    }

    /// Reopens the store on the same file (a restart).
    pub async fn reopen(&mut self) {
        self.store = open(&self.server, self.dir.path()).await;
    }

    /// Reopens the store with a [`Gate`] as its token provider, to pause it mid-request.
    pub async fn reopen_gated(&mut self) -> Gate {
        let gate = Gate::default();
        self.store = Store::open_with(
            self.dir.path(),
            ACCOUNT,
            self.server.endpoints(),
            gate.clone(),
            options(),
        )
        .await
        .expect("store opens");
        gate
    }

    /// Every thread of `list`, paging through.
    pub async fn all(&self, list: &ListId) -> Vec<ThreadSummary> {
        let mut threads = Vec::new();
        let mut after = None;
        loop {
            let page = self
                .store
                .list(list, after.as_ref(), 7)
                .await
                .expect("list");
            threads.extend(page.threads);
            match page.next {
                Some(next) => after = Some(next),
                None => return threads,
            }
        }
    }

    pub async fn ids(&self, list: &ListId) -> BTreeSet<String> {
        self.all(list)
            .await
            .into_iter()
            .map(|thread| thread.id.as_str().to_owned())
            .collect()
    }

    pub async fn summary(&self, list: &ListId, id: &ThreadId) -> Option<ThreadSummary> {
        self.all(list)
            .await
            .into_iter()
            .find(|thread| &thread.id == id)
    }

    /// Thread ids Gmail lists for `query`, spam and trash included when asked.
    pub async fn gmail_ids(&self, query: ListQuery) -> BTreeSet<String> {
        let mut ids = BTreeSet::new();
        let mut query = ListQuery {
            max_results: Some(500),
            ..query
        };
        loop {
            let page = self
                .gmail
                .gmail()
                .threads(&query)
                .await
                .expect("threads.list");
            ids.extend(page.threads.into_iter().filter_map(|thread| thread.id));
            match page.next_page_token {
                Some(token) => query.page_token = Some(token),
                None => return ids,
            }
        }
    }

    /// Each message's labels, as Gmail has them now.
    pub async fn gmail_labels(&self, thread: &ThreadId) -> Vec<(String, BTreeSet<String>)> {
        self.gmail
            .gmail()
            .thread(thread.as_str(), Format::Minimal, &[])
            .await
            .expect("threads.get")
            .messages
            .into_iter()
            .map(|message| {
                (
                    message.id.unwrap_or_default(),
                    message.label_ids.into_iter().collect(),
                )
            })
            .collect()
    }

    /// Delivers a message as if by SMTP (`/__mock/deliver`).
    pub async fn deliver(&self, body: Value) -> gmail::Message {
        let response = self
            .admin(reqwest::Method::POST, "/__mock/deliver", body)
            .await;
        serde_json::from_value(response).expect("delivered message")
    }

    /// Arms a fault (`/__mock/faults`).
    pub async fn fault(&self, fault: Value) {
        self.admin(reqwest::Method::POST, "/__mock/faults", fault)
            .await;
    }

    pub async fn clear_faults(&self) {
        self.admin(reqwest::Method::DELETE, "/__mock/faults", json!({}))
            .await;
    }

    pub async fn admin(&self, method: reqwest::Method, path: &str, body: Value) -> Value {
        let url = self.server.url().join(path).expect("admin path");
        let response = reqwest::Client::new()
            .request(method, url)
            .json(&body)
            .send()
            .await
            .expect("admin request");
        assert!(
            response.status().is_success(),
            "{path}: {}",
            response.status()
        );
        response.json().await.unwrap_or(Value::Null)
    }
}

/// A token provider that can hold one request until released. Every Gmail call asks for a
/// token first, so holding one pauses the store right before that call goes out.
#[derive(Clone)]
pub struct Gate(Arc<GateState>);

struct GateState {
    /// Requests still let through before one is held; negative once it has been.
    pass: AtomicI64,
    held: Notify,
    release: Notify,
}

impl Default for Gate {
    fn default() -> Self {
        Self(Arc::new(GateState {
            pass: AtomicI64::new(i64::MAX),
            held: Notify::new(),
            release: Notify::new(),
        }))
    }
}

impl Gate {
    /// Lets `requests` more requests through, then holds the next one.
    pub fn hold_after(&self, requests: i64) {
        self.0.pass.store(requests, Ordering::SeqCst);
    }

    /// Waits until a request is being held.
    pub async fn held(&self) {
        self.0.held.notified().await;
    }

    /// Lets the held request go.
    pub fn release(&self) {
        self.0.release.notify_one();
    }
}

impl TokenProvider for Gate {
    fn access_token(&self) -> TokenFuture<'_> {
        Box::pin(async move {
            if self.0.pass.fetch_sub(1, Ordering::SeqCst) == 0 {
                self.0.held.notify_one();
                self.0.release.notified().await;
            }
            Ok(DEV_ACCESS_TOKEN.to_owned())
        })
    }
}

pub async fn open(server: &MockServer, dir: &Path) -> Store {
    try_open(server, dir).await.expect("store opens")
}

/// [`open`], which may fail.
pub async fn try_open(server: &MockServer, dir: &Path) -> exo_store::Result<Store> {
    Store::open_with(
        dir,
        ACCOUNT,
        server.endpoints(),
        gmail::StaticToken(DEV_ACCESS_TOKEN.into()),
        options(),
    )
    .await
}
