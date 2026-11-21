//! Drafts: saved locally, mirrored to Gmail drafts, discarded, and sent after their hold.

mod support;

use std::time::Duration;

use exo_store::{
    Address, Draft, DraftAction, DraftRecord, ListId, MessageDetail, Options, Search, ThreadId,
    Unsent,
};
use gmail::{DraftQuery, Format};
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use support::Harness;

fn record(subject: &str, body: &str) -> DraftRecord {
    DraftRecord {
        action: DraftAction::Compose,
        from: Some(Address {
            name: Some("Maya Okafor".into()),
            email: support::ACCOUNT.into(),
        }),
        to: vec![Address {
            name: Some("Ann".into()),
            email: "ann@example.com".into(),
        }],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: subject.into(),
        body: body.into(),
        quoted_content: None,
        quoted_inlined: false,
        in_reply_to: None,
        in_reply_to_rfc822: None,
        references: Vec::new(),
        rfc822_id: "a.b@exo".into(),
        fingerprint: String::new(),
        attachments: Vec::new(),
        send_later: None,
        reminder: None,
    }
}

fn raw(subject: &str, body: &str) -> String {
    raw_dated(subject, body, None)
}

/// [`raw`], dated `date` (by default, when it's written).
fn raw_dated(subject: &str, body: &str, date: Option<jiff::Zoned>) -> String {
    let email = gmail::mime::Email {
        from: Some(gmail::mime::Mailbox::new("Maya Okafor", support::ACCOUNT)),
        to: vec![gmail::mime::Mailbox::new("Ann", "ann@example.com")],
        subject: subject.into(),
        html: Some(format!("<div>{body}</div>")),
        text: Some(body.into()),
        message_id: Some("a.b@exo".into()),
        date,
        ..gmail::mime::Email::default()
    };
    String::from_utf8(email.to_rfc822()).expect("MIME is ASCII")
}

fn draft(id: &str, subject: &str, body: &str) -> Draft {
    Draft {
        id: id.into(),
        thread: None,
        adopted: None,
        record: record(subject, body),
        raw: raw(subject, body),
        complete: true,
        sending: false,
    }
}

/// [`draft`] as compose saves it again `secs` from now with nothing changed: the same message,
/// dated anew.
fn saved_again(id: &str, subject: &str, body: &str, secs: i64) -> Draft {
    let date =
        (Timestamp::now() + SignedDuration::from_secs(secs)).to_zoned(jiff::tz::TimeZone::UTC);
    Draft {
        raw: raw_dated(subject, body, Some(date)),
        ..draft(id, subject, body)
    }
}

async fn gmail_drafts(harness: &Harness) -> Vec<gmail::Draft> {
    harness
        .gmail
        .gmail()
        .drafts(&DraftQuery::default())
        .await
        .expect("drafts.list")
        .drafts
}

async fn gmail_draft_subject(harness: &Harness, id: &str) -> String {
    let draft = harness
        .gmail
        .gmail()
        .draft(id, Format::Full)
        .await
        .expect("drafts.get");
    draft
        .message
        .and_then(|message| message.header("Subject").map(str::to_owned))
        .unwrap_or_default()
}

#[tokio::test]
async fn a_saved_draft_becomes_a_gmail_draft_and_updates_in_place() {
    // Written again at once: the pace has tests of its own.
    let harness = Harness::synced_with(support::unpaced()).await;
    let before = gmail_drafts(&harness).await.len();
    harness
        .store
        .save_draft(draft("draft00000000000001", "Lunch", "Hi"))
        .await
        .expect("saves");
    let stored = harness
        .store
        .draft("draft00000000000001")
        .await
        .expect("reads")
        .expect("stored");
    assert!(!stored.synced, "not in Gmail before a push");

    let report = harness.store.push_drafts().await.expect("pushes");
    assert_eq!(report.saved, ["draft00000000000001"]);
    let stored = harness
        .store
        .draft("draft00000000000001")
        .await
        .expect("reads")
        .expect("stored");
    assert!(stored.synced);
    let gmail_id = stored.gmail_id.clone().expect("a Gmail draft");
    assert_eq!(gmail_drafts(&harness).await.len(), before + 1);
    assert_eq!(gmail_draft_subject(&harness, &gmail_id).await, "Lunch");
    // The new message is a thread of its own, routed as Exo's own drafts are.
    let thread = stored.thread.clone().expect("Gmail gave it a thread");
    assert!(
        harness.ids(&ListId::Inbox).await.contains(thread.as_str()),
        "a new message Exo wrote sits in the Inbox"
    );

    harness
        .store
        .save_draft(draft("draft00000000000001", "Lunch tomorrow", "Hi there"))
        .await
        .expect("saves again");
    harness.store.push_drafts().await.expect("pushes");
    let stored = harness
        .store
        .draft("draft00000000000001")
        .await
        .expect("reads")
        .expect("stored");
    assert_eq!(
        stored.gmail_id.as_deref(),
        Some(gmail_id.as_str()),
        "the same Gmail draft"
    );
    assert_eq!(
        gmail_draft_subject(&harness, &gmail_id).await,
        "Lunch tomorrow"
    );
    assert_eq!(gmail_drafts(&harness).await.len(), before + 1);
}

#[tokio::test]
async fn a_discarded_draft_leaves_gmail_and_undo_brings_it_back() {
    let harness = Harness::synced().await;
    let before = gmail_drafts(&harness).await.len();
    let saved = draft("draft00000000000002", "Plans", "Hello");
    harness
        .store
        .save_draft(saved.clone())
        .await
        .expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    let thread = harness
        .store
        .draft("draft00000000000002")
        .await
        .expect("reads")
        .and_then(|draft| draft.thread)
        .expect("thread");

    harness
        .store
        .discard_draft("draft00000000000002")
        .await
        .expect("discards");
    assert!(
        harness
            .store
            .draft("draft00000000000002")
            .await
            .expect("reads")
            .is_none(),
        "gone at once"
    );
    assert!(
        !harness.ids(&ListId::Inbox).await.contains(thread.as_str()),
        "its row goes before Gmail hears"
    );
    harness.store.push_drafts().await.expect("pushes");
    assert_eq!(gmail_drafts(&harness).await.len(), before);

    // Undo: saving it again creates a new Gmail draft.
    harness.store.save_draft(saved).await.expect("restores");
    harness.store.push_drafts().await.expect("pushes");
    assert_eq!(gmail_drafts(&harness).await.len(), before + 1);
    assert!(
        harness
            .store
            .draft("draft00000000000002")
            .await
            .expect("reads")
            .is_some()
    );
}

