//! Sync: snapshot the history cursor before anything else, refresh labels and identity, apply
//! Gmail history incrementally, and backfill lists page by page with the coverage rule. Expired
//! history triggers a soft bankruptcy that keeps the outbox.

mod arrivals;
mod backfill;
mod bodies;
mod history;

pub use arrivals::Arrival;
pub use backfill::RECENT_THREADS;
pub(crate) use backfill::{LIST_CAP, selection};

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;

use futures::future;
use gmail::{Format, Label, LabelListVisibility, Message, MessageListVisibility, MessagePart};
use tokio::sync::MutexGuard;

use crate::api::{Api, Failure};
use crate::db::{
    self, ALIASES, AttachmentRow, BodyRow, HISTORY_ID, IDENTITY, KvRow, LabelRow, ListCursor,
    ListEntry, MessageRow, ParticipantEntry, ThreadRow,
};
use crate::error::Result;
use crate::ids::ListId;
use crate::ids::labels::{CHAT, EXO_MUTED, EXO_PARENT, EXO_REMINDER_UNREAD, EXO_SNOOZED};
use crate::model::Alias;
use crate::quota::Lane;
use crate::store::{Inner, Store, reindex};
use crate::view::{Identity, Router};

/// Tries per sync call before giving up.
pub(crate) const ATTEMPTS: u32 = 5;
/// The largest `text/html` body Gmail moved to an attachment that's put back in its part.
const MAX_INLINED_BODY: u32 = 5 * 1024 * 1024;
/// The headers a message's row reads (lists, splits, the search index, the contact pane and
/// replies), all `format=metadata` is asked for. `Content-Type` stands in for the top-level
/// MIME type should Gmail leave `mimeType` out, and `Content-Class` marks Exchange's invites
/// ([`needs_structure`]).
pub(crate) const ROW_HEADERS: [&str; 15] = [
    "From",
    "To",
    "Cc",
    "Bcc",
    "Reply-To",
    "Subject",
    "Message-ID",
    "In-Reply-To",
    "References",
    "List-Id",
    "List-Unsubscribe",
    "List-Unsubscribe-Post",
    "Delivered-To",
    "Content-Type",
    "Content-Class",
];
/// Top-level MIME types whose message holds no file or invite, so its row needs none of its
/// structure. Anything else (`multipart/mixed`, `multipart/related`, `text/calendar`, …) may.
const BODY_ONLY: [&str; 3] = ["text/plain", "text/html", "multipart/alternative"];

/// How far Gmail's history is applied this session, as the footer shows it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncProgress {
    /// A round of history ended this session, however it ended.
    pub caught_up: bool,
    /// While a round applies history: the changes it has to its messages, and how many it has
    /// applied.
    pub changes: Option<(usize, usize)>,
}

/// What one [`Store::sync`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Threads Gmail history changed.
    pub changed: usize,
    /// The background backfill isn't finished: call `sync` again soon (on an idle tick).
    pub backfilling: bool,
    /// History had expired, so the cache was rebuilt from scratch. Queued mutations were kept.
    pub rebuilt: bool,
    /// The messages Gmail history brought that the store didn't have, oldest first: new mail.
    /// None from a rebuild or a backfill.
    pub arrived: Vec<Arrival>,
}

impl Store {
    /// One round of sync: first-run setup, Gmail history since the last round, then one page
    /// of each background backfill (All Mail for 31 days; Inbox, Important and Other up to
    /// 1250 threads each). The sync lock is let go between pages, so a list opened meanwhile
    /// ([`backfill`](Self::backfill)) waits for one page at most.
    ///
    /// # Errors
    /// [`Error::Offline`](crate::Error::Offline), [`Error::Throttled`](crate::Error::Throttled)
    /// and the like when Gmail can't be reached or refuses; local data stays as it was.
    pub async fn sync(&self) -> Result<SyncReport> {
        let inner = &self.inner;
        let mut report = SyncReport::default();
        {
            let _sync = inner.lock_sync().await?;
            match inner.sync_history().await? {
                Some(applied) => {
                    report.changed = applied.changed;
                    report.arrived = inner.arrivals(&applied.arrived).await?;
                }
                None => {
                    inner.bootstrapped.store(false, Ordering::Release);
                    inner.bankrupt().await?;
                    inner.bootstrap().await?;
                    inner.bootstrapped.store(true, Ordering::Release);
                    report.rebuilt = true;
                }
            }
        }
        report.backfilling = inner.backfill_round().await?;
        Ok(report)
    }

