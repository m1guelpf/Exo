//! A thread's effective state, Gmail's copy with the outbox's pending steps applied, and
//! everything derived from it: list membership and per-list sort, importance, mute,
//! participants.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::db::{AttachmentRow, BodyRow, Mailbox, MessageRow, OutboxRow, ReminderRow};
use crate::drafts::SendFailure;
use crate::ids::labels::*;
use crate::ids::{ListId, ThreadId};
use crate::model::{
    Attachment, Attachments, Body, Contact, MessageDetail, ReminderStatus, ThreadDetail,
    ThreadSummary,
};
use crate::mutation::{Mutation, Step, resolve};
use crate::parse::unescape_html;
use crate::split::{RoutedSplit, Splits};

/// Who the user is, as far as routing cares.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Identity {
    /// The primary address and the send-as aliases Gmail treats as the account.
    pub addresses: Vec<String>,
    /// The account's display name (for the Important backfill query).
    pub name: Option<String>,
    /// The id of `[Exo]/Muted`.
    pub muted_label: Option<String>,
    /// The id of `[Exo]/Is Snoozed`.
    pub snoozed_label: Option<String>,
    /// The id of `[Exo]/ru`.
    pub reminder_unread_label: Option<String>,
}

impl Identity {
    pub fn is_me(&self, email: &str) -> bool {
        self.addresses
            .iter()
            .any(|address| address.eq_ignore_ascii_case(email))
    }

    fn contact(&self, mailbox: &Mailbox) -> Contact {
        Contact {
            name: mailbox.name.clone(),
            email: mailbox.email.clone(),
            is_me: self.is_me(&mailbox.email),
        }
    }
}

/// Everything routing reads: who the user is, and the account's splits.
#[derive(Debug, Clone, Default)]
pub(crate) struct Router {
    pub me: Identity,
    pub splits: Arc<Splits>,
}

impl Router {
    /// The thread's lists: [`ThreadView::list_ids`], then every enabled split an inbox thread
    /// matches. A split that doesn't leave its threads in Important and Other takes them out of
    /// both. `attachments` holds at least the thread's attachment rows.
    pub fn list_ids<'a>(
        &'a self,
        view: &'a ThreadView,
        attachments: &[AttachmentRow],
    ) -> Vec<&'a str> {
        let mut ids = view.list_ids(&self.me);
        if ids.contains(&INBOX) {
            let matched: Vec<&RoutedSplit> = self
                .splits
                .routed
                .iter()
                .filter(|split| split.is_match(view, attachments, &self.me.addresses))
                .collect();
            if matched
                .iter()
                .any(|split| !split.leave_threads_in_important_other)
            {
                ids.retain(|id| *id != SH_IMPORTANT && *id != SH_OTHER);
            }
            ids.extend(matched.iter().map(|split| split.id.as_str()));
        }
        ids
    }
}

pub(crate) struct ThreadView {
    pub id: String,
    /// The thread's messages, oldest first: every one but drafts Exo didn't write. Routing,
    /// rows and actions all work on these.
    pub messages: Vec<ViewMessage>,
    /// Drafts written outside Exo (Gmail web, …), oldest first. They only put the thread in
    /// Drafts and only show there.
    pub foreign_drafts: Vec<ViewMessage>,
    /// A send of the thread's is held or waiting to go (its message is left out of `messages`,
    /// unless it's scheduled).
    pub sending: bool,
    /// That send's text, for its row in Scheduled while it waits for the network.
    pub sending_snippet: Option<String>,
    /// The thread has a scheduled send: the earliest one's time, and whether Gmail has the
    /// latest version of every one.
    pub scheduled: Option<(Timestamp, bool)>,
    /// Its reminder, if one was ever set and not removed.
    pub reminder: Option<ReminderRow>,
}

/// How long a failed draft keeps its thread in the inbox, by the draft's date: three weeks.
const RECENT_FAILURE_MS: i64 = 21 * 24 * 60 * 60 * 1000;

pub(crate) struct ViewMessage {
    pub row: MessageRow,
    /// Labels after pending steps.
    pub labels: BTreeSet<String>,
    /// A failed draft's: why and when its send failed.
    pub failed: Option<(SendFailure, Timestamp)>,
    /// A scheduled send's: when it goes.
    pub scheduled: Option<Timestamp>,
}

impl ViewMessage {
    pub fn has(&self, label: &str) -> bool {
        self.labels.contains(label)
    }

    pub fn is_draft(&self) -> bool {
        self.has(DRAFT)
    }

    /// A draft being written: not one whose send failed or is scheduled.
    fn is_editable_draft(&self) -> bool {
        self.is_draft() && self.failed.is_none() && self.scheduled.is_none()
    }

    /// A failed draft dated in the last three weeks.
    fn is_recent_failure(&self) -> bool {
        self.failed.is_some() && Timestamp::now().as_millisecond() - self.date() < RECENT_FAILURE_MS
    }

    fn date(&self) -> i64 {
        self.row.internal_date
    }

    fn recipients(&self) -> impl Iterator<Item = &Mailbox> {
        self.row
            .to
            .iter()
            .chain(self.row.cc.iter())
            .chain(self.row.bcc.iter())
    }

    fn is_from_me(&self, me: &Identity) -> bool {
        self.row
            .sender
            .0
            .as_ref()
            .is_some_and(|from| me.is_me(&from.email))
    }

    fn is_note_to_self(&self, me: &Identity) -> bool {
        self.is_from_me(me) && self.recipients().any(|to| me.is_me(&to.email))
    }

    /// Whether the message is important: Gmail's Primary category, or a note to self or Google
    /// Docs invitation that isn't Social. Gmail's `IMPORTANT` label has no say.
    fn is_important(&self, me: &Identity) -> bool {
        self.has(CATEGORY_PERSONAL)
            || ((self.is_note_to_self(me) || self.row.sender_raw.contains("via Google"))
                && !self.has(CATEGORY_SOCIAL))
    }
}

