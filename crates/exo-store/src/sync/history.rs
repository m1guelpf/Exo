//! Incremental sync from Gmail history: every page is collected first, changes are grouped per
//! thread and applied idempotently, and the cursor moves only once everything is stored.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use futures::{StreamExt, stream};
use gmail::{History, HistoryQuery};

use super::{ATTEMPTS, fetch_message, fetch_rows};
use crate::db::{self, HISTORY_ID, LabelRow, MessageRow};
use crate::error::Result;
use crate::ids::labels::{CHAT, DRAFT};
use crate::parse::parse;
use crate::provider;
use crate::quota::Lane;
use crate::store::{Inner, reindex};

const PAGE_SIZE: u32 = 500;
/// More than this many pages or changes means the cache is too far behind to patch: start over.
const MAX_PAGES: usize = 100;
const MAX_CHANGES: usize = 10_000;
/// Threads applied at once, and messages fetched at once per thread.
const THREAD_CONCURRENCY: usize = 3;
const MESSAGE_CONCURRENCY: usize = 5;

/// One thread's changes from a history run.
#[derive(Debug, Default)]
struct ThreadChanges {
    added: BTreeSet<String>,
    deleted: BTreeSet<String>,
    /// Each message's labels after its newest change, with that change's id.
    labels: BTreeMap<String, (u64, Vec<String>)>,
    /// The newest record touching the thread.
    latest: u64,
    /// The changes to its messages, counted for [`SyncProgress`](crate::SyncProgress).
    changes: usize,
}

/// What a run of history did: how many threads it changed, and the messages it added that the
/// store didn't have (thread and message ids).
#[derive(Debug, Default)]
pub(super) struct Applied {
    pub changed: usize,
    pub arrived: Vec<(String, String)>,
}

impl Inner {
    /// Applies Gmail history since the stored cursor, or returns `None` when history can't be
    /// used and the cache must be rebuilt.
    pub(super) async fn sync_history(&self) -> Result<Option<Applied>> {
        // However it ends, a failure included, the round is over.
        let _caught_up = CaughtUp(&self.progress);
        let mut db = self.db.clone();
        let Some(start) = db::kv_get::<u64>(&mut db, HISTORY_ID).await? else {
            return Ok(Some(Applied::default()));
        };
        let mut records: Vec<History> = Vec::new();
        let (mut page_token, mut pages, mut changes) = (None, 0, 0);
        loop {
            let query = HistoryQuery {
                start_history_id: start,
                max_results: Some(PAGE_SIZE),
                page_token: page_token.take(),
                ..HistoryQuery::default()
            };
            let page = match self
                .api
                .call(Lane::Sync, ATTEMPTS, |gmail| gmail.history(&query))
                .await
            {
                Ok(page) => page,
                Err(failure) if failure.is_not_found() => {
                    // Expired, unless the cursor already is the mailbox's current id.
                    let profile = self
                        .api
                        .call(Lane::Sync, ATTEMPTS, |gmail| gmail.profile())
                        .await?;
                    if profile.history_id == Some(start) {
                        log::info!("history.list 404 at the current history id {start}");
                        return Ok(Some(Applied::default()));
                    }
                    return Ok(None);
                }
                Err(failure) => return Err(failure.into()),
            };
            pages += 1;
            changes += page
                .history
                .iter()
                .map(|record| {
                    record.messages_added.len()
                        + record.messages_deleted.len()
                        + record.labels_added.len()
                        + record.labels_removed.len()
                })
                .sum::<usize>();
            records.extend(page.history);
            if pages > MAX_PAGES || changes > MAX_CHANGES {
                log::warn!("{pages} pages / {changes} changes of history: rebuilding instead");
                return Ok(None);
            }
            match page.next_page_token {
                Some(token) => page_token = Some(token),
                None => break,
            }
        }
        let Some(cursor) = records.iter().filter_map(|record| record.id).max() else {
            return Ok(Some(Applied::default()));
        };

        // A label created elsewhere: pick up its name and color first, so that if this fails
        // the next sync replays the same history and tries again.
        let known: HashSet<String> = LabelRow::all()
            .select(LabelRow::fields().id())
            .exec(&mut db)
            .await?
            .into_iter()
            .collect();
        let new_label = records
            .iter()
            .flat_map(|record| {
                let added = record.messages_added.iter().map(|added| &added.message);
                let changed = record.labels_added.iter().map(|change| &change.message);
                let removed = record.labels_removed.iter().map(|change| &change.message);
                added.chain(changed).chain(removed)
            })
            .flat_map(|message| &message.label_ids)
            .any(|label| !known.contains(label));
        if new_label {
            self.refresh_labels().await?;
        }

        let threads = group(records);
        let changed = threads.len();
        let total: usize = threads.values().map(|changes| changes.changes).sum();
        let progress = &self.progress;
        progress.send_modify(|progress| progress.changes = Some((total, 0)));
        let touched: HashSet<String> = threads.keys().cloned().collect();
        // Messages no longer drafts: deleted, or (should Gmail keep the id) sent.
        let undrafted: HashSet<String> = threads
            .values()
            .flat_map(|changes| {
                let relabelled = changes
                    .labels
                    .iter()
                    .filter(|(_, (_, labels))| !labels.iter().any(|label| label == DRAFT));
                changes.deleted.iter().chain(relabelled.map(|(id, _)| id))
            })
            .cloned()
            .collect();
        let arrived = stream::iter(threads)
            .map(|(thread, changes)| async move {
                let count = changes.changes;
                let applied = self.apply_history(thread, changes).await;
                progress.send_modify(|progress| {
                    if let Some((_, done)) = &mut progress.changes {
                        *done += count;
                    }
                });
                applied
            })
            .buffer_unordered(THREAD_CONCURRENCY)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<Vec<(String, String)>>>>()?
            .concat();
        self.forget_drafts_gone(&undrafted).await?;
        // A reply in has called off an `if no reply` send.
        self.unschedule_replied(&touched).await?;
        let mut write = self.begin(&mut db).await?;
        db::kv_set(write.exec(), HISTORY_ID, &cursor).await?;
        write.commit().await?;
        Ok(Some(Applied { changed, arrived }))
    }