    /// How far Gmail's history is applied, as it changes.
    #[must_use]
    pub fn sync_progress(&self) -> tokio::sync::watch::Receiver<SyncProgress> {
        self.inner.progress.subscribe()
    }

    /// Says which list is on screen: when it's one of the background passes (Inbox, Important,
    /// Other or a split), each round backfills it first, starting with a screenful and on to
    /// the app's first 100 threads, so after a rebuild the list in front of the user fills
    /// before the others.
    pub fn show(&self, list: &ListId) {
        *self
            .inner
            .on_screen
            .write()
            .expect("on-screen lock is never poisoned") = Some(list.clone());
    }

    /// Says how many rows of the list on screen the app has loaded: sync fetches the bodies of
    /// them all, and eviction keeps them.
    pub fn loaded(&self, rows: usize) {
        self.inner.loaded_rows.store(
            u32::try_from(rows).unwrap_or(u32::MAX),
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    /// Backfills `list` further back by at least `threads` threads (when Gmail has them), for
    /// opening a list or scrolling past its end. Returns whether Gmail has more.
    ///
    /// # Errors
    /// As [`sync`](Self::sync).
    pub async fn backfill(&self, list: &ListId, threads: usize) -> Result<bool> {
        let inner = &self.inner;
        let mut listed = 0;
        loop {
            let _sync = inner.lock_sync().await?;
            let page = inner
                .backfill_page(list, backfill::Limit::None, u32::MAX)
                .await?;
            listed += page.listed;
            if !page.more || listed >= threads {
                return Ok(page.more);
            }
        }
    }
}

/// The ids of Exo's hidden labels, once they exist.
struct HiddenLabels {
    /// `[Exo]/Muted`.
    muted: Option<String>,
    /// `[Exo]/Is Snoozed`.
    snoozed: Option<String>,
    /// `[Exo]/ru`.
    reminder_unread: Option<String>,
}

impl Inner {
    /// Takes the sync lock, bootstrapping first if this store hasn't yet.
    pub(crate) async fn lock_sync(&self) -> Result<MutexGuard<'_, ()>> {
        let sync = self.sync.lock().await;
        if !self.bootstrapped.load(Ordering::Acquire) {
            self.bootstrap().await?;
            self.bootstrapped.store(true, Ordering::Release);
        }
        Ok(sync)
    }

    /// Waits until this store has bootstrapped (labels and identity, which saving a thread
    /// needs), without waiting for a sync that's running.
    pub(crate) async fn bootstrapped(&self) -> Result<()> {
        if !self.bootstrapped.load(Ordering::Acquire) {
            drop(self.lock_sync().await?);
        }
        Ok(())
    }

    /// Snapshots the history cursor (once per database), then refreshes labels and identity.
    async fn bootstrap(&self) -> Result<()> {
        let mut db = self.db.clone();
        if db::kv_get::<u64>(&mut db, HISTORY_ID).await?.is_none() {
            let profile = self
                .api
                .call(Lane::Show, ATTEMPTS, |gmail| gmail.profile())
                .await?;
            let history_id = profile.history_id.unwrap_or_default();
            let mut write = self.begin(&mut db).await?;
            db::kv_set(write.exec(), HISTORY_ID, &history_id).await?;
            // A new store: threads another store snoozed may be waiting with no reminder here.
            db::kv_set(write.exec(), db::ORPHANED_SNOOZES, &true).await?;
            write.commit().await?;
        }
        let hidden = self.refresh_labels().await?;
        // The splits' label names resolve against the labels just listed.
        self.reload_splits(false).await?;
        self.refresh_identity(hidden).await
    }

    /// Re-lists labels, creating Exo's hidden ones first if they're missing. Returns their ids.
    async fn refresh_labels(&self) -> Result<HiddenLabels> {
        let named = |labels: &[Label], name: &str| {
            labels
                .iter()
                .any(|label| label.name.as_deref() == Some(name))
        };
        let mut labels = self
            .api
            .call(Lane::Show, ATTEMPTS, |gmail| gmail.labels())
            .await?
            .labels;
        let missing: Vec<&str> = [EXO_PARENT, EXO_MUTED, EXO_SNOOZED, EXO_REMINDER_UNREAD]
            .into_iter()
            .filter(|name| !named(&labels, name))
            .collect();
        for name in &missing {
            let hidden = Label {
                name: Some((*name).to_owned()),
                label_list_visibility: Some(LabelListVisibility::LabelHide),
                message_list_visibility: Some(MessageListVisibility::Hide),
                ..Label::default()
            };
            match self
                .api
                .call(Lane::Show, ATTEMPTS, |gmail| gmail.create_label(&hidden))
                .await
            {
                // Another client created it meanwhile.
                Err(Failure::Permanent(error)) if error.status() == Some(409) => {}
                result => {
                    result?;
                }
            }
        }
        if !missing.is_empty() {
            labels = self
                .api
                .call(Lane::Show, ATTEMPTS, |gmail| gmail.labels())
                .await?
                .labels;
        }
        let id = |name: &str| {
            labels
                .iter()
                .find(|label| label.name.as_deref() == Some(name))
                .and_then(|label| label.id.clone())
        };
        let hidden = HiddenLabels {
            muted: id(EXO_MUTED),
            snoozed: id(EXO_SNOOZED),
            reminder_unread: id(EXO_REMINDER_UNREAD),
        };
        let mut db = self.db.clone();
        let mut write = self.begin(&mut db).await?;
        LabelRow::all().delete().exec(write.exec()).await?;
        let mut rows = LabelRow::create_many();
        for label in &labels {
            let Some(id) = &label.id else { continue };
            let color = label.color.clone().unwrap_or_default();
            rows = rows.with_item(|row| {
                row.id(id)
                    .name(label.name.clone().unwrap_or_else(|| id.clone()))
                    .system(label.kind == Some(gmail::LabelType::System))
                    .hidden(label.label_list_visibility == Some(LabelListVisibility::LabelHide))
                    .text_color(color.text_color)
                    .background_color(color.background_color)
            });
        }
        rows.exec(write.exec()).await?;
        write.publish().await?;
        Ok(hidden)
    }

    /// Learns which addresses are "me" (the primary and aliases Gmail treats as the account)
    /// and Exo's hidden labels. A change re-routes every stored thread.
    async fn refresh_identity(&self, hidden: HiddenLabels) -> Result<()> {
        let aliases = self
            .api
            .call(Lane::Show, ATTEMPTS, |gmail| gmail.send_as_list())
            .await?
            .send_as;
        let mut addresses = Vec::new();
        let mut name = None;
        for alias in &aliases {
            let primary = alias.is_primary == Some(true);
            if primary {
                name = alias.display_name.clone().filter(|name| !name.is_empty());
            }
            if primary || alias.treat_as_alias == Some(true) {
                addresses.extend(
                    alias
                        .send_as_email
                        .as_ref()
                        .map(|email| email.to_lowercase()),
                );
            }
        }
        self.save_aliases(&aliases).await?;
        let identity = Identity {
            addresses,
            name,
            muted_label: hidden.muted,
            snoozed_label: hidden.snoozed,
            reminder_unread_label: hidden.reminder_unread,
        };
        if identity == self.me() {
            return Ok(());
        }
        let mut db = self.db.clone();
        let threads: Vec<String> = ThreadRow::all()
            .select(ThreadRow::fields().id())
            .exec(&mut db)
            .await?;
        let mut write = self.begin(&mut db).await?;
        db::kv_set(write.exec(), IDENTITY, &identity).await?;
        let router = Router {
            me: identity.clone(),
            ..self.router()
        };
        for thread in &threads {
            reindex(write.exec(), thread, &router).await?;
        }
        // Only once it's stored: a failure above leaves the old identity, so the next
        // bootstrap tries again.
        write.commit().await?;
        *self
            .identity
            .write()
            .expect("identity lock is never poisoned") = identity;
        self.notify();
        Ok(())
    }

    /// Keeps the account's send-as aliases and their signatures for compose.
    async fn save_aliases(&self, aliases: &[gmail::SendAs]) -> Result<()> {
        let aliases: Vec<Alias> = aliases
            .iter()
            .filter_map(|alias| {
                let email = alias.send_as_email.clone()?;
                let default = alias.is_default == Some(true);
                let primary = alias.is_primary == Some(true);
                Some(Alias {
                    name: alias.display_name.clone().filter(|name| !name.is_empty()),
                    signature: alias
                        .signature
                        .clone()
                        .filter(|signature| !signature.is_empty()),
                    active: default
                        || primary
                        || alias.verification_status == Some(gmail::VerificationStatus::Accepted),
                    email,
                    default,
                    primary,
                })
            })
            .collect();
        let mut db = self.db.clone();
        if db::kv_get::<Vec<Alias>>(&mut db, ALIASES).await?.as_ref() == Some(&aliases) {
            return Ok(());
        }
        let mut write = self.begin(&mut db).await?;
        db::kv_set(write.exec(), ALIASES, &aliases).await?;
        write.publish().await
    }

    /// Soft bankruptcy: forget everything fetched from Gmail, keep the outbox. The next
    /// bootstrap snapshots a fresh cursor and backfill starts over.
    async fn bankrupt(&self) -> Result<()> {
        log::warn!("Gmail history expired; rebuilding the local cache (queued mutations are kept)");
        let mut db = self.db.clone();
        let mut write = self.begin(&mut db).await?;
        ThreadRow::all().delete().exec(write.exec()).await?;
        MessageRow::all().delete().exec(write.exec()).await?;
        AttachmentRow::all().delete().exec(write.exec()).await?;
        BodyRow::all().delete().exec(write.exec()).await?;
        ListEntry::all().delete().exec(write.exec()).await?;
        ParticipantEntry::all().delete().exec(write.exec()).await?;
        ListCursor::all().delete().exec(write.exec()).await?;
        LabelRow::all().delete().exec(write.exec()).await?;
        KvRow::filter_by_key(HISTORY_ID)
            .delete()
            .exec(write.exec())
            .await?;
        write.publish().await
    }
}

/// `threads.get` in full for `lane`, with large HTML bodies inlined. `None` when the thread
/// is gone.
pub(crate) async fn fetch_thread(
    api: &Api,
    lane: Lane,
    attempts: u32,
    id: &str,
) -> Result<Option<gmail::Thread>, Failure> {
    let fetched = api
        .call(lane, attempts, |gmail| gmail.thread(id, Format::Full, &[]))
        .await;
    let Some(mut thread) = found(fetched)? else {
        return Ok(None);
    };
    for message in &mut thread.messages {
        inline_bodies(api, lane, attempts, message).await?;
    }
    Ok(Some(thread))
}

/// A thread fetched for its rows: `threads.get` with `format=metadata` (a third of `full`'s
/// quota units), and in full each new message whose row needs its structure.
pub(crate) struct Rows {
    pub thread: gmail::Thread,
    /// The ids of the messages in `thread` fetched in full.
    pub full: HashSet<String>,
}

/// Fetches thread `id` for its rows, in `lane`: its messages' labels, snippets and
/// [`ROW_HEADERS`], plus, in full, those that may hold files or an invite (the row's icon and
/// `has:attachment`/`filename:` need their parts) unless they're among `stored`. `None` when
/// the thread is gone.
pub(crate) async fn fetch_rows(
    api: &Api,
    lane: Lane,
    attempts: u32,
    id: &str,
    stored: &HashSet<String>,
) -> Result<Option<Rows>, Failure> {
    let fetched = api
        .call(lane, attempts, |gmail| {
            gmail.thread(id, Format::Metadata, &ROW_HEADERS)
        })
        .await;
    let Some(mut thread) = found(fetched)? else {
        return Ok(None);
    };
    let wanted: Vec<String> = thread
        .messages
        .iter()
        .filter(|message| needs_structure(message))
        .filter_map(|message| message.id.clone())
        .filter(|id| !stored.contains(id))
        .collect();
    let fetched = future::try_join_all(
        wanted
            .iter()
            .map(|id| fetch_message(api, lane, attempts, id)),
    )
    .await?;
    let mut full = HashSet::new();
    // One gone since the thread was read keeps its row until history says so.
    for message in fetched.into_iter().flatten() {
        let Some(id) = message.id.clone() else {
            continue;
        };
        if let Some(slot) = thread
            .messages
            .iter_mut()
            .find(|slot| slot.id.as_ref() == Some(&id))
        {
            *slot = message;
            full.insert(id);
        }
    }
    Ok(Some(Rows { thread, full }))
}

/// A message fetched for its row may hold a file or an invite: its top-level type isn't one
/// of [`BODY_ONLY`], or it's an Exchange meeting message, whose invite or RSVP is a
/// `text/calendar` alternative inside `multipart/alternative` (Outlook's `Content-Class:
/// urn:content-classes:calendarmessage`). Chats are never stored.
fn needs_structure(message: &Message) -> bool {
    let Some(payload) = &message.payload else {
        return true;
    };
    let content_type = || {
        payload
            .header("Content-Type")
            .and_then(|value| value.split(';').next())
            .map(str::trim)
    };
    let mime = payload.mime_type.as_deref().or_else(content_type);
    let meeting = payload
        .header("Content-Class")
        .is_some_and(|class| class.trim().eq_ignore_ascii_case(CALENDAR_MESSAGE));
    !message.label_ids.iter().any(|label| label == CHAT)
        && (meeting
            || !mime.is_some_and(|mime| {
                BODY_ONLY
                    .iter()
                    .any(|plain| mime.eq_ignore_ascii_case(plain))
            }))
}

/// The `Content-Class` Exchange gives its meeting requests, updates, cancellations and replies.
const CALENDAR_MESSAGE: &str = "urn:content-classes:calendarmessage";

/// `messages.get` for message `id`'s row alone (`format=metadata` with [`ROW_HEADERS`], a third
/// of `full`'s units). `None` when the message is gone.
pub(crate) async fn fetch_row(
    api: &Api,
    lane: Lane,
    attempts: u32,
    id: &str,
) -> Result<Option<Message>, Failure> {
    found(
        api.call(lane, attempts, |gmail| {
            gmail.message(id, Format::Metadata, &ROW_HEADERS)
        })
        .await,
    )
}

/// `messages.get` in full for `lane`, with large HTML bodies inlined. `None` when the message
/// is gone.
pub(crate) async fn fetch_message(
    api: &Api,
    lane: Lane,
    attempts: u32,
    id: &str,
) -> Result<Option<Message>, Failure> {
    let fetched = api
        .call(lane, attempts, |gmail| gmail.message(id, Format::Full, &[]))
        .await;
    let Some(mut message) = found(fetched)? else {
        return Ok(None);
    };
    inline_bodies(api, lane, attempts, &mut message).await?;
    Ok(Some(message))
}

/// `None` when Gmail says what was asked for doesn't exist.
fn found<T>(fetched: Result<T, Failure>) -> Result<Option<T>, Failure> {
    match fetched {
        Ok(value) => Ok(Some(value)),
        Err(failure) if failure.is_not_found() => Ok(None),
        Err(failure) => Err(failure),
    }
}

/// Gmail moves big `text/html` bodies into attachments; fetch them back into the part.
async fn inline_bodies(
    api: &Api,
    lane: Lane,
    attempts: u32,
    message: &mut Message,
) -> Result<(), Failure> {
    let (Some(message_id), Some(payload)) = (message.id.clone(), message.payload.as_mut()) else {
        return Ok(());
    };
    let wanted: Vec<String> = payload
        .walk()
        .filter(|part| part.mime_type.as_deref() == Some("text/html"))
        .filter_map(|part| {
            let body = part.body.as_ref()?;
            let small = body.size.unwrap_or_default() <= MAX_INLINED_BODY;
            (body.data.is_none() && small).then(|| body.attachment_id.clone())?
        })
        .collect();
    let mut fetched = HashMap::new();
    for attachment in wanted {
        let body = api
            .call(lane, attempts, |gmail| {
                gmail.attachment(&message_id, &attachment)
            })
            .await?;
        fetched.insert(attachment, body.data);
    }
    fill(payload, &mut fetched);
    Ok(())
}

fn fill(part: &mut MessagePart, fetched: &mut HashMap<String, Option<Vec<u8>>>) {
    if let Some(body) = part.body.as_mut()
        && let Some(data) = body
            .attachment_id
            .as_ref()
            .and_then(|id| fetched.remove(id))
    {
        body.data = data;
    }
    for child in &mut part.parts {
        fill(child, fetched);
    }
}

#[cfg(test)]
mod tests {
    use gmail::MessagePartHeader;

