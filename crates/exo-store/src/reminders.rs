//! Reminders (Remind me): each thread's reminder is kept in `reminders`, and Exo brings threads
//! back itself when they're due, from [`Store::return_reminders`], which the app calls when
//! one is due while it runs and once it starts (so a reminder due while Exo was closed comes
//! back at the next launch).
//!
//! Gmail sees reminders as labels: `[Exo]/Is Snoozed` while a reminder is set and, for Remind
//! me's snooze, the thread out of the inbox; when it comes back, the inbox again and `[Exo]/ru`
//! until it's read.

use std::collections::{HashMap, HashSet};

use jiff::Timestamp;
use toasty::Executor;

use crate::db::{self, DraftRow, GIVEN_REMINDERS, ReminderRow};
use crate::drafts::DraftReminder;
use crate::error::{Error, Result};
use crate::ids::ThreadId;
use crate::ids::labels::{SH_SNOOZED, SPAM, TRASH};
use crate::model::ReminderStatus;
use crate::mutation::{Mutation, Plan, change};
use crate::outbox::enqueue;
use crate::store::{Inner, Store, load_view, reindex};
use crate::view::ThreadView;

/// The reminder change `mutation` makes to `view`'s thread, made: what the thread had before,
/// for undo, or `None` for a mutation that doesn't touch reminders.
pub(crate) async fn apply(
    exec: &mut dyn Executor,
    view: &ThreadView,
    mutation: &Mutation,
    now: Timestamp,
) -> Result<Option<Option<ReminderRow>>> {
    let previous = view.reminder.clone();
    match mutation {
        Mutation::Remind {
            at, keep_on_reply, ..
        } => {
            // Set on every message it has now; a reply is any other.
            let row = reminder_row(view, *at, *keep_on_reply, None, now);
            restore(exec, &view.id, Some(row)).await?;
        }
        Mutation::CancelReminder { .. } => restore(exec, &view.id, None).await?,
        // Moving out of Reminders removes it.
        Mutation::Move {
            from: Some(from), ..
        } if from == SH_SNOOZED => restore(exec, &view.id, None).await?,
        _ => return Ok(None),
    }
    Ok(Some(previous))
}

/// A reminder for `view`'s thread at `at`, set on `set_on` (its messages now, when `None`).
fn reminder_row(
    view: &ThreadView,
    at: Timestamp,
    keep_on_reply: bool,
    set_on: Option<Vec<String>>,
    now: Timestamp,
) -> ReminderRow {
    ReminderRow {
        thread_id: view.id.clone(),
        at,
        keep_on_reply,
        message_ids: set_on.unwrap_or_else(|| {
            view.messages
                .iter()
                .filter(|m| !m.is_draft())
                .map(|m| m.row.id.clone())
                .collect()
        }),
        created_at: now,
        fired_at: None,
        aborted: false,
    }
}

/// Gives `thread` the reminder `row`, or none.
pub(crate) async fn restore(
    exec: &mut dyn Executor,
    thread: &str,
    row: Option<ReminderRow>,
) -> Result<()> {
    ReminderRow::filter_by_thread_id(thread)
        .delete()
        .exec(exec)
        .await?;
    if let Some(row) = row {
        ReminderRow::create()
            .thread_id(row.thread_id)
            .at(row.at)
            .keep_on_reply(row.keep_on_reply)
            .message_ids(row.message_ids)
            .created_at(row.created_at)
            .fired_at(row.fired_at)
            .aborted(row.aborted)
            .exec(exec)
            .await?;
    }
    Ok(())
}

impl Store {
    /// When the soonest reminder is due, if one is set.
    ///
    /// # Errors
    /// On database failures.
    pub async fn next_reminder(&self) -> Result<Option<Timestamp>> {
        let mut db = self.inner.db.clone();
        Ok(waiting(&mut db).await?.iter().map(|row| row.at).min())
    }

