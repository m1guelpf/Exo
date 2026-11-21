//! The search index's text: each message's visible text, without the quoted reply and the
//! signature the thread view hides behind the `…`. Working it out takes exo-mail's render
//! pipeline, which the store doesn't depend on, so the app hands the store the function that
//! does ([`Store::index_text_with`]).
//!
//! Storing a body queues its conversation (`text_due`, which the migration that made it filled
//! with every conversation stored before); the app's pre-render pass then works out the text of
//! what's queued ([`Store::index_texts`]) off the runtime's threads, keeps it beside the body
//! ([`BodyRow::visible`]) and indexes the conversation again. A message whose reply-to's body
//! isn't stored waits for it: the conversation is queued again when it comes. Until then, and in
//! a store without the function, the index holds a body's whole text.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use toasty::Executor;

use crate::db::{AttachmentRow, BodyRow, MessageRow, strings};
use crate::error::Result;
use crate::ids::labels::DRAFT;
use crate::model::Body;
use crate::store::{Inner, Store, reindex};

/// One message of a conversation, for working out its visible text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSource {
    /// The Gmail message id.
    pub id: String,
    /// Its HTML and plain-text parts.
    pub body: Body,
    /// `From`'s address (empty without one).
    pub from_email: String,
    /// `From`'s name (empty without one).
    pub from_name: String,
    /// `Message-ID`, without angle brackets (empty without one).
    pub rfc822_id: String,
    /// The names of its files (not its inline images).
    pub attachment_names: Vec<String>,
    /// The message it replies to, before it in the slice: the one its `In-Reply-To` names, else
    /// the one before it in the conversation, as the thread view reads each message against.
    pub replying_to: Option<usize>,
}

/// Works out the visible text of each of `sources` (one conversation's messages, each after the
/// one it replies to), in their order.
pub type IndexText = Arc<dyn Fn(&[TextSource]) -> Vec<String> + Send + Sync>;

/// Conversations worked out per batch, between which the pass pauses.
const BATCH: usize = 50;
/// The pause after each batch, so sync and the user's writes go first.
const PAUSE: std::time::Duration = std::time::Duration::from_millis(50);
/// `?1` of the queued conversations, the last queued first: new mail before what was stored
/// before visible text.
const DUE: &str = "SELECT thread_id FROM text_due ORDER BY rowid DESC LIMIT ?1";
/// The messages of conversation `?1` with a body and no visible text.
const LACKING: &str = "SELECT b.message_id FROM bodies b JOIN messages m ON m.id = b.message_id WHERE m.thread_id = ?1 AND b.visible IS NULL";

impl Store {
    /// Has the search index hold each message's visible text, worked out by `text` (exo-mail's
    /// render pipeline, in the app), rather than its whole body. The first call counts.
    pub fn index_text_with(&self, text: IndexText) {
        let _ = self.inner.index_text.set(text);
    }

    /// Works out the visible text of every conversation whose bodies came in since the last
    /// time, in batches, newest first, and indexes them again with it. It calls Gmail for
    /// nothing, and tells [`changes`](Self::changes) nothing: lists and conversations look the
    /// same. Does nothing without [`index_text_with`](Self::index_text_with). Returns how many
    /// it went through.
    ///
    /// # Errors
    /// On database failures; what was worked out before stays.
    pub async fn index_texts(&self) -> Result<usize> {
        let mut done = 0;
        loop {
            let batch = self.inner.index_texts(BATCH).await?;
            done += batch;
            if batch < BATCH {
                return Ok(done);
            }
            tokio::time::sleep(PAUSE).await;
        }
    }
}

impl Inner {
    /// Works out the visible text of up to `limit` queued conversations and indexes them again
    /// with it, one at a time, the work itself on a blocking thread. Returns how many it did.
    async fn index_texts(&self, limit: usize) -> Result<usize> {
        let Some(text) = self.index_text.get().cloned() else {
            return Ok(0);
        };
        let mut db = self.db.clone();
        let due: Vec<String> = strings(
            toasty::sql::query(DUE)
                .bind(i64::try_from(limit).unwrap_or(i64::MAX))
                .exec(&mut db)
                .await?,
        )
        .collect();
        for thread in &due {
            let work = Work::read(&mut db, thread).await?;
            let mut texts = Vec::new();
            if !work.wanted.is_empty() {
                let (text, sources) = (text.clone(), work.sources.clone());
                match tokio::task::spawn_blocking(move || text(&sources)).await {
                    Ok(worked_out) => texts = worked_out,
                    // Not tried again: the index keeps the whole text.
                    Err(error) => {
                        log::error!("working out the visible text of thread {thread}: {error}")
                    }
                }
            }
            let mut write = self.begin(&mut db).await?;
            let worked_out: Vec<(&str, String)> = texts
                .into_iter()
                .enumerate()
                .filter(|(at, _)| work.wanted.contains(at))
                .map(|(at, text)| (work.sources[at].id.as_str(), text))
                .collect();
            for (id, text) in &worked_out {
                BodyRow::filter(BodyRow::fields().message_id().eq(*id))
                    .update()
                    .visible(Some(text.clone()))
                    .exec(write.exec())
                    .await?;
            }
            if !worked_out.is_empty() {
                // The index keeps a conversation's text while its bodies are the same ones.
                crate::search::unindex(write.exec(), thread).await?;
                reindex(write.exec(), thread, &self.router()).await?;
            }
            // A body stored meanwhile keeps it queued.
            if work.bodied_since(write.exec(), thread).await?.is_empty() {
                toasty::sql::statement("DELETE FROM text_due WHERE thread_id = ?1")
                    .bind(thread.clone())
                    .exec(write.exec())
                    .await?;
            }
            write.commit().await?;
        }
        Ok(due.len())
    }
}