#[tokio::test]
async fn a_held_send_goes_out_when_due_and_can_be_taken_back_before() {
    let harness = Harness::synced().await;
    let before = gmail_drafts(&harness).await.len();
    harness
        .store
        .save_draft(draft("draft00000000000003", "Report", "Attached"))
        .await
        .expect("saves");
    let later = Timestamp::now()
        .checked_add(jiff::SignedDuration::from_secs(20))
        .expect("in range");
    harness
        .store
        .send_draft("draft00000000000003", later)
        .await
        .expect("holds");
    let report = harness.store.push_drafts().await.expect("pushes");
    assert!(report.sent.is_empty(), "held");
    assert_eq!(
        report.saved,
        ["draft00000000000003"],
        "but saved to Gmail, so it can be sent"
    );
    assert!(
        report.retry_at.is_some_and(|at| at <= later),
        "push again when it's due"
    );

    let thread = harness
        .store
        .draft("draft00000000000003")
        .await
        .expect("reads")
        .and_then(|draft| draft.thread)
        .expect("thread");
    assert!(
        !harness.ids(&ListId::Inbox).await.contains(thread.as_str()),
        "a sending draft leaves the Inbox"
    );

    assert!(
        harness
            .store
            .cancel_send("draft00000000000003")
            .await
            .expect("cancels"),
        "undo send takes it back"
    );
    assert!(
        harness.ids(&ListId::Inbox).await.contains(thread.as_str()),
        "and it's a draft in the Inbox again"
    );
    assert!(
        !harness
            .store
            .cancel_send("draft00000000000003")
            .await
            .expect("cancels"),
        "only once"
    );

    harness
        .store
        .send_draft("draft00000000000003", Timestamp::now())
        .await
        .expect("sends now");
    let report = harness.store.push_drafts().await.expect("pushes");
    let [sent] = report.sent.as_slice() else {
        panic!("sent one: {report:?}")
    };
    assert_eq!(sent.id, "draft00000000000003");
    assert!(
        harness
            .store
            .draft("draft00000000000003")
            .await
            .expect("reads")
            .is_none(),
        "no longer a draft"
    );
    assert_eq!(
        gmail_drafts(&harness).await.len(),
        before,
        "Gmail's draft became the sent message"
    );
    let labels = harness.gmail_labels(&sent.thread).await;
    assert!(
        labels
            .iter()
            .any(|(id, labels)| *id == sent.message && labels.contains("SENT")),
        "{labels:?}"
    );
    assert!(
        harness
            .ids(&ListId::Sent)
            .await
            .contains(sent.thread.as_str()),
        "stored at once, without waiting for sync"
    );
}

#[tokio::test]
async fn unsent_counts_sends_and_every_other_change() {
    let harness = Harness::synced().await;
    let unsent = || async { harness.store.unsent().await.expect("counts") };
    assert_eq!(unsent().await, Unsent::default());

    harness
        .store
        .save_draft(draft("draft00000000000011", "Notes", "Hi"))
        .await
        .expect("saves");
    harness
        .store
        .save_draft(draft("draft00000000000012", "Report", "Attached"))
        .await
        .expect("saves");
    let later = Timestamp::now()
        .checked_add(jiff::SignedDuration::from_secs(20))
        .expect("in range");
    harness
        .store
        .send_draft("draft00000000000012", later)
        .await
        .expect("holds");
    let inbox = harness.all(&ListId::Inbox).await;
    harness
        .store
        .apply(&[inbox[0].id.clone()], exo_store::Mutation::MarkDone)
        .await
        .expect("applies");
    assert_eq!(
        unsent().await,
        Unsent {
            sends: 1,
            changes: 2
        },
        "the held send, the unsaved draft and Mark Done"
    );

    harness.store.push_drafts().await.expect("pushes");
    harness.store.flush().await.expect("flushes");
    assert_eq!(
        unsent().await,
        Unsent {
            sends: 1,
            changes: 0
        },
        "a held send counts until it's gone"
    );
    harness
        .store
        .discard_draft("draft00000000000011")
        .await
        .expect("discards");
    assert_eq!(
        unsent().await,
        Unsent {
            sends: 1,
            changes: 1
        },
        "deleting the Gmail draft is a change too"
    );
}

/// Draft `id`, a reply saying `body` to the newest message of an inbox thread without drafts,
/// and that thread.
async fn reply(harness: &Harness, id: &str, body: &str) -> (Draft, ThreadId) {
    let inbox = harness.all(&ListId::Inbox).await;
    let thread = inbox
        .iter()
        .find(|thread| thread.drafts == 0)
        .expect("a thread without drafts")
        .id
        .clone();
    let detail = harness
        .store
        .thread(&thread)
        .await
        .expect("reads")
        .expect("stored");
    let last = detail
        .messages
        .iter()
        .rev()
        .find(|message| !message.is_draft())
        .expect("a message");
    let subject = gmail::mime::reply_subject(&detail.subject);
    let mut reply = draft(id, &subject, body);
    let email = gmail::mime::Email {
        to: vec![gmail::mime::Mailbox::address("ann@example.com")],
        subject,
        html: Some(format!("<div>{body}</div>")),
        message_id: Some("c.d@exo".into()),
        in_reply_to: last.rfc822_id.clone(),
        references: last
            .references
            .iter()
            .cloned()
            .chain(last.rfc822_id.clone())
            .collect(),
        ..gmail::mime::Email::default()
    };
    reply.raw = String::from_utf8(email.to_rfc822()).expect("ASCII");
    reply.thread = Some(ThreadId::new(thread.as_str()));
    reply.record.action = DraftAction::Reply;
    reply.record.in_reply_to = Some(last.id.clone());
    (reply, thread)
}

/// Draft `id`, a new message saying `body` with a small PDF listed.
fn with_file(id: &str, subject: &str, body: &str) -> Draft {
    let mut saved = draft(id, subject, body);
    saved.record.attachments.push(exo_store::DraftAttachment {
        uuid: "deck".into(),
        name: "deck.pdf".into(),
        mime_type: "application/pdf".into(),
        size: 4,
        content_id: None,
        source: exo_store::AttachmentSource::Local,
    });
    let email = gmail::mime::Email {
        to: vec![gmail::mime::Mailbox::address("ann@example.com")],
        subject: subject.into(),
        html: Some(format!("<div>{body}</div>")),
        message_id: Some("a.b@exo".into()),
        attachments: vec![gmail::mime::Attachment {
            filename: "deck.pdf".into(),
            content_type: "application/pdf".into(),
            data: b"%PDF".to_vec(),
            content_id: None,
        }],
        ..gmail::mime::Email::default()
    };
    saved.raw = String::from_utf8(email.to_rfc822()).expect("ASCII");
    saved
}

/// Gmail quota units the store has spent so far (`/__mock/quota`).
async fn units(harness: &Harness) -> u64 {
    harness
        .admin(
            reqwest::Method::GET,
            "/__mock/quota",
            serde_json::Value::Null,
        )
        .await["units"]
        .as_u64()
        .expect("units")
}

#[tokio::test]
async fn a_reply_draft_joins_its_thread() {
    let harness = Harness::synced().await;
    let (reply, thread) = reply(&harness, "draft00000000000004", "Sounds good").await;
    harness.store.save_draft(reply).await.expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    let stored = harness
        .store
        .draft("draft00000000000004")
        .await
        .expect("reads")
        .expect("stored");
    assert_eq!(stored.thread.as_ref(), Some(&thread));
    let summary = harness
        .summary(&ListId::Inbox, &thread)
        .await
        .expect("still listed");
    assert_eq!(summary.drafts, 1, "the row shows Exo's reply draft");
}

#[tokio::test]
async fn aliases_carry_their_signatures() {
    let harness = Harness::synced().await;
    let aliases = harness.store.aliases().await.expect("reads");
    let primary = aliases
        .iter()
        .find(|alias| alias.primary)
        .expect("the primary address");
    assert_eq!(primary.email, support::ACCOUNT);
    assert!(primary.default && primary.active);
    assert!(
        primary
            .signature
            .as_deref()
            .is_some_and(|signature| signature.contains("brightmoor.co"))
    );
    assert!(
        aliases.iter().any(|alias| !alias.primary && alias.active),
        "a verified alias"
    );
}

