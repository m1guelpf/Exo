//! Drafts: Exo's record of each draft, kept locally and mirrored to a Gmail draft, and the held
//! sends behind undo send.
//!
//! Compose saves with [`Store::save_draft`], a local write the UI sees at once.
//! [`Store::push_drafts`] then brings Gmail up to date: it creates or updates the Gmail draft,
//! deletes discarded ones, and sends (`drafts.send`) those whose hold is over. A draft being
//! typed in is written at most every 5 s, 30 s with files, which coalesces its saves: its latest
//! save goes once the pace allows. A save Gmail already has, its content unchanged, isn't written
//! at all. Like the outbox, a push that fails for a reason retrying can fix backs off and tries
//! again; held sends survive a restart and go out once due. A send Gmail won't take is a failed
//! draft until it's edited, sent again or discarded.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::sync::Arc;

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

use crate::api::Failure;
use crate::db::{self, DraftRow, GIVEN_REMINDERS, MessageRow, ThreadRow};
use crate::error::Result;
use crate::ids::ThreadId;
use crate::ids::labels::{DRAFT, SENT};
use crate::parse::parse;
use crate::provider;
use crate::quota::Lane;
use crate::store::{Inner, Store, reindex};
use crate::sync::{fetch_message, fetch_row, fetch_rows};

/// What a draft is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DraftAction {
    /// A new message.
    Compose,
    /// A reply to the sender.
    Reply,
    /// A reply to everyone.
    ReplyAll,
    /// A forward.
    Forward,
    /// A reply whose subject was changed, which starts a new thread.
    EditSubjectLineReply,
}

/// A name and address on a draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    /// Display name, if any.
    pub name: Option<String>,
    /// The address as typed; may be invalid (a red chip).
    pub email: String,
}

/// A draft as compose keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftRecord {
    /// What it's for.
    pub action: DraftAction,
    /// The send-as alias it's from.
    pub from: Option<Address>,
    /// `To`.
    pub to: Vec<Address>,
    /// `Cc`.
    pub cc: Vec<Address>,
    /// `Bcc`.
    pub bcc: Vec<Address>,
    /// The subject.
    pub subject: String,
    /// The editor's HTML: the body only, without signature or quote until they're revealed.
    pub body: String,
    /// The quoted message of a reply or forward, outside the body until revealed.
    pub quoted_content: Option<String>,
    /// The quote and signature are in the body (revealed with "…", or deleted).
    pub quoted_inlined: bool,
    /// The Gmail message replied to or forwarded.
    pub in_reply_to: Option<String>,
    /// That message's `Message-ID`, without angle brackets.
    pub in_reply_to_rfc822: Option<String>,
    /// `References`, oldest first, without angle brackets.
    pub references: Vec<String>,
    /// The draft's own `Message-ID`, without angle brackets, kept across saves.
    pub rfc822_id: String,
    /// To, Cc and the files as created (`fingerprint`): a reply whose recipients and files are
    /// untouched is unmodified.
    pub fingerprint: String,
    /// Its files, in the order they were added.
    pub attachments: Vec<DraftAttachment>,
    /// The time Send later picked: Send schedules the message for then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_later: Option<SendLater>,
    /// Remind me's choice for a draft: its thread gets the reminder at Send (or Schedule), or
    /// once it's stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reminder: Option<DraftReminder>,
}

/// A draft's reminder: when the thread comes back, and whether a reply cancels it. Nothing is set
/// until the message is sent or scheduled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftReminder {
    /// When.
    pub at: Timestamp,
    /// `regardless`: a reply doesn't cancel it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep_on_reply: bool,
    /// Automatic Reminders put it there: it comes off again once no recipient is external.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub automatic: bool,
}

/// Send later's choice for a draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendLater {
    /// When to send.
    pub at: Timestamp,
    /// `if no reply`: a reply before then calls the send off.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub abort_on_reply: bool,
    /// The thread's messages when the send was scheduled, which [`Store::schedule_draft`]
    /// records: any other message that isn't the user's or a draft is the reply that calls an
    /// `abort_on_reply` send off.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub current_message_ids: Vec<String>,
}

/// A file on a draft. Its bytes are kept under its `uuid` in [`Store::draft_files`]; a Gmail part
/// is fetched there the first time it's needed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftAttachment {
    /// Exo's id for it (a v4 UUID).
    pub uuid: String,
    /// The file name.
    pub name: String,
    /// Its MIME type.
    pub mime_type: String,
    /// Bytes.
    pub size: u64,
    /// The `Content-ID` an inline image is shown by; `None` for a listed file.
    pub content_id: Option<String>,
    /// Where its bytes come from.
    pub source: AttachmentSource,
}

/// Where an attachment's bytes come from (`source`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttachmentSource {
    /// A file the user added, copied into the draft files.
    Local,
    /// A part of a Gmail message (a forward's files, a quoted message's inline images).
    Gmail {
        /// The message.
        message_id: String,
        /// Gmail's attachment id; `None` when the part's data comes with the message.
        attachment_id: Option<String>,
        /// The MIME part.
        part_id: String,
    },
}

/// How a draft's files stand with Gmail, for compose's cards: which of them Gmail's copy of the
/// draft has, and how far the write carrying the others has come. A file goes up with the first
/// write of the draft that has it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Upload {
    /// The ids of the files Gmail's copy of the draft has.
    pub gmail_has: HashSet<String>,
    /// While a write is under way: the ids of the files it carries that Gmail lacks, and the
    /// share of its message sent so far, 0–100.
    pub sending: Option<(HashSet<String>, f32)>,
}

impl Upload {
    /// How far `attachment` has gone up, 0–100, while Gmail's copy of the draft lacks it (0 with
    /// no write carrying it yet); `None` once Gmail has it, or for a Gmail part, which never goes
    /// up.
    #[must_use]
    pub fn progress(&self, attachment: &DraftAttachment) -> Option<f32> {
        if attachment.source != AttachmentSource::Local || self.gmail_has.contains(&attachment.uuid)
        {
            return None;
        }
        Some(
            self.sending
                .as_ref()
                .filter(|(files, _)| files.contains(&attachment.uuid))
                .map_or(0.0, |(_, percent)| *percent),
        )
    }
}

/// Every draft's [`Upload`] the store knows of this session, by draft id.
pub type Uploads = HashMap<String, Upload>;

/// The ids of `record`'s files that go up to Gmail ([`AttachmentSource::Local`]).
fn local_files(record: &DraftRecord) -> HashSet<String> {
    record
        .attachments
        .iter()
        .filter(|attachment| attachment.source == AttachmentSource::Local)
        .map(|attachment| attachment.uuid.clone())
        .collect()
}

/// A draft to save.
#[derive(Debug, Clone)]
pub struct Draft {
    /// Exo's draft id.
    pub id: String,
    /// The Gmail thread it belongs to, when known (a reply's; a new message gets one on its
    /// first save).
    pub thread: Option<ThreadId>,
    /// On its first save, the Gmail message of the draft written elsewhere (Gmail web) it takes
    /// over: that Gmail draft is the one written, found by its message when it's pushed.
    pub adopted: Option<String>,
    /// What compose shows.
    pub record: DraftRecord,
    /// The RFC 822 message the Gmail draft holds: exactly what sending sends.
    pub raw: String,
    /// `raw` has every file. A draft missing one isn't written to Gmail, whose copy would lose
    /// it, until a save that has them all.
    pub complete: bool,
    /// Saved to be sent: written to Gmail even when Gmail has its content already, since that
    /// write dates the message (`Date:`) it sends.
    pub sending: bool,
}

/// `not_before` of a draft saved without some of its files: no push until a save that has them.
const UNTIL_COMPLETE: Timestamp = Timestamp::MAX;

/// A saved draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDraft {
    /// Exo's draft id.
    pub id: String,
    /// Its Gmail thread, once it has one.
    pub thread: Option<ThreadId>,
    /// Its Gmail draft, once created.
    pub gmail_id: Option<String>,
    /// The Gmail message of the latest version Gmail has.
    pub message_id: Option<String>,
    /// What compose shows.
    pub record: DraftRecord,
    /// The message it sends.
    pub raw: String,
    /// Held for sending until then.
    pub send_at: Option<Timestamp>,
    /// Gmail has the latest version.
    pub synced: bool,
    /// Its last save had every file ([`Draft::complete`]); one that didn't waits for a save that
    /// has them before it's written to Gmail.
    pub complete: bool,
    /// Its send failed (a failed draft): it shows in its thread as a message until it's edited,
    /// sent again or discarded.
    pub failure: Option<SendFailure>,
}