    /// Brings back every thread whose reminder is due at `now`: into the inbox, sorted as if it
    /// had just come in, "reminder unread" until it's read. A reminder a reply cancelled
    /// ("if no reply") doesn't, and its label comes off; so does that of a thread in Spam or
    /// Trash, which has nothing to come back to. Returns the threads brought back, oldest
    /// reminder first. Flush afterwards.
    ///
    /// # Errors
    /// On database failures, or [`Error::NotSynced`](crate::Error::NotSynced) before the first
    /// sync has made the reminder labels.
    pub async fn return_reminders(&self, now: Timestamp) -> Result<Vec<ThreadId>> {
        let router = self.inner.router();
        let me = &router.me;
        let mut db = self.inner.db.clone();
        // Most looks find nothing to do: they don't need the write lock.
        let rows = waiting(&mut db).await?;
        let ids: Vec<String> = rows.iter().map(|row| row.thread_id.clone()).collect();
        let views = crate::store::load_views(&mut db, &ids).await?;
        let ends = |view: Option<&ThreadView>, at: Timestamp| match view
            .map(|view| view.reminder_status(me))
        {
            None | Some(Some(ReminderStatus::Aborted { .. })) => true,
            Some(Some(ReminderStatus::Set { .. })) => at <= now,
            Some(_) => false,
        };
        if !rows
            .iter()
            .any(|row| ends(views.get(&row.thread_id), row.at))
        {
            return Ok(Vec::new());
        }
        let mut write = self.inner.begin(&mut db).await?;
        let mut rows = waiting(write.exec()).await?;
        rows.sort_by_key(|row| row.at);
        let mut returned = Vec::new();
        let mut changed = false;
        for row in rows {
            let Some(view) = load_view(write.exec(), &row.thread_id).await? else {
                restore(write.exec(), &row.thread_id, None).await?;
                continue;
            };
            let take_label_off = |aborted: bool| ReminderRow {
                aborted,
                ..row.clone()
            };
            let ended = match view.reminder_status(me) {
                Some(ReminderStatus::Aborted { .. }) => Some(Some(take_label_off(true))),
                Some(ReminderStatus::Set { at, .. })
                    if at <= now && (view.has(SPAM) || view.has(TRASH)) =>
                {
                    Some(None)
                }
                Some(ReminderStatus::Set { at, .. }) if at <= now => {
                    let Some((forward, _)) = Mutation::ReturnFromReminder.plan(&view, me)? else {
                        continue;
                    };
                    restore(
                        write.exec(),
                        &view.id,
                        Some(ReminderRow {
                            fired_at: Some(now),
                            ..row
                        }),
                    )
                    .await?;
                    enqueue(
                        write.exec(),
                        &view,
                        &forward,
                        &Mutation::ReturnFromReminder,
                        false,
                        me,
                    )
                    .await?;
                    reindex(write.exec(), &view.id, &router).await?;
                    returned.push(ThreadId::new(view.id));
                    changed = true;
                    continue;
                }
                _ => None,
            };
            let Some(kept) = ended else { continue };
            restore(write.exec(), &view.id, kept).await?;
            if let Some(label) = me.snoozed_label.as_deref().filter(|label| view.has(label)) {
                let plan = Plan {
                    intents: vec![change(&[], &[label])],
                    replace_key: None,
                };
                enqueue(
                    write.exec(),
                    &view,
                    &plan,
                    &Mutation::CancelReminder { to_inbox: false },
                    false,
                    me,
                )
                .await?;
            }
            reindex(write.exec(), &view.id, &router).await?;
            changed = true;
        }
        if changed {
            write.publish().await?;
        } else {
            write.commit().await?;
        }
        Ok(returned)
    }
}