#[tokio::test]
async fn drafts_written_elsewhere_come_with_their_bodies() {
    let harness = Harness::synced().await;
    let coffee = harness
        .all(&ListId::Drafts)
        .await
        .into_iter()
        .find(|thread| thread.subject == "Coffee next week?")
        .expect("a Gmail-web reply draft");
    let detail = harness
        .store
        .thread(&coffee.id)
        .await
        .expect("reads")
        .expect("stored");
    let draft = detail
        .messages
        .iter()
        .find(|message| message.is_draft())
        .expect("the draft");
    assert!(
        draft
            .body
            .html
            .as_deref()
            .is_some_and(|html| html.contains("Wednesday morning")),
        "{:?}",
        draft.body
    );
}

#[tokio::test]
async fn kept_files_go_once_no_draft_refers_to_them() {
    let harness = Harness::synced().await;
    let folder = harness.store.draft_files().to_owned();
    std::fs::create_dir_all(&folder).expect("a folder");
    let mut kept = draft("draft00000000000007", "Files", "<div>See attached</div>");
    kept.record.attachments.push(exo_store::DraftAttachment {
        uuid: "kept".into(),
        name: "plan.csv".into(),
        mime_type: "text/csv".into(),
        size: 3,
        content_id: None,
        source: exo_store::AttachmentSource::Local,
    });
    harness.store.save_draft(kept).await.expect("saves");
    std::fs::write(folder.join("kept"), "a,b").expect("written");
    std::fs::write(folder.join("orphan"), "x").expect("written");
    harness.store.prune_draft_files().await.expect("prunes");
    assert!(folder.join("kept").exists());
    assert!(!folder.join("orphan").exists());
}

/// Gmail's sent messages carrying Message-ID `id`.
async fn sent_with_id(harness: &Harness, id: &str) -> usize {
    let query = gmail::ListQuery {
        q: Some(format!("rfc822msgid:{id}")),
        label_ids: vec!["SENT".into()],
        ..gmail::ListQuery::default()
    };
    harness
        .gmail
        .gmail()
        .messages(&query)
        .await
        .expect("messages.list")
        .messages
        .len()
}

/// Lets the outbox's backoff (5 ms in tests) run out.
async fn past_backoff() {
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
}

#[tokio::test]
async fn undo_send_while_the_push_writes_the_final_version_keeps_it_unsent() {
    let mut harness = Harness::synced().await;
    let gate = harness.reopen_gated().await;
    harness
        .store
        .save_draft(draft("draft00000000000011", "Offer", "Final terms"))
        .await
        .expect("saves");
    harness
        .store
        .send_draft("draft00000000000011", Timestamp::now())
        .await
        .expect("holds");
    // The push writes the draft to Gmail before sending it; hold it on that write.
    gate.hold_after(0);
    let store = harness.store.clone();
    let push = tokio::spawn(async move { store.push_drafts().await.expect("pushes") });
    gate.held().await;
    let undone = harness
        .store
        .cancel_send("draft00000000000011")
        .await
        .expect("cancels");
    gate.release();
    let report = push.await.expect("the push ends");
    assert!(undone, "undo came before anything was sent");
    assert!(
        report.sent.is_empty(),
        "an undone send stays unsent: {report:?}"
    );
    assert_eq!(
        sent_with_id(&harness, "a.b@exo").await,
        0,
        "Gmail sent nothing"
    );
    assert!(
        harness
            .store
            .draft("draft00000000000011")
            .await
            .expect("reads")
            .is_some_and(|draft| draft.send_at.is_none()),
        "a draft again"
    );
}

