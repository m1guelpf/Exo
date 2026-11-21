//! Reminders: what setting, removing, undoing and returning one does to the lists and to Gmail,
//! and what a reply does to an "if no reply" one.

mod support;

use std::collections::BTreeSet;

use exo_store::{ListId, Mutation, ReminderStatus, ThreadId, ThreadSummary};
use jiff::{Timestamp, ToSpan};
use serde_json::json;
use support::Harness;

async fn inbox_thread(harness: &Harness) -> ThreadSummary {
    harness
        .all(&ListId::Inbox)
        .await
        .into_iter()
        .find(|thread| thread.drafts == 0)
        .unwrap()
}

/// The id of the label named `name`.
async fn label(harness: &Harness, name: &str) -> String {
    harness
        .store
        .labels()
        .await
        .unwrap()
        .into_iter()
        .find(|label| label.name == name)
        .unwrap_or_else(|| panic!("{name} exists"))
        .id
}

/// Every label on the thread's messages in Gmail.
async fn gmail_labels(harness: &Harness, thread: &ThreadId) -> BTreeSet<String> {
    harness
        .gmail_labels(thread)
        .await
        .into_iter()
        .flat_map(|(_, labels)| labels)
        .collect()
}

/// `span` from now, to the second.
fn from_now(span: jiff::Span) -> Timestamp {
    Timestamp::now()
        .checked_add(span)
        .unwrap()
        .round(jiff::Unit::Second)
        .unwrap()
}

fn remind(at: Timestamp, keep_on_reply: bool) -> Mutation {
    Mutation::Remind {
        at,
        keep_on_reply,
        snooze: true,
    }
}

async fn flush(harness: &Harness) {
    harness.store.flush().await.unwrap();
    assert_eq!(
        harness.store.pending().await.unwrap(),
        0,
        "Gmail took every change"
    );
}

#[tokio::test]
async fn a_snoozed_thread_waits_in_reminders_and_comes_back_unread() {
    let harness = Harness::synced().await;
    let thread = inbox_thread(&harness).await;
    let id = thread.id.clone();
    let snoozed = label(&harness, "[Exo]/Is Snoozed").await;
    let returned = label(&harness, "[Exo]/ru").await;
    let at = from_now(2.hours());

    harness
        .store
        .apply(std::slice::from_ref(&id), remind(at, false))
        .await
        .unwrap();
    assert!(
        !harness.ids(&ListId::Inbox).await.contains(id.as_str()),
        "out of the inbox at once"
    );
    let waiting = harness
        .summary(&ListId::Reminders, &id)
        .await
        .expect("in Reminders");
    assert_eq!(
        waiting.reminder,
        Some(ReminderStatus::Set {
            at,
            keep_on_reply: false
        })
    );
    assert_eq!(harness.store.next_reminder().await.unwrap(), Some(at));
    flush(&harness).await;
    let labels = gmail_labels(&harness, &id).await;
    assert!(
        labels.contains(&snoozed) && !labels.contains("INBOX"),
        "Gmail sees it leave the inbox: {labels:?}"
    );

    assert!(
        harness
            .store
            .return_reminders(at.checked_sub(1.second()).unwrap())
            .await
            .unwrap()
            .is_empty(),
        "not before it's due"
    );
    let back = at.checked_add(5.minutes()).unwrap();
    assert_eq!(
        harness.store.return_reminders(back).await.unwrap(),
        std::slice::from_ref(&id)
    );
    assert!(
        harness
            .store
            .return_reminders(back)
            .await
            .unwrap()
            .is_empty(),
        "once"
    );
    assert!(!harness.ids(&ListId::Reminders).await.contains(id.as_str()));
    let row = harness
        .summary(&ListId::Inbox, &id)
        .await
        .expect("back in the inbox");
    assert_eq!(
        row.reminder,
        Some(ReminderStatus::Returned {
            at: back,
            unread: true
        })
    );
    assert!(row.unread, "reminder unread counts as unread");
    assert_eq!(row.date, back, "sorted as if it had just come in");
    assert_eq!(harness.store.next_reminder().await.unwrap(), None);
    flush(&harness).await;
    let labels = gmail_labels(&harness, &id).await;
    assert!(
        labels.contains("INBOX") && labels.contains(&returned) && !labels.contains(&snoozed),
        "{labels:?}"
    );

    harness
        .store
        .apply(std::slice::from_ref(&id), Mutation::MarkRead)
        .await
        .unwrap();
    let row = harness.summary(&ListId::Inbox, &id).await.unwrap();
    assert_eq!(
        (row.reminder, row.unread),
        (
            Some(ReminderStatus::Returned {
                at: back,
                unread: false
            }),
            false
        )
    );
    flush(&harness).await;
    assert!(
        !gmail_labels(&harness, &id).await.contains(&returned),
        "reading it takes the mark off"
    );
}

