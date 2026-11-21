//! Triage mutations: what each one sends to Gmail, and how each one is undone.
//!
//! A mutation becomes [`Intent`]s (change labels, star, mark unread, refresh), which [`resolve`]
//! turns into concrete [`Step`]s against the thread as the user sees it. Steps are what the outbox
//! persists and replays.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::ids::labels::*;
use crate::view::{Identity, ThreadView};

/// A triage action on a thread. Applying one to a thread already in its target state does
/// nothing (no Gmail call, nothing to undo).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mutation {
    /// Remove from the inbox (`e`).
    MarkDone,
    /// Back to the inbox, unmuting.
    MarkNotDone,
    /// Star the latest message not from you (`s`).
    Star,
    /// Unstar every message.
    Unstar,
    /// Mark every message read.
    MarkRead,
    /// Mark the latest message unread (`u`).
    MarkUnread,
    /// Move to Trash (`#`).
    Trash,
    /// Out of Trash, back to the inbox ("Moved to Inbox.").
    Untrash,
    /// Report spam (`!`).
    Spam,
    /// Not spam: back to the inbox.
    NotSpam,
    /// Add a label by id.
    AddLabel(String),
    /// Remove a label by id.
    RemoveLabel(String),
    /// Remove whichever of these labels the thread carries (`⇧Y` passes the account's visible
    /// labels and `INBOX`); undo puts back the ones it had.
    RemoveLabels(Vec<String>),
    /// Move out of the list keyed `from` (`INBOX`, `SH_IMPORTANT`, a label id…) to `to`
    /// (`INBOX`, `SH_IMPORTANT`, `SH_OTHER`, `SH_ARCHIVED`, `SPAM`, `TRASH` or a label id):
    /// the labels change as `move_change` has it.
    Move {
        /// The list on screen, if any.
        from: Option<String>,
        /// The destination.
        to: String,
    },
    /// Mute: out of the inbox, and replies stay out (`m`).
    Mute,
    /// Unmute: back to the inbox.
    Unmute,
    /// Remind me (`H`): the thread comes back at `at`, and is marked done meanwhile when
    /// `snooze`. A reply cancels it unless `keep_on_reply` (the picker's `regardless`). Replaces
    /// the reminder the thread had.
    Remind {
        /// When it comes back.
        at: Timestamp,
        /// `regardless` rather than `if no reply`.
        keep_on_reply: bool,
        /// Out of the inbox until then.
        snooze: bool,
    },
    /// Remove Reminder: back to the inbox too when `to_inbox` and it isn't there
    /// (`Remove Reminder & Move to Inbox`).
    CancelReminder {
        /// Move it to the inbox.
        to_inbox: bool,
    },
    /// What a due reminder does, which [`Store::return_reminders`](crate::Store::return_reminders)
    /// applies: back to the inbox, and "reminder unread" until read.
    ReturnFromReminder,
}

/// A change to a thread, before it's resolved against the thread as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Intent {
    /// Labels added to and removed from the thread.
    Change {
        add: Vec<String>,
        remove: Vec<String>,
    },
    /// A star on the message `message_to_star` picks.
    Star,
    /// Unread on the message `message_to_mark_unread` picks.
    MarkUnread,
    /// The thread read again from Gmail.
    Refresh,
}

/// Intents plus the coalescing key the outbox row gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Plan {
    pub intents: Vec<Intent>,
    pub replace_key: Option<String>,
}

/// One Gmail call, as persisted in the outbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case")]
pub(crate) enum Step {
    /// `messages.batchModify` removing `labels` from the messages that carry them.
    RemoveFromMessages {
        ids: Vec<String>,
        labels: Vec<String>,
    },
    /// `threads.modify`.
    ModifyThread {
        add: Vec<String>,
        remove: Vec<String>,
    },
    /// `messages.modify` adding one label to one message (star, mark unread).
    AddToMessage { id: String, label: String },
    /// `threads.get` for the thread's rows, replacing the local copy's (bodies stay).
    Refresh,
}

/// Removing these is done per message, on the messages that carry them; every other change goes
/// to `threads.modify`.
const MESSAGE_LEVEL: [&str; 3] = [UNREAD, INBOX, CATEGORY_PERSONAL];