impl ThreadView {
    /// Gmail's copy of a thread with `ops` (queue order) applied on top. `exo_drafts` are the
    /// draft messages Exo wrote; any other draft is foreign.
    pub fn new(
        id: String,
        mut rows: Vec<MessageRow>,
        ops: &[OutboxRow],
        exo_drafts: &BTreeSet<String>,
    ) -> Self {
        rows.sort_by(|a, b| (a.internal_date, &a.id).cmp(&(b.internal_date, &b.id)));
        let (foreign_drafts, messages) = rows
            .into_iter()
            .map(|row| ViewMessage {
                labels: row.label_ids.iter().cloned().collect(),
                row,
                failed: None,
                scheduled: None,
            })
            .partition(|message| message.is_draft() && !exo_drafts.contains(&message.row.id));
        let mut view = Self {
            id,
            messages,
            foreign_drafts,
            sending: false,
            sending_snippet: None,
            scheduled: None,
            reminder: None,
        };
        for step in ops.iter().flat_map(|op| op.steps.iter()) {
            view.apply(step);
        }
        view
    }

    /// Every stored message, foreign drafts included.
    pub fn all_messages(&self) -> impl Iterator<Item = &ViewMessage> {
        self.messages.iter().chain(&self.foreign_drafts)
    }

    /// A step's effect on labels, as Gmail applies it.
    pub fn apply(&mut self, step: &Step) {
        let messages = self.messages.iter_mut().chain(&mut self.foreign_drafts);
        match step {
            Step::RemoveFromMessages { ids, labels } => {
                for message in messages.filter(|m| ids.contains(&m.row.id)) {
                    for label in labels {
                        message.labels.remove(label);
                    }
                }
            }
            Step::ModifyThread { add, remove } => {
                for message in messages {
                    message.labels.extend(add.iter().cloned());
                    for label in remove.iter().filter(|label| !add.contains(label)) {
                        message.labels.remove(label);
                    }
                }
            }
            Step::AddToMessage { id, label } => {
                if let Some(message) = messages.into_iter().find(|m| &m.row.id == id) {
                    message.labels.insert(label.clone());
                }
            }
            Step::Refresh => {}
        }
    }

    /// Every label on the thread's messages.
    pub fn labels(&self) -> BTreeSet<&str> {
        self.messages
            .iter()
            .flat_map(|message| message.labels.iter().map(String::as_str))
            .collect()
    }

    pub fn has(&self, label: &str) -> bool {
        self.messages.iter().any(|message| message.has(label))
    }

    /// Whether the thread is muted: some message carries the mute label, and no newer message
    /// added one of your addresses as a new recipient.
    pub fn is_muted(&self, me: &Identity) -> bool {
        let Some(muted) = me.muted_label.as_deref() else {
            return false;
        };
        let newest_first: Vec<&ViewMessage> = self
            .messages
            .iter()
            .rev()
            .filter(|m| !m.is_draft())
            .collect();
        let Some(anchor) = newest_first.iter().find(|m| m.has(muted)) else {
            return false;
        };
        !newest_first.iter().any(|message| {
            message.date() > anchor.date()
                && message
                    .row
                    .to
                    .iter()
                    .chain(message.row.cc.iter())
                    .filter(|to| me.is_me(&to.email))
                    .any(|to| {
                        !anchor
                            .recipients()
                            .any(|known| known.email.eq_ignore_ascii_case(&to.email))
                    })
        })
    }

    /// Where the thread's reminder stands. A reminder that brought the thread back stops
    /// showing once a newer message comes in. One that came due while Exo wasn't running fires
    /// at the next launch, however overdue.
    pub fn reminder_status(&self, me: &Identity) -> Option<ReminderStatus> {
        let reminder = self.reminder.as_ref()?;
        if let Some(fired) = reminder.fired_at {
            let returned = self.latest_for(SH_IMPORTANT, me) <= fired.as_millisecond();
            let unread = me
                .reminder_unread_label
                .as_deref()
                .is_some_and(|label| self.has(label));
            return returned.then_some(ReminderStatus::Returned { at: fired, unread });
        }
        if let Some(reply) = self.aborting_reply(reminder, me) {
            // A `From` that didn't parse is named as written.
            let by = reply.row.sender.0.as_ref().map_or_else(
                || Contact {
                    name: None,
                    email: reply.row.sender_raw.clone(),
                    is_me: false,
                },
                |from| me.contact(from),
            );
            return Some(ReminderStatus::Aborted { by });
        }
        Some(ReminderStatus::Set {
            at: reminder.at,
            keep_on_reply: reminder.keep_on_reply,
        })
    }

    /// The reply that cancelled an "if no reply" reminder: the latest message someone else sent
    /// isn't one the reminder was set on, and the first of theirs that isn't is who replied.
    fn aborting_reply(&self, reminder: &ReminderRow, me: &Identity) -> Option<&ViewMessage> {
        if reminder.keep_on_reply {
            return None;
        }
        let mut received = self
            .messages
            .iter()
            .filter(|m| !m.is_draft() && !m.is_from_me(me));
        let new = |m: &&ViewMessage| !reminder.message_ids.contains(&m.row.id);
        received.clone().next_back().filter(new)?;
        received.find(new)
    }

    /// The reminder is set and due to bring the thread back.
    pub fn has_reminder(&self, me: &Identity) -> bool {
        matches!(self.reminder_status(me), Some(ReminderStatus::Set { .. }))
    }

