//! Writes of provider truth: what Gmail says threads and messages are. The outbox overlay is
//! never written here; it's applied on read and when list membership is recomputed.
//!
//! Label writes only move forward in `historyId`, so history, backfill and the outbox can store
//! what they fetched in any order and the newest copy wins.

use std::collections::BTreeSet;

use toasty::Executor;

use crate::db::{AttachmentRow, BodyRow, ListEntry, MessageRow, ReminderRow, ThreadRow};
use crate::error::Result;
use crate::mutation::Step;
use crate::parse::{Parsed, parse};
use crate::sync::Rows;
use crate::view::ThreadView;

/// Stores a thread fetched for its rows, unless the local copy is newer: the messages it holds
/// in full are stored whole, the rest as rows alone. Messages Gmail no longer lists are
/// deleted. Returns the thread id when something was written.
pub(crate) async fn save_rows(exec: &mut dyn Executor, rows: &Rows) -> Result<Option<String>> {
    store_thread(exec, &rows.thread, |id| rows.full.contains(id)).await
}

/// Stores `thread`, the messages `full` holds as fetched in full and the rest as rows alone.
async fn store_thread(
    exec: &mut dyn Executor,
    thread: &gmail::Thread,
    full: impl Fn(&str) -> bool,
) -> Result<Option<String>> {
    let Some(id) = thread.id.clone() else {
        return Ok(None);
    };
    let history_id = thread.history_id.unwrap_or_default();
    if let Some(local) = ThreadRow::filter_by_id(&id).first().exec(exec).await?
        && local.history_id > history_id
    {
        return Ok(None);
    }
    let parsed: Vec<Parsed> = thread
        .messages
        .iter()
        .filter_map(|message| parse(message, message.id.as_deref().is_some_and(&full)))
        .collect();
    let existing = MessageRow::filter_by_thread_id(&id).exec(exec).await?;
    let gone: Vec<String> = existing
        .iter()
        .filter(|row| !parsed.iter().any(|parsed| parsed.message.id == row.id))
        .map(|row| row.id.clone())
        .collect();
    delete_messages(exec, &gone).await?;
    for parsed in parsed {
        let stored = existing.iter().any(|row| row.id == parsed.message.id);
        upsert_message(exec, parsed, stored).await?;
    }
    ThreadRow::upsert_by_id(&id)
        .history_id(history_id)
        .exec(exec)
        .await?;
    Ok(Some(id))
}

/// Adds what a row alone lacks to the stored messages among `messages` (fetched in full) that
/// lack it: body, attachments, every header. Their labels move forward as usual; nothing else
/// changes, so a copy older than what history stored since can't undo it.
pub(crate) async fn save_bodies(
    exec: &mut dyn Executor,
    messages: &[gmail::Message],
) -> Result<()> {
    for parsed in messages.iter().filter_map(|message| parse(message, true)) {
        if MessageRow::filter_by_id(&parsed.message.id)
            .first()
            .exec(exec)
            .await?
            .is_some()
        {
            upsert_message(exec, parsed, true).await?;
        }
    }
    Ok(())
}

/// Stores one message fetched with `messages.get` in full (a history addition).
pub(crate) async fn save_message(exec: &mut dyn Executor, parsed: Parsed) -> Result<()> {
    let (thread, history_id) = (parsed.message.thread_id.clone(), parsed.message.history_id);
    let stored = MessageRow::filter_by_id(&parsed.message.id)
        .first()
        .exec(exec)
        .await?
        .is_some();
    upsert_message(exec, parsed, stored).await?;
    bump_thread(exec, &thread, history_id).await
}

/// Raises a thread's stored `historyId` to `history_id`.
pub(crate) async fn bump_thread(exec: &mut dyn Executor, id: &str, history_id: u64) -> Result<()> {
    let current = ThreadRow::filter_by_id(id).first().exec(exec).await?;
    if current.is_none_or(|thread| thread.history_id < history_id) {
        ThreadRow::upsert_by_id(id)
            .history_id(history_id)
            .exec(exec)
            .await?;
    }
    Ok(())
}

/// Replaces a message's labels with what Gmail reported at `history_id`, unless the stored
/// copy is at least that new.
pub(crate) async fn set_labels(
    exec: &mut dyn Executor,
    id: &str,
    labels: &[String],
    history_id: u64,
) -> Result<()> {
    let fields = MessageRow::fields();
    MessageRow::filter(fields.id().eq(id).and(fields.history_id().lt(history_id)))
        .update()
        .label_ids(labels.to_vec())
        .history_id(history_id)
        .exec(exec)
        .await?;
    Ok(())
}