impl Mutation {
    /// What applying `self` to `view` does, and what undoes it; `None` when the thread is
    /// already in the target state. The undo plan is resolved later, against the thread as it
    /// is when the user undoes.
    pub(crate) fn plan(&self, view: &ThreadView, me: &Identity) -> Result<Option<(Plan, Plan)>> {
        use Intent::{MarkUnread, Refresh, Star};
        let refreshed = |mut intents: Vec<Intent>| {
            intents.push(Refresh);
            Plan {
                intents,
                replace_key: None,
            }
        };
        // Reading the thread, or marking it not done, takes the reminder's unread mark off.
        let returned = me
            .reminder_unread_label
            .as_deref()
            .filter(|label| view.has(label));
        // Mark read: no refresh, and coalesced while unsent, so reading a thread again as mail
        // arrives rewrites the one queued change.
        let read = || Plan {
            intents: vec![change(
                &[],
                &[Some(UNREAD), returned]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>(),
            )],
            replace_key: Some(format!("set-read-{}", view.id)),
        };
        let muted = || {
            me.muted_label
                .clone()
                .ok_or(Error::NotSynced("the mute label"))
        };
        let snoozed = || {
            me.snoozed_label
                .clone()
                .ok_or(Error::NotSynced("the reminder labels"))
        };
        let (done, spam, trash) = (view.is_done(me), view.has(SPAM), view.has(TRASH));
        Ok(match self {
            Self::MarkDone => (!done).then(|| {
                (
                    refreshed(vec![change(&[], &[INBOX])]),
                    refreshed(vec![change(&[INBOX], &[])]),
                )
            }),
            Self::MarkNotDone if done => {
                let muted = muted()?;
                let mut undo = Vec::new();
                if view.is_muted(me) {
                    undo.push(change(&[&muted], &[INBOX]));
                }
                undo.push(change(&[], &[INBOX]));
                let mut forward = vec![change(&[INBOX], &[&muted]), change(&[INBOX], &[])];
                forward.extend(returned.map(|label| change(&[], &[label])));
                Some((refreshed(forward), refreshed(undo)))
            }
            Self::MarkNotDone => None,
            Self::Star => (!view.has(STARRED)).then(|| {
                (
                    refreshed(vec![Star]),
                    refreshed(vec![change(&[], &[STARRED])]),
                )
            }),
            Self::Unstar => view.has(STARRED).then(|| {
                (
                    refreshed(vec![change(&[], &[STARRED])]),
                    refreshed(vec![Star]),
                )
            }),
            Self::MarkRead => (view.has(UNREAD) || returned.is_some())
                .then(|| (read(), refreshed(vec![MarkUnread]))),
            Self::MarkUnread => (!view.has(UNREAD)).then(|| (refreshed(vec![MarkUnread]), read())),
            Self::Trash => (!trash).then(|| {
                let undo = if spam {
                    change(&[SPAM], &[TRASH])
                } else if done {
                    change(&[], &[TRASH])
                } else {
                    change(&[INBOX], &[TRASH])
                };
                (
                    refreshed(vec![change(&[TRASH], &[SPAM, INBOX])]),
                    refreshed(vec![undo]),
                )
            }),
            Self::Untrash => trash.then(|| {
                (
                    refreshed(vec![change(&[INBOX], &[SPAM, TRASH])]),
                    refreshed(vec![change(&[TRASH], &[INBOX])]),
                )
            }),
            Self::Spam => (!spam).then(|| {
                let undo = if trash {
                    change(&[TRASH], &[SPAM])
                } else if done {
                    change(&[], &[SPAM])
                } else {
                    change(&[INBOX], &[SPAM])
                };
                (
                    refreshed(vec![change(&[SPAM], &[INBOX, TRASH])]),
                    refreshed(vec![undo]),
                )
            }),
            Self::NotSpam => spam.then(|| {
                (
                    refreshed(vec![change(&[INBOX], &[SPAM, TRASH])]),
                    refreshed(vec![change(&[SPAM], &[INBOX])]),
                )
            }),
            Self::AddLabel(label) => (!view.has(label)).then(|| {
                (
                    refreshed(vec![change(&[label], &[])]),
                    refreshed(vec![change(&[], &[label])]),
                )
            }),
            Self::RemoveLabel(label) => view.has(label).then(|| {
                (
                    refreshed(vec![change(&[], &[label])]),
                    refreshed(vec![change(&[label], &[])]),
                )
            }),
            Self::RemoveLabels(labels) => {
                let carried: Vec<&str> = labels
                    .iter()
                    .map(String::as_str)
                    .filter(|label| view.has(label))
                    .collect();
                (!carried.is_empty()).then(|| {
                    (
                        refreshed(vec![change(&[], &carried)]),
                        refreshed(vec![change(&carried, &[])]),
                    )
                })
            }
            Self::Move { from, to } => {
                let (add, remove) = move_change(from.as_deref(), to);
                // Undo puts back what was removed and was there, and takes away what was added
                // and wasn't, keeping one of Inbox, Spam and Trash should that ever name several.
                let mut exclusive = false;
                let put_back: Vec<&str> = remove
                    .iter()
                    .copied()
                    .filter(|label| view.has(label))
                    .filter(|label| {
                        ![INBOX, SPAM, TRASH].contains(label)
                            || !std::mem::replace(&mut exclusive, true)
                    })
                    .collect();
                let take_away: Vec<&str> = add
                    .iter()
                    .copied()
                    .filter(|label| !view.has(label))
                    .collect();
                let mut forward = vec![change(&add, &remove)];
                let mut undo = vec![change(&put_back, &take_away)];
                // Out of Reminders, the reminder goes too, and undo puts it back.
                if from.as_deref() == Some(SH_SNOOZED) && view.has_reminder(me) {
                    let label = snoozed()?;
                    forward.push(change(&[], &[&label]));
                    undo.push(change(&[&label], &[]));
                }
                Some((refreshed(forward), refreshed(undo)))
            }
            Self::Mute if !view.is_muted(me) => {
                let muted = muted()?;
                Some((
                    refreshed(vec![change(&[&muted], &[INBOX])]),
                    refreshed(vec![change(&[INBOX], &[&muted])]),
                ))
            }
            Self::Unmute if view.is_muted(me) => {
                let muted = muted()?;
                Some((
                    refreshed(vec![change(&[INBOX], &[&muted])]),
                    refreshed(vec![change(&[&muted], &[INBOX])]),
                ))
            }
            Self::Mute | Self::Unmute => None,
            Self::Remind { snooze, .. } => {
                let label = snoozed()?;
                // Undo: the previous reminder comes back (keeping the label), and a thread taken
                // out of the inbox goes back to it.
                let restore_inbox: &[&str] = if *snooze && !done { &[INBOX] } else { &[] };
                let take_off = [label.as_str()];
                let take_off: &[&str] = if view.has_reminder(me) {
                    &[]
                } else {
                    &take_off
                };
                Some((
                    refreshed(vec![change(
                        &[&label],
                        if *snooze { &[INBOX] } else { &[] },
                    )]),
                    refreshed(vec![change(restore_inbox, take_off)]),
                ))
            }
            Self::CancelReminder { to_inbox } => {
                if !view.has_reminder(me) {
                    return Ok(None);
                }
                let label = snoozed()?;
                let inbox: &[&str] = if *to_inbox && done { &[INBOX] } else { &[] };
                Some((
                    refreshed(vec![change(inbox, &[&label])]),
                    refreshed(vec![change(&[&label], inbox)]),
                ))
            }
            Self::ReturnFromReminder => {
                let label = snoozed()?;
                let unread = me
                    .reminder_unread_label
                    .clone()
                    .ok_or(Error::NotSynced("the reminder labels"))?;
                Some((
                    refreshed(vec![change(&[INBOX, &unread], &[&label])]),
                    refreshed(vec![change(&[&label], &[INBOX, &unread])]),
                ))
            }
        })
    }