    /// The thread's lists without splits ([`Router::list_ids`] adds them), over `messages`.
    /// Drafts written elsewhere only add Drafts: a thread that is nothing else is in no other
    /// list.
    pub fn list_ids(&self, me: &Identity) -> Vec<&str> {
        if self.messages.is_empty() {
            return if self.foreign_drafts.is_empty() {
                Vec::new()
            } else {
                vec![DRAFT]
            };
        }
        let mut ids: Vec<&str> = self.labels().into_iter().collect();
        let push = |ids: &mut Vec<&str>, id: &'static str| {
            if !ids.contains(&id) {
                ids.push(id);
            }
        };
        if ids.contains(&SPAM) && self.messages.iter().all(|m| m.has(SPAM)) {
            ids = if self.has(UNREAD) {
                vec![UNREAD, SPAM]
            } else {
                vec![SPAM]
            };
        } else if self.is_muted(me) {
            ids.retain(|id| *id != INBOX);
            for id in [SH_ARCHIVED, SH_ALL, SH_MUTED] {
                push(&mut ids, id);
            }
        } else {
            // A thread that is only a draft (a new message being written) sits in Important, but
            // not one only scheduled.
            let draft_only = ids.contains(&DRAFT)
                && (self.messages.iter().all(ViewMessage::is_draft) || ids == [DRAFT]);
            let sending = self.messages.first().is_some_and(|m| m.scheduled.is_some());
            // So is one with a recent failed draft.
            if (draft_only && !sending) || self.messages.iter().any(ViewMessage::is_recent_failure)
            {
                push(&mut ids, SH_IMPORTANT);
                push(&mut ids, INBOX);
            }
            push(&mut ids, SH_ALL);
            if ids.contains(&INBOX) && !ids.contains(&SH_IMPORTANT) && !ids.contains(&SH_OTHER) {
                let important = self
                    .messages
                    .iter()
                    .any(|m| m.has(INBOX) && m.is_important(me));
                push(&mut ids, if important { SH_IMPORTANT } else { SH_OTHER });
            }
            if !ids.contains(&INBOX) && !draft_only {
                push(&mut ids, SH_ARCHIVED);
            }
            match self.reminder_status(me) {
                Some(ReminderStatus::Set { .. }) => push(&mut ids, SH_SNOOZED),
                Some(ReminderStatus::Returned { unread: true, .. }) => push(&mut ids, UNREAD),
                _ => {}
            }
            // Awaiting a reply: the last message, drafts aside, is the user's.
            if self
                .messages
                .iter()
                .rev()
                .find(|m| !m.is_draft())
                .is_some_and(|m| m.is_from_me(me))
            {
                push(&mut ids, SH_NO_REPLY);
            }
            if self.scheduled.is_some() {
                push(&mut ids, SH_SCHEDULED);
            }
        }
        if !self.foreign_drafts.is_empty() {
            push(&mut ids, DRAFT);
        }
        ids
    }

    /// Whether the thread is done: not routed to the inbox. A thread that is only a draft written
    /// elsewhere isn't anywhere but Drafts, so it isn't done either.
    pub fn is_done(&self, me: &Identity) -> bool {
        !self.messages.is_empty() && !self.list_ids(me).contains(&INBOX)
    }

    /// The thread's sort key in `list`: the latest message relevant to it, where your own sent
    /// mail doesn't count unless it's to yourself. A reminder that brought the thread back counts
    /// as a message then. A failed draft doubles it (and counts when it failed), which keeps the
    /// thread on top. Reminders sort by when they're due, the soonest first.
    pub fn sort_for(&self, list: &str, me: &Identity) -> i64 {
        // Soonest first.
        if let (SH_SCHEDULED, Some((at, _))) = (list, self.scheduled) {
            return -at.as_millisecond();
        }
        if list == SH_SNOOZED
            && let Some(reminder) = &self.reminder
        {
            return -reminder.at.as_millisecond();
        }
        let mut sort = self.latest_for(list, me);
        if let Some(fired) = self
            .reminder
            .as_ref()
            .and_then(|reminder| reminder.fired_at)
            && ![SENT, DRAFT, SPAM, TRASH].contains(&list)
        {
            sort = sort.max(fired.as_millisecond());
        }
        let failed = self
            .messages
            .iter()
            .filter_map(|m| m.failed.map(|(_, at)| at.as_millisecond()))
            .max();
        failed.map_or(sort, |failed| 2 * sort.max(failed))
    }

    fn latest_for(&self, list: &str, me: &Identity) -> i64 {
        let latest = |messages: &[&ViewMessage]| messages.iter().map(|m| m.date()).max();
        // A scheduled send is a read-only draft, which counts, dated when it was last written.
        let non_draft = || {
            self.messages
                .iter()
                .filter(|m| !m.is_draft() || m.scheduled.is_some())
        };
        if list == SENT {
            let sent: Vec<_> = non_draft().filter(|m| m.has(SENT)).collect();
            if let Some(date) = latest(&sent) {
                return date;
            }
        }
        let candidates: Vec<&ViewMessage> = match list {
            SPAM | TRASH => non_draft().filter(|m| m.has(list)).collect(),
            DRAFT => self.all_messages().collect(),
            _ => non_draft()
                .filter(|m| !m.has(SPAM) && !m.has(TRASH))
                .collect(),
        };
        if candidates.is_empty() {
            return self
                .all_messages()
                .map(|m| m.date())
                .max()
                .unwrap_or_default();
        }
        let received: Vec<&ViewMessage> = candidates
            .iter()
            .copied()
            .filter(|m| !m.has(SENT) || m.is_note_to_self(me))
            .collect();
        latest(if received.is_empty() {
            &candidates
        } else {
            &received
        })
        .unwrap_or_default()
    }

    /// Everyone but you who sent or received the messages a contact's Gmail search finds
    /// (`-in:DRAFT`, outside Spam and Trash), lower-cased, and the latest such message's date:
    /// what a contact's latest conversations are read from.
    pub fn participants(&self, me: &Identity) -> (BTreeSet<String>, i64) {
        let found: Vec<&ViewMessage> = self
            .messages
            .iter()
            .filter(|m| !m.is_draft() && !m.has(SPAM) && !m.has(TRASH))
            .collect();
        let emails = found
            .iter()
            .flat_map(|m| m.row.sender.0.iter().chain(m.recipients()))
            .filter(|mailbox| !me.is_me(&mailbox.email))
            .map(|mailbox| mailbox.email.to_lowercase())
            .collect();
        let date = found.iter().map(|m| m.date()).max().unwrap_or_default();
        (emails, date)
    }

    /// The thread's subject: its first received message's, or its first draft's.
    pub fn subject(&self) -> String {
        subject(&self.messages.iter().collect::<Vec<_>>())
    }

    /// The latest message's date, which is roughly where Gmail lists the thread.
    pub fn gmail_sort(&self) -> i64 {
        self.messages
            .iter()
            .filter(|m| !m.is_draft())
            .map(|m| m.date())
            .max()
            .or_else(|| self.messages.iter().map(|m| m.date()).max())
            .unwrap_or_default()
    }

    /// Where Gmail lists the thread: its latest message, drafts written elsewhere included.
    /// Backfill measures how far back a pass has listed with it: a thread that's only such a
    /// draft has no [`gmail_sort`](Self::gmail_sort) (0), which ended a 31-day pass at the first
    /// page that listed one.
    pub fn listed_at(&self) -> i64 {
        self.all_messages()
            .map(ViewMessage::date)
            .max()
            .unwrap_or_default()
    }

    /// Ids of the messages carrying any of `labels`.
    pub fn carriers(&self, labels: &[String]) -> Vec<String> {
        self.messages
            .iter()
            .filter(|m| labels.iter().any(|label| m.has(label)))
            .map(|m| m.row.id.clone())
            .collect()
    }

    /// The message a star goes on: the latest message not from you, else the latest.
    pub fn message_to_star(&self, me: &Identity) -> Option<String> {
        let mut received = self.messages.iter().rev().filter(|m| !m.is_draft());
        let latest = received.clone().next();
        received
            .find(|m| !m.is_from_me(me))
            .or(latest)
            .map(|m| m.row.id.clone())
    }

    /// The message Mark Unread marks: the latest message; a draft-only thread's first.
    pub fn message_to_mark_unread(&self) -> Option<String> {
        let message = if self.messages.iter().all(ViewMessage::is_draft) {
            self.messages.first()
        } else {
            self.messages.iter().rev().find(|m| !m.is_draft())
        };
        message.map(|m| m.row.id.clone())
    }

    /// The messages a row in `list` is built from: [`ThreadView::messages`], and in Drafts, the
    /// drafts written elsewhere too, oldest first.
    fn shown(&self, list: &ListId) -> Vec<&ViewMessage> {
        let mut shown: Vec<&ViewMessage> = self.messages.iter().collect();
        if *list == ListId::Drafts && !self.foreign_drafts.is_empty() {
            shown.extend(&self.foreign_drafts);
            shown.sort_by(|a, b| (a.date(), &a.row.id).cmp(&(b.date(), &b.row.id)));
        }
        shown
    }

    /// The thread as a row of `list`, whose sort key in that list is `sort`.
    /// `attachments` holds at least this thread's attachment rows. (Mutable only to try
    /// mutations on: the view is left as it was.)
    pub fn summary(
        &mut self,
        list: &ListId,
        sort: i64,
        router: &Router,
        attachments: &[AttachmentRow],
    ) -> ThreadSummary {
        let removed_by = self.removed_by(list, router, attachments);
        let me = &router.me;
        let shown = self.shown(list);
        // Spam and trashed messages give no snippet but in Trash, so Spam's rows have none.
        // A send waiting for the network is its row's in Scheduled, which lists it for that,
        // with its own text.
        let waiting = self
            .sending_snippet
            .as_deref()
            .filter(|_| *list == ListId::Scheduled);
        let snippet = match waiting {
            Some(text) => strip_placeholder_braces(text),
            None => shown
                .iter()
                .rev()
                .find(|m| *list == ListId::Trash || !(m.has("SPAM") || m.has("TRASH")))
                .map_or_else(String::new, |m| {
                    let snippet = unescape_html(&m.row.snippet);
                    if m.is_draft() {
                        strip_placeholder_braces(&snippet)
                    } else {
                        snippet
                    }
                }),
        };
        let any = |label: &str| shown.iter().any(|m| m.has(label));
        let reminder = self.reminder_status(me);
        let labels: BTreeSet<&str> = shown
            .iter()
            .flat_map(|m| m.labels.iter().map(String::as_str))
            .collect();
        ThreadSummary {
            id: ThreadId::new(self.id.clone()),
            failure: self.failure(),
            sending: self.sending,
            scheduled: self.scheduled,
            subject: subject(&shown),
            snippet,
            participants: participants(&shown, list, me),
            date: if matches!(list, ListId::Scheduled | ListId::Reminders) {
                timestamp(sort.saturating_neg())
            } else {
                timestamp(sort)
            },
            unread: any(UNREAD)
                || matches!(
                    reminder,
                    Some(ReminderStatus::Returned { unread: true, .. })
                ),
            starred: any(STARRED),
            muted: self.is_muted(me),
            done: self.is_done(me),
            labels: labels.into_iter().map(str::to_owned).collect(),
            attachments: attachments_of(&shown, attachments),
            files: attached_files(&shown, attachments),
            contact: self.pane_contact(me),
            messages: shown.len(),
            drafts: shown.iter().filter(|m| m.is_editable_draft()).count(),
            removed_by,
            reminder,
        }
    }

    /// Why the thread's failed drafts failed: their reason when they share one, else
    /// [`SendFailure::Other`]; `None` without a failed draft.
    fn failure(&self) -> Option<SendFailure> {
        let mut reasons = self
            .messages
            .iter()
            .filter_map(|m| m.failed.map(|(reason, _)| reason));
        let first = reasons.next()?;
        Some(if reasons.all(|reason| reason == first) {
            first
        } else {
            SendFailure::Other
        })
    }

    /// The triage mutations that would take the thread out of `list`.
    fn removed_by(
        &mut self,
        list: &ListId,
        router: &Router,
        attachments: &[AttachmentRow],
    ) -> Vec<Mutation> {
        let members = list.members();
        let extra = match list {
            ListId::Label(label) => Some(Mutation::RemoveLabel(label.clone())),
            _ => None,
        };
        self.removed_by_with(router, attachments, extra, |lists, _| {
            !lists.iter().any(|id| members.contains(id))
        })
    }

    /// The triage mutations (and `extra`) after which `leaves` holds of the thread's list ids
    /// and labels.
    pub fn removed_by_with(
        &mut self,
        router: &Router,
        attachments: &[AttachmentRow],
        extra: Option<Mutation>,
        leaves: impl Fn(&[&str], &BTreeSet<&str>) -> bool,
    ) -> Vec<Mutation> {
        use Mutation::*;
        let mut mutations = vec![
            MarkDone,
            MarkNotDone,
            Star,
            Unstar,
            MarkRead,
            MarkUnread,
            Trash,
            Untrash,
            Spam,
            NotSpam,
            Mute,
            Unmute,
        ];
        mutations.extend(extra);
        mutations.retain(|mutation| self.would_leave(mutation, router, attachments, &leaves));
        mutations
    }

    /// Whether applying `mutation` the way the outbox does makes `leaves` hold. The steps are
    /// applied to the view and then undone.
    fn would_leave(
        &mut self,
        mutation: &Mutation,
        router: &Router,
        attachments: &[AttachmentRow],
        leaves: &impl Fn(&[&str], &BTreeSet<&str>) -> bool,
    ) -> bool {
        let me = &router.me;
        let Ok(Some((forward, _))) = mutation.plan(self, me) else {
            return false;
        };
        let labels: Vec<BTreeSet<String>> = self.all_messages().map(|m| m.labels.clone()).collect();
        for step in resolve(&forward.intents, self, me) {
            self.apply(&step);
        }
        let left = leaves(&router.list_ids(self, attachments), &self.labels());
        let messages = self.messages.iter_mut().chain(&mut self.foreign_drafts);
        for (message, labels) in messages.zip(labels) {
            message.labels = labels;
        }
        left
    }

    /// Whom the contact pane shows: unread messages first, then the rest newest first; the first
    /// sender who isn't you.
    fn pane_contact(&self, me: &Identity) -> Option<Contact> {
        let received: Vec<&ViewMessage> = self.messages.iter().filter(|m| !m.is_draft()).collect();
        let (unread, read): (Vec<&ViewMessage>, Vec<&ViewMessage>) =
            received.into_iter().partition(|m| m.has(UNREAD));
        unread
            .into_iter()
            .chain(read.into_iter().rev())
            .filter_map(|m| m.row.sender.0.as_ref())
            .find(|sender| !me.is_me(&sender.email))
            .map(|sender| me.contact(sender))
    }

    /// The whole thread for the thread view.
    pub fn detail(
        self,
        attachments: Vec<AttachmentRow>,
        bodies: Vec<BodyRow>,
        me: &Identity,
    ) -> ThreadDetail {
        let muted = self.is_muted(me);
        let reminder = self.reminder_status(me);
        // Everything stored, drafts written elsewhere included.
        let mut all: Vec<ViewMessage> = self.messages;
        all.extend(self.foreign_drafts);
        all.sort_by(|a, b| (a.date(), &a.row.id).cmp(&(b.date(), &b.row.id)));
        let subject = subject(&all.iter().collect::<Vec<_>>());
        let labels = all
            .iter()
            .flat_map(|m| m.labels.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut bodies: HashMap<String, BodyRow> = bodies
            .into_iter()
            .map(|body| (body.message_id.clone(), body))
            .collect();
        let mut attachments_by_message: HashMap<String, Vec<Attachment>> = HashMap::new();
        for row in attachments {
            attachments_by_message
                .entry(row.message_id.clone())
                .or_default()
                .push(Attachment {
                    message_id: row.message_id,
                    attachment_id: row.attachment_id,
                    part_id: row.part_id,
                    filename: row.filename,
                    mime_type: row.mime_type,
                    size: row.size,
                    content_id: row.content_id,
                    inline: row.inline,
                });
        }
        let contacts = |list: &[Mailbox]| list.iter().map(|m| me.contact(m)).collect();
        let messages = all
            .into_iter()
            .map(|message| {
                let row = message.row;
                let body = bodies.remove(&row.id);
                let mut attachments = attachments_by_message.remove(&row.id).unwrap_or_default();
                attachments.sort_by(|a, b| a.part_id.cmp(&b.part_id));
                MessageDetail {
                    failure: message.failed.map(|(reason, _)| reason),
                    scheduled: message.scheduled,
                    sending: false,
                    labels: message.labels.into_iter().collect(),
                    date: timestamp(row.internal_date),
                    from: row.sender.0.as_ref().map(|from| me.contact(from)),
                    to: contacts(&row.to),
                    cc: contacts(&row.cc),
                    bcc: contacts(&row.bcc),
                    reply_to: contacts(&row.reply_to),
                    snippet: unescape_html(&row.snippet),
                    body: Body {
                        html: body.as_ref().and_then(|body| body.html.clone()),
                        text: body.and_then(|body| body.text),
                    },
                    attachments,
                    headers: row.headers.0,
                    id: row.id,
                    subject: row.subject,
                    rfc822_id: row.rfc822_id,
                    in_reply_to: row.in_reply_to,
                    references: row.references,
                    list_id: row.list_id,
                    list_unsubscribe: row.list_unsubscribe,
                    list_unsubscribe_post: row.list_unsubscribe_post,
                }
            })
            .collect();
        ThreadDetail {
            id: ThreadId::new(self.id),
            subject,
            labels,
            muted,
            reminder,
            messages,
        }
    }
}