/// A draft Gmail sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentDraft {
    /// Exo's draft id.
    pub id: String,
    /// The thread the message went into.
    pub thread: ThreadId,
    /// The sent message.
    pub message: String,
}

/// Why a send failed, as far as Gmail's answers tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendFailure {
    /// Google signed the account out (`invalid_auth`).
    LoggedOut,
    /// The account's sending limit (`quota_exceeded`).
    QuotaExceeded,
    /// Over Gmail's size limit (`message_too_large`).
    TooLarge,
    /// A recipient Gmail refuses (`invalid_recipients`).
    InvalidRecipients,
    /// Gmail is rate limiting the account for longer than a send waits (`provider-rate-limit`).
    RateLimited,
    /// Gmail kept failing for a while (`provider-transient-error`).
    Temporary,
    /// Anything else (no reason).
    Other,
}

impl SendFailure {
    /// The failure's code, as it's stored.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::LoggedOut => "invalid_auth",
            Self::QuotaExceeded => "quota_exceeded",
            Self::TooLarge => "message_too_large",
            Self::InvalidRecipients => "invalid_recipients",
            Self::RateLimited => "provider-rate-limit",
            Self::Temporary => "provider-transient-error",
            Self::Other => "",
        }
    }

    fn from_code(code: &str) -> Self {
        [
            Self::LoggedOut,
            Self::QuotaExceeded,
            Self::TooLarge,
            Self::InvalidRecipients,
            Self::RateLimited,
            Self::Temporary,
        ]
        .into_iter()
        .find(|reason| reason.code() == code)
        .unwrap_or(Self::Other)
    }
}

/// Attempts at `drafts.send` before Gmail's server errors fail the send (with the call's backoff,
/// about 7 s of trying).
const SEND_ATTEMPTS: u32 = 4;
/// A rate limit a due send waits out; Gmail asking for longer fails it.
const SEND_WAITS: SignedDuration = SignedDuration::from_mins(2);
/// A rate limit this long is Gmail's daily sending limit ("multiple hours", Gmail's usage limits).
const SENDING_LIMIT: SignedDuration = SignedDuration::from_hours(1);

/// A draft Gmail didn't take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftFailure {
    /// Saving it was refused (`Sorry! Failed to save draft.`); the next save tries again.
    Save {
        /// Exo's draft id.
        id: String,
        /// Gmail's error.
        error: String,
    },
    /// Its send failed: it's a failed draft, no longer held ([`StoredDraft::failure`]).
    Send {
        /// Exo's draft id.
        id: String,
        /// Why.
        reason: SendFailure,
    },
}

/// What a [`Store::push_drafts`] did.
#[derive(Debug, Default)]
pub struct DraftReport {
    /// Drafts whose latest version Gmail now has.
    pub saved: Vec<String>,
    /// Drafts Gmail sent.
    pub sent: Vec<SentDraft>,
    /// Drafts Gmail refused.
    pub failed: Vec<DraftFailure>,
    /// Some draft is waiting for the network.
    pub offline: bool,
    /// When to push again: a backoff ending, or a held send coming due.
    pub retry_at: Option<Timestamp>,
    /// A held send's reminder went on its thread: its label is queued, to flush.
    pub reminded: bool,
}

impl From<DraftRow> for StoredDraft {
    fn from(row: DraftRow) -> Self {
        Self {
            synced: row.pushed >= row.revision,
            complete: complete(&row),
            id: row.id,
            thread: row.thread_id.map(ThreadId::new),
            gmail_id: row.gmail_id,
            message_id: row.message_id,
            record: row.record.0,
            raw: row.raw,
            send_at: row.send_at,
            failure: row.failure.as_deref().map(SendFailure::from_code),
        }
    }
}

impl Store {
    /// The folder draft attachments' bytes are kept in, each named by its `uuid`.
    pub fn draft_files(&self) -> &Path {
        &self.inner.files
    }

    /// How each draft's files stand with Gmail, as it changes: which of them Gmail's copy has
    /// (all of a draft Gmail has the latest version of, as of the first time the store saw it
    /// this session, then those of each write), and how far a write carrying the others has come.
    /// A draft the store doesn't know of yet has none up.
    #[must_use]
    pub fn uploads(&self) -> tokio::sync::watch::Receiver<Uploads> {
        self.inner.uploads.subscribe()
    }

    /// Deletes kept attachment bytes no draft or snippet refers to any more.
    ///
    /// # Errors
    /// On database failures; files that can't be listed or removed are skipped.
    pub async fn prune_draft_files(&self) -> Result<()> {
        let mut db = self.inner.db.clone();
        let mut kept: HashSet<String> = DraftRow::all()
            .exec(&mut db)
            .await?
            .into_iter()
            .flat_map(|row| {
                row.record
                    .0
                    .attachments
                    .into_iter()
                    .map(|attachment| attachment.uuid)
            })
            .collect();
        let settings: crate::Settings = self.settings().await?;
        kept.extend(
            settings
                .snippets
                .into_iter()
                .flat_map(|snippet| snippet.attachments)
                .map(|attachment| attachment.uuid),
        );
        let Ok(entries) = std::fs::read_dir(&self.inner.files) else {
            return Ok(());
        };
        for entry in entries.flatten() {
            if !entry
                .file_name()
                .to_str()
                .is_some_and(|name| kept.contains(name))
                && let Err(error) = std::fs::remove_file(entry.path())
            {
                log::warn!("removing a draft's file: {error}");
            }
        }
        Ok(())
    }

    /// Saves `draft` locally, to be written to Gmail by the next
    /// [`push_drafts`](Self::push_drafts) unless Gmail has its content already. Saving a
    /// discarded draft brings it back (undo discard), saving a held one keeps its hold, and a
    /// failed one is a draft again.
    ///
    /// # Errors
    /// On database failures.
    pub async fn save_draft(&self, draft: Draft) -> Result<()> {
        let Draft {
            id,
            thread,
            adopted,
            record,
            raw,
            complete,
            sending,
        } = draft;
        let not_before = (!complete).then_some(UNTIL_COMPLETE);
        let mut db = self.inner.db.clone();
        let mut write = self.inner.begin(&mut db).await?;
        let thread = thread.map(|thread| thread.as_str().to_owned());
        let files = local_files(&record);
        let thread = match DraftRow::filter_by_id(&id)
            .first()
            .exec(write.exec())
            .await?
        {
            Some(row) => {
                // Gmail has this content already: the save is Gmail's too, with nothing to
                // write. A write of an older revision under way marks only that one as Gmail's
                // when it ends, so this one is written after it.
                let unchanged =
                    complete && !sending && self.inner.gmail_has(&row) == Some(content(&raw));
                self.inner.note_draft(&row);
                if unchanged {
                    self.inner.set_gmail_files(&id, files);
                }
                let revision = row.revision + 1;
                let thread = thread.or(row.thread_id);
                DraftRow::update_by_id(&id)
                    .thread_id(thread.clone())
                    .record(record)
                    .raw(raw)
                    .revision(revision)
                    .pushed(if unchanged { revision } else { row.pushed })
                    .discarded(false)
                    .failures(0)
                    .not_before(not_before)
                    .failure(None)
                    .failed_at(None)
                    .exec(write.exec())
                    .await?;
                thread
            }
            None => {
                // Gmail has none of a new draft's files: one Gmail refuses keeps them waiting.
                self.inner.uploads.send_modify(|uploads| {
                    uploads.insert(id.clone(), Upload::default());
                });
                // A draft taken over holds the message it took over until its first push, which
                // keeps compose from taking that message over a second time.
                DraftRow::create()
                    .id(&id)
                    .thread_id(thread.clone())
                    .gmail_id(None)
                    .message_id(adopted)
                    .record(record)
                    .raw(raw)
                    .revision(1)
                    .pushed(0)
                    .discarded(false)
                    .send_at(None)
                    .failures(0)
                    .not_before(not_before)
                    .failure(None)
                    .failed_at(None)
                    .exec(write.exec())
                    .await?;
                thread
            }
        };
        // The thread's search index has the draft's text, from its record; a failed draft being
        // a draft again also changes the thread's lists and row.
        if let Some(thread) = &thread {
            reindex(write.exec(), thread, &self.inner.router()).await?;
        }
        write.publish().await
    }

    /// The draft `id`, unless discarded.
    ///
    /// # Errors
    /// On database failures.
    pub async fn draft(&self, id: &str) -> Result<Option<StoredDraft>> {
        let mut db = self.inner.db.clone();
        let row = DraftRow::filter_by_id(id).first().exec(&mut db).await?;
        Ok(row
            .filter(|row| !row.discarded)
            .inspect(|row| self.inner.note_draft(row))
            .map(StoredDraft::from))
    }