impl Store {
    /// Brings back to the inbox, once, the threads Gmail has under `[Exo]/Is Snoozed` that a new
    /// store has no reminder for: their reminders were in a store signed out or cleared, or on
    /// another Mac, and nothing here would ever return them. Until it has, a call lists the
    /// label, stores what it lacks and returns them as due reminders return; after, it does
    /// nothing. Returns the threads it brought back.
    ///
    /// # Errors
    /// [`Error::Offline`] and the like when Gmail can't be reached (it tries again next time), and
    /// on database failures.
    pub async fn return_orphaned_snoozes(&self) -> Result<Vec<ThreadId>> {
        let inner = &self.inner;
        let mut db = inner.db.clone();
        if db::kv_get::<bool>(&mut db, db::ORPHANED_SNOOZES)
            .await?
            .is_none()
        {
            return Ok(Vec::new());
        }
        inner.bootstrapped().await?;
        let router = inner.router();
        let me = &router.me;
        let label = me
            .snoozed_label
            .clone()
            .ok_or(Error::NotSynced("the reminder labels"))?;
        let mut listed: Vec<(String, u64)> = Vec::new();
        let mut query = gmail::ListQuery {
            label_ids: vec![label],
            max_results: Some(100),
            ..gmail::ListQuery::default()
        };
        loop {
            let page = inner
                .api
                .call(crate::quota::Lane::Sync, crate::sync::ATTEMPTS, |gmail| {
                    gmail.threads(&query)
                })
                .await?;
            listed.extend(page.threads.iter().filter_map(|thread| {
                Some((thread.id.clone()?, thread.history_id.unwrap_or_default()))
            }));
            match page.next_page_token {
                Some(token) => query.page_token = Some(token),
                None => break,
            }
        }
        let fetched = inner.fetch_stale(crate::quota::Lane::Sync, &listed).await?;
        let mut write = inner.begin(&mut db).await?;
        for rows in &fetched {
            if let Some(id) = crate::provider::save_rows(write.exec(), rows).await? {
                reindex(write.exec(), &id, &router).await?;
            }
        }
        let remembered: HashSet<String> = ReminderRow::all()
            .exec(write.exec())
            .await?
            .into_iter()
            .map(|row| row.thread_id)
            .collect();
        let mut returned = Vec::new();
        for (thread, _) in listed
            .iter()
            .filter(|(thread, _)| !remembered.contains(thread))
        {
            let Some(view) = load_view(write.exec(), thread).await? else {
                continue;
            };
            let Some((forward, _)) = Mutation::ReturnFromReminder.plan(&view, me)? else {
                continue;
            };
            enqueue(
                write.exec(),
                &view,
                &forward,
                &Mutation::ReturnFromReminder,
                false,
                me,
            )
            .await?;
            reindex(write.exec(), &view.id, &router).await?;
            returned.push(ThreadId::new(view.id));
        }
        crate::db::KvRow::filter_by_key(db::ORPHANED_SNOOZES)
            .delete()
            .exec(write.exec())
            .await?;
        write.publish().await?;
        if !returned.is_empty() {
            log::info!(
                "brought back {} snoozed threads this store has no reminder for",
                returned.len()
            );
        }
        Ok(returned)
    }
}