/// The From column for `messages` (oldest first).
fn participants(messages: &[&ViewMessage], list: &ListId, me: &Identity) -> Vec<Contact> {
    // A failed draft is no draft here: it's yours among the senders.
    let entries: Vec<&Mailbox> =
        if let Some(draft) = messages.iter().rev().find(|m| m.is_editable_draft()) {
            draft.recipients().collect()
        } else if *list == ListId::Sent {
            messages
                .iter()
                .rev()
                .filter(|m| m.has(SENT))
                .flat_map(|m| m.recipients())
                .collect()
        } else {
            messages
                .iter()
                .filter_map(|m| m.row.sender.0.as_ref())
                .collect()
        };
    // More than two: keep the first, then each person's last appearance.
    let entries: Vec<&Mailbox> = if entries.len() > 2 {
        let rest = &entries[1..];
        std::iter::once(entries[0])
            .chain(rest.iter().enumerate().filter_map(|(at, entry)| {
                let later = rest[at + 1..]
                    .iter()
                    .any(|other| other.email.eq_ignore_ascii_case(&entry.email));
                (!later).then_some(*entry)
            }))
            .collect()
    } else {
        entries
    };
    let mut contacts: Vec<Contact> = Vec::new();
    for entry in entries {
        if !contacts
            .iter()
            .any(|c| c.email.eq_ignore_ascii_case(&entry.email))
        {
            contacts.push(me.contact(entry));
        }
    }
    if contacts.iter().all(|c| c.is_me) {
        contacts.truncate(1);
    } else {
        contacts.retain(|c| !c.is_me);
    }
    contacts
}