#[tokio::test]
async fn undoing_a_reminder_puts_the_thread_back() {
    let harness = Harness::synced().await;
    let id = inbox_thread(&harness).await.id;
    let before = gmail_labels(&harness, &id).await;
    let at = from_now(24.hours());

    let undo = harness
        .store
        .apply(std::slice::from_ref(&id), remind(at, false))
        .await
        .unwrap();
    harness.store.undo(undo).await.unwrap();
    assert!(harness.ids(&ListId::Inbox).await.contains(id.as_str()));
    assert_eq!(
        harness.summary(&ListId::Inbox, &id).await.unwrap().reminder,
        None
    );
    assert_eq!(
        harness.store.pending().await.unwrap(),
        0,
        "Gmail never heard of it"
    );

    let undo = harness
        .store
        .apply(std::slice::from_ref(&id), remind(at, false))
        .await
        .unwrap();
    flush(&harness).await;
    harness.store.undo(undo).await.unwrap();
    assert_eq!(harness.store.next_reminder().await.unwrap(), None);
    flush(&harness).await;
    assert_eq!(gmail_labels(&harness, &id).await, before);
}

#[tokio::test]
async fn a_new_reminder_replaces_the_last_and_undo_restores_it() {
    let harness = Harness::synced().await;
    let id = inbox_thread(&harness).await.id;
    let first = from_now(24.hours());
    let second = first.checked_add(24.hours()).unwrap();
    harness
        .store
        .apply(std::slice::from_ref(&id), remind(first, false))
        .await
        .unwrap();
    let undo = harness
        .store
        .apply(
            std::slice::from_ref(&id),
            Mutation::Remind {
                at: second,
                keep_on_reply: true,
                snooze: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        harness
            .summary(&ListId::Reminders, &id)
            .await
            .unwrap()
            .reminder,
        Some(ReminderStatus::Set {
            at: second,
            keep_on_reply: true
        })
    );
    harness.store.undo(undo).await.unwrap();
    assert_eq!(
        harness
            .summary(&ListId::Reminders, &id)
            .await
            .unwrap()
            .reminder,
        Some(ReminderStatus::Set {
            at: first,
            keep_on_reply: false
        })
    );
    assert!(
        !harness.ids(&ListId::Inbox).await.contains(id.as_str()),
        "still snoozed"
    );
}

#[tokio::test]
async fn removing_a_reminder_can_move_the_thread_back_to_the_inbox() {
    let harness = Harness::synced().await;
    let id = inbox_thread(&harness).await.id;
    let at = from_now(24.hours());
    harness
        .store
        .apply(std::slice::from_ref(&id), remind(at, false))
        .await
        .unwrap();

    let undo = harness
        .store
        .apply(
            std::slice::from_ref(&id),
            Mutation::CancelReminder { to_inbox: true },
        )
        .await
        .unwrap();
    assert!(harness.ids(&ListId::Inbox).await.contains(id.as_str()));
    assert!(!harness.ids(&ListId::Reminders).await.contains(id.as_str()));
    assert!(
        harness
            .store
            .apply(
                std::slice::from_ref(&id),
                Mutation::CancelReminder { to_inbox: true }
            )
            .await
            .unwrap()
            .is_empty(),
        "nothing left to remove"
    );

    harness.store.undo(undo).await.unwrap();
    assert!(
        !harness.ids(&ListId::Inbox).await.contains(id.as_str()),
        "out of the inbox again"
    );
    assert_eq!(
        harness
            .summary(&ListId::Reminders, &id)
            .await
            .unwrap()
            .reminder,
        Some(ReminderStatus::Set {
            at,
            keep_on_reply: false
        })
    );
}

#[tokio::test]
async fn a_reply_cancels_an_if_no_reply_reminder_but_not_a_regardless_one() {
    let harness = Harness::synced().await;
    let inbox: Vec<ThreadSummary> = harness
        .all(&ListId::Inbox)
        .await
        .into_iter()
        .filter(|thread| thread.drafts == 0)
        .take(2)
        .collect();
    let [if_no_reply, regardless] = [inbox[0].id.clone(), inbox[1].id.clone()];
    let snoozed = label(&harness, "[Exo]/Is Snoozed").await;
    let at = from_now(24.hours());
    harness
        .store
        .apply(std::slice::from_ref(&if_no_reply), remind(at, false))
        .await
        .unwrap();
    harness
        .store
        .apply(std::slice::from_ref(&regardless), remind(at, true))
        .await
        .unwrap();
    flush(&harness).await;

    for thread in [&if_no_reply, &regardless] {
        harness.deliver(json!({ "from": "Ana Ruiz <ana@example.com>", "threadId": thread.as_str(), "text": "Any news?", "labelIds": ["INBOX", "UNREAD"] })).await;
    }
    harness.sync_all().await;
    let cancelled = harness
        .summary(&ListId::Inbox, &if_no_reply)
        .await
        .expect("the reply brought it back");
    let Some(ReminderStatus::Aborted { by }) = cancelled.reminder else {
        panic!("cancelled: {:?}", cancelled.reminder)
    };
    assert_eq!(by.email, "ana@example.com");
    assert!(
        !harness
            .ids(&ListId::Reminders)
            .await
            .contains(if_no_reply.as_str())
    );
    assert_eq!(
        harness
            .summary(&ListId::Reminders, &regardless)
            .await
            .unwrap()
            .reminder,
        Some(ReminderStatus::Set {
            at,
            keep_on_reply: true
        })
    );

    assert!(
        harness
            .store
            .return_reminders(Timestamp::now())
            .await
            .unwrap()
            .is_empty()
    );
    flush(&harness).await;
    assert!(
        !gmail_labels(&harness, &if_no_reply)
            .await
            .contains(&snoozed),
        "the cancelled one's label comes off"
    );
    assert_eq!(
        harness.store.next_reminder().await.unwrap(),
        Some(at),
        "the other still waits"
    );
    assert_eq!(
        harness.store.return_reminders(at).await.unwrap(),
        [regardless]
    );
}

#[tokio::test]
async fn reminders_survive_a_restart() {
    let mut harness = Harness::synced().await;
    let id = inbox_thread(&harness).await.id;
    let at = from_now(24.hours());
    harness
        .store
        .apply(std::slice::from_ref(&id), remind(at, false))
        .await
        .unwrap();
    harness.reopen().await;
    assert_eq!(harness.store.next_reminder().await.unwrap(), Some(at));
    assert_eq!(harness.store.return_reminders(at).await.unwrap(), [id]);
}

#[tokio::test]
async fn moving_a_thread_out_of_reminders_removes_its_reminder() {
    let harness = Harness::synced().await;
    let id = inbox_thread(&harness).await.id;
    let snoozed = label(&harness, "[Exo]/Is Snoozed").await;
    let at = from_now(24.hours());
    harness
        .store
        .apply(std::slice::from_ref(&id), remind(at, false))
        .await
        .unwrap();
    flush(&harness).await;

    let undo = harness
        .store
        .apply(
            std::slice::from_ref(&id),
            Mutation::Move {
                from: Some("SH_SNOOZED".into()),
                to: "INBOX".into(),
            },
        )
        .await
        .unwrap();
    assert!(
        harness.ids(&ListId::Inbox).await.contains(id.as_str()),
        "moved to the inbox"
    );
    assert!(
        !harness.ids(&ListId::Reminders).await.contains(id.as_str()),
        "and out of Reminders"
    );
    assert_eq!(
        harness.summary(&ListId::Inbox, &id).await.unwrap().reminder,
        None
    );
    assert_eq!(harness.store.next_reminder().await.unwrap(), None);
    flush(&harness).await;
    let labels = gmail_labels(&harness, &id).await;
    assert!(
        labels.contains("INBOX") && !labels.contains(&snoozed),
        "{labels:?}"
    );

    harness.store.undo(undo).await.unwrap();
    assert_eq!(
        harness
            .summary(&ListId::Reminders, &id)
            .await
            .unwrap()
            .reminder,
        Some(ReminderStatus::Set {
            at,
            keep_on_reply: false
        }),
        "undo puts it back"
    );
    flush(&harness).await;
    let labels = gmail_labels(&harness, &id).await;
    assert!(
        !labels.contains("INBOX") && labels.contains(&snoozed),
        "{labels:?}"
    );
}

/// A new message to Ann carrying `reminder`, saved and written to Gmail.
async fn send_with_reminder(
    harness: &Harness,
    id: &str,
    reminder: Option<exo_store::DraftReminder>,
    send_later: Option<exo_store::SendLater>,
) {
    save_with_reminder(harness, id, None, reminder, send_later).await;
}

/// A reply to Ann on an inbox thread carrying `reminder`, saved and written to Gmail, and that
/// thread.
async fn reply_with_reminder(
    harness: &Harness,
    id: &str,
    reminder: Option<exo_store::DraftReminder>,
    send_later: Option<exo_store::SendLater>,
) -> ThreadId {
    let thread = inbox_thread(harness).await.id;
    save_with_reminder(harness, id, Some(&thread), reminder, send_later).await;
    thread
}

/// A message to Ann carrying `reminder`, in `thread` or new, saved and written to Gmail.
async fn save_with_reminder(
    harness: &Harness,
    id: &str,
    thread: Option<&ThreadId>,
    reminder: Option<exo_store::DraftReminder>,
    send_later: Option<exo_store::SendLater>,
) {
    let (replied, subject) = match thread {
        Some(thread) => {
            let detail = harness.store.thread(thread).await.unwrap().unwrap();
            (
                detail
                    .messages
                    .iter()
                    .rev()
                    .find(|message| !message.is_draft())
                    .cloned(),
                gmail::mime::reply_subject(&detail.subject),
            )
        }
        None => (None, "Proposal".to_owned()),
    };
    let email = gmail::mime::Email {
        from: Some(gmail::mime::Mailbox::new("Maya Okafor", support::ACCOUNT)),
        to: vec![gmail::mime::Mailbox::new("Ann", "ann@example.com")],
        subject: subject.clone(),
        html: Some("<div>Thoughts?</div>".into()),
        message_id: Some(format!("{id}@exo")),
        in_reply_to: replied
            .as_ref()
            .and_then(|message| message.rfc822_id.clone()),
        references: replied
            .iter()
            .flat_map(|message| {
                message
                    .references
                    .iter()
                    .cloned()
                    .chain(message.rfc822_id.clone())
            })
            .collect(),
        ..gmail::mime::Email::default()
    };
    let address = |name: &str, email: &str| exo_store::Address {
        name: Some(name.into()),
        email: email.into(),
    };
    let record = exo_store::DraftRecord {
        action: if replied.is_some() {
            exo_store::DraftAction::Reply
        } else {
            exo_store::DraftAction::Compose
        },
        from: Some(address("Maya Okafor", support::ACCOUNT)),
        to: vec![address("Ann", "ann@example.com")],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject,
        body: "<div>Thoughts?</div>".into(),
        quoted_content: None,
        quoted_inlined: false,
        in_reply_to: replied.as_ref().map(|message| message.id.clone()),
        in_reply_to_rfc822: None,
        references: Vec::new(),
        rfc822_id: format!("{id}@exo"),
        fingerprint: String::new(),
        attachments: Vec::new(),
        send_later,
        reminder,
    };
    let draft = exo_store::Draft {
        id: id.into(),
        thread: thread.cloned(),
        adopted: None,
        record,
        raw: String::from_utf8(email.to_rfc822()).unwrap(),
        complete: true,
        sending: true,
    };
    harness.store.save_draft(draft).await.unwrap();
    harness.store.push_drafts().await.unwrap();
}

#[tokio::test]
async fn a_reply_shows_its_reminder_from_send_and_undo_send_puts_back_the_last() {
    let harness = Harness::synced().await;
    let snoozed = label(&harness, "[Exo]/Is Snoozed").await;
    let (had, sent_with) = (from_now(24.hours()), from_now(72.hours()));
    let id = "draft00000000000041";
    let thread = reply_with_reminder(
        &harness,
        id,
        Some(exo_store::DraftReminder {
            at: sent_with,
            keep_on_reply: false,
            automatic: false,
        }),
        None,
    )
    .await;
    harness
        .store
        .apply(
            std::slice::from_ref(&thread),
            Mutation::Remind {
                at: had,
                keep_on_reply: true,
                snooze: false,
            },
        )
        .await
        .unwrap();
    flush(&harness).await;

    // Held for undo: the thread has the draft's reminder at once, not marked done.
    harness
        .store
        .send_draft(id, from_now(30.seconds()))
        .await
        .unwrap();
    let row = harness
        .summary(&ListId::Reminders, &thread)
        .await
        .expect("in Reminders");
    assert_eq!(
        row.reminder,
        Some(ReminderStatus::Set {
            at: sent_with,
            keep_on_reply: false
        })
    );
    assert!(
        harness.ids(&ListId::Inbox).await.contains(thread.as_str()),
        "still in the inbox"
    );
    flush(&harness).await;
    assert!(gmail_labels(&harness, &thread).await.contains(&snoozed));

    // Undo send: the one it had comes back.
    assert!(harness.store.undo_send(id).await.unwrap());
    assert_eq!(
        harness
            .summary(&ListId::Reminders, &thread)
            .await
            .unwrap()
            .reminder,
        Some(ReminderStatus::Set {
            at: had,
            keep_on_reply: true
        })
    );
    flush(&harness).await;
    assert!(
        gmail_labels(&harness, &thread).await.contains(&snoozed),
        "it still has one"
    );

    // Sent this time: the draft's again, and Gmail sending it changes nothing more.
    harness
        .store
        .send_draft(id, Timestamp::now())
        .await
        .unwrap();
    assert_eq!(
        harness
            .summary(&ListId::Reminders, &thread)
            .await
            .unwrap()
            .reminder,
        Some(ReminderStatus::Set {
            at: sent_with,
            keep_on_reply: false
        })
    );
    let report = harness.store.push_drafts().await.unwrap();
    assert_eq!(report.sent.len(), 1);
    assert!(!report.reminded, "nothing to give once sent");
    assert_eq!(
        harness
            .summary(&ListId::Reminders, &thread)
            .await
            .unwrap()
            .reminder,
        Some(ReminderStatus::Set {
            at: sent_with,
            keep_on_reply: false
        })
    );
    flush(&harness).await;
}

#[tokio::test]
async fn undo_send_takes_the_reminder_off_a_thread_that_had_none() {
    let harness = Harness::synced().await;
    let snoozed = label(&harness, "[Exo]/Is Snoozed").await;
    let id = "draft00000000000042";
    let thread = reply_with_reminder(
        &harness,
        id,
        Some(exo_store::DraftReminder {
            at: from_now(48.hours()),
            keep_on_reply: false,
            automatic: false,
        }),
        None,
    )
    .await;
    harness
        .store
        .send_draft(id, from_now(30.seconds()))
        .await
        .unwrap();
    flush(&harness).await;
    assert!(gmail_labels(&harness, &thread).await.contains(&snoozed));

    assert!(harness.store.undo_send(id).await.unwrap());
    assert!(
        harness.summary(&ListId::Reminders, &thread).await.is_none(),
        "out of Reminders"
    );
    assert!(harness.store.next_reminder().await.unwrap().is_none());
    flush(&harness).await;
    assert!(
        !gmail_labels(&harness, &thread).await.contains(&snoozed),
        "and Gmail's label off"
    );
}

#[tokio::test]
async fn a_scheduled_send_shows_its_reminder_from_scheduling() {
    let harness = Harness::synced().await;
    let (send_at, remind_at) = (from_now(1.hours()), from_now(72.hours()));
    let later = || {
        Some(exo_store::SendLater {
            at: send_at,
            abort_on_reply: false,
            current_message_ids: Vec::new(),
        })
    };
    let reminder = Some(exo_store::DraftReminder {
        at: remind_at,
        keep_on_reply: false,
        automatic: false,
    });

    // A reply.
    let id = "draft00000000000043";
    let thread = reply_with_reminder(&harness, id, reminder, later()).await;
    harness.store.schedule_draft(id).await.unwrap();
    assert!(
        harness
            .ids(&ListId::Scheduled)
            .await
            .contains(thread.as_str())
    );
    assert_eq!(
        harness
            .summary(&ListId::Reminders, &thread)
            .await
            .expect("in Reminders")
            .reminder,
        Some(ReminderStatus::Set {
            at: remind_at,
            keep_on_reply: false
        })
    );
    // Removed meanwhile, it stays removed when the message goes.
    harness
        .store
        .apply(
            std::slice::from_ref(&thread),
            Mutation::CancelReminder { to_inbox: false },
        )
        .await
        .unwrap();
    harness
        .store
        .send_draft(id, Timestamp::now())
        .await
        .unwrap();
    assert_eq!(harness.store.push_drafts().await.unwrap().sent.len(), 1);
    assert!(
        harness.summary(&ListId::Reminders, &thread).await.is_none(),
        "not given again"
    );

    // A new message: its thread is stored once Gmail has its draft.
    let id = "draft00000000000044";
    send_with_reminder(&harness, id, reminder, later()).await;
    harness.store.schedule_draft(id).await.unwrap();
    let thread = harness
        .store
        .draft(id)
        .await
        .unwrap()
        .unwrap()
        .thread
        .expect("Gmail's thread");
    assert!(
        harness
            .ids(&ListId::Scheduled)
            .await
            .contains(thread.as_str())
    );
    assert!(
        harness
            .ids(&ListId::Reminders)
            .await
            .contains(thread.as_str())
    );
    // `Scheduling undone.` takes it back.
    assert!(harness.store.undo_send(id).await.unwrap());
    assert!(
        !harness
            .ids(&ListId::Reminders)
            .await
            .contains(thread.as_str())
    );
}

#[tokio::test]
async fn a_message_sent_with_a_reminder_gives_its_thread_one() {
    let harness = Harness::synced().await;
    let snoozed = label(&harness, "[Exo]/Is Snoozed").await;
    let at = from_now(48.hours());
    send_with_reminder(
        &harness,
        "draft00000000000031",
        Some(exo_store::DraftReminder {
            at,
            keep_on_reply: false,
            automatic: false,
        }),
        None,
    )
    .await;
    let held = harness
        .store
        .draft("draft00000000000031")
        .await
        .unwrap()
        .unwrap();
    assert!(
        harness.store.next_reminder().await.unwrap().is_none(),
        "nothing until it's sent"
    );

    harness
        .store
        .send_draft(&held.id, Timestamp::now())
        .await
        .unwrap();
    let report = harness.store.push_drafts().await.unwrap();
    let [sent] = report.sent.as_slice() else {
        panic!("sent once: {:?}", report.sent)
    };
    let thread = sent.thread.clone();
    let row = harness
        .summary(&ListId::Reminders, &thread)
        .await
        .expect("in Reminders");
    assert_eq!(
        row.reminder,
        Some(ReminderStatus::Set {
            at,
            keep_on_reply: false
        })
    );
    assert!(
        harness.ids(&ListId::Sent).await.contains(thread.as_str()),
        "still in Sent: a reminder on a sent message doesn't snooze it"
    );
    flush(&harness).await;
    assert!(
        gmail_labels(&harness, &thread).await.contains(&snoozed),
        "Gmail sees the reminder's label"
    );

    // Ann answers: "if no reply" ends it.
    harness.deliver(json!({ "from": "Ann <ann@example.com>", "threadId": thread.as_str(), "text": "Love it", "labelIds": ["INBOX", "UNREAD"] })).await;
    harness.sync_all().await;
    assert!(matches!(
        harness
            .summary(&ListId::Inbox, &thread)
            .await
            .and_then(|row| row.reminder),
        Some(ReminderStatus::Aborted { .. })
    ));
}

#[tokio::test]
async fn a_reminder_picked_while_a_send_is_held_goes_with_it() {
    let harness = Harness::synced().await;
    send_with_reminder(&harness, "draft00000000000032", None, None).await;
    let at = from_now(72.hours());
    let reminder = Some(exo_store::DraftReminder {
        at,
        keep_on_reply: true,
        automatic: false,
    });
    assert!(
        !harness
            .store
            .remind_draft("draft00000000000032", reminder)
            .await
            .unwrap(),
        "not held: a draft being written keeps its own"
    );

    harness
        .store
        .send_draft("draft00000000000032", from_now(1.hours()))
        .await
        .unwrap();
    assert!(
        harness
            .store
            .remind_draft("draft00000000000032", reminder)
            .await
            .unwrap(),
        "held"
    );
    harness
        .store
        .send_draft("draft00000000000032", Timestamp::now())
        .await
        .unwrap();
    let report = harness.store.push_drafts().await.unwrap();
    let thread = report.sent.first().expect("sent").thread.clone();
    assert_eq!(
        harness
            .summary(&ListId::Reminders, &thread)
            .await
            .unwrap()
            .reminder,
        Some(ReminderStatus::Set {
            at,
            keep_on_reply: true
        })
    );
    assert!(
        !harness
            .store
            .remind_draft("draft00000000000032", None)
            .await
            .unwrap(),
        "sent: nothing to change"
    );
}

#[tokio::test]
async fn a_new_store_brings_back_what_another_one_snoozed() {
    let harness = Harness::synced().await;
    let thread = inbox_thread(&harness).await.id;
    // Snoozed in a store that's then gone (signed out, or another Mac's).
    let at = from_now(72.hours());
    harness
        .store
        .apply(
            std::slice::from_ref(&thread),
            Mutation::Remind {
                at,
                keep_on_reply: false,
                snooze: true,
            },
        )
        .await
        .unwrap();
    harness.store.flush().await.unwrap();
    let snoozed = label(&harness, "[Exo]/Is Snoozed").await;
    assert!(gmail_labels(&harness, &thread).await.contains(&snoozed));
    assert!(
        harness
            .store
            .return_orphaned_snoozes()
            .await
            .unwrap()
            .is_empty(),
        "the store that snoozed it has its reminder"
    );

    let dir = support::TempDir::new();
    let fresh = support::open(&harness.server, dir.path()).await;
    while fresh.sync().await.unwrap().backfilling {}
    assert_eq!(
        fresh.return_orphaned_snoozes().await.unwrap(),
        std::slice::from_ref(&thread),
        "back at once"
    );
    fresh.flush().await.unwrap();
    let labels = gmail_labels(&harness, &thread).await;
    assert!(
        labels.contains("INBOX") && !labels.contains(&snoozed),
        "{labels:?}"
    );
    assert!(
        fresh.return_orphaned_snoozes().await.unwrap().is_empty(),
        "once"
    );
}