    /// Every draft compose has, discarded ones aside.
    ///
    /// # Errors
    /// On database failures.
    pub async fn drafts(&self) -> Result<Vec<StoredDraft>> {
        let mut db = self.inner.db.clone();
        let rows = DraftRow::all().exec(&mut db).await?;
        Ok(rows
            .into_iter()
            .filter(|row| !row.discarded)
            .inspect(|row| self.inner.note_draft(row))
            .map(StoredDraft::from)
            .collect())
    }

    /// Discards draft `id`: it's gone from reads at once, and its Gmail draft is deleted on the
    /// next push. [`save_draft`](Self::save_draft) brings it back.
    ///
    /// # Errors
    /// On database failures.
    pub async fn discard_draft(&self, id: &str) -> Result<()> {
        let mut db = self.inner.db.clone();
        let mut write = self.inner.begin(&mut db).await?;
        let Some(row) = DraftRow::filter_by_id(id)
            .first()
            .exec(write.exec())
            .await?
        else {
            return write.commit().await;
        };
        DraftRow::update_by_id(id)
            .discarded(true)
            .send_at(None)
            .failures(0)
            .not_before(None)
            .failure(None)
            .failed_at(None)
            .exec(write.exec())
            .await?;
        if let Some(thread) = &row.thread_id {
            reindex(write.exec(), thread, &self.inner.router()).await?;
        }
        write.publish().await
    }

    /// Holds draft `id` for sending at `at` (a later push sends it then); a held draft's hold
    /// moves to `at`, and a failed one is sent again. One due now is tried on the next push, even
    /// while its saves back off (Instant Send, quitting, Google signing the account out).
    ///
    /// # Errors
    /// On database failures.
    pub async fn send_draft(&self, id: &str, at: Timestamp) -> Result<()> {
        let mut db = self.inner.db.clone();
        let mut write = self.inner.begin(&mut db).await?;
        if let Some(row) = DraftRow::filter_by_id(id)
            .first()
            .exec(write.exec())
            .await?
        {
            let not_before = if at <= Timestamp::now() {
                row.not_before.filter(|at| *at == UNTIL_COMPLETE)
            } else {
                row.not_before
            };
            DraftRow::update_by_id(id)
                .send_at(Some(at))
                .not_before(not_before)
                .failure(None)
                .failed_at(None)
                .exec(write.exec())
                .await?;
            if let Some(thread) = &row.thread_id {
                reindex(write.exec(), thread, &self.inner.router()).await?;
            }
        }
        write.publish().await?;
        self.inner.give_reminder(id).await?;
        Ok(())
    }

    /// Schedules draft `id` for its Send later time: it's held until then, a Gmail draft
    /// meanwhile, and sent by the push once due, at the next launch if Exo isn't running then.
    /// The thread's messages are recorded, so a reply that calls off an `if no reply` send can be
    /// told apart. A draft without a time isn't touched.
    ///
    /// # Errors
    /// On database failures.
    pub async fn schedule_draft(&self, id: &str) -> Result<()> {
        let mut db = self.inner.db.clone();
        let mut write = self.inner.begin(&mut db).await?;
        let Some(row) = DraftRow::filter_by_id(id)
            .first()
            .exec(write.exec())
            .await?
        else {
            return write.commit().await;
        };
        let mut record = row.record.0;
        let Some(later) = record.send_later.as_mut() else {
            return write.commit().await;
        };
        later.current_message_ids = match &row.thread_id {
            Some(thread) => {
                MessageRow::filter_by_thread_id(thread)
                    .select(MessageRow::fields().id())
                    .exec(write.exec())
                    .await?
            }
            None => Vec::new(),
        };
        let at = later.at;
        let not_before = if at <= Timestamp::now() {
            row.not_before.filter(|at| *at == UNTIL_COMPLETE)
        } else {
            row.not_before
        };
        DraftRow::update_by_id(id)
            .record(record)
            .send_at(Some(at))
            .not_before(not_before)
            .failure(None)
            .failed_at(None)
            .exec(write.exec())
            .await?;
        if let Some(thread) = &row.thread_id {
            reindex(write.exec(), thread, &self.inner.router()).await?;
        }
        write.publish().await?;
        self.inner.give_reminder(id).await?;
        Ok(())
    }

    /// Makes failed draft `id` a draft again, to edit (`Edit Message`). Its first edit is
    /// written at once, not at the pace of the write its send just made.
    ///
    /// # Errors
    /// On database failures.
    pub async fn recover_draft(&self, id: &str) -> Result<()> {
        self.inner
            .written
            .lock()
            .expect("never poisoned")
            .remove(id);
        let mut db = self.inner.db.clone();
        let mut write = self.inner.begin(&mut db).await?;
        if let Some(row) = DraftRow::filter_by_id(id)
            .first()
            .exec(write.exec())
            .await?
            .filter(|row| row.failure.is_some())
        {
            DraftRow::update_by_id(id)
                .failure(None)
                .failed_at(None)
                .exec(write.exec())
                .await?;
            if let Some(thread) = &row.thread_id {
                reindex(write.exec(), thread, &self.inner.router()).await?;
            }
        }
        write.publish().await
    }

    /// Gives held draft `id` `reminder` (or takes it off), and its thread with it: Remind me
    /// picked for a message just sent (the follow-up hint's `Set reminder`, and its Undo).
    /// Returns whether it was still unsent; `false` means it already went (or isn't there).
    ///
    /// # Errors
    /// On database failures.
    pub async fn remind_draft(&self, id: &str, reminder: Option<DraftReminder>) -> Result<bool> {
        // A send in flight finishes first, as for undo send.
        let sending = self.inner.sending.lock().await;
        let mut db = self.inner.db.clone();
        let mut write = self.inner.begin(&mut db).await?;
        let Some(row) = DraftRow::filter_by_id(id)
            .first()
            .exec(write.exec())
            .await?
            .filter(|row| row.send_at.is_some() && !row.discarded)
        else {
            write.commit().await?;
            return Ok(false);
        };
        let mut record = row.record.0;
        record.reminder = reminder;
        DraftRow::update_by_id(id)
            .record(record)
            .exec(write.exec())
            .await?;
        write.publish().await?;
        drop(sending);
        if reminder.is_some() {
            self.inner.give_reminder(id).await?;
        } else {
            self.inner.take_back_reminder(id).await?;
        }
        Ok(true)
    }

    /// Undo send: takes back a held send ([`cancel_send`](Self::cancel_send)) and the reminder
    /// it put on its thread, which gets back the one it had. Returns whether it was still
    /// unsent.
    ///
    /// # Errors
    /// On database failures.
    pub async fn undo_send(&self, id: &str) -> Result<bool> {
        let held = self.cancel_send(id).await?;
        if held {
            self.inner.take_back_reminder(id).await?;
        }
        Ok(held)
    }

    /// Takes back a held send, or one that failed meanwhile, leaving the reminder it put on its
    /// thread (`Edit Message` on a scheduled send; [`undo_send`](Self::undo_send) takes that too).
    /// Returns whether it was still unsent; `false` means it already went (or isn't there).
    ///
    /// # Errors
    /// On database failures.
    pub async fn cancel_send(&self, id: &str) -> Result<bool> {
        // A send in flight finishes first: then it either went (the row is gone) or didn't.
        let _sending = self.inner.sending.lock().await;
        let mut db = self.inner.db.clone();
        let mut write = self.inner.begin(&mut db).await?;
        let row = DraftRow::filter_by_id(id)
            .first()
            .exec(write.exec())
            .await?;
        let held = row
            .as_ref()
            .is_some_and(|row| (row.send_at.is_some() || row.failure.is_some()) && !row.discarded);
        if held {
            DraftRow::update_by_id(id)
                .send_at(None)
                .failure(None)
                .failed_at(None)
                .exec(write.exec())
                .await?;
            if let Some(thread) = row.and_then(|row| row.thread_id) {
                reindex(write.exec(), &thread, &self.inner.router()).await?;
            }
        }
        write.publish().await?;
        Ok(held)
    }

    /// Writes what Gmail doesn't have yet: discarded drafts are deleted, saved ones created or
    /// updated, and held ones whose time has come sent. A draft's edits wait out its pace
    /// ([`Options::draft_pace`](crate::Options::draft_pace)) after its last write; a send's
    /// don't. One push runs at a time.
    ///
    /// # Errors
    /// On database failures. Gmail failures are retried later or reported in the result.
    pub async fn push_drafts(&self) -> Result<DraftReport> {
        self.push_drafts_with(false).await
    }