/// The first received message's subject, or the first draft's.
fn subject(messages: &[&ViewMessage]) -> String {
    messages
        .iter()
        .find(|m| !m.is_draft())
        .or(messages.first())
        .map(|m| m.row.subject.clone())
        .unwrap_or_default()
}

/// Whether attachment `row` is a calendar invite.
fn is_invite(row: &AttachmentRow) -> bool {
    let mime = row.mime_type.to_ascii_lowercase();
    mime == "text/calendar"
        || mime == "application/ics"
        || row.filename.to_ascii_lowercase().ends_with(".ics")
}

/// `messages`' attached files, oldest message first, a later file replacing an earlier one of
/// the same name where it was. `rows` holds at least their attachment rows.
fn attached_files(messages: &[&ViewMessage], rows: &[AttachmentRow]) -> Vec<Attachment> {
    let mut files: Vec<Attachment> = Vec::new();
    for message in messages {
        for row in rows
            .iter()
            .filter(|row| row.message_id == message.row.id && !row.inline && !is_invite(row))
        {
            let file = Attachment {
                message_id: row.message_id.clone(),
                attachment_id: row.attachment_id.clone(),
                part_id: row.part_id.clone(),
                filename: row.filename.clone(),
                mime_type: row.mime_type.clone(),
                size: row.size,
                content_id: row.content_id.clone(),
                inline: row.inline,
            };
            match files
                .iter_mut()
                .find(|known| known.filename == file.filename)
            {
                Some(known) => *known = file,
                None => files.push(file),
            }
        }
    }
    files
}