    /// The labels a bulk action adds to and removes from whole threads, `muted` being the mute
    /// label's id. `None` for removing every label, which only runs on loaded conversations.
    pub(crate) fn label_change(&self, muted: &str) -> Option<(Vec<String>, Vec<String>)> {
        let owned = |labels: &[&str]| {
            labels
                .iter()
                .map(|label| (*label).to_owned())
                .collect::<Vec<_>>()
        };
        let (add, remove): (&[&str], &[&str]) = match self {
            Self::MarkDone => (&[], &[INBOX]),
            Self::MarkNotDone => (&[INBOX], &[]),
            Self::Star => (&[STARRED], &[]),
            Self::Unstar => (&[], &[STARRED]),
            Self::MarkRead => (&[], &[UNREAD]),
            Self::MarkUnread => (&[UNREAD], &[]),
            Self::Trash => (&[TRASH], &[INBOX, SPAM]),
            Self::Untrash => (&[INBOX], &[SPAM, TRASH]),
            Self::Spam => (&[SPAM], &[INBOX, TRASH]),
            Self::NotSpam => (&[INBOX], &[SPAM, TRASH]),
            Self::Mute => (&[muted], &[INBOX]),
            Self::Unmute => (&[INBOX], &[muted]),
            Self::AddLabel(label) => return Some((vec![label.clone()], Vec::new())),
            Self::RemoveLabel(label) => return Some((Vec::new(), vec![label.clone()])),
            Self::Move { from, to } => {
                let (add, remove) = move_change(from.as_deref(), to);
                return Some((owned(&add), owned(&remove)));
            }
            Self::RemoveLabels(_)
            | Self::Remind { .. }
            | Self::CancelReminder { .. }
            | Self::ReturnFromReminder => return None,
        };
        Some((owned(add), owned(remove)))
    }
}