    /// [`push_drafts`](Self::push_drafts) without waiting out paces or backoffs, for an account
    /// closing or left (quitting, Google signing it out, switching to another, signing out):
    /// each draft is tried once more, so Gmail has every edit, and a send due then goes or fails
    /// now rather than on the next launch.
    ///
    /// # Errors
    /// On database failures.
    pub async fn push_drafts_now(&self) -> Result<DraftReport> {
        self.push_drafts_with(true).await
    }

    async fn push_drafts_with(&self, closing: bool) -> Result<DraftReport> {
        let inner = &self.inner;
        let _flush = inner.flush.lock().await;
        let mut report = DraftReport::default();
        let mut db = inner.db.clone();
        let now = Timestamp::now();
        let rows = DraftRow::all().exec(&mut db).await?;
        inner
            .written
            .lock()
            .expect("never poisoned")
            .retain(|id, _| rows.iter().any(|row| row.id == *id));
        inner
            .acked
            .lock()
            .expect("never poisoned")
            .retain(|id, _| rows.iter().any(|row| row.id == *id));
        inner.uploads.send_if_modified(|uploads| {
            let before = uploads.len();
            uploads.retain(|id, _| rows.iter().any(|row| row.id == *id));
            uploads.len() != before
        });
        for row in rows {
            inner.note_draft(&row);
            if let Some(at) = row
                .not_before
                .filter(|at| *at > now && !(closing && *at != UNTIL_COMPLETE))
            {
                if at != UNTIL_COMPLETE {
                    report.retry_at = earliest(report.retry_at, at);
                }
                continue;
            }
            if let Some(at) = inner.paced(&row, now).filter(|_| !closing) {
                report.retry_at = earliest(report.retry_at, at);
                continue;
            }
            let id = row.id.clone();
            let failures = row.failures;
            if let Err(failure) = inner.push(row, now, &mut report).await? {
                report.offline |= matches!(failure, Failure::Offline(_));
                let at = match failure {
                    Failure::Throttled(until) | Failure::SendingLimited(until) => until,
                    _ => inner.api.postpone(failures),
                };
                log::debug!("postponing draft {id} until {at}: {}", failure.message());
                let mut write = inner.begin(&mut db).await?;
                DraftRow::update_by_id(&id)
                    .failures(failures.saturating_add(1))
                    .not_before(Some(at))
                    .exec(write.exec())
                    .await?;
                write.commit().await?;
                report.retry_at = earliest(report.retry_at, at);
            }
        }
        // Held sends coming due.
        for row in DraftRow::all().exec(&mut db).await? {
            if let Some(at) = row.send_at.filter(|at| *at > now && !row.discarded) {
                report.retry_at = earliest(report.retry_at, at);
            }
        }
        Ok(report)
    }
}

/// `call`, reporting its upload to `progress` when there's one.
fn reporting<'a>(
    call: gmail::Call<'a, gmail::Draft>,
    progress: Option<&gmail::Progress>,
) -> gmail::Call<'a, gmail::Draft> {
    match progress {
        Some(progress) => call.on_progress(progress.clone()),
        None => call,
    }
}

fn earliest(current: Option<Timestamp>, at: Timestamp) -> Option<Timestamp> {
    Some(current.map_or(at, |current| current.min(at)))
}

/// What Gmail keeps of a draft's message `raw`, hashed: all of it but its `Date:`, which every
/// save stamps afresh. The rest follows from the draft (its Message-ID is kept across saves,
/// and MIME boundaries are derived from the content).
pub(crate) fn content(raw: &str) -> u64 {
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw, ""));
    let mut hasher = DefaultHasher::new();
    head.split("\r\n")
        .filter(|line| !line.starts_with("Date:"))
        .for_each(|line| line.hash(&mut hasher));
    body.hash(&mut hasher);
    hasher.finish()
}

impl Inner {
    /// The [`content`] of what Gmail holds for `row`, when Exo knows it: what Exo last wrote
    /// of it this session, while Gmail's draft still holds that message, else the row's own
    /// once Gmail has its latest revision.
    fn gmail_has(&self, row: &DraftRow) -> Option<u64> {
        let message = row.message_id.as_ref()?;
        let acked = self
            .acked
            .lock()
            .expect("never poisoned")
            .get(&row.id)
            .filter(|(acked, _)| acked == message)
            .map(|(_, content)| *content);
        acked.or_else(|| (row.pushed >= row.revision).then(|| content(&row.raw)))
    }

    /// Notes which of `row`'s files Gmail's copy has, the first time this session the store sees
    /// the draft: all of them once Gmail has its latest revision, else none it knows of. Later,
    /// only writes say.
    fn note_draft(&self, row: &DraftRow) {
        self.uploads.send_if_modified(|uploads| {
            if uploads.contains_key(&row.id) {
                return false;
            }
            let gmail_has = if row.pushed >= row.revision {
                local_files(&row.record.0)
            } else {
                HashSet::new()
            };
            uploads.insert(
                row.id.clone(),
                Upload {
                    gmail_has,
                    sending: None,
                },
            );
            true
        });
    }

    /// Gmail's copy of draft `id` now has `files`, and no write of it is under way.
    fn set_gmail_files(&self, id: &str, files: HashSet<String>) {
        self.uploads.send_if_modified(|uploads| {
            let upload = Upload {
                gmail_has: files,
                sending: None,
            };
            let changed = uploads.get(id) != Some(&upload);
            uploads.insert(id.to_owned(), upload);
            changed
        });
    }

    /// Whether `row` has a file Gmail's copy lacks, which goes up at once: a file's upload
    /// starts as it's added.
    fn uploads_files(&self, row: &DraftRow) -> bool {
        let uploads = self.uploads.borrow();
        let gmail_has = uploads.get(&row.id).map(|upload| &upload.gmail_has);
        local_files(&row.record.0)
            .iter()
            .any(|file| !gmail_has.is_some_and(|has| has.contains(file)))
    }

    /// Reports a write of `row` to its [`Upload`] as it goes: the share of the message sent, for
    /// the files it carries that Gmail lacks. `None` when it carries none.
    fn upload_progress(&self, row: &DraftRow) -> Option<gmail::Progress> {
        let id = row.id.clone();
        let mut carried = local_files(&row.record.0);
        let uploads = self.uploads.clone();
        let started = uploads.send_if_modified(|uploads| {
            let upload = uploads.entry(id.clone()).or_default();
            carried.retain(|file| !upload.gmail_has.contains(file));
            let starts = !carried.is_empty();
            if starts {
                upload.sending = Some((carried, 0.0));
            }
            starts
        });
        started.then(|| -> gmail::Progress {
            Arc::new(move |transfer: gmail::Transfer| {
                let percent = transfer
                    .total
                    .filter(|total| *total > 0)
                    .map_or(0.0, |total| {
                        (transfer.done as f64 / total as f64 * 100.0) as f32
                    });
                uploads.send_if_modified(|uploads| {
                    match uploads
                        .get_mut(&id)
                        .and_then(|upload| upload.sending.as_mut())
                    {
                        // A retried call starts again from 0; the bar doesn't go back.
                        Some((_, shown)) if percent > *shown => {
                            *shown = percent;
                            true
                        }
                        _ => false,
                    }
                });
            })
        })
    }

    /// A write of draft `id` ended without Gmail taking it: no upload is under way.
    fn upload_ended(&self, id: &str) {
        self.uploads.send_if_modified(|uploads| {
            uploads
                .get_mut(id)
                .and_then(|upload| upload.sending.take())
                .is_some()
        });
    }

    /// When `row`'s edits may be written to Gmail, if not now: a draft being typed in waits its
    /// pace after its last write this session ([`Options::draft_pace`](crate::Options::draft_pace),
    /// or `draft_files_pace` with files). A write that makes its Gmail draft, a send's, a discard
    /// and one carrying a file Gmail lacks go at once.
    fn paced(&self, row: &DraftRow, now: Timestamp) -> Option<Timestamp> {
        if row.pushed >= row.revision
            || row.discarded
            || row.send_at.is_some()
            || row.gmail_id.is_none()
            || self.uploads_files(row)
        {
            return None;
        }
        let pace = if row.record.0.attachments.is_empty() {
            self.draft_pace
        } else {
            self.draft_files_pace
        };
        let written = *self.written.lock().expect("never poisoned").get(&row.id)?;
        written.checked_add(pace).ok().filter(|at| *at > now)
    }