/// What a row's attachment icon shows for `messages`; files win over an invite.
/// `rows` holds at least their attachment rows.
fn attachments_of(messages: &[&ViewMessage], rows: &[AttachmentRow]) -> Attachments {
    let mine = |row: &&AttachmentRow| messages.iter().any(|m| m.row.id == row.message_id);
    if rows
        .iter()
        .filter(mine)
        .any(|row| !row.inline && !is_invite(row))
    {
        Attachments::Files
    } else if messages.iter().any(|m| m.row.has_attachments) {
        // `has_attachments` is set for files and invites alike, and there are no files.
        Attachments::CalendarInvite
    } else {
        Attachments::None
    }
}

fn timestamp(millis: i64) -> Timestamp {
    Timestamp::from_millisecond(millis).unwrap_or(Timestamp::UNIX_EPOCH)
}

/// A draft's snippet with its placeholders' braces dropped: `{FIRST_NAME}` reads `FIRST_NAME`.
fn strip_placeholder_braces(snippet: &str) -> String {
    let mut out = String::with_capacity(snippet.len());
    let mut rest = snippet;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let name_len = after
            .char_indices()
            .find(|&(_, c)| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
            .map_or(after.len(), |(at, _)| at);
        let name = &after[..name_len];
        if name.starts_with(|c: char| c.is_ascii_uppercase()) && after[name_len..].starts_with('}')
        {
            out.push_str(name);
            rest = &after[name_len + 1..];
        } else {
            out.push('{');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use toasty::Json;

    use super::*;

    pub const ME: &str = "maya@brightmoor.co";
    pub const MUTED: &str = "Label_99";

    pub fn me() -> Identity {
        Identity {
            addresses: vec![ME.into()],
            name: Some("Maya Okafor".into()),
            muted_label: Some(MUTED.into()),
            ..Identity::default()
        }
    }

    pub fn router() -> Router {
        Router {
            me: me(),
            splits: Arc::default(),
        }
    }

    fn mailbox(email: &str) -> Mailbox {
        Mailbox {
            name: None,
            email: email.into(),
        }
    }

    /// A message `id` at `date` from `from` to `to`, carrying `labels`.
    pub fn message(id: &str, date: i64, from: &str, to: &[&str], labels: &[&str]) -> MessageRow {
        MessageRow {
            id: id.into(),
            thread_id: "t".into(),
            history_id: 1,
            internal_date: date,
            label_ids: labels.iter().map(|label| (*label).to_owned()).collect(),
            subject: format!("Subject {id}"),
            snippet: String::new(),
            sender: Json(Some(mailbox(from))),
            sender_raw: from.into(),
            to: Json(to.iter().map(|to| mailbox(to)).collect()),
            cc: Json(Vec::new()),
            bcc: Json(Vec::new()),
            reply_to: Json(Vec::new()),
            rfc822_id: None,
            in_reply_to: None,
            references: Vec::new(),
            list_id: None,
            list_unsubscribe: None,
            list_unsubscribe_post: None,
            headers: Json(Vec::new()),
            size: 0,
            has_attachments: false,
        }
    }

    pub fn thread(messages: Vec<MessageRow>) -> ThreadView {
        ThreadView::new("t".into(), messages, &[], &BTreeSet::new())
    }

    fn lists(view: &ThreadView) -> Vec<String> {
        let mut ids: Vec<String> = view
            .list_ids(&me())
            .into_iter()
            .map(str::to_owned)
            .collect();
        ids.sort();
        ids
    }

    #[test]
    fn a_draft_written_elsewhere_is_only_in_drafts() {
        let rows = || vec![message("d", 1, ME, &["ann@x.com"], &["DRAFT"])];
        let theirs = ThreadView::new("t".into(), rows(), &[], &BTreeSet::new());
        assert_eq!(theirs.list_ids(&me()), [DRAFT]);
        assert!(!theirs.is_done(&me()), "nor in Done");
        // One Exo wrote is a new message being written, in Important.
        let ours = ThreadView::new("t".into(), rows(), &[], &BTreeSet::from(["d".to_owned()]));
        assert_eq!(lists(&ours), ["DRAFT", "INBOX", "SH_ALL", "SH_IMPORTANT"]);
    }

    #[test]
    fn a_reply_drafted_elsewhere_leaves_the_row_alone_outside_drafts() {
        let rows = || {
            let mut received = message("a", 1, "ann@x.com", &[ME], &["INBOX"]);
            received.snippet = "Lunch?".into();
            let mut draft = message("d", 2, ME, &["ann@x.com", "bob@x.com"], &["DRAFT"]);
            draft.snippet = "Sure, {DAY} works".into();
            vec![received, draft]
        };
        let emails = |summary: &ThreadSummary| {
            summary
                .participants
                .iter()
                .map(|c| c.email.clone())
                .collect::<Vec<_>>()
        };
        let mut theirs = ThreadView::new("t".into(), rows(), &[], &BTreeSet::new());
        assert!(
            lists(&theirs).contains(&DRAFT.to_owned())
                && lists(&theirs).contains(&INBOX.to_owned())
        );
        assert_eq!(theirs.sort_for(INBOX, &me()), 1);
        let row = theirs.summary(&ListId::Inbox, 1, &router(), &[]);
        assert_eq!(
            (row.drafts, row.messages, row.snippet.as_str()),
            (0, 1, "Lunch?")
        );
        assert_eq!(emails(&row), ["ann@x.com"]);
        // Drafts shows the draft.
        let row = theirs.summary(&ListId::Drafts, 2, &router(), &[]);
        assert_eq!((row.drafts, row.snippet.as_str()), (1, "Sure, DAY works"));
        assert_eq!(emails(&row), ["ann@x.com", "bob@x.com"]);
        // Exo's own draft shows everywhere.
        let mut ours = ThreadView::new("t".into(), rows(), &[], &BTreeSet::from(["d".to_owned()]));
        let row = ours.summary(&ListId::Inbox, 1, &router(), &[]);
        assert_eq!((row.drafts, row.snippet.as_str()), (1, "Sure, DAY works"));
    }

    #[test]
    fn draft_placeholders_lose_their_braces() {
        assert_eq!(
            strip_placeholder_braces("Hi {FIRST_NAME}, {lower} {X1}{ {"),
            "Hi FIRST_NAME, {lower} X1{ {"
        );
    }

    #[test]
    fn the_pane_shows_the_first_unread_sender_else_the_latest() {
        let read = thread(vec![
            message("a", 1, "ann@x.com", &[ME], &["INBOX"]),
            message("b", 2, "bob@x.com", &[ME], &["INBOX"]),
            message("c", 3, ME, &["bob@x.com"], &["SENT"]),
        ]);
        let contact = |view: &ThreadView| view.pane_contact(&me()).map(|c| c.email);
        assert_eq!(contact(&read).as_deref(), Some("bob@x.com"));
        let unread = thread(vec![
            message("a", 1, "ann@x.com", &[ME], &["INBOX", "UNREAD"]),
            message("b", 2, "bob@x.com", &[ME], &["INBOX"]),
        ]);
        assert_eq!(contact(&unread).as_deref(), Some("ann@x.com"));
    }

    #[test]
    fn importance_is_the_primary_category_not_gmails_label() {
        let personal = thread(vec![message(
            "a",
            1,
            "ann@x.com",
            &[ME],
            &["INBOX", "CATEGORY_PERSONAL"],
        )]);
        assert!(lists(&personal).contains(&SH_IMPORTANT.into()));
        // Gmail's own IMPORTANT label doesn't count.
        let updates = thread(vec![message(
            "a",
            1,
            "ann@x.com",
            &[ME],
            &["INBOX", "IMPORTANT", "CATEGORY_UPDATES"],
        )]);
        assert_eq!(
            lists(&updates),
            [
                "CATEGORY_UPDATES",
                "IMPORTANT",
                "INBOX",
                "SH_ALL",
                "SH_OTHER"
            ]
        );
        let note_to_self = thread(vec![message("a", 1, ME, &[ME], &["INBOX", "SENT"])]);
        assert!(lists(&note_to_self).contains(&SH_IMPORTANT.into()));
        let mut docs = message("a", 1, "x@docs.google.com", &[ME], &["INBOX"]);
        docs.sender_raw = "\"Ann (via Google Docs)\" <x@docs.google.com>".into();
        assert!(lists(&thread(vec![docs])).contains(&SH_IMPORTANT.into()));
        let mut social_docs = message(
            "a",
            1,
            "x@docs.google.com",
            &[ME],
            &["INBOX", "CATEGORY_SOCIAL"],
        );
        social_docs.sender_raw = "Ann (via Google Docs) <x@docs.google.com>".into();
        assert!(lists(&thread(vec![social_docs])).contains(&SH_OTHER.into()));
        // Only inbox messages decide: an archived personal message doesn't.
        let mixed = thread(vec![
            message("a", 1, "ann@x.com", &[ME], &["CATEGORY_PERSONAL"]),
            message(
                "b",
                2,
                "ann@x.com",
                &[ME],
                &["INBOX", "CATEGORY_PROMOTIONS"],
            ),
        ]);
        assert!(lists(&mixed).contains(&SH_OTHER.into()));
    }

    #[test]
    fn done_spam_and_drafts_route_to_their_lists() {
        let done = thread(vec![message(
            "a",
            1,
            "ann@x.com",
            &[ME],
            &["CATEGORY_PERSONAL"],
        )]);
        assert_eq!(lists(&done), ["CATEGORY_PERSONAL", "SH_ALL", "SH_ARCHIVED"]);
        let spam = thread(vec![message(
            "a",
            1,
            "ann@x.com",
            &[ME],
            &["SPAM", "UNREAD", "CATEGORY_UPDATES"],
        )]);
        assert_eq!(lists(&spam), ["SPAM", "UNREAD"]);
        // A new message Exo is writing (drafts written elsewhere: see below).
        let draft_only = ThreadView::new(
            "t".into(),
            vec![message("a", 1, ME, &["ann@x.com"], &["DRAFT"])],
            &[],
            &BTreeSet::from(["a".to_owned()]),
        );
        assert_eq!(
            lists(&draft_only),
            ["DRAFT", "INBOX", "SH_ALL", "SH_IMPORTANT"]
        );
        let reply_draft = thread(vec![
            message("a", 1, "ann@x.com", &[ME], &["CATEGORY_PERSONAL"]),
            message("b", 2, ME, &["ann@x.com"], &["DRAFT"]),
        ]);
        assert_eq!(
            lists(&reply_draft),
            ["CATEGORY_PERSONAL", "DRAFT", "SH_ALL", "SH_ARCHIVED"]
        );
    }

    #[test]
    fn muted_threads_stay_out_until_you_are_added() {
        let view = thread(vec![
            message("a", 1, "ann@x.com", &[ME, "bob@x.com"], &["INBOX", MUTED]),
            message("b", 2, "bob@x.com", &[ME], &["INBOX", "UNREAD"]),
        ]);
        assert!(view.is_muted(&me()));
        assert_eq!(
            lists(&view),
            [MUTED, "SH_ALL", "SH_ARCHIVED", "SH_MUTED", "UNREAD"]
        );
        // You weren't on the muted message; a newer one adds you.
        let added = thread(vec![
            message("a", 1, "ann@x.com", &["bob@x.com"], &["INBOX", MUTED]),
            message("b", 2, "bob@x.com", &[ME], &["INBOX"]),
        ]);
        assert!(!added.is_muted(&me()));
        assert!(lists(&added).contains(&INBOX.into()));
    }

    #[test]
    fn your_own_replies_do_not_bump_a_thread() {
        let view = thread(vec![
            message("a", 100, "ann@x.com", &[ME], &["INBOX"]),
            message("b", 200, ME, &["ann@x.com"], &["SENT"]),
        ]);
        assert_eq!(view.sort_for(INBOX, &me()), 100);
        assert_eq!(view.sort_for(SENT, &me()), 200);
        assert_eq!(view.gmail_sort(), 200);
    }

    #[test]
    fn participants_dedupe_like_the_thread_list() {
        let view = thread(vec![
            message("a", 1, "ann@x.com", &[ME], &["INBOX"]),
            message("b", 2, "bob@x.com", &[ME], &["INBOX"]),
            message("c", 3, ME, &["ann@x.com"], &["SENT"]),
            message("d", 4, "cat@x.com", &[ME], &["INBOX"]),
            message("e", 5, "bob@x.com", &[ME], &["INBOX"]),
        ]);
        let shown = view.shown(&ListId::Inbox);
        let names: Vec<String> = participants(&shown, &ListId::Inbox, &me())
            .into_iter()
            .map(|c| c.email)
            .collect();
        // First sender, then each person's last appearance, without me.
        assert_eq!(names, ["ann@x.com", "cat@x.com", "bob@x.com"]);
        let sent: Vec<String> = participants(&shown, &ListId::Sent, &me())
            .into_iter()
            .map(|c| c.email)
            .collect();
        assert_eq!(sent, ["ann@x.com"]);
        let alone = thread(vec![message("a", 1, ME, &[ME], &["INBOX", "SENT"])]);
        let only_me = participants(&alone.shown(&ListId::Inbox), &ListId::Inbox, &me());
        assert_eq!(only_me.len(), 1);
        assert!(only_me[0].is_me);
    }

    #[test]
    fn pending_steps_overlay_gmails_copy() {
        let mut view = thread(vec![
            message("a", 1, "ann@x.com", &[ME], &["INBOX", "UNREAD"]),
            message("b", 2, ME, &["ann@x.com"], &["SENT"]),
        ]);
        view.apply(&Step::RemoveFromMessages {
            ids: vec!["a".into()],
            labels: vec!["INBOX".into()],
        });
        view.apply(&Step::ModifyThread {
            add: vec!["STARRED".into()],
            remove: vec!["UNREAD".into()],
        });
        assert!(view.is_done(&me()));
        assert!(!view.has(UNREAD));
        assert!(view.messages.iter().all(|m| m.has(STARRED)));
    }
}