impl Inner {
    /// Puts held send `id`'s reminder on its thread, as Send and Schedule do, replacing the one
    /// it had and leaving the thread where it is: from then on the thread shows it and is in
    /// Reminders, and Gmail has `[Exo]/Is Snoozed`. A thread that isn't stored yet (a new
    /// message's, before Gmail has it) gets it when it is, at the draft's first write or else its
    /// send. Returns whether it changed the thread; flush then.
    pub(crate) async fn give_reminder(&self, id: &str) -> Result<bool> {
        let _giving = self.giving.lock().await;
        let mut db = self.db.clone();
        let Some(row) = DraftRow::filter_by_id(id).first().exec(&mut db).await? else {
            return Ok(false);
        };
        let held = row.send_at.is_some() && !row.discarded && row.failure.is_none();
        let (true, Some(reminder), Some(thread)) = (held, row.record.0.reminder, row.thread_id)
        else {
            return Ok(false);
        };
        if given(&mut db).await?.get(id) == Some(&reminder) {
            return Ok(false);
        }
        // Given before in this session (a send called off by Edit Message and scheduled again):
        // that one comes off first, so undo puts back what the thread had before either.
        let earlier = self.given.lock().expect("never poisoned").remove(id);
        if let Some(earlier) = earlier {
            self.undo(earlier).await?;
        }
        let mutation = Mutation::Remind {
            at: reminder.at,
            keep_on_reply: reminder.keep_on_reply,
            snooze: false,
        };
        let undo = match self.apply(&[ThreadId::new(thread)], mutation).await {
            Ok(undo) => undo,
            // Not stored yet, or before the first sync made the reminder labels: later, then.
            Err(Error::UnknownThread(_) | Error::NotSynced(_)) => return Ok(false),
            Err(error) => return Err(error),
        };
        let mut write = self.begin(&mut db).await?;
        let mut given = given(write.exec()).await?;
        let drafts: HashSet<String> = DraftRow::all()
            .select(DraftRow::fields().id())
            .exec(write.exec())
            .await?
            .into_iter()
            .collect();
        given.retain(|draft, _| drafts.contains(draft));
        given.insert(id.to_owned(), reminder);
        db::kv_set(write.exec(), GIVEN_REMINDERS, &given).await?;
        write.commit().await?;
        self.given
            .lock()
            .expect("never poisoned")
            .insert(id.to_owned(), undo);
        Ok(true)
    }

    /// Undo send for held send `id`: the reminder it put on its thread comes off, and the one
    /// the thread had comes back.
    pub(crate) async fn take_back_reminder(&self, id: &str) -> Result<()> {
        let _giving = self.giving.lock().await;
        let undo = self.given.lock().expect("never poisoned").remove(id);
        let mut db = self.db.clone();
        let mut write = self.begin(&mut db).await?;
        let mut given = given(write.exec()).await?;
        if given.remove(id).is_some() {
            db::kv_set(write.exec(), GIVEN_REMINDERS, &given).await?;
        }
        write.commit().await?;
        match undo {
            Some(undo) => self.undo(undo).await,
            None => Ok(()),
        }
    }

    /// Gives `thread` the reminder a draft carried, once Gmail has sent it into the thread, when
    /// the send didn't (its thread wasn't stored yet), replacing the one it had. Like Remind me's
    /// reminder mode it leaves the thread where it is. It's set on `set_on`, the messages a
    /// scheduled send recorded (a reply since cancels an "if no reply" one), or else the thread's
    /// messages now.
    pub(crate) async fn remind_sent(
        &self,
        thread: &str,
        reminder: DraftReminder,
        set_on: Option<Vec<String>>,
    ) -> Result<()> {
        let router = self.router();
        let mut db = self.db.clone();
        let mut write = self.begin(&mut db).await?;
        let Some(view) = load_view(write.exec(), thread).await? else {
            return write.commit().await;
        };
        let mutation = Mutation::Remind {
            at: reminder.at,
            keep_on_reply: reminder.keep_on_reply,
            snooze: false,
        };
        let Some((forward, _)) = mutation.plan(&view, &router.me)? else {
            return write.commit().await;
        };
        restore(
            write.exec(),
            &view.id,
            Some(reminder_row(
                &view,
                reminder.at,
                reminder.keep_on_reply,
                set_on,
                Timestamp::now(),
            )),
        )
        .await?;
        enqueue(write.exec(), &view, &forward, &mutation, false, &router.me).await?;
        reindex(write.exec(), &view.id, &router).await?;
        write.publish().await
    }
}

/// The held sends whose reminder is on their thread, and which.
pub(crate) async fn given(exec: &mut dyn Executor) -> Result<HashMap<String, DraftReminder>> {
    Ok(db::kv_get(exec, GIVEN_REMINDERS).await?.unwrap_or_default())
}

/// The reminders that haven't brought their thread back or been cancelled by a reply.
async fn waiting(exec: &mut dyn Executor) -> Result<Vec<ReminderRow>> {
    let mut rows = ReminderRow::all().exec(exec).await?;
    rows.retain(|row| row.fired_at.is_none() && !row.aborted);
    Ok(rows)
}