    /// Brings one draft's Gmail copy up to date. A failure worth retrying is returned for the
    /// caller to back off; database errors are the outer `Result`.
    async fn push(
        &self,
        row: DraftRow,
        now: Timestamp,
        report: &mut DraftReport,
    ) -> Result<Result<(), Failure>> {
        if row.discarded {
            return self.delete_gmail_draft(row).await;
        }
        let mut row = row;
        if row.send_at.is_some_and(|at| at <= now) && row.record.0.send_later.is_some() {
            // Rewritten in Gmail web since Exo last wrote it: it goes as Gmail has it, edits and
            // all, rather than Exo's copy written over them.
            let rewritten = if row.pushed >= row.revision {
                match self.rewritten_elsewhere(&row).await {
                    Ok(rewritten) => rewritten,
                    Err(failure) => return Ok(Err(failure)),
                }
            } else {
                false
            };
            if rewritten {
                log::info!(
                    "draft {} was rewritten outside Exo: sending Gmail's copy",
                    row.id
                );
            } else {
                self.redate(&mut row, now).await?;
            }
        }
        if row.pushed < row.revision {
            let (id, revision, send_at) = (row.id.clone(), row.revision, row.send_at);
            match self.write_gmail_draft(row).await? {
                Ok(written) => {
                    row = written;
                    report.saved.push(row.id.clone());
                    // A new message's thread is stored now: a held send's reminder can go on it.
                    if row.send_at.is_some() {
                        report.reminded |= self.give_reminder(&row.id).await?;
                    }
                }
                Err(failure) => {
                    let refused = failure.is_permanent();
                    if refused {
                        log::warn!("Gmail refused draft {id}: {}", failure.message());
                        // Given up for this version; the next save tries again.
                        let mut db = self.db.clone();
                        let mut write = self.begin(&mut db).await?;
                        DraftRow::update_by_id(&id)
                            .pushed(revision)
                            .failures(0)
                            .not_before(None)
                            .exec(write.exec())
                            .await?;
                        write.publish().await?;
                    }
                    // The message a send holds can't be written: the send fails. A save's server
                    // errors are retried, as for any draft.
                    let failed = send_at.and_then(|at| match &failure {
                        Failure::Transient(_) => None,
                        failure => verdict(failure, at <= now, now),
                    });
                    if let Some(reason) = failed {
                        self.fail_send(&id, reason, report).await?;
                        return Ok(Ok(()));
                    }
                    if !refused {
                        return Ok(Err(failure));
                    }
                    report.failed.push(DraftFailure::Save {
                        id,
                        error: failure.message(),
                    });
                    return Ok(Ok(()));
                }
            }
        }
        if row.send_at.is_some_and(|at| at <= now) {
            return self.send_gmail_draft(&row.id, now, report).await;
        }
        if row.failures > 0 {
            let mut db = self.db.clone();
            let mut write = self.begin(&mut db).await?;
            DraftRow::update_by_id(&row.id)
                .failures(0)
                .not_before(None)
                .exec(write.exec())
                .await?;
            write.commit().await?;
        }
        Ok(Ok(()))
    }

    /// Dates a scheduled send's message now, as it goes, rather than when Schedule was pressed:
    /// Gmail sends a draft's `Date:` as written, and recipients see it. A new revision, so the
    /// push writes it before sending. Within a minute of now it's left as it is.
    async fn redate(&self, row: &mut DraftRow, now: Timestamp) -> Result<()> {
        let zoned = now.to_zoned(jiff::tz::TimeZone::system());
        let fresh = |line: &str| {
            line.strip_prefix("Date: ")
                .and_then(|date| jiff::fmt::rfc2822::parse(date).ok())
                .is_some_and(|date| {
                    date.timestamp().duration_until(now).abs() < SignedDuration::from_mins(1)
                })
        };
        let (head, body) = row.raw.split_once("\r\n\r\n").unwrap_or((&row.raw, ""));
        if head.split("\r\n").any(fresh) {
            return Ok(());
        }
        let Ok(date) = jiff::fmt::rfc2822::to_string(&zoned) else {
            return Ok(());
        };
        let mut lines: Vec<String> = head
            .split("\r\n")
            .filter(|line| !line.starts_with("Date:"))
            .map(str::to_owned)
            .collect();
        lines.insert(0, format!("Date: {date}"));
        row.raw = format!("{}\r\n\r\n{body}", lines.join("\r\n"));
        row.revision += 1;
        let mut db = self.db.clone();
        let mut write = self.begin(&mut db).await?;
        DraftRow::update_by_id(&row.id)
            .raw(row.raw.clone())
            .revision(row.revision)
            .exec(write.exec())
            .await?;
        write.commit().await
    }

    /// Whether `row`'s Gmail draft holds another message than the one Exo last wrote (someone
    /// edited it in Gmail web, say). A draft that's gone is the send's to find out about.
    async fn rewritten_elsewhere(&self, row: &DraftRow) -> std::result::Result<bool, Failure> {
        let (Some(gmail_id), Some(message)) = (&row.gmail_id, &row.message_id) else {
            return Ok(false);
        };
        match self
            .api
            .call(Lane::Act, 2, |gmail| {
                gmail.draft(gmail_id, gmail::Format::Minimal)
            })
            .await
        {
            Ok(draft) => Ok(draft
                .message
                .and_then(|held| held.id)
                .is_some_and(|held| held != *message)),
            Err(failure) if failure.is_not_found() => Ok(false),
            Err(failure) => Err(failure),
        }
    }

    /// The reply that calls off scheduled send `row`, if it's `if no reply` and the thread as
    /// Gmail has it now has one: a message it didn't have when the send was scheduled, that isn't
    /// a draft or the user's (Gmail labels the user's own `SENT`).
    async fn aborting_reply(&self, row: &DraftRow) -> std::result::Result<Option<String>, Failure> {
        let (Some(later), Some(thread)) = (
            row.record
                .0
                .send_later
                .as_ref()
                .filter(|later| later.abort_on_reply && !later.current_message_ids.is_empty()),
            &row.thread_id,
        ) else {
            return Ok(None);
        };
        let fetched = match self
            .api
            .call(Lane::Act, 2, |gmail| {
                gmail.thread(thread, gmail::Format::Minimal, &[])
            })
            .await
        {
            Err(failure) if failure.is_not_found() => return Ok(None),
            fetched => fetched?,
        };
        Ok(fetched
            .messages
            .into_iter()
            .find(|message| {
                is_reply(
                    later,
                    message.id.as_deref().unwrap_or_default(),
                    &message.label_ids,
                )
            })
            .and_then(|message| message.id))
    }

    /// Calls off draft `id`'s scheduled send, which a reply came before (`if no reply`): it's a
    /// draft again with its time kept, which compose shows as `Unscheduled: {name} replied`.
    async fn unschedule(&self, id: &str) -> Result<()> {
        let mut db = self.db.clone();
        let mut write = self.begin(&mut db).await?;
        let Some(row) = DraftRow::filter_by_id(id)
            .first()
            .exec(write.exec())
            .await?
        else {
            return write.commit().await;
        };
        log::info!("a reply called off draft {id}'s scheduled send");
        DraftRow::update_by_id(id)
            .send_at(None)
            .failures(0)
            .not_before(None)
            .exec(write.exec())
            .await?;
        if let Some(thread) = &row.thread_id {
            reindex(write.exec(), thread, &self.router()).await?;
        }
        write.publish().await
    }

    /// Calls off the `if no reply` sends scheduled in `threads` that a stored reply came before,
    /// once sync has brought it in.
    pub(crate) async fn unschedule_replied(&self, threads: &HashSet<String>) -> Result<()> {
        let mut db = self.db.clone();
        let rows = DraftRow::all().exec(&mut db).await?;
        let scheduled = rows.iter().filter(|row| {
            row.send_at.is_some()
                && !row.discarded
                && row
                    .thread_id
                    .as_ref()
                    .is_some_and(|thread| threads.contains(thread))
        });
        for row in scheduled {
            let (Some(later), Some(thread)) =
                (
                    row.record.0.send_later.as_ref().filter(|later| {
                        later.abort_on_reply && !later.current_message_ids.is_empty()
                    }),
                    &row.thread_id,
                )
            else {
                continue;
            };
            let stored = MessageRow::filter_by_thread_id(thread)
                .exec(&mut db)
                .await?;
            if stored
                .iter()
                .any(|message| is_reply(later, &message.id, &message.label_ids))
            {
                self.unschedule(&row.id).await?;
            }
        }
        Ok(())
    }