#[tokio::test]
async fn undo_send_while_gmail_is_sending_says_it_went() {
    let mut harness = Harness::synced().await;
    let gate = harness.reopen_gated().await;
    harness
        .store
        .save_draft(draft("draft00000000000012", "Offer", "Final terms"))
        .await
        .expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    harness
        .store
        .send_draft("draft00000000000012", Timestamp::now())
        .await
        .expect("holds");
    // Hold `drafts.send` itself.
    gate.hold_after(0);
    let store = harness.store.clone();
    let push = tokio::spawn(async move { store.push_drafts().await.expect("pushes") });
    gate.held().await;
    let store = harness.store.clone();
    let undo = tokio::spawn(async move {
        store
            .cancel_send("draft00000000000012")
            .await
            .expect("cancels")
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    gate.release();
    let report = push.await.expect("the push ends");
    let undone = undo.await.expect("the undo ends");
    let sent = sent_with_id(&harness, "a.b@exo").await;
    assert_eq!(sent, report.sent.len(), "{report:?}");
    assert_eq!(
        sent == 0,
        undone,
        "undo says it took the send back exactly when nothing went (sent {sent}, undone {undone})"
    );
}

#[tokio::test]
async fn a_send_whose_answer_was_lost_is_not_reported_as_failed() {
    let harness = Harness::synced().await;
    harness
        .store
        .save_draft(draft("draft00000000000013", "Invoice", "Attached"))
        .await
        .expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    harness
        .store
        .send_draft("draft00000000000013", Timestamp::now())
        .await
        .expect("holds");
    // Gmail sends it, but the answer never arrives; trying again finds it sent.
    harness.fault(serde_json::json!({"method": "POST", "path": "/gmail/v1/users/me/drafts/send", "status": 503, "after": true})).await;
    let report = harness.store.push_drafts().await.expect("pushes");
    assert!(
        report.failed.is_empty(),
        "it went, so sending didn't fail: {report:?}"
    );
    let [sent] = report.sent.as_slice() else {
        panic!("reported as sent: {report:?}")
    };
    assert_eq!(sent.id, "draft00000000000013");
    assert!(
        harness
            .store
            .draft("draft00000000000013")
            .await
            .expect("reads")
            .is_none(),
        "no longer a draft"
    );
    assert_eq!(sent_with_id(&harness, "a.b@exo").await, 1, "sent once");
}

#[tokio::test]
async fn a_draft_whose_creation_answer_was_lost_is_not_created_twice() {
    let harness = Harness::synced().await;
    let before = gmail_drafts(&harness).await.len();
    harness
        .store
        .save_draft(draft("draft00000000000014", "Agenda", "Draft one"))
        .await
        .expect("saves");
    harness.fault(serde_json::json!({"method": "POST", "path": "/upload/gmail/v1/users/me/drafts", "status": 503, "after": true})).await;
    let report = harness.store.push_drafts().await.expect("pushes");
    assert!(
        report.saved.is_empty() && report.retry_at.is_some(),
        "retried later: {report:?}"
    );
    past_backoff().await;
    let report = harness.store.push_drafts().await.expect("pushes");
    assert_eq!(report.saved, ["draft00000000000014"]);
    assert_eq!(
        gmail_drafts(&harness).await.len(),
        before + 1,
        "one Gmail draft, not two"
    );
    let stored = harness
        .store
        .draft("draft00000000000014")
        .await
        .expect("reads")
        .expect("stored");
    assert_eq!(
        gmail_draft_subject(&harness, &stored.gmail_id.expect("a Gmail draft")).await,
        "Agenda"
    );
}

#[tokio::test]
async fn a_draft_sent_deleted_or_rewritten_elsewhere_leaves_exos_drafts() {
    let harness = Harness::synced().await;
    let ids = [
        "draft00000000000021",
        "draft00000000000022",
        "draft00000000000023",
    ];
    for id in ids {
        harness
            .store
            .save_draft(draft(id, "Elsewhere", "Written in Exo"))
            .await
            .expect("saves");
    }
    harness.store.push_drafts().await.expect("pushes");
    let mut gmail_ids = Vec::new();
    for id in ids {
        gmail_ids.push(
            harness
                .store
                .draft(id)
                .await
                .expect("reads")
                .and_then(|draft| draft.gmail_id)
                .expect("a Gmail draft"),
        );
    }
    let [sent, deleted, rewritten] = gmail_ids.try_into().expect("three");
    // Sent from Gmail on the phone, deleted in Gmail web, finished in Gmail web.
    harness
        .gmail
        .gmail()
        .send_draft(&sent)
        .await
        .expect("drafts.send");
    harness
        .gmail
        .gmail()
        .delete_draft(&deleted)
        .await
        .expect("drafts.delete");
    harness
        .gmail
        .gmail()
        .update_draft(
            &rewritten,
            raw("Elsewhere", "Finished in Gmail").into_bytes(),
            None,
        )
        .await
        .expect("drafts.update");
    harness.store.sync().await.expect("syncs");
    for id in ids {
        assert!(
            harness.store.draft(id).await.expect("reads").is_none(),
            "{id} would reopen in compose with what Gmail no longer has"
        );
    }
    // The rewritten one is still a Gmail draft, which compose opens as one written elsewhere.
    assert!(
        gmail_drafts(&harness)
            .await
            .iter()
            .any(|draft| draft.id.as_deref() == Some(rewritten.as_str()))
    );
}

#[tokio::test]
async fn a_draft_with_changes_gmail_lacks_stays_when_its_gmail_draft_goes() {
    let harness = Harness::synced().await;
    harness
        .store
        .save_draft(draft("draft00000000000024", "Kept", "One"))
        .await
        .expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    let gmail_id = harness
        .store
        .draft("draft00000000000024")
        .await
        .expect("reads")
        .and_then(|draft| draft.gmail_id)
        .expect("a Gmail draft");
    // Edited in Exo while offline, deleted in Gmail web meanwhile.
    harness
        .store
        .save_draft(draft("draft00000000000024", "Kept", "Two"))
        .await
        .expect("saves");
    harness
        .gmail
        .gmail()
        .delete_draft(&gmail_id)
        .await
        .expect("drafts.delete");
    harness.store.sync().await.expect("syncs");
    assert!(
        harness
            .store
            .draft("draft00000000000024")
            .await
            .expect("reads")
            .is_some(),
        "unsaved changes aren't dropped"
    );
}

#[tokio::test]
async fn a_draft_taken_over_from_gmail_web_updates_that_draft_once_it_can_be_found() {
    let harness = Harness::synced().await;
    let written = harness
        .gmail
        .gmail()
        .create_draft(raw("Taken over", "Written in Gmail web").into_bytes(), None)
        .await
        .expect("drafts.create");
    let message = written
        .message
        .and_then(|message| message.id)
        .expect("its message");
    let before = gmail_drafts(&harness).await.len();
    let taken = Draft {
        adopted: Some(message.clone()),
        ..draft("draft00000000000031", "Taken over", "Finished in Exo")
    };
    harness.store.save_draft(taken).await.expect("saves");
    assert_eq!(
        harness
            .store
            .draft("draft00000000000031")
            .await
            .expect("reads")
            .and_then(|draft| draft.message_id),
        Some(message),
        "it holds the message it took over, so compose won't take it again"
    );
    // Gmail's drafts can't be listed: the push waits rather than make a second draft.
    harness.fault(serde_json::json!({"method": "GET", "path": "/gmail/v1/users/me/drafts", "status": 503, "times": 3})).await;
    let report = harness.store.push_drafts().await.expect("pushes");
    assert!(
        report.saved.is_empty() && report.retry_at.is_some(),
        "retried later: {report:?}"
    );
    assert_eq!(
        gmail_drafts(&harness).await.len(),
        before,
        "no second draft"
    );
    past_backoff().await;
    let report = harness.store.push_drafts().await.expect("pushes");
    assert_eq!(report.saved, ["draft00000000000031"]);
    assert_eq!(gmail_drafts(&harness).await.len(), before, "still one");
    assert_eq!(
        gmail_draft_subject(&harness, written.id.as_deref().expect("an id")).await,
        "Taken over"
    );
    let stored = harness
        .store
        .draft("draft00000000000031")
        .await
        .expect("reads")
        .expect("stored");
    assert_eq!(stored.gmail_id, written.id, "the draft it took over");
}

#[tokio::test]
async fn a_draft_saved_without_its_files_waits_for_a_save_with_them() {
    let harness = Harness::synced().await;
    let before = gmail_drafts(&harness).await.len();
    let incomplete = Draft {
        complete: false,
        ..draft("draft00000000000032", "Files", "Without its file")
    };
    harness.store.save_draft(incomplete).await.expect("saves");
    let report = harness.store.push_drafts().await.expect("pushes");
    assert!(
        report.saved.is_empty() && report.retry_at.is_none(),
        "not written, and nothing to wait for: {report:?}"
    );
    assert_eq!(gmail_drafts(&harness).await.len(), before);
    harness
        .store
        .save_draft(draft("draft00000000000032", "Files", "With it"))
        .await
        .expect("saves");
    let report = harness.store.push_drafts().await.expect("pushes");
    assert_eq!(report.saved, ["draft00000000000032"]);
}

#[tokio::test]
async fn each_save_while_typing_costs_its_write_and_the_new_versions_row() {
    // Every save written at once: what one write costs, apart from the pace.
    let harness = Harness::synced_with(support::unpaced()).await;
    let id = "draft00000000000041";
    let (first, thread) = reply(&harness, id, "Sounds").await;
    harness.store.save_draft(first).await.expect("saves");
    let spent = units(&harness).await;
    harness.store.push_drafts().await.expect("pushes");
    assert_eq!(
        units(&harness).await - spent,
        10 + 40,
        "drafts.create, then the thread's rows"
    );
    for body in ["Sounds good, see", "Sounds good, see you then"] {
        let (again, _) = reply(&harness, id, body).await;
        harness.store.save_draft(again).await.expect("saves");
        let spent = units(&harness).await;
        harness.store.push_drafts().await.expect("pushes");
        assert_eq!(
            units(&harness).await - spent,
            15 + 20,
            "drafts.update and the new version's row (messages.get, metadata), not the thread in full (120)"
        );
    }
    // The thread holds Gmail's latest version alone, as Exo's: nothing for compose to take up as
    // a draft written elsewhere.
    let stored = harness
        .store
        .draft(id)
        .await
        .expect("reads")
        .expect("stored");
    let detail = harness
        .store
        .thread(&thread)
        .await
        .expect("reads")
        .expect("stored");
    let drafts: Vec<&MessageDetail> = detail
        .messages
        .iter()
        .filter(|message| message.is_draft())
        .collect();
    assert_eq!(
        drafts
            .iter()
            .map(|message| Some(message.id.as_str()))
            .collect::<Vec<_>>(),
        [stored.message_id.as_deref()]
    );
    assert!(
        drafts[0].snippet.contains("see you then"),
        "{:?}",
        drafts[0].snippet
    );
    // Nothing is fetched for it later: history finds it stored, and compose has its body.
    assert!(
        harness.store.bodies_stored(&thread).await.expect("reads"),
        "opening the thread fetches nothing"
    );
    let spent = units(&harness).await;
    harness.store.sync().await.expect("syncs");
    assert_eq!(units(&harness).await - spent, 2, "a history check alone");
}

#[tokio::test]
async fn a_new_messages_saves_cost_as_much_and_leave_sync_nothing_to_fetch() {
    // Every save written at once: what one write costs, apart from the pace.
    let harness = Harness::synced_with(support::unpaced()).await;
    let id = "draft00000000000044";
    harness
        .store
        .save_draft(draft(id, "Plans", "Hi"))
        .await
        .expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    for body in ["Hi Ann", "Hi Ann, about Friday"] {
        harness
            .store
            .save_draft(draft(id, "Plans", body))
            .await
            .expect("saves");
        let spent = units(&harness).await;
        harness.store.push_drafts().await.expect("pushes");
        assert_eq!(units(&harness).await - spent, 15 + 20);
    }
    let thread = harness
        .store
        .draft(id)
        .await
        .expect("reads")
        .and_then(|draft| draft.thread)
        .expect("a thread");
    let row = harness
        .summary(&ListId::Inbox, &thread)
        .await
        .expect("in the Inbox");
    assert_eq!(
        (row.drafts, row.messages),
        (1, 1),
        "the latest version alone"
    );
    // gmail-mock starts a new thread for each version of a message that isn't a reply; history's
    // record of the earlier ones, added and gone since, isn't fetched.
    let spent = units(&harness).await;
    harness.store.sync().await.expect("syncs");
    assert_eq!(units(&harness).await - spent, 2, "a history check alone");
}

#[tokio::test]
async fn a_send_costs_the_send_and_the_sent_message() {
    let harness = Harness::synced().await;
    let (reply, thread) = reply(&harness, "draft00000000000042", "On my way").await;
    harness.store.save_draft(reply).await.expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    harness
        .store
        .send_draft("draft00000000000042", Timestamp::now())
        .await
        .expect("holds");
    let spent = units(&harness).await;
    let report = harness.store.push_drafts().await.expect("pushes");
    assert_eq!(report.sent.len(), 1, "{report:?}");
    assert_eq!(
        units(&harness).await - spent,
        100 + 60,
        "drafts.send, then the sent message in full (messages.get), not the thread (120)"
    );
    let detail = harness
        .store
        .thread(&thread)
        .await
        .expect("reads")
        .expect("stored");
    assert!(
        !detail.messages.iter().any(MessageDetail::is_draft),
        "no draft left"
    );
    assert!(
        detail
            .messages
            .iter()
            .any(|message| message.id == report.sent[0].message),
        "the sent message is in"
    );
    assert!(
        harness.store.bodies_stored(&thread).await.expect("reads"),
        "with its body, for the thread on screen"
    );
    assert!(harness.ids(&ListId::Sent).await.contains(thread.as_str()));
}

#[tokio::test]
async fn a_draft_with_files_shows_them_on_its_row() {
    // Every save written at once: what one write costs, apart from the pace.
    let harness = Harness::synced_with(support::unpaced()).await;
    let id = "draft00000000000043";
    harness
        .store
        .save_draft(with_file(id, "Deck", "Attached"))
        .await
        .expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    harness
        .store
        .save_draft(with_file(id, "Deck", "Attached, v2"))
        .await
        .expect("saves");
    let spent = units(&harness).await;
    harness.store.push_drafts().await.expect("pushes");
    assert_eq!(
        units(&harness).await - spent,
        15 + 60,
        "drafts.update and the new version in full, for its files"
    );
    let thread = harness
        .store
        .draft(id)
        .await
        .expect("reads")
        .and_then(|draft| draft.thread)
        .expect("a thread");
    let row = harness
        .summary(&ListId::Inbox, &thread)
        .await
        .expect("listed");
    assert_eq!(
        row.attachments,
        exo_store::Attachments::Files,
        "the paperclip"
    );
}

/// [`support::options`] with drafts paced `plain` apart, and `files` apart with files.
fn paced(plain: Duration, files: Duration) -> Options {
    Options {
        draft_pace: plain,
        draft_files_pace: files,
        ..support::options()
    }
}

/// Sleeps until `at`.
async fn until(at: Timestamp) {
    tokio::time::sleep(Duration::try_from(at.duration_since(Timestamp::now())).unwrap_or_default())
        .await;
}

#[tokio::test]
async fn a_draft_being_typed_in_waits_its_pace_then_its_latest_save_goes() {
    let harness =
        Harness::synced_with(paced(Duration::from_millis(300), Duration::from_secs(60))).await;
    let id = "draft00000000000051";
    harness
        .store
        .save_draft(draft(id, "Pace 1", "One"))
        .await
        .expect("saves");
    assert_eq!(
        harness.store.push_drafts().await.expect("pushes").saved,
        [id],
        "the first write goes at once"
    );
    let gmail_id = harness
        .store
        .draft(id)
        .await
        .expect("reads")
        .and_then(|draft| draft.gmail_id)
        .expect("a Gmail draft");
    let mut retry_at = None;
    for subject in ["Pace 2", "Pace 3"] {
        harness
            .store
            .save_draft(draft(id, subject, "One two"))
            .await
            .expect("saves");
        let report = harness.store.push_drafts().await.expect("pushes");
        assert!(
            report.saved.is_empty(),
            "an edit within the pace waits: {report:?}"
        );
        retry_at = report.retry_at;
    }
    assert_eq!(gmail_draft_subject(&harness, &gmail_id).await, "Pace 1");
    assert!(
        !harness
            .store
            .draft(id)
            .await
            .expect("reads")
            .expect("stored")
            .synced,
        "the device has the edits"
    );
    let retry_at = retry_at.expect("pushed again once the pace is up");
    assert!(
        retry_at <= Timestamp::now() + SignedDuration::from_millis(300),
        "{retry_at}"
    );
    until(retry_at).await;
    assert_eq!(
        harness.store.push_drafts().await.expect("pushes").saved,
        [id]
    );
    assert_eq!(
        gmail_draft_subject(&harness, &gmail_id).await,
        "Pace 3",
        "the latest save went"
    );
}

#[tokio::test]
async fn a_draft_with_files_waits_its_own_longer_pace() {
    let harness =
        Harness::synced_with(paced(Duration::from_millis(50), Duration::from_secs(60))).await;
    let id = "draft00000000000052";
    harness
        .store
        .save_draft(with_file(id, "Deck", "Attached"))
        .await
        .expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    harness
        .store
        .save_draft(with_file(id, "Deck", "Attached, v2"))
        .await
        .expect("saves");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let report = harness.store.push_drafts().await.expect("pushes");
    assert!(
        report.saved.is_empty(),
        "past a plain draft's pace, not past one with files: {report:?}"
    );
    assert!(
        report
            .retry_at
            .is_some_and(|at| at > Timestamp::now() + SignedDuration::from_secs(50)),
        "{report:?}"
    );
}

/// Writes draft `id` to Gmail as `Before`, then saves it as `After`, an edit that waits out the
/// pace (5 s). Returns its Gmail draft.
async fn edited_within_the_pace(harness: &Harness, id: &str) -> String {
    harness
        .store
        .save_draft(draft(id, "Before", "Written"))
        .await
        .expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    harness
        .store
        .save_draft(draft(id, "After", "Edited"))
        .await
        .expect("saves");
    assert!(
        harness
            .store
            .push_drafts()
            .await
            .expect("pushes")
            .saved
            .is_empty(),
        "the edit waits"
    );
    harness
        .store
        .draft(id)
        .await
        .expect("reads")
        .and_then(|draft| draft.gmail_id)
        .expect("a Gmail draft")
}

#[tokio::test]
async fn sending_right_after_typing_sends_the_latest_text() {
    let harness = Harness::synced().await;
    let id = "draft00000000000053";
    edited_within_the_pace(&harness, id).await;
    harness
        .store
        .send_draft(id, Timestamp::now())
        .await
        .expect("sends");
    let report = harness.store.push_drafts().await.expect("pushes");
    let [sent] = report.sent.as_slice() else {
        panic!("sent: {report:?}")
    };
    let message = harness
        .gmail
        .gmail()
        .message(&sent.message, Format::Metadata, &["Subject"])
        .await
        .expect("messages.get");
    assert_eq!(message.header("Subject"), Some("After"));
}

#[tokio::test]
async fn undo_sends_hold_writes_the_latest_text_at_once() {
    let harness = Harness::synced().await;
    let id = "draft00000000000054";
    let gmail_id = edited_within_the_pace(&harness, id).await;
    let later = Timestamp::now() + SignedDuration::from_secs(20);
    harness.store.send_draft(id, later).await.expect("holds");
    assert_eq!(
        harness.store.push_drafts().await.expect("pushes").saved,
        [id],
        "written for its send, during the hold"
    );
    assert_eq!(gmail_draft_subject(&harness, &gmail_id).await, "After");
}

#[tokio::test]
async fn discarding_and_leaving_the_account_dont_wait_for_the_pace() {
    let harness = Harness::synced().await;
    let discarded = "draft00000000000055";
    let gmail_id = edited_within_the_pace(&harness, discarded).await;
    harness
        .store
        .discard_draft(discarded)
        .await
        .expect("discards");
    harness.store.push_drafts().await.expect("pushes");
    assert!(
        !gmail_drafts(&harness)
            .await
            .iter()
            .any(|draft| draft.id.as_deref() == Some(gmail_id.as_str())),
        "deleted at once"
    );

    let kept = "draft00000000000056";
    let gmail_id = edited_within_the_pace(&harness, kept).await;
    assert_eq!(
        harness.store.push_drafts_now().await.expect("pushes").saved,
        [kept],
        "quitting, switching away and signing out write it now"
    );
    assert_eq!(gmail_draft_subject(&harness, &gmail_id).await, "After");
}

#[tokio::test]
async fn editing_a_failed_draft_writes_its_first_edit_at_once() {
    let harness = Harness::synced().await;
    let id = "draft00000000000057";
    harness
        .store
        .save_draft(draft(id, "Before", "Written"))
        .await
        .expect("saves");
    harness
        .store
        .send_draft(id, Timestamp::now())
        .await
        .expect("sends");
    harness.fault(json!({"method": "POST", "path": "/gmail/v1/users/me/drafts/send", "status": 400, "message": "Invalid To header", "reason": "invalidArgument", "times": 1})).await;
    let report = harness.store.push_drafts().await.expect("pushes");
    assert_eq!(
        (report.saved.len(), report.failed.len()),
        (1, 1),
        "written for its send, which Gmail refused: {report:?}"
    );
    harness.store.recover_draft(id).await.expect("Edit Message");
    harness
        .store
        .save_draft(draft(id, "After", "Fixed"))
        .await
        .expect("saves");
    assert_eq!(
        harness.store.push_drafts().await.expect("pushes").saved,
        [id],
        "not held back by the write its send just made"
    );
}

/// The threads an offline search for `query` finds.
async fn found_offline(harness: &Harness, query: &str) -> Vec<ThreadId> {
    let page = harness
        .store
        .search(&mut Search::new(query), 100, false, 15)
        .await
        .expect("searches");
    page.results
        .into_iter()
        .map(|result| result.thread.id)
        .collect()
}

#[tokio::test]
async fn an_offline_search_finds_what_was_typed_in_a_draft() {
    let harness = Harness::synced().await;
    let id = "draft00000000000058";
    let (typed, thread) = reply(&harness, id, "Bring the zanzibar slides").await;
    harness
        .store
        .save_draft(typed.clone())
        .await
        .expect("saves");
    assert_eq!(
        found_offline(&harness, "zanzibar").await,
        std::slice::from_ref(&thread),
        "typed, before Gmail has it"
    );
    harness.store.push_drafts().await.expect("pushes");
    assert_eq!(
        found_offline(&harness, "zanzibar").await,
        std::slice::from_ref(&thread),
        "in Gmail, which Exo stores without its body"
    );

    let mut edited = typed;
    edited.record.body = "<div>Bring the quixotic slides</div>".into();
    harness.store.save_draft(edited).await.expect("saves");
    assert_eq!(
        found_offline(&harness, "quixotic").await,
        std::slice::from_ref(&thread),
        "each save, written to Gmail or not"
    );
    assert!(
        found_offline(&harness, "zanzibar").await.is_empty(),
        "what was deleted goes"
    );

    harness.store.discard_draft(id).await.expect("discards");
    assert!(
        found_offline(&harness, "quixotic").await.is_empty(),
        "discarded"
    );

    // Sent, it's found as the sent message, stored whole.
    let (sent, thread) = reply(&harness, "draft00000000000059", "The xylophone is booked").await;
    harness.store.save_draft(sent).await.expect("saves");
    harness
        .store
        .send_draft("draft00000000000059", Timestamp::now())
        .await
        .expect("sends");
    assert_eq!(
        harness
            .store
            .push_drafts()
            .await
            .expect("pushes")
            .sent
            .len(),
        1
    );
    assert_eq!(found_offline(&harness, "xylophone").await, [thread]);
}

/// A tenth of real time: compose's save a second is a save every 100 ms, and paces of 5 s and
/// 30 s are 0.5 s and 3 s.
const TENTH: u32 = 10;

/// A minute of typing in draft `id` at a tenth of the time: once written to Gmail and the pace
/// past, `version(n)` saved a second apart (compose's `SAVE_EVERY`) 60 times, pushed after each
/// save and whenever a push asks to be woken, as the account's push loop does, until Gmail has
/// the last. Returns the writes made while typing and in all, and the units they cost.
async fn a_minute_of_typing(
    harness: &Harness,
    id: &str,
    version: impl Fn(usize) -> Draft,
) -> (usize, usize, u64) {
    harness.store.save_draft(version(0)).await.expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    tokio::time::sleep(Duration::from_secs(30) / TENTH).await;
    let every = Duration::from_secs(1) / TENTH;
    let spent = units(harness).await;
    let started = tokio::time::Instant::now();
    let minute = started + every * 60;
    let (mut saves, mut writes, mut typing) = (0, 0, 0);
    let mut retry_at = None::<Timestamp>;
    loop {
        let save_at = (saves < 60).then(|| started + every * saves);
        let push_at = retry_at.map(|at| {
            tokio::time::Instant::now()
                + Duration::try_from(at.duration_since(Timestamp::now())).unwrap_or_default()
        });
        let Some(wake) = save_at.into_iter().chain(push_at).min() else {
            break;
        };
        tokio::time::sleep_until(wake).await;
        if save_at.is_some_and(|at| at <= tokio::time::Instant::now()) {
            saves += 1;
            harness
                .store
                .save_draft(version(saves as usize))
                .await
                .expect("saves");
        }
        let report = harness.store.push_drafts().await.expect("pushes");
        assert!(report.saved.iter().all(|saved| saved == id), "{report:?}");
        writes += report.saved.len();
        if tokio::time::Instant::now() < minute {
            typing = writes;
        }
        retry_at = report.retry_at;
    }
    assert!(
        harness
            .store
            .draft(id)
            .await
            .expect("reads")
            .expect("stored")
            .synced,
        "Gmail has the last save"
    );
    (typing, writes, units(harness).await - spent)
}

#[tokio::test]
async fn a_minute_of_typing_writes_a_draft_about_twelve_times() {
    let harness = Harness::synced_with(paced(
        Duration::from_secs(5) / TENTH,
        Duration::from_secs(30) / TENTH,
    ))
    .await;
    let id = "draft00000000000061";
    let (typing, writes, units) =
        a_minute_of_typing(&harness, id, |n| draft(id, &format!("Typing {n}"), "Hello")).await;
    // At 0 s, 5 s, … 55 s, and the last save once the minute's last pace is up.
    assert!(
        (11..=12).contains(&typing),
        "{typing} writes while typing, not 60"
    );
    assert!((12..=13).contains(&writes), "{writes} with the last save");
    assert_eq!(
        units,
        35 * writes as u64,
        "drafts.update and the new version's row each"
    );
    let gmail_id = harness
        .store
        .draft(id)
        .await
        .expect("reads")
        .and_then(|draft| draft.gmail_id)
        .expect("a Gmail draft");
    assert_eq!(gmail_draft_subject(&harness, &gmail_id).await, "Typing 60");
}

#[tokio::test]
async fn a_minute_of_typing_writes_a_draft_with_files_twice() {
    let harness = Harness::synced_with(paced(
        Duration::from_secs(5) / TENTH,
        Duration::from_secs(30) / TENTH,
    ))
    .await;
    let id = "draft00000000000062";
    let (typing, writes, units) = a_minute_of_typing(&harness, id, |n| {
        with_file(id, &format!("Typing {n}"), "Attached")
    })
    .await;
    // At 0 s and 30 s, and the last save once the pace is up again.
    assert_eq!((typing, writes), (2, 3));
    assert_eq!(
        units,
        75 * writes as u64,
        "drafts.update and the new version in full each"
    );
    let gmail_id = harness
        .store
        .draft(id)
        .await
        .expect("reads")
        .and_then(|draft| draft.gmail_id)
        .expect("a Gmail draft");
    assert_eq!(gmail_draft_subject(&harness, &gmail_id).await, "Typing 60");
}

/// The message Gmail's draft `id` holds.
async fn gmail_draft_message(harness: &Harness, id: &str) -> Option<String> {
    harness
        .gmail
        .gmail()
        .draft(id, Format::Minimal)
        .await
        .expect("drafts.get")
        .message
        .and_then(|message| message.id)
}

#[tokio::test]
async fn a_save_gmail_has_already_writes_nothing() {
    let mut harness = Harness::synced().await;
    let id = "draft00000000000071";
    harness
        .store
        .save_draft(draft(id, "Unchanged", "Hello"))
        .await
        .expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    let gmail_id = harness
        .store
        .draft(id)
        .await
        .expect("reads")
        .and_then(|draft| draft.gmail_id)
        .expect("a Gmail draft");
    let message = gmail_draft_message(&harness, &gmail_id).await;
    let spent = units(&harness).await;
    // Saved again with nothing changed (left with Escape, quitting): only its Date differs.
    harness
        .store
        .save_draft(saved_again(id, "Unchanged", "Hello", 60))
        .await
        .expect("saves");
    assert!(
        harness
            .store
            .draft(id)
            .await
            .expect("reads")
            .expect("stored")
            .synced,
        "Gmail has it"
    );
    assert!(
        harness
            .store
            .push_drafts()
            .await
            .expect("pushes")
            .saved
            .is_empty()
    );
    assert!(
        harness
            .store
            .push_drafts_now()
            .await
            .expect("pushes")
            .saved
            .is_empty(),
        "quitting, switching away and signing out write nothing either"
    );
    // Relaunched, when a write would go at once: the draft's row says what Gmail has.
    harness.reopen().await;
    harness
        .store
        .save_draft(saved_again(id, "Unchanged", "Hello", 120))
        .await
        .expect("saves");
    assert!(
        harness
            .store
            .push_drafts()
            .await
            .expect("pushes")
            .saved
            .is_empty(),
        "nothing written after a relaunch"
    );
    assert_eq!(units(&harness).await - spent, 0);
    assert_eq!(
        gmail_draft_message(&harness, &gmail_id).await,
        message,
        "Gmail's draft is as it was"
    );
}

#[tokio::test]
async fn an_edit_undone_before_its_write_writes_nothing() {
    let harness = Harness::synced().await;
    let id = "draft00000000000072";
    harness
        .store
        .save_draft(draft(id, "Undone", "Hello"))
        .await
        .expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    let spent = units(&harness).await;
    harness
        .store
        .save_draft(draft(id, "Undone", "Hello there"))
        .await
        .expect("saves");
    assert!(
        harness
            .store
            .push_drafts()
            .await
            .expect("pushes")
            .saved
            .is_empty(),
        "the edit waits out its pace"
    );
    harness
        .store
        .save_draft(saved_again(id, "Undone", "Hello", 2))
        .await
        .expect("saves");
    assert!(
        harness
            .store
            .draft(id)
            .await
            .expect("reads")
            .expect("stored")
            .synced,
        "back to what Gmail has"
    );
    assert!(
        harness
            .store
            .push_drafts_now()
            .await
            .expect("pushes")
            .saved
            .is_empty()
    );
    assert_eq!(units(&harness).await - spent, 0);
}

#[tokio::test]
async fn an_edit_undone_while_the_edit_is_being_written_is_written_too() {
    let mut harness = Harness::synced().await;
    let gate = harness.reopen_gated().await;
    let id = "draft00000000000073";
    harness
        .store
        .save_draft(draft(id, "Raced 1", "Hello"))
        .await
        .expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    let gmail_id = harness
        .store
        .draft(id)
        .await
        .expect("reads")
        .and_then(|draft| draft.gmail_id)
        .expect("a Gmail draft");
    harness
        .store
        .save_draft(draft(id, "Raced 2", "Hello"))
        .await
        .expect("saves");
    // Hold its write on the way to Gmail, and undo the edit meanwhile.
    gate.hold_after(0);
    let store = harness.store.clone();
    let push = tokio::spawn(async move { store.push_drafts_now().await.expect("pushes") });
    gate.held().await;
    harness
        .store
        .save_draft(saved_again(id, "Raced 1", "Hello", 1))
        .await
        .expect("saves");
    gate.release();
    assert_eq!(push.await.expect("the push ends").saved, [id]);
    assert!(
        !harness
            .store
            .draft(id)
            .await
            .expect("reads")
            .expect("stored")
            .synced,
        "Gmail has the edit, not its undoing"
    );
    harness.store.push_drafts_now().await.expect("pushes");
    assert_eq!(gmail_draft_subject(&harness, &gmail_id).await, "Raced 1");
}

#[tokio::test]
async fn a_save_made_to_send_is_written_even_unchanged() {
    let harness = Harness::synced().await;
    let id = "draft00000000000074";
    harness
        .store
        .save_draft(draft(id, "Dated", "Hello"))
        .await
        .expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    let spent = units(&harness).await;
    let send = Draft {
        sending: true,
        ..saved_again(id, "Dated", "Hello", 60)
    };
    let date = send
        .raw
        .lines()
        .find_map(|line| line.strip_prefix("Date: "))
        .expect("a Date")
        .to_owned();
    harness.store.save_draft(send).await.expect("saves");
    harness
        .store
        .send_draft(id, Timestamp::now())
        .await
        .expect("sends");
    let report = harness.store.push_drafts().await.expect("pushes");
    let [sent] = report.sent.as_slice() else {
        panic!("sent: {report:?}")
    };
    assert_eq!(
        units(&harness).await - spent,
        15 + 20 + 100 + 60,
        "the write, then the send"
    );
    let message = harness
        .gmail
        .gmail()
        .message(&sent.message, Format::Metadata, &["Date"])
        .await
        .expect("messages.get");
    assert_eq!(
        message.header("Date"),
        Some(date.as_str()),
        "dated when Send was pressed"
    );
}

#[tokio::test]
async fn undoing_a_discard_before_gmail_hears_of_it_writes_nothing() {
    let harness = Harness::synced().await;
    let id = "draft00000000000075";
    harness
        .store
        .save_draft(draft(id, "Kept", "Hello"))
        .await
        .expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    let gmail_id = harness
        .store
        .draft(id)
        .await
        .expect("reads")
        .and_then(|draft| draft.gmail_id)
        .expect("a Gmail draft");
    let spent = units(&harness).await;
    harness.store.discard_draft(id).await.expect("discards");
    harness
        .store
        .save_draft(saved_again(id, "Kept", "Hello", 5))
        .await
        .expect("Undo");
    assert!(
        harness
            .store
            .push_drafts_now()
            .await
            .expect("pushes")
            .saved
            .is_empty()
    );
    assert_eq!(units(&harness).await - spent, 0);
    assert!(
        gmail_drafts(&harness)
            .await
            .iter()
            .any(|draft| draft.id.as_deref() == Some(gmail_id.as_str())),
        "Gmail's draft is still there"
    );
}

#[tokio::test]
async fn editing_a_failed_draft_and_leaving_it_unchanged_writes_nothing() {
    let harness = Harness::synced().await;
    let id = "draft00000000000076";
    harness
        .store
        .save_draft(draft(id, "Failed", "Hello"))
        .await
        .expect("saves");
    harness
        .store
        .send_draft(id, Timestamp::now())
        .await
        .expect("sends");
    harness.fault(json!({"method": "POST", "path": "/gmail/v1/users/me/drafts/send", "status": 400, "message": "Invalid To header", "reason": "invalidArgument", "times": 1})).await;
    assert_eq!(
        harness
            .store
            .push_drafts()
            .await
            .expect("pushes")
            .failed
            .len(),
        1
    );
    let spent = units(&harness).await;
    harness.store.recover_draft(id).await.expect("Edit Message");
    harness
        .store
        .save_draft(saved_again(id, "Failed", "Hello", 5))
        .await
        .expect("saves");
    assert!(
        harness
            .store
            .push_drafts()
            .await
            .expect("pushes")
            .saved
            .is_empty()
    );
    assert_eq!(units(&harness).await - spent, 0);
    assert_eq!(
        harness
            .store
            .draft(id)
            .await
            .expect("reads")
            .expect("stored")
            .failure,
        None,
        "a draft again"
    );
}

/// [`with_file`] with a second file, `notes.txt`, added after the deck.
fn with_two_files(id: &str, subject: &str, body: &str) -> Draft {
    let mut saved = with_file(id, subject, body);
    saved.record.attachments.push(exo_store::DraftAttachment {
        uuid: "notes".into(),
        name: "notes.txt".into(),
        mime_type: "text/plain".into(),
        size: 5,
        content_id: None,
        source: exo_store::AttachmentSource::Local,
    });
    let email = gmail::mime::Email {
        to: vec![gmail::mime::Mailbox::address("ann@example.com")],
        subject: subject.into(),
        html: Some(format!("<div>{body}</div>")),
        message_id: Some("a.b@exo".into()),
        attachments: vec![
            gmail::mime::Attachment {
                filename: "deck.pdf".into(),
                content_type: "application/pdf".into(),
                data: b"%PDF".to_vec(),
                content_id: None,
            },
            gmail::mime::Attachment {
                filename: "notes.txt".into(),
                content_type: "text/plain".into(),
                data: b"notes".to_vec(),
                content_id: None,
            },
        ],
        ..gmail::mime::Email::default()
    };
    saved.raw = String::from_utf8(email.to_rfc822()).expect("ASCII");
    saved
}

/// How far `draft`'s file `uuid` has gone up, as the store says now.
fn upload_of(harness: &Harness, draft: &Draft, uuid: &str) -> Option<f32> {
    let attachment = draft
        .record
        .attachments
        .iter()
        .find(|attachment| attachment.uuid == uuid)
        .expect("the draft has the file");
    harness
        .store
        .uploads()
        .borrow()
        .get(&draft.id)
        .cloned()
        .unwrap_or_default()
        .progress(attachment)
}

#[tokio::test]
async fn a_file_is_up_once_a_write_carried_it_and_a_new_one_goes_at_once() {
    let harness =
        Harness::synced_with(paced(Duration::from_millis(50), Duration::from_secs(60))).await;
    let id = "draft00000000000077";
    let first = with_file(id, "Deck", "Attached");
    harness
        .store
        .save_draft(first.clone())
        .await
        .expect("saves");
    assert_eq!(
        upload_of(&harness, &first, "deck"),
        Some(0.0),
        "on its way, no write yet"
    );
    harness.store.push_drafts().await.expect("pushes");
    assert_eq!(upload_of(&harness, &first, "deck"), None, "Gmail has it");

    // Within the pace for a draft with files, but a file Gmail lacks goes up at once.
    let second = with_two_files(id, "Deck", "Attached");
    harness
        .store
        .save_draft(second.clone())
        .await
        .expect("saves");
    assert_eq!(upload_of(&harness, &second, "deck"), None, "still Gmail's");
    assert_eq!(upload_of(&harness, &second, "notes"), Some(0.0));
    assert_eq!(
        harness.store.push_drafts().await.expect("pushes").saved,
        [id],
        "written at once"
    );
    assert_eq!(upload_of(&harness, &second, "notes"), None);
    assert!(
        harness.store.uploads().borrow()[id].sending.is_none(),
        "no write under way"
    );
}

#[tokio::test]
async fn a_file_gmail_refused_stays_on_its_way() {
    let harness = Harness::synced_with(support::unpaced()).await;
    let id = "draft00000000000078";
    let saved = with_file(id, "Deck", "Attached");
    harness
        .store
        .save_draft(saved.clone())
        .await
        .expect("saves");
    harness.fault(json!({"method": "POST", "path": "/upload/gmail/v1/users/me/drafts", "status": 400, "message": "Invalid attachment", "reason": "invalidArgument", "times": 1})).await;
    let report = harness.store.push_drafts().await.expect("pushes");
    assert!(
        matches!(
            report.failed.as_slice(),
            [exo_store::DraftFailure::Save { .. }]
        ),
        "{report:?}"
    );
    assert!(
        harness
            .store
            .draft(id)
            .await
            .expect("reads")
            .expect("stored")
            .synced,
        "given up for this version"
    );
    assert_eq!(
        upload_of(&harness, &saved, "deck"),
        Some(0.0),
        "Gmail doesn't have it"
    );
}
