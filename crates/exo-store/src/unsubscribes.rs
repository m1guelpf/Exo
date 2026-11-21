//! One-click unsubscribes (RFC 8058) waiting to be posted. The app posts them itself, and keeps
//! them here (a JSON list in `kv`, so no schema change) until it has, so one asked for offline,
//! or just before a quit, still goes.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::db;
use crate::error::Result;
use crate::store::Store;

/// Where the queue is kept in `kv`.
pub(crate) const KEY: &str = "unsubscribes";

/// An unsubscribe waiting to be posted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingUnsubscribe {
    /// The link to post to.
    pub url: String,
    /// When it was asked for.
    pub queued: Timestamp,
    /// Attempts that failed for a reason trying again could fix.
    pub failures: u32,
    /// When to try again, after a failure.
    pub not_before: Option<Timestamp>,
}

impl Store {
    /// Queues a one-click unsubscribe from `url`, unless it's already waiting.
    ///
    /// # Errors
    /// On database failures.
    pub async fn queue_unsubscribe(&self, url: &str) -> Result<()> {
        self.change_unsubscribes(|queue| {
            if !queue.iter().any(|pending| pending.url == url) {
                queue.push(PendingUnsubscribe {
                    url: url.to_owned(),
                    queued: Timestamp::now(),
                    failures: 0,
                    not_before: None,
                });
            }
        })
        .await
    }

    /// The unsubscribes waiting, oldest first.
    ///
    /// # Errors
    /// On database failures.
    pub async fn pending_unsubscribes(&self) -> Result<Vec<PendingUnsubscribe>> {
        let mut db = self.inner.db.clone();
        Ok(db::kv_get(&mut db, KEY).await?.unwrap_or_default())
    }

    /// Takes `url` off the queue: posted, or given up on.
    ///
    /// # Errors
    /// On database failures.
    pub async fn finish_unsubscribe(&self, url: &str) -> Result<()> {
        self.change_unsubscribes(|queue| queue.retain(|pending| pending.url != url))
            .await
    }

    /// Counts a failed attempt at `url`, to be tried again at `until`.
    ///
    /// # Errors
    /// On database failures.
    pub async fn postpone_unsubscribe(&self, url: &str, until: Timestamp) -> Result<()> {
        self.change_unsubscribes(|queue| {
            if let Some(pending) = queue.iter_mut().find(|pending| pending.url == url) {
                pending.failures += 1;
                pending.not_before = Some(until);
            }
        })
        .await
    }

    /// Reads, changes and writes the queue in one transaction, so a post finishing and a new
    /// unsubscribe can't lose each other's change.
    async fn change_unsubscribes(
        &self,
        change: impl FnOnce(&mut Vec<PendingUnsubscribe>),
    ) -> Result<()> {
        let mut db = self.inner.db.clone();
        let mut write = self.inner.begin(&mut db).await?;
        let mut queue: Vec<PendingUnsubscribe> =
            db::kv_get(write.exec(), KEY).await?.unwrap_or_default();
        change(&mut queue);
        db::kv_set(write.exec(), KEY, &queue).await?;
        write.commit().await
    }
}