/// The labels a move from list `from` to `to` adds and removes. Exo's own ids (`SH_…`), `SENT`,
/// `DRAFT` and `STARRED` are never changed, so leaving Starred or Done doesn't unstar or
/// unarchive. Moving from an inbox view to the Inbox takes the thread out of it.
fn move_change<'a>(from: Option<&'a str>, to: &'a str) -> (Vec<&'a str>, Vec<&'a str>) {
    const OTHER_CATEGORIES: [&str; 4] = [
        CATEGORY_FORUMS,
        CATEGORY_PROMOTIONS,
        CATEGORY_SOCIAL,
        CATEGORY_UPDATES,
    ];
    // An inbox list: Inbox, Important, Other or a split.
    let inbox_like = from.is_some_and(|from| {
        [INBOX, "SH_INBOX", SH_IMPORTANT, SH_OTHER].contains(&from)
            || from.starts_with(crate::split::SPLIT_ID_PREFIX)
    });
    let (mut add, mut remove) = (Vec::new(), Vec::new());
    match to {
        INBOX | SH_IMPORTANT | SH_OTHER => {
            let other = to == SH_OTHER;
            add.push(if other {
                CATEGORY_SOCIAL
            } else {
                CATEGORY_PERSONAL
            });
            if !inbox_like {
                add.push(INBOX);
            }
            remove.extend([SPAM, TRASH]);
            if other {
                remove.push(CATEGORY_PERSONAL);
            } else {
                remove.extend(OTHER_CATEGORIES);
            }
        }
        SH_ARCHIVED => remove.extend([INBOX, SPAM, TRASH]),
        SPAM => {
            add.push(SPAM);
            remove.extend([INBOX, TRASH]);
        }
        TRASH => {
            add.push(TRASH);
            remove.extend([INBOX, SPAM]);
        }
        label => {
            add.push(label);
            remove.extend([INBOX, SPAM, TRASH]);
        }
    }
    remove.extend(from);
    let mut seen = Vec::new();
    remove.retain(|label| {
        !seen.contains(label) && {
            seen.push(*label);
            true
        }
    });
    let changeable =
        |label: &&str| !label.starts_with("SH_") && ![SENT, DRAFT, STARRED].contains(label);
    add.retain(changeable);
    remove.retain(changeable);
    (add, remove)
}

pub(crate) fn change(add: &[&str], remove: &[&str]) -> Intent {
    let owned = |labels: &[&str]| labels.iter().map(|label| (*label).to_owned()).collect();
    Intent::Change {
        add: owned(add),
        remove: owned(remove),
    }
}