/// Queues `thread`, one of whose bodies was just stored.
pub(crate) async fn queue(exec: &mut dyn Executor, thread: &str) -> Result<()> {
    toasty::sql::statement("INSERT OR IGNORE INTO text_due (thread_id) VALUES (?1)")
        .bind(thread.to_owned())
        .exec(exec)
        .await?;
    Ok(())
}

/// What one conversation's visible text takes: the messages to work it out for and those they
/// reply to.
struct Work {
    /// In the conversation's order, each after the one it replies to.
    sources: Vec<TextSource>,
    /// Which of `sources` lack their visible text (the rest are what they reply to).
    wanted: HashSet<usize>,
    /// The messages with a body when this was read.
    bodied: HashSet<String>,
}

impl Work {
    /// Reads `thread`: every message with a body and no visible text yet whose reply-to's body
    /// is stored too, with what it replies to. Drafts reply to nothing, as the thread view leaves
    /// them to compose.
    async fn read(exec: &mut dyn Executor, thread: &str) -> Result<Self> {
        let mut rows = MessageRow::filter_by_thread_id(thread).exec(exec).await?;
        rows.sort_by(|a, b| (a.internal_date, &a.id).cmp(&(b.internal_date, &b.id)));
        let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
        let mut bodies: HashMap<String, BodyRow> =
            BodyRow::filter(BodyRow::fields().message_id().in_list(ids.as_slice()))
                .exec(exec)
                .await?
                .into_iter()
                .map(|body| (body.message_id.clone(), body))
                .collect();
        let mut files: HashMap<String, Vec<String>> = HashMap::new();
        for row in
            AttachmentRow::filter(AttachmentRow::fields().message_id().in_list(ids.as_slice()))
                .exec(exec)
                .await?
        {
            if !row.inline {
                files.entry(row.message_id).or_default().push(row.filename);
            }
        }
        let bodied: HashSet<String> = bodies.keys().cloned().collect();
        let (drafts, chain): (Vec<&MessageRow>, Vec<&MessageRow>) = rows
            .iter()
            .partition(|row| row.label_ids.iter().any(|label| label == DRAFT));
        // Each message of the chain that needs its text, and the one it replies to.
        let mut needed: Vec<(usize, Option<usize>)> = Vec::new();
        for (at, row) in chain.iter().enumerate() {
            if !bodies
                .get(&row.id)
                .is_some_and(|body| body.visible.is_none())
            {
                continue;
            }
            let parent = row
                .in_reply_to
                .as_ref()
                .and_then(|parent| {
                    chain[..at]
                        .iter()
                        .position(|earlier| earlier.rfc822_id.as_ref() == Some(parent))
                })
                .or(at.checked_sub(1));
            if parent.is_none_or(|parent| bodies.contains_key(&chain[parent].id)) {
                needed.push((at, parent));
            }
        }
        let mut taken: Vec<usize> = needed
            .iter()
            .flat_map(|(at, parent)| [Some(*at), *parent])
            .flatten()
            .collect();
        taken.sort_unstable();
        taken.dedup();
        let wanted_chain: HashSet<usize> = needed.iter().map(|(at, _)| *at).collect();
        let parents: HashMap<usize, Option<usize>> = needed.into_iter().collect();
        let drafts: Vec<&MessageRow> = drafts
            .into_iter()
            .filter(|draft| {
                bodies
                    .get(&draft.id)
                    .is_some_and(|body| body.visible.is_none())
            })
            .collect();
        let mut source = |row: &MessageRow, replying_to: Option<usize>| {
            let body = bodies
                .remove(&row.id)
                .map(|body| Body {
                    html: body.html,
                    text: body.text,
                })
                .unwrap_or_default();
            let sender = row.sender.0.as_ref();
            TextSource {
                id: row.id.clone(),
                body,
                from_email: sender
                    .map(|sender| sender.email.clone())
                    .unwrap_or_default(),
                from_name: sender
                    .and_then(|sender| sender.name.clone())
                    .unwrap_or_default(),
                rfc822_id: row.rfc822_id.clone().unwrap_or_default(),
                attachment_names: files.remove(&row.id).unwrap_or_default(),
                replying_to,
            }
        };
        let mut sources = Vec::new();
        let mut wanted = HashSet::new();
        for at in &taken {
            let replying_to = parents
                .get(at)
                .copied()
                .flatten()
                .and_then(|parent| taken.iter().position(|taken| *taken == parent));
            if wanted_chain.contains(at) {
                wanted.insert(sources.len());
            }
            sources.push(source(chain[*at], replying_to));
        }
        for draft in drafts {
            wanted.insert(sources.len());
            sources.push(source(draft, None));
        }
        Ok(Self {
            sources,
            wanted,
            bodied,
        })
    }

    /// The messages of `thread` that have a body without visible text and didn't have a body
    /// when this was read.
    async fn bodied_since(&self, exec: &mut dyn Executor, thread: &str) -> Result<Vec<String>> {
        let lacking = toasty::sql::query(LACKING)
            .bind(thread.to_owned())
            .exec(exec)
            .await?;
        Ok(strings(lacking)
            .filter(|id| !self.bodied.contains(id))
            .collect())
    }
}