    /// Applies one thread's changes. Returns the messages history added that the store didn't
    /// have and now does (thread and message ids).
    async fn apply_history(
        &self,
        thread: String,
        changes: ThreadChanges,
    ) -> Result<Vec<(String, String)>> {
        let mut db = self.db.clone();
        let stored: HashSet<String> = MessageRow::filter_by_thread_id(&thread)
            .select(MessageRow::fields().id())
            .exec(&mut db)
            .await?
            .into_iter()
            .collect();
        if stored.is_empty() {
            // A thread we don't have: fetch its rows if it gained mail or labels that are still
            // there (it may now belong in a list), ignore deletions. A message added and deleted
            // since (a draft's earlier version, say) leaves nothing to fetch.
            if changes
                .added
                .iter()
                .chain(changes.labels.keys())
                .all(|id| changes.deleted.contains(id))
            {
                return Ok(Vec::new());
            }
            let Some(fetched) =
                fetch_rows(&self.api, Lane::Sync, ATTEMPTS, &thread, &stored).await?
            else {
                return Ok(Vec::new());
            };
            let arrived = fetched
                .thread
                .messages
                .iter()
                .filter_map(|message| message.id.clone())
                .filter(|id| changes.added.contains(id))
                .map(|id| (thread.clone(), id))
                .collect();
            let mut write = self.begin(&mut db).await?;
            if let Some(id) = provider::save_rows(write.exec(), &fetched).await? {
                reindex(write.exec(), &id, &self.router()).await?;
            }
            write.publish().await?;
            return Ok(arrived);
        }

        let missing: BTreeSet<String> = changes
            .added
            .iter()
            .chain(changes.labels.keys())
            .filter(|id| !stored.contains(*id) && !changes.deleted.contains(*id))
            .cloned()
            .collect();
        let api = &self.api;
        let fetched: Vec<_> = stream::iter(missing)
            .map(|id| async move { fetch_message(api, Lane::Sync, ATTEMPTS, &id).await })
            .buffer_unordered(MESSAGE_CONCURRENCY)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<_, _>>()?;

        let arrived = fetched
            .iter()
            .flatten()
            .filter_map(|message| message.id.clone())
            .filter(|id| changes.added.contains(id))
            .map(|id| (thread.clone(), id))
            .collect();
        let mut write = self.begin(&mut db).await?;
        for parsed in fetched
            .iter()
            .flatten()
            .filter_map(|message| parse(message, true))
        {
            provider::save_message(write.exec(), parsed).await?;
        }
        for (id, (record, labels)) in &changes.labels {
            provider::set_labels(write.exec(), id, labels, *record).await?;
        }
        let deleted: Vec<String> = changes.deleted.into_iter().collect();
        provider::delete_messages(write.exec(), &deleted).await?;
        let emptied = MessageRow::filter_by_thread_id(&thread)
            .first()
            .exec(write.exec())
            .await?
            .is_none();
        if emptied {
            provider::delete_thread(write.exec(), &thread).await?;
        } else {
            provider::bump_thread(write.exec(), &thread, changes.latest).await?;
            reindex(write.exec(), &thread, &self.router()).await?;
        }
        write.publish().await?;
        Ok(arrived)
    }
}

/// Groups history records by thread. Chat messages are skipped; drafts are kept, since Exo
/// shows Gmail drafts.
fn group(records: Vec<History>) -> BTreeMap<String, ThreadChanges> {
    let mut threads: BTreeMap<String, ThreadChanges> = BTreeMap::new();
    for record in records {
        let id = record.id.unwrap_or_default();
        for added in &record.messages_added {
            if added.message.label_ids.iter().any(|label| label == CHAT) {
                continue;
            }
            if let Some((thread, message)) = touch(&mut threads, id, &added.message) {
                thread.added.insert(message);
            }
        }
        for deleted in &record.messages_deleted {
            if let Some((thread, message)) = touch(&mut threads, id, &deleted.message) {
                thread.deleted.insert(message);
            }
        }
        let changed = record
            .labels_added
            .iter()
            .map(|change| &change.message)
            .chain(record.labels_removed.iter().map(|change| &change.message));
        for message in changed {
            let labels = message.label_ids.clone();
            if let Some((thread, message)) = touch(&mut threads, id, message) {
                let entry = thread.labels.entry(message).or_insert((0, Vec::new()));
                if id >= entry.0 {
                    *entry = (id, labels);
                }
            }
        }
    }
    threads
}

/// The thread a record touches (bumping its newest record id), and the message id.
fn touch<'a>(
    threads: &'a mut BTreeMap<String, ThreadChanges>,
    record: u64,
    message: &gmail::Message,
) -> Option<(&'a mut ThreadChanges, String)> {
    let thread = threads.entry(message.thread_id.clone()?).or_default();
    thread.latest = thread.latest.max(record);
    thread.changes += 1;
    Some((thread, message.id.clone()?))
}

/// Marks a round of history over when dropped: none running, and the session caught up.
struct CaughtUp<'a>(&'a tokio::sync::watch::Sender<crate::SyncProgress>);

impl Drop for CaughtUp<'_> {
    fn drop(&mut self) {
        self.0.send_replace(crate::SyncProgress {
            caught_up: true,
            changes: None,
        });
    }
}