/// Applies a step Gmail just acknowledged to the stored labels, so the overlay can drop it
/// without the thread flickering back until the next history sync.
pub(crate) async fn apply_step(exec: &mut dyn Executor, thread: &str, step: &Step) -> Result<()> {
    let rows = MessageRow::filter_by_thread_id(thread).exec(exec).await?;
    // Which drafts are Exo's doesn't matter here: every message's labels are written back.
    let mut view = ThreadView::new(thread.to_owned(), rows, &[], &BTreeSet::new());
    view.apply(step);
    for message in view.all_messages() {
        let labels: Vec<String> = message.labels.iter().cloned().collect();
        let mut stored = message.row.label_ids.clone();
        stored.sort();
        if stored != labels {
            MessageRow::update_by_id(&message.row.id)
                .label_ids(labels)
                .exec(exec)
                .await?;
        }
    }
    Ok(())
}

/// Inserts a message, or, when it's `stored` already, refreshes its labels (the only part of a
/// Gmail message that changes) and adds what a row alone lacks when `parsed` has it.
async fn upsert_message(exec: &mut dyn Executor, parsed: Parsed, stored: bool) -> Result<()> {
    let Parsed {
        message,
        attachments,
        body,
    } = parsed;
    if stored {
        if let Some(body) = body
            && BodyRow::filter_by_message_id(&message.id)
                .first()
                .exec(exec)
                .await?
                .is_none()
        {
            // Every header, not just the row's; the rest of the row is the same.
            MessageRow::update_by_id(&message.id)
                .headers(message.headers)
                .has_attachments(message.has_attachments)
                .exec(exec)
                .await?;
            // An evicted body's message kept its attachment rows: these have Gmail's ids now.
            AttachmentRow::filter(AttachmentRow::fields().message_id().eq(&message.id))
                .delete()
                .exec(exec)
                .await?;
            insert_parts(exec, &message.thread_id, attachments, body).await?;
        }
        return set_labels(exec, &message.id, &message.label_ids, message.history_id).await;
    }
    let thread = message.thread_id.clone();
    MessageRow::create()
        .id(message.id)
        .thread_id(message.thread_id)
        .history_id(message.history_id)
        .internal_date(message.internal_date)
        .label_ids(message.label_ids)
        .subject(message.subject)
        .snippet(message.snippet)
        .sender(message.sender)
        .sender_raw(message.sender_raw)
        .to(message.to)
        .cc(message.cc)
        .bcc(message.bcc)
        .reply_to(message.reply_to)
        .rfc822_id(message.rfc822_id)
        .in_reply_to(message.in_reply_to)
        .references(message.references)
        .list_id(message.list_id)
        .list_unsubscribe(message.list_unsubscribe)
        .list_unsubscribe_post(message.list_unsubscribe_post)
        .headers(message.headers)
        .size(message.size)
        .has_attachments(message.has_attachments)
        .exec(exec)
        .await?;
    match body {
        Some(body) => insert_parts(exec, &thread, attachments, body).await,
        None => Ok(()),
    }
}

/// A message's attachment rows and body, which queues its conversation `thread` for the body's
/// visible text (`crate::text`).
async fn insert_parts(
    exec: &mut dyn Executor,
    thread: &str,
    attachments: Vec<AttachmentRow>,
    body: BodyRow,
) -> Result<()> {
    if !attachments.is_empty() {
        let mut rows = AttachmentRow::create_many();
        for attachment in attachments {
            rows = rows.with_item(|row| {
                row.message_id(attachment.message_id)
                    .part_id(attachment.part_id)
                    .attachment_id(attachment.attachment_id)
                    .filename(attachment.filename)
                    .mime_type(attachment.mime_type)
                    .size(attachment.size)
                    .content_id(attachment.content_id)
                    .inline(attachment.inline)
            });
        }
        rows.exec(exec).await?;
    }
    BodyRow::create()
        .message_id(body.message_id)
        .html(body.html)
        .text(body.text)
        .visible(body.visible)
        .exec(exec)
        .await?;
    crate::text::queue(exec, thread).await
}

/// Deletes messages with their attachments and bodies.
pub(crate) async fn delete_messages(exec: &mut dyn Executor, ids: &[String]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    MessageRow::filter(MessageRow::fields().id().in_list(ids))
        .delete()
        .exec(exec)
        .await?;
    AttachmentRow::filter(AttachmentRow::fields().message_id().in_list(ids))
        .delete()
        .exec(exec)
        .await?;
    BodyRow::filter(BodyRow::fields().message_id().in_list(ids))
        .delete()
        .exec(exec)
        .await?;
    Ok(())
}

/// Deletes a thread Gmail no longer has.
pub(crate) async fn delete_thread(exec: &mut dyn Executor, id: &str) -> Result<()> {
    let ids: Vec<String> = MessageRow::filter_by_thread_id(id)
        .exec(exec)
        .await?
        .into_iter()
        .map(|row| row.id)
        .collect();
    delete_messages(exec, &ids).await?;
    ThreadRow::filter_by_id(id).delete().exec(exec).await?;
    ListEntry::filter_by_thread_id(id)
        .delete()
        .exec(exec)
        .await?;
    // Gone from Gmail, it has nothing to come back to.
    ReminderRow::filter_by_thread_id(id)
        .delete()
        .exec(exec)
        .await?;
    Ok(())
}