    use super::*;

    #[test]
    fn rows_need_the_parts_of_what_may_hold_files_or_invites() {
        let message = |mime: Option<&str>, content_type: Option<&str>| Message {
            payload: Some(MessagePart {
                mime_type: mime.map(str::to_owned),
                headers: content_type
                    .map(|value| MessagePartHeader {
                        name: "Content-Type".into(),
                        value: value.into(),
                    })
                    .into_iter()
                    .collect(),
                ..MessagePart::default()
            }),
            ..Message::default()
        };
        for body_only in ["multipart/alternative", "text/html", "TEXT/PLAIN"] {
            assert!(
                !needs_structure(&message(Some(body_only), None)),
                "{body_only}"
            );
        }
        for structured in [
            "multipart/mixed",
            "multipart/related",
            "multipart/signed",
            "text/calendar",
            "application/pdf",
        ] {
            assert!(
                needs_structure(&message(Some(structured), None)),
                "{structured}"
            );
        }
        assert!(
            !needs_structure(&message(
                None,
                Some("multipart/alternative; boundary=\"b\"")
            )),
            "the header stands in for a missing type"
        );
        assert!(
            needs_structure(&message(None, None)),
            "an unknown type is fetched in full"
        );
        let mut outlook = message(Some("multipart/alternative"), None);
        if let Some(payload) = outlook.payload.as_mut() {
            payload.headers.push(MessagePartHeader {
                name: "Content-class".into(),
                value: "urn:content-classes:calendarmessage".into(),
            });
        }
        assert!(
            needs_structure(&outlook),
            "an Exchange invite's calendar part is an alternative"
        );
    }
}