    /// The row's Gmail draft: the one it made, or the one holding the message it took over
    /// (none if that's gone since).
    async fn gmail_draft_id(&self, row: &DraftRow) -> std::result::Result<Option<String>, Failure> {
        let (None, Some(message)) = (&row.gmail_id, &row.message_id) else {
            return Ok(row.gmail_id.clone());
        };
        let mut page_token = None;
        loop {
            let query = gmail::DraftQuery {
                max_results: Some(500),
                page_token: page_token.take(),
                ..gmail::DraftQuery::default()
            };
            let page = self
                .api
                .call(Lane::Act, 3, |gmail| gmail.drafts(&query))
                .await?;
            let found = page
                .drafts
                .into_iter()
                .find(|draft| draft.message.as_ref().and_then(|m| m.id.as_ref()) == Some(message));
            if let Some(draft) = found {
                return Ok(draft.id);
            }
            match page.next_page_token {
                Some(token) => page_token = Some(token),
                None => return Ok(None),
            }
        }
    }

    /// Creates or updates the Gmail draft with the row's message, and returns the row as Gmail
    /// now has it. A Gmail draft deleted elsewhere is created again.
    async fn write_gmail_draft(&self, row: DraftRow) -> Result<Result<DraftRow, Failure>> {
        let raw = row.raw.clone().into_bytes();
        let thread = row.thread_id.as_deref();
        let gmail_id = match self.gmail_draft_id(&row).await {
            Ok(gmail_id) => gmail_id,
            Err(failure) => return Ok(Err(failure)),
        };
        let progress = self.upload_progress(&row);
        // Updating the Gmail draft that holds the row's message replaces that message; a draft
        // made anew leaves it as it was (sent, say).
        let attempt = async {
            Ok::<_, crate::Error>(match &gmail_id {
                Some(gmail_id) => match self
                    .api
                    .call(Lane::Act, 1, |gmail| {
                        reporting(
                            gmail.update_draft(gmail_id, raw.clone(), thread),
                            progress.as_ref(),
                        )
                    })
                    .await
                {
                    Err(failure) if failure.is_not_found() => (
                        self.create_gmail_draft(&row, &raw, thread, progress.as_ref())
                            .await?,
                        None,
                    ),
                    written => (written, row.message_id.as_deref()),
                },
                None => (
                    self.create_gmail_draft(&row, &raw, thread, progress.as_ref())
                        .await?,
                    None,
                ),
            })
        }
        .await;
        if !matches!(attempt, Ok((Ok(_), _))) {
            self.upload_ended(&row.id);
        }
        let (written, replaced) = attempt?;
        let draft = match written {
            Ok(draft) => draft,
            Err(failure) => return Ok(Err(failure)),
        };
        self.set_gmail_files(&row.id, local_files(&row.record.0));
        let message = draft.message.unwrap_or_default();
        let thread_id = message.thread_id.or_else(|| row.thread_id.clone());
        // Known before the row says Gmail has its message, so a save meanwhile that matches what
        // Gmail had before isn't taken for one Gmail has.
        if let Some(message) = &message.id {
            self.acked
                .lock()
                .expect("never poisoned")
                .insert(row.id.clone(), (message.clone(), content(&row.raw)));
        }
        let mut db = self.db.clone();
        let mut write = self.begin(&mut db).await?;
        // Saves made while Gmail was being written keep their newer revision, to push next.
        DraftRow::update_by_id(&row.id)
            .gmail_id(draft.id.clone())
            .message_id(message.id.clone())
            .thread_id(thread_id.clone())
            .pushed(row.revision)
            .failures(0)
            .not_before(None)
            .exec(write.exec())
            .await?;
        write.commit().await?;
        if let Some(message) = &message.id {
            self.record_draft(message).await?;
        }
        match (&message.id, &thread_id, replaced) {
            (Some(message), Some(thread), Some(replaced)) => {
                // A listed file shows on the draft's row, which only its parts tell; a scheduled
                // send shows in its thread as a message.
                let full = scheduled(&row).is_some()
                    || row
                        .record
                        .0
                        .attachments
                        .iter()
                        .any(|attachment| attachment.content_id.is_none());
                self.refresh_message(
                    thread,
                    row.thread_id.as_deref(),
                    message,
                    Some(replaced),
                    full,
                )
                .await?;
            }
            // A new Gmail draft: the thread's rows say what else is there now. A scheduled one's
            // thread shows it as a message, body and all.
            (message, Some(thread), None) => {
                self.refresh_thread(thread).await?;
                if let Some(message) = message.as_deref().filter(|_| scheduled(&row).is_some()) {
                    self.refresh_message(thread, Some(thread), message, None, true)
                        .await?;
                }
            }
            _ => {}
        }
        // The pace counts from the write's end.
        self.written
            .lock()
            .expect("never poisoned")
            .insert(row.id.clone(), Timestamp::now());
        self.notify();
        Ok(Ok(DraftRow {
            gmail_id: draft.id,
            message_id: message.id,
            thread_id,
            pushed: row.revision,
            ..row
        }))
    }

    /// Creates `row`'s Gmail draft. After a failed attempt, which may have created it with only
    /// the answer lost, the Gmail draft with its Message-ID is updated instead, if there's one
    /// no other draft of Exo's has (a reply made a new message keeps the reply's Message-ID).
    async fn create_gmail_draft(
        &self,
        row: &DraftRow,
        raw: &[u8],
        thread: Option<&str>,
        progress: Option<&gmail::Progress>,
    ) -> Result<std::result::Result<gmail::Draft, Failure>> {
        if row.failures > 0 && !row.record.0.rfc822_id.is_empty() {
            let query = gmail::DraftQuery {
                q: Some(format!("rfc822msgid:{}", row.record.0.rfc822_id)),
                ..gmail::DraftQuery::default()
            };
            let found = match self
                .api
                .call(Lane::Act, 2, |gmail| gmail.drafts(&query))
                .await
            {
                Ok(found) => found.drafts,
                Err(failure) => return Ok(Err(failure)),
            };
            let mut db = self.db.clone();
            let others: HashSet<String> = DraftRow::all()
                .exec(&mut db)
                .await?
                .into_iter()
                .filter(|other| other.id != row.id)
                .filter_map(|other| other.gmail_id)
                .collect();
            if let Some(id) = found
                .into_iter()
                .filter_map(|draft| draft.id)
                .find(|id| !others.contains(id))
            {
                return Ok(self
                    .api
                    .call(Lane::Act, 1, |gmail| {
                        reporting(gmail.update_draft(&id, raw.to_vec(), thread), progress)
                    })
                    .await);
            }
        }
        Ok(self
            .api
            .call(Lane::Act, 1, |gmail| {
                reporting(gmail.create_draft(raw.to_vec(), thread), progress)
            })
            .await)
    }

    /// Sends draft `id`'s Gmail draft (`drafts.send`) and drops the row, unless an undo, a
    /// discard or a save got to it after the push read it: the row is read again, under the
    /// lock undo send takes.
    async fn send_gmail_draft(
        &self,
        id: &str,
        now: Timestamp,
        report: &mut DraftReport,
    ) -> Result<Result<(), Failure>> {
        let sending = self.sending.lock().await;
        let mut db = self.db.clone();
        let Some(row) = DraftRow::filter_by_id(id).first().exec(&mut db).await? else {
            return Ok(Ok(()));
        };
        // Taken back, held longer, discarded, or saved again (sent once Gmail has that version).
        if row.discarded || row.pushed < row.revision || !row.send_at.is_some_and(|at| at <= now) {
            return Ok(Ok(()));
        }
        let Some(gmail_id) = row.gmail_id.clone() else {
            return Ok(Ok(()));
        };
        // A reply sync hasn't brought in yet (Exo was closed, say) still calls it off.
        match self.aborting_reply(&row).await {
            Ok(None) => {}
            Ok(Some(_)) => {
                drop(sending);
                self.unschedule(id).await?;
                return Ok(Ok(()));
            }
            Err(failure) => return Ok(Err(failure)),
        }
        let mut result = self
            .api
            .call(Lane::Act, SEND_ATTEMPTS, |gmail| {
                gmail.send_draft(&gmail_id)
            })
            .await;
        if result.as_ref().is_err_and(|failure| {
            failure.is_not_found() || matches!(failure, Failure::Transient(_))
        }) {
            // The draft is gone, or a server error may have lost Gmail's answer. If Gmail sent it
            // (an attempt whose answer was lost, or Send pressed elsewhere), it went.
            match self.sent_message(&row.record.0.rfc822_id).await {
                Ok(Some(message)) => result = Ok(message),
                Ok(None) => {}
                Err(failure) => return Ok(Err(failure)),
            }
        }
        let sent = match result {
            Ok(sent) => sent,
            Err(failure) => {
                let Some(reason) = verdict(&failure, true, now) else {
                    return Ok(Err(failure));
                };
                log::warn!(
                    "Gmail wouldn't send draft {}: {}",
                    row.id,
                    failure.message()
                );
                self.fail_send(&row.id, reason, report).await?;
                return Ok(Ok(()));
            }
        };
        let thread = sent
            .thread_id
            .clone()
            .or(row.thread_id.clone())
            .unwrap_or_default();
        // A reminder being given meanwhile is given before the row goes, or finds it gone.
        let giving = self.giving.lock().await;
        let mut db = self.db.clone();
        let mut write = self.begin(&mut db).await?;
        DraftRow::filter_by_id(&row.id)
            .delete()
            .exec(write.exec())
            .await?;
        let mut given = crate::reminders::given(write.exec()).await?;
        let given_reminder = given.remove(&row.id);
        if given_reminder.is_some() {
            db::kv_set(write.exec(), GIVEN_REMINDERS, &given).await?;
        }
        write.publish().await?;
        self.given.lock().expect("never poisoned").remove(&row.id);
        drop(giving);
        drop(sending);
        if let Some(message) = sent.id.as_deref().filter(|_| !thread.is_empty()) {
            // The thread shows it, so it's fetched whole.
            self.refresh_message(
                &thread,
                row.thread_id.as_deref(),
                message,
                row.message_id.as_deref(),
                true,
            )
            .await?;
        }
        let record = row.record.0;
        // The send put it on its thread already, unless the thread wasn't stored then.
        if let Some(reminder) = record
            .reminder
            .filter(|reminder| !thread.is_empty() && given_reminder != Some(*reminder))
        {
            let set_on = record
                .send_later
                .map(|later| later.current_message_ids)
                .filter(|ids| !ids.is_empty());
            match self.remind_sent(&thread, reminder, set_on).await {
                Ok(()) => report.reminded = true,
                Err(error) => {
                    log::error!("setting the reminder sent with draft {}: {error}", row.id)
                }
            }
        }
        report.sent.push(SentDraft {
            id: row.id,
            thread: ThreadId::new(thread),
            message: sent.id.unwrap_or_default(),
        });
        Ok(Ok(()))
    }