/// Resolves intents into Gmail calls. Every intent sees `view` as it is now: one snapshot of the
/// thread for the whole mutation.
pub(crate) fn resolve(intents: &[Intent], view: &ThreadView, me: &Identity) -> Vec<Step> {
    let mut steps = Vec::new();
    for intent in intents {
        match intent {
            Intent::Change { add, remove } => {
                if remove
                    .iter()
                    .any(|label| MESSAGE_LEVEL.contains(&label.as_str()))
                {
                    let ids = view.carriers(remove);
                    if !ids.is_empty() {
                        steps.push(Step::RemoveFromMessages {
                            ids,
                            labels: remove.clone(),
                        });
                    }
                    if !add.is_empty() {
                        steps.push(Step::ModifyThread {
                            add: add.clone(),
                            remove: Vec::new(),
                        });
                    }
                } else if !add.is_empty() || !remove.is_empty() {
                    steps.push(Step::ModifyThread {
                        add: add.clone(),
                        remove: remove.clone(),
                    });
                }
            }
            Intent::Star => steps.extend(view.message_to_star(me).map(|id| Step::AddToMessage {
                id,
                label: STARRED.to_owned(),
            })),
            Intent::MarkUnread => {
                steps.extend(view.message_to_mark_unread().map(|id| Step::AddToMessage {
                    id,
                    label: UNREAD.to_owned(),
                }));
            }
            Intent::Refresh => steps.push(Step::Refresh),
        }
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::tests::{ME, MUTED, me, message, thread};

    fn steps(mutation: Mutation, view: &ThreadView) -> Option<(Vec<Step>, Option<String>)> {
        let (forward, _) = mutation.plan(view, &me()).unwrap()?;
        Some((resolve(&forward.intents, view, &me()), forward.replace_key))
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn inbox_thread() -> ThreadView {
        thread(vec![
            message(
                "a",
                1,
                "ann@x.com",
                &[ME],
                &["INBOX", "UNREAD", "CATEGORY_PERSONAL"],
            ),
            message("b", 2, ME, &["ann@x.com"], &["SENT"]),
            message(
                "c",
                3,
                "ann@x.com",
                &[ME],
                &["INBOX", "UNREAD", "CATEGORY_PERSONAL"],
            ),
        ])
    }

    #[test]
    fn done_removes_inbox_from_the_messages_that_carry_it() {
        assert_eq!(
            steps(Mutation::MarkDone, &inbox_thread()).unwrap().0,
            [
                Step::RemoveFromMessages {
                    ids: strings(&["a", "c"]),
                    labels: strings(&["INBOX"]),
                },
                Step::Refresh,
            ]
        );
    }

    #[test]
    fn not_done_unmutes_then_adds_inbox_to_the_thread() {
        let done = thread(vec![message(
            "a",
            1,
            "ann@x.com",
            &[ME],
            &["CATEGORY_PERSONAL"],
        )]);
        assert_eq!(
            steps(Mutation::MarkNotDone, &done).unwrap().0,
            [
                Step::ModifyThread {
                    add: strings(&["INBOX"]),
                    remove: strings(&[MUTED]),
                },
                Step::ModifyThread {
                    add: strings(&["INBOX"]),
                    remove: Vec::new(),
                },
                Step::Refresh,
            ]
        );
        assert!(steps(Mutation::MarkNotDone, &inbox_thread()).is_none());
    }

    #[test]
    fn star_marks_the_latest_message_not_from_me() {
        let view = thread(vec![
            message("a", 1, "ann@x.com", &[ME], &["INBOX"]),
            message("b", 2, ME, &["ann@x.com"], &["SENT"]),
        ]);
        assert_eq!(
            steps(Mutation::Star, &view).unwrap().0,
            [
                Step::AddToMessage {
                    id: "a".into(),
                    label: "STARRED".into(),
                },
                Step::Refresh,
            ]
        );
        let mut starred = view;
        starred.apply(&Step::AddToMessage {
            id: "a".into(),
            label: "STARRED".into(),
        });
        assert!(steps(Mutation::Star, &starred).is_none());
        assert_eq!(
            steps(Mutation::Unstar, &starred).unwrap().0,
            [
                Step::ModifyThread {
                    add: Vec::new(),
                    remove: strings(&["STARRED"]),
                },
                Step::Refresh,
            ]
        );
    }

    #[test]
    fn read_and_unread_work_per_message() {
        let (read, key) = steps(Mutation::MarkRead, &inbox_thread()).unwrap();
        assert_eq!(
            read,
            [Step::RemoveFromMessages {
                ids: strings(&["a", "c"]),
                labels: strings(&["UNREAD"]),
            }]
        );
        assert_eq!(key.as_deref(), Some("set-read-t"));
        let seen = thread(vec![
            message("a", 1, "ann@x.com", &[ME], &["INBOX"]),
            message("b", 2, ME, &["ann@x.com"], &["SENT"]),
        ]);
        assert_eq!(
            steps(Mutation::MarkUnread, &seen).unwrap().0,
            [
                Step::AddToMessage {
                    id: "b".into(),
                    label: "UNREAD".into(),
                },
                Step::Refresh,
            ]
        );
    }

    #[test]
    fn trash_and_spam_split_message_and_thread_changes() {
        assert_eq!(
            steps(Mutation::Trash, &inbox_thread()).unwrap().0,
            [
                Step::RemoveFromMessages {
                    ids: strings(&["a", "c"]),
                    labels: strings(&["SPAM", "INBOX"]),
                },
                Step::ModifyThread {
                    add: strings(&["TRASH"]),
                    remove: Vec::new(),
                },
                Step::Refresh,
            ]
        );
        assert_eq!(
            steps(Mutation::Spam, &inbox_thread()).unwrap().0,
            [
                Step::RemoveFromMessages {
                    ids: strings(&["a", "c"]),
                    labels: strings(&["INBOX", "TRASH"]),
                },
                Step::ModifyThread {
                    add: strings(&["SPAM"]),
                    remove: Vec::new(),
                },
                Step::Refresh,
            ]
        );
        let spam = thread(vec![message("a", 1, "ann@x.com", &[ME], &["SPAM"])]);
        assert_eq!(
            steps(Mutation::NotSpam, &spam).unwrap().0,
            [
                Step::ModifyThread {
                    add: strings(&["INBOX"]),
                    remove: strings(&["SPAM", "TRASH"]),
                },
                Step::Refresh,
            ]
        );
        assert!(steps(Mutation::Untrash, &spam).is_none());
    }

    #[test]
    fn labels_and_mute_use_the_thread() {
        let view = inbox_thread();
        assert_eq!(
            steps(Mutation::AddLabel("Label_1".into()), &view)
                .unwrap()
                .0,
            [
                Step::ModifyThread {
                    add: strings(&["Label_1"]),
                    remove: Vec::new(),
                },
                Step::Refresh,
            ]
        );
        assert!(steps(Mutation::RemoveLabel("Label_1".into()), &view).is_none());
        assert_eq!(
            steps(Mutation::Mute, &view).unwrap().0,
            [
                Step::RemoveFromMessages {
                    ids: strings(&["a", "c"]),
                    labels: strings(&["INBOX"]),
                },
                Step::ModifyThread {
                    add: strings(&[MUTED]),
                    remove: Vec::new(),
                },
                Step::Refresh,
            ]
        );
        assert!(steps(Mutation::Unmute, &view).is_none());
    }

    #[test]
    fn undo_restores_the_state_before() {
        // Trash from the inbox comes back to the inbox; trash from Done doesn't.
        let undo = |view: &ThreadView, mutation: Mutation| {
            let (_, inverse) = mutation.plan(view, &me()).unwrap().unwrap();
            resolve(&inverse.intents, view, &me())
        };
        assert_eq!(
            undo(&inbox_thread(), Mutation::Trash)[0],
            Step::ModifyThread {
                add: strings(&["INBOX"]),
                remove: strings(&["TRASH"]),
            }
        );
        let done = thread(vec![message("a", 1, "ann@x.com", &[ME], &[])]);
        assert_eq!(
            undo(&done, Mutation::Trash)[0],
            Step::ModifyThread {
                add: Vec::new(),
                remove: strings(&["TRASH"]),
            }
        );
        assert_eq!(
            undo(&inbox_thread(), Mutation::MarkDone)[0],
            Step::ModifyThread {
                add: strings(&["INBOX"]),
                remove: Vec::new(),
            }
        );
    }

    #[test]
    fn moving_to_a_label_takes_the_thread_out_of_the_inbox() {
        let mv = |from: Option<&str>, to: &str| Mutation::Move {
            from: from.map(str::to_owned),
            to: to.to_owned(),
        };
        assert_eq!(
            steps(mv(Some("INBOX"), "Label_1"), &inbox_thread())
                .unwrap()
                .0,
            [
                Step::RemoveFromMessages {
                    ids: strings(&["a", "c"]),
                    labels: strings(&["INBOX", "SPAM", "TRASH"]),
                },
                Step::ModifyThread {
                    add: strings(&["Label_1"]),
                    remove: Vec::new(),
                },
                Step::Refresh,
            ]
        );
        // Done, Starred and the like are views, not labels: leaving them changes nothing.
        let done = thread(vec![message(
            "a",
            1,
            "ann@x.com",
            &[ME],
            &["STARRED", "Label_2"],
        )]);
        assert_eq!(
            steps(mv(Some("STARRED"), "SH_ARCHIVED"), &done).unwrap().0,
            [Step::Refresh]
        );
        assert_eq!(
            steps(mv(Some("Label_2"), "SH_IMPORTANT"), &done).unwrap().0,
            [
                Step::ModifyThread {
                    add: strings(&["CATEGORY_PERSONAL", "INBOX"]),
                    remove: strings(&[
                        "SPAM",
                        "TRASH",
                        "CATEGORY_FORUMS",
                        "CATEGORY_PROMOTIONS",
                        "CATEGORY_SOCIAL",
                        "CATEGORY_UPDATES",
                        "Label_2"
                    ]),
                },
                Step::Refresh,
            ]
        );
        // Inbox → Inbox archives.
        let (forward, _) = mv(Some("INBOX"), "INBOX")
            .plan(&inbox_thread(), &me())
            .unwrap()
            .unwrap();
        assert!(resolve(&forward.intents, &inbox_thread(), &me()).contains(
            &Step::RemoveFromMessages {
                ids: strings(&["a", "c"]),
                labels: strings(&[
                    "SPAM",
                    "TRASH",
                    "CATEGORY_FORUMS",
                    "CATEGORY_PROMOTIONS",
                    "CATEGORY_SOCIAL",
                    "CATEGORY_UPDATES",
                    "INBOX"
                ]),
            }
        ));
    }

    #[test]
    fn undoing_a_move_restores_only_what_changed() {
        let view = inbox_thread();
        let (_, inverse) = Mutation::Move {
            from: Some("INBOX".into()),
            to: "Label_1".into(),
        }
        .plan(&view, &me())
        .unwrap()
        .unwrap();
        assert_eq!(
            resolve(&inverse.intents, &view, &me()),
            [
                Step::ModifyThread {
                    add: strings(&["INBOX"]),
                    remove: strings(&["Label_1"]),
                },
                Step::Refresh,
            ]
        );
    }

    #[test]
    fn removing_labels_takes_off_only_those_the_thread_has() {
        let view = thread(vec![message(
            "a",
            1,
            "ann@x.com",
            &[ME],
            &["INBOX", "Label_1"],
        )]);
        let all = Mutation::RemoveLabels(strings(&["Label_1", "Label_2", "TRASH", "INBOX"]));
        assert_eq!(
            steps(all.clone(), &view).unwrap().0,
            [
                Step::RemoveFromMessages {
                    ids: strings(&["a"]),
                    labels: strings(&["Label_1", "INBOX"]),
                },
                Step::Refresh,
            ]
        );
        let (_, inverse) = all.plan(&view, &me()).unwrap().unwrap();
        assert_eq!(
            resolve(&inverse.intents, &view, &me())[0],
            Step::ModifyThread {
                add: strings(&["Label_1", "INBOX"]),
                remove: Vec::new(),
            }
        );
        let bare = thread(vec![message("a", 1, "ann@x.com", &[ME], &["SENT"])]);
        assert!(steps(all, &bare).is_none());
    }

    #[test]
    fn mute_needs_the_hidden_label() {
        let mut nobody = me();
        nobody.muted_label = None;
        assert!(matches!(
            Mutation::Mute.plan(&inbox_thread(), &nobody),
            Err(Error::NotSynced(_))
        ));
    }
}