    /// Gives up on draft `id`'s send for `reason`. It becomes a failed draft, which its thread
    /// shows by Gmail's copy (once Gmail has one: a save that waits for sign-in) until it's
    /// edited, sent again or discarded. One whose only save Gmail refused will have no copy to
    /// show, so it's only a draft again.
    async fn fail_send(
        &self,
        id: &str,
        reason: SendFailure,
        report: &mut DraftReport,
    ) -> Result<()> {
        let mut db = self.db.clone();
        let mut write = self.begin(&mut db).await?;
        let Some(row) = DraftRow::filter_by_id(id)
            .first()
            .exec(write.exec())
            .await?
        else {
            return write.commit().await;
        };
        let kept = row.message_id.is_some() || row.pushed < row.revision;
        DraftRow::update_by_id(id)
            .send_at(None)
            .failures(0)
            .not_before(None)
            .failure(kept.then(|| reason.code().to_owned()))
            .failed_at(kept.then(Timestamp::now))
            .exec(write.exec())
            .await?;
        if let Some(thread) = &row.thread_id {
            reindex(write.exec(), thread, &self.router()).await?;
        }
        write.publish().await?;
        // The thread shows it as a message now, so it needs the body its saves didn't store.
        if kept && let (Some(thread), Some(message)) = (&row.thread_id, &row.message_id) {
            self.refresh_message(thread, Some(thread), message, None, true)
                .await?;
        }
        report.failed.push(DraftFailure::Send {
            id: id.to_owned(),
            reason,
        });
        Ok(())
    }

    /// Gmail's sent message with Message-ID `rfc822_id`, if there is one.
    async fn sent_message(
        &self,
        rfc822_id: &str,
    ) -> std::result::Result<Option<gmail::Message>, Failure> {
        if rfc822_id.is_empty() {
            return Ok(None);
        }
        let query = gmail::ListQuery {
            q: Some(format!("rfc822msgid:{rfc822_id}")),
            label_ids: vec![SENT.to_owned()],
            ..gmail::ListQuery::default()
        };
        let page = self
            .api
            .call(Lane::Act, 2, |gmail| gmail.messages(&query))
            .await?;
        Ok(page.messages.into_iter().next())
    }

    /// Deletes a discarded draft's Gmail draft, then the row (unless it was saved again since).
    async fn delete_gmail_draft(&self, row: DraftRow) -> Result<Result<(), Failure>> {
        let gmail_id = match self.gmail_draft_id(&row).await {
            Ok(gmail_id) => gmail_id,
            Err(failure) => return Ok(Err(failure)),
        };
        if let Some(gmail_id) = &gmail_id {
            match self
                .api
                .call(Lane::Act, 1, |gmail| gmail.delete_draft(gmail_id))
                .await
            {
                Ok(()) => {}
                Err(failure) if failure.is_not_found() => {}
                Err(failure @ Failure::Permanent(_)) => log::warn!(
                    "Gmail wouldn't delete draft {}: {}",
                    row.id,
                    failure.message()
                ),
                Err(failure) => return Ok(Err(failure)),
            }
        }
        let mut db = self.db.clone();
        let mut write = self.begin(&mut db).await?;
        match DraftRow::filter_by_id(&row.id)
            .first()
            .exec(write.exec())
            .await?
        {
            Some(current) if current.discarded => {
                DraftRow::filter_by_id(&row.id)
                    .delete()
                    .exec(write.exec())
                    .await?;
                // Its message goes with it: the row no longer hides it, and a thread whose
                // newest message was the draft can come back from Gmail with an older history
                // id, which leaves the stored copy alone until history says it's gone.
                if let (Some(message), Some(thread)) = (&current.message_id, &row.thread_id) {
                    provider::delete_messages(write.exec(), std::slice::from_ref(message)).await?;
                    reindex(write.exec(), thread, &self.router()).await?;
                }
            }
            // Undone meanwhile: the Gmail draft is gone, so the next push creates it again.
            Some(_) => {
                DraftRow::update_by_id(&row.id)
                    .gmail_id(None)
                    .message_id(None)
                    .pushed(0)
                    .exec(write.exec())
                    .await?;
            }
            None => {}
        }
        write.publish().await?;
        if let Some(thread) = &row.thread_id {
            self.refresh_thread(thread).await?;
        }
        Ok(Ok(()))
    }

    /// Drops the drafts whose Gmail draft was sent, deleted or rewritten elsewhere: those whose
    /// message is in `undrafted` (gone from Gmail's drafts, by its history), that Gmail had the
    /// latest version of, and that aren't held or discarded. Compose then opens what Gmail has,
    /// if anything, as a draft written elsewhere, instead of Exo's stale copy (which saving would
    /// write back, or Send send again). A push under way replaces messages itself, so with one
    /// running they're looked at again once it's done.
    pub(crate) async fn forget_drafts_gone(&self, undrafted: &HashSet<String>) -> Result<()> {
        let gone = |rows: Vec<DraftRow>| -> Vec<DraftRow> {
            rows.into_iter()
                .filter(|row| {
                    !row.discarded
                        && row.send_at.is_none()
                        && row.pushed >= row.revision
                        && row
                            .message_id
                            .as_ref()
                            .is_some_and(|message| undrafted.contains(message))
                })
                .collect()
        };
        // Gmail no longer holds those messages, so a save matching one has changes to write.
        self.acked
            .lock()
            .expect("never poisoned")
            .retain(|_, (message, _)| !undrafted.contains(message));
        let mut db = self.db.clone();
        if undrafted.is_empty() || gone(DraftRow::all().exec(&mut db).await?).is_empty() {
            return Ok(());
        }
        let _flush = self.flush.lock().await;
        let mut write = self.begin(&mut db).await?;
        for row in gone(DraftRow::all().exec(write.exec()).await?) {
            log::info!(
                "draft {} was sent, deleted or rewritten outside Exo",
                row.id
            );
            DraftRow::filter_by_id(&row.id)
                .delete()
                .exec(write.exec())
                .await?;
            // Its text leaves the thread's search index with it.
            if let Some(thread) = &row.thread_id {
                reindex(write.exec(), thread, &self.router()).await?;
            }
        }
        write.publish().await
    }

    /// Replaces the local copy of `thread` with Gmail's rows (stored messages aren't fetched in
    /// full again), so lists show a draft's change without waiting for sync. A failure here is
    /// left to the next sync.
    async fn refresh_thread(&self, thread: &str) -> Result<()> {
        let mut db = self.db.clone();
        let stored: HashSet<String> = MessageRow::filter_by_thread_id(thread)
            .select(MessageRow::fields().id())
            .exec(&mut db)
            .await?
            .into_iter()
            .collect();
        let fetched = match fetch_rows(&self.api, Lane::Act, 2, thread, &stored).await {
            Ok(fetched) => fetched,
            Err(failure) => {
                log::debug!(
                    "refreshing thread {thread} after a draft change: {}",
                    failure.message()
                );
                return Ok(());
            }
        };
        let mut write = self.begin(&mut db).await?;
        match fetched {
            Some(fetched) => {
                provider::save_rows(write.exec(), &fetched).await?;
            }
            None => provider::delete_thread(write.exec(), thread).await?,
        }
        reindex(write.exec(), thread, &self.router()).await?;
        write.publish().await
    }

    /// Stores `message`, which Gmail just wrote into `thread` (asked to keep it in `requested`)
    /// in place of `replaced`, the draft version it replaced, so lists show it without waiting
    /// for sync. It's fetched whole when `full` (a sent message, which the thread shows; a
    /// draft with files), else for its row alone: compose shows Exo's drafts from their record.
    /// `replaced` goes at once, since compose would take up a draft message that isn't Exo's
    /// latest as one written elsewhere. A failure to fetch is left to the next sync.
    async fn refresh_message(
        &self,
        thread: &str,
        requested: Option<&str>,
        message: &str,
        replaced: Option<&str>,
        full: bool,
    ) -> Result<()> {
        let mut db = self.db.clone();
        // A thread Gmail started for the message holds it alone, but the one asked for may hold
        // more: if it isn't stored, its rows bring the rest.
        if requested == Some(thread)
            && ThreadRow::filter_by_id(thread)
                .first()
                .exec(&mut db)
                .await?
                .is_none()
        {
            return self.refresh_thread(thread).await;
        }
        let fetched = if full {
            fetch_message(&self.api, Lane::Act, 2, message).await
        } else {
            fetch_row(&self.api, Lane::Act, 2, message).await
        };
        let fetched = match fetched {
            Ok(fetched) => fetched,
            Err(failure) => {
                log::debug!(
                    "fetching message {message} after a draft change: {}",
                    failure.message()
                );
                return Ok(());
            }
        };
        let mut write = self.begin(&mut db).await?;
        let mut changed = vec![thread.to_owned()];
        if let Some(replaced) = replaced.filter(|replaced| *replaced != message) {
            if let Some(old) = MessageRow::filter_by_id(replaced)
                .first()
                .exec(write.exec())
                .await?
                && old.thread_id != thread
            {
                changed.push(old.thread_id);
            }
            provider::delete_messages(write.exec(), &[replaced.to_owned()]).await?;
        }
        if let Some(parsed) = fetched.as_ref().and_then(|fetched| parse(fetched, full)) {
            provider::save_message(write.exec(), parsed).await?;
        }
        for thread in &changed {
            if MessageRow::filter_by_thread_id(thread)
                .first()
                .exec(write.exec())
                .await?
                .is_none()
            {
                provider::delete_thread(write.exec(), thread).await?;
            } else {
                reindex(write.exec(), thread, &self.router()).await?;
            }
        }
        write.publish().await
    }
}

/// Whether message `id`, labelled `labels`, is a reply that calls off scheduled send `later`:
/// one the thread didn't have when it was scheduled, that isn't a draft or the user's.
fn is_reply(later: &SendLater, id: &str, labels: &[String]) -> bool {
    !later.current_message_ids.iter().any(|known| known == id)
        && !labels.iter().any(|label| label == DRAFT || label == SENT)
}

/// What a Gmail failure means for a send, `due` or still held: `None` to try again later, or why
/// the send fails. While it's held only Gmail refusing its message fails it. Being offline never
/// does: the send goes once Gmail is back.
fn verdict(failure: &Failure, due: bool, now: Timestamp) -> Option<SendFailure> {
    match failure {
        Failure::Permanent(error) => Some(send_failure(error)),
        _ if !due => None,
        Failure::Offline(_) => None,
        Failure::Unauthorized(_) => Some(SendFailure::LoggedOut),
        Failure::Transient(_) => Some(SendFailure::Temporary),
        Failure::Throttled(until) | Failure::SendingLimited(until) => {
            let wait = now.duration_until(*until);
            if wait >= SENDING_LIMIT {
                Some(SendFailure::QuotaExceeded)
            } else if wait > SEND_WAITS {
                Some(SendFailure::RateLimited)
            } else {
                None
            }
        }
    }
}

/// The send failure Gmail's refusal stands for.
fn send_failure(error: &gmail::Error) -> SendFailure {
    // Google's upload front end may answer an oversized message with a page, not its JSON.
    if error.status() == Some(413) {
        return SendFailure::TooLarge;
    }
    let gmail::Error::Api(api) = error else {
        return SendFailure::Other;
    };
    let reason = |wanted: &str| api.errors.iter().any(|e| e.reason == wanted);
    if reason("dailyLimitExceeded") || reason("quotaExceeded") {
        SendFailure::QuotaExceeded
    } else if api.message.to_lowercase().contains("too large") {
        SendFailure::TooLarge
    } else if api.message.contains("Invalid To header")
        || api.message.contains("Invalid Cc header")
        || api.message.contains("Invalid Bcc header")
        || api.message.contains("Recipient address required")
    {
        SendFailure::InvalidRecipients
    } else {
        SendFailure::Other
    }
}

/// How much of a message's text Gmail's snippets hold.
const SNIPPET_CHARS: usize = 200;

/// What Exo's drafts change in reads.
#[derive(Default)]
pub(crate) struct Marks {
    /// Draft messages reads leave out: discarded drafts', and held sends' (a send on its way is
    /// no longer an unsent draft, so it leaves the Inbox and the `Draft to …` row).
    pub hidden: HashSet<String>,
    /// Failed drafts' messages, with why and when each send failed.
    pub failed: HashMap<String, (SendFailure, Timestamp)>,
    /// Threads with a send held or waiting to go.
    pub sending: HashSet<String>,
    /// Those sends' text, as Gmail would snippet it: a waiting one's row in Scheduled shows it.
    pub sending_snippets: HashMap<String, String>,
    /// Scheduled sends' messages, with their time: they show in their thread as messages, not
    /// hidden like the sends undo send holds.
    pub scheduled: HashMap<String, Timestamp>,
    /// Threads with a scheduled send: the earliest one's time, and whether Gmail has the latest
    /// version of every one.
    pub scheduled_threads: HashMap<String, (Timestamp, bool)>,
}

/// Whether the draft's last save had every file ([`Draft::complete`]).
pub(crate) fn complete(row: &DraftRow) -> bool {
    row.not_before != Some(UNTIL_COMPLETE)
}

/// When draft `row` is scheduled to go, if it is: a send held for its Send later time.
pub(crate) fn scheduled(row: &DraftRow) -> Option<Timestamp> {
    row.send_at
        .filter(|_| !row.discarded && row.failure.is_none() && row.record.0.send_later.is_some())
}

/// Exo's drafts in `thread` being written (not held, failed or discarded), whose text the search
/// index takes from their records, as their messages are stored without bodies. Held and discarded
/// drafts' messages are hidden; a failed or scheduled one's has its body.
pub(crate) async fn editable(
    exec: &mut dyn toasty::Executor,
    thread: &str,
) -> Result<Vec<DraftRow>> {
    let rows = DraftRow::all().exec(exec).await?;
    Ok(rows
        .into_iter()
        .filter(|row| {
            row.thread_id.as_deref() == Some(thread)
                && !row.discarded
                && row.send_at.is_none()
                && row.failure.is_none()
        })
        .collect())
}

pub(crate) async fn marks(exec: &mut dyn toasty::Executor) -> Result<Marks> {
    let mut marks = Marks::default();
    for row in DraftRow::all().exec(exec).await? {
        let held = row.send_at.is_some() && !row.discarded;
        let scheduled = scheduled(&row);
        if let (true, Some(thread)) = (held, &row.thread_id) {
            marks.sending.insert(thread.clone());
            let text = crate::search::whole_text(Some(&row.record.0.body), None);
            marks
                .sending_snippets
                .insert(thread.clone(), text.chars().take(SNIPPET_CHARS).collect());
        }
        if let (Some(at), Some(thread)) = (scheduled, &row.thread_id) {
            let persisted = row.pushed >= row.revision;
            let entry = marks
                .scheduled_threads
                .entry(thread.clone())
                .or_insert((at, persisted));
            *entry = (entry.0.min(at), entry.1 && persisted);
        }
        let Some(message) = row.message_id else {
            continue;
        };
        if let Some(at) = scheduled {
            marks.scheduled.insert(message, at);
        } else if row.discarded || held {
            marks.hidden.insert(message);
        } else if let (Some(reason), Some(at)) = (row.failure.as_deref(), row.failed_at) {
            marks
                .failed
                .insert(message, (SendFailure::from_code(reason), at));
        }
    }
    Ok(marks)
}
