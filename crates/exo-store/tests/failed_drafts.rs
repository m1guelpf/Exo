//! Failed drafts: a send Gmail won't take stays in its thread as a failed draft, with its
//! reason, until it's edited, sent again or discarded, across restarts.

mod support;

use exo_store::{
    Address, Draft, DraftAction, DraftFailure, DraftRecord, ListId, SendFailure, ThreadId,
};
use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};
use support::Harness;

const SEND: &str = "/gmail/v1/users/me/drafts/send";

fn draft(id: &str, rfc822_id: &str) -> Draft {
    let email = gmail::mime::Email {
        from: Some(gmail::mime::Mailbox::new("Maya Okafor", support::ACCOUNT)),
        to: vec![gmail::mime::Mailbox::new("Ann", "ann@example.com")],
        subject: "Quarterly plan".into(),
        html: Some("<div>Here it is</div>".into()),
        text: Some("Here it is".into()),
        message_id: Some(rfc822_id.into()),
        ..gmail::mime::Email::default()
    };
    let record = DraftRecord {
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
        subject: "Quarterly plan".into(),
        body: "<div>Here it is</div>".into(),
        quoted_content: None,
        quoted_inlined: false,
        in_reply_to: None,
        in_reply_to_rfc822: None,
        references: Vec::new(),
        rfc822_id: rfc822_id.into(),
        fingerprint: String::new(),
        attachments: Vec::new(),
        send_later: None,
        reminder: None,
    };
    Draft {
        id: id.into(),
        thread: None,
        adopted: None,
        record,
        raw: String::from_utf8(email.to_rfc822()).expect("ASCII"),
        complete: true,
        sending: false,
    }
}

/// Draft `id`, a reply to the last message of `thread`.
async fn reply(harness: &Harness, thread: &ThreadId, id: &str) -> Draft {
    let detail = harness
        .store
        .thread(thread)
        .await
        .expect("reads")
        .expect("stored");
    let last = detail
        .messages
        .iter()
        .rev()
        .find(|message| !message.is_draft())
        .expect("a message");
    let mut reply = draft(id, &format!("{id}@exo"));
    let email = gmail::mime::Email {
        to: vec![gmail::mime::Mailbox::address("ann@example.com")],
        subject: gmail::mime::reply_subject(&detail.subject),
        html: Some("<div>Sounds good</div>".into()),
        message_id: Some(format!("{id}@exo")),
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
    reply.thread = Some(thread.clone());
    reply.record.action = DraftAction::Reply;
    reply.record.in_reply_to = Some(last.id.clone());
    reply
}

/// Saves draft `id` to Gmail, holds it until now and, with `fault` armed on `drafts.send`,
/// pushes once. Returns the push's failures and the draft's thread.
async fn send_with(
    harness: &Harness,
    id: &str,
    fault: Option<Value>,
) -> (Vec<DraftFailure>, ThreadId) {
    harness
        .store
        .save_draft(draft(id, &format!("{id}@exo")))
        .await
        .expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    let thread = harness
        .store
        .draft(id)
        .await
        .expect("reads")
        .and_then(|draft| draft.thread)
        .expect("a thread");
    harness
        .store
        .send_draft(id, Timestamp::now())
        .await
        .expect("holds");
    if let Some(fault) = fault {
        harness.fault(fault).await;
    }
    (
        harness.store.push_drafts().await.expect("pushes").failed,
        thread,
    )
}

fn sending_fault(status: u16, message: &str, reason: &str) -> Value {
    json!({"method": "POST", "path": SEND, "status": status, "message": message, "reason": reason, "times": 100})
}

/// Gmail's rate-limit answer to a send, retry `after` from now as Gmail writes it into the
/// message: the account's (`sending` false) or its sending limit's, which says `(Mail sending)`.
fn rate_limit(after: SignedDuration, sending: bool) -> Value {
    let at = Timestamp::now().checked_add(after).expect("in range");
    let limit = if sending { " (Mail sending)" } else { "" };
    sending_fault(
        429,
        &format!("User-rate limit exceeded.  Retry after {at}{limit}"),
        "rateLimitExceeded",
    )
}

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

/// The failed draft's reason as the store shows it: the draft, its thread's row in the Inbox,
/// and its message in the thread.
async fn shown_failure(
    harness: &Harness,
    id: &str,
    thread: &ThreadId,
) -> (
    Option<SendFailure>,
    Option<SendFailure>,
    Option<SendFailure>,
) {
    let draft = harness
        .store
        .draft(id)
        .await
        .expect("reads")
        .and_then(|draft| draft.failure);
    let row = harness
        .summary(&ListId::Inbox, thread)
        .await
        .and_then(|row| row.failure);
    let detail = harness
        .store
        .thread(thread)
        .await
        .expect("reads")
        .expect("stored");
    let message = detail.messages.iter().find_map(|message| message.failure);
    (draft, row, message)
}

#[tokio::test]
async fn a_send_gmail_refuses_is_a_failed_draft_until_edited() {
    let mut harness = Harness::synced().await;
    let (failed, thread) = send_with(
        &harness,
        "draft00000000000001",
        Some(sending_fault(400, "Invalid To header", "invalidArgument")),
    )
    .await;
    assert_eq!(
        failed,
        [DraftFailure::Send {
            id: "draft00000000000001".into(),
            reason: SendFailure::InvalidRecipients
        }]
    );
    let invalid = Some(SendFailure::InvalidRecipients);
    assert_eq!(
        shown_failure(&harness, "draft00000000000001", &thread).await,
        (invalid, invalid, invalid),
        "the draft, its row and its message"
    );
    let row = harness
        .summary(&ListId::Inbox, &thread)
        .await
        .expect("in the Inbox");
    assert_eq!(
        row.drafts, 0,
        "a failed draft isn't one being written (no `Draft` marker)"
    );
    let detail = harness
        .store
        .thread(&thread)
        .await
        .expect("reads")
        .expect("stored");
    let message = detail
        .messages
        .iter()
        .find(|message| message.failure.is_some())
        .expect("the failed draft");
    assert!(
        message
            .body
            .html
            .as_deref()
            .is_some_and(|html| html.contains("Here it is")),
        "with the body its saves didn't store: {:?}",
        message.body
    );
    assert!(
        harness.store.bodies_stored(&thread).await.expect("reads"),
        "so opening it fetches nothing"
    );
    let stored = harness
        .store
        .draft("draft00000000000001")
        .await
        .expect("reads")
        .expect("kept");
    assert_eq!(stored.send_at, None, "no longer held");

    // Nothing retries it, and a relaunch keeps it.
    harness.clear_faults().await;
    harness.store.push_drafts().await.expect("pushes");
    harness.reopen().await;
    harness.store.push_drafts().await.expect("pushes");
    assert_eq!(
        sent_with_id(&harness, "draft00000000000001@exo").await,
        0,
        "it waits for the user"
    );
    assert_eq!(
        shown_failure(&harness, "draft00000000000001", &thread).await,
        (invalid, invalid, invalid),
        "still failed after a relaunch"
    );

    // Edit Message: a draft again, its row and thread with it.
    harness
        .store
        .recover_draft("draft00000000000001")
        .await
        .expect("recovers");
    assert_eq!(
        shown_failure(&harness, "draft00000000000001", &thread).await,
        (None, None, None)
    );
    assert_eq!(
        harness
            .summary(&ListId::Inbox, &thread)
            .await
            .expect("listed")
            .drafts,
        1,
        "a draft being written again"
    );
}

#[tokio::test]
async fn sending_a_failed_draft_again_sends_it() {
    let harness = Harness::synced().await;
    let (failed, thread) = send_with(
        &harness,
        "draft00000000000002",
        Some(sending_fault(400, "Invalid To header", "invalidArgument")),
    )
    .await;
    assert_eq!(failed.len(), 1);
    harness.clear_faults().await;
    harness
        .store
        .send_draft("draft00000000000002", Timestamp::now())
        .await
        .expect("sends again");
    let held = harness
        .store
        .draft("draft00000000000002")
        .await
        .expect("reads")
        .expect("kept");
    assert_eq!(
        (held.failure, held.send_at.is_some()),
        (None, true),
        "sending again clears the failure"
    );
    assert!(
        harness.summary(&ListId::Inbox, &thread).await.is_none(),
        "and its row goes while it's sending"
    );
    let report = harness.store.push_drafts().await.expect("pushes");
    assert_eq!(report.sent.len(), 1, "{report:?}");
    assert_eq!(sent_with_id(&harness, "draft00000000000002@exo").await, 1);
}

#[tokio::test]
async fn a_send_while_google_has_signed_the_account_out_fails_as_logged_out() {
    let harness = Harness::synced().await;
    let fault = json!({"path": "/", "status": 401, "times": 100_000});
    let (failed, thread) = send_with(&harness, "draft00000000000003", Some(fault)).await;
    assert_eq!(
        failed,
        [DraftFailure::Send {
            id: "draft00000000000003".into(),
            reason: SendFailure::LoggedOut
        }]
    );
    // Signed in again: it doesn't go by itself.
    harness.clear_faults().await;
    harness.store.push_drafts().await.expect("pushes");
    assert_eq!(
        sent_with_id(&harness, "draft00000000000003@exo").await,
        0,
        "not sent late"
    );
    let logged_out = Some(SendFailure::LoggedOut);
    assert_eq!(
        shown_failure(&harness, "draft00000000000003", &thread).await,
        (logged_out, logged_out, logged_out)
    );
}

#[tokio::test]
async fn a_held_send_whose_message_cant_be_written_while_signed_out_fails_once_due() {
    let harness = Harness::synced().await;
    harness
        .store
        .save_draft(draft("draft00000000000004", "draft00000000000004@exo"))
        .await
        .expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    // Its last edit, then Send; Google signs the account out before Gmail has either.
    let mut edited = draft("draft00000000000004", "draft00000000000004@exo");
    edited.record.body = "<div>Here it is, finally</div>".into();
    edited.raw = edited.raw.replace("Here it is", "Here it is, finally");
    harness.store.save_draft(edited).await.expect("saves");
    let later = Timestamp::now()
        .checked_add(SignedDuration::from_secs(20))
        .expect("in range");
    harness
        .store
        .send_draft("draft00000000000004", later)
        .await
        .expect("holds");
    harness
        .fault(json!({"path": "/", "status": 401, "times": 100_000}))
        .await;
    let report = harness.store.push_drafts().await.expect("pushes");
    assert!(report.failed.is_empty(), "held, it waits: {report:?}");
    // Closing the account (quitting, or the sign-out itself) ends the hold and pushes at once,
    // the save's backoff notwithstanding.
    harness
        .store
        .send_draft("draft00000000000004", Timestamp::now())
        .await
        .expect("due");
    let report = harness.store.push_drafts_now().await.expect("pushes");
    assert_eq!(
        report.failed,
        [DraftFailure::Send {
            id: "draft00000000000004".into(),
            reason: SendFailure::LoggedOut
        }]
    );
}

#[tokio::test]
async fn gmails_refusals_fail_sends_with_their_reasons() {
    let harness = Harness::synced().await;
    let cases = [
        (
            "draft00000000000011",
            sending_fault(413, "Request Entity Too Large", "uploadTooLarge"),
            SendFailure::TooLarge,
        ),
        (
            "draft00000000000012",
            sending_fault(403, "Daily Limit Exceeded", "dailyLimitExceeded"),
            SendFailure::QuotaExceeded,
        ),
        (
            "draft00000000000013",
            sending_fault(400, "Recipient address required", "invalidArgument"),
            SendFailure::InvalidRecipients,
        ),
        (
            "draft00000000000014",
            sending_fault(400, "Invalid id value", "invalidArgument"),
            SendFailure::Other,
        ),
    ];
    for (id, fault, reason) in cases {
        let (failed, _) = send_with(&harness, id, Some(fault)).await;
        harness.clear_faults().await;
        assert_eq!(
            failed,
            [DraftFailure::Send {
                id: id.into(),
                reason
            }],
            "{reason:?}"
        );
        assert_eq!(
            harness
                .store
                .draft(id)
                .await
                .expect("reads")
                .and_then(|draft| draft.failure),
            Some(reason)
        );
    }
}

#[tokio::test]
async fn gmails_sending_limit_fails_a_send_and_a_long_rate_limit_too() {
    let harness = Harness::synced().await;
    let (failed, _) = send_with(
        &harness,
        "draft00000000000021",
        Some(rate_limit(SignedDuration::from_hours(3), true)),
    )
    .await;
    assert_eq!(
        failed,
        [DraftFailure::Send {
            id: "draft00000000000021".into(),
            reason: SendFailure::QuotaExceeded
        }],
        "Gmail's daily sending limit"
    );

    let harness = Harness::synced().await;
    let (failed, _) = send_with(
        &harness,
        "draft00000000000022",
        Some(rate_limit(SignedDuration::from_mins(10), true)),
    )
    .await;
    assert_eq!(
        failed,
        [DraftFailure::Send {
            id: "draft00000000000022".into(),
            reason: SendFailure::RateLimited
        }],
        "a sending limit of minutes"
    );

    let harness = Harness::synced().await;
    let (failed, _) = send_with(
        &harness,
        "draft00000000000024",
        Some(rate_limit(SignedDuration::from_mins(10), false)),
    )
    .await;
    assert_eq!(
        failed,
        [DraftFailure::Send {
            id: "draft00000000000024".into(),
            reason: SendFailure::RateLimited
        }],
        "the account rate limited for longer than a send waits"
    );
}

#[tokio::test]
async fn a_short_rate_limit_only_delays_a_send() {
    for (at, sending) in [(23, false), (25, true)] {
        let harness = Harness::synced().await;
        let mut fault = rate_limit(SignedDuration::from_secs(1), sending);
        fault["times"] = json!(1);
        let id = format!("draft000000000000{at}");
        let (failed, _) = send_with(&harness, &id, Some(fault)).await;
        assert!(failed.is_empty(), "{failed:?}");
        assert_eq!(
            sent_with_id(&harness, &format!("{id}@exo")).await,
            1,
            "sent once Gmail took it (sending limit: {sending})"
        );
    }
}

#[tokio::test]
async fn gmails_sending_limit_holds_sending_alone() {
    let harness = Harness::synced().await;
    let (failed, _) = send_with(
        &harness,
        "draft00000000000026",
        Some(rate_limit(SignedDuration::from_hours(3), true)),
    )
    .await;
    assert_eq!(
        failed,
        [DraftFailure::Send {
            id: "draft00000000000026".into(),
            reason: SendFailure::QuotaExceeded
        }]
    );
    // Archive and sync go on, and nothing says Gmail is throttling the account.
    let inbox = harness.all(&ListId::Inbox).await;
    harness
        .store
        .apply(
            std::slice::from_ref(&inbox[1].id),
            exo_store::Mutation::MarkDone,
        )
        .await
        .expect("applies");
    let report = harness.store.flush().await.expect("flushes");
    assert_eq!(
        (report.completed, report.failed.len(), report.retry_at),
        (1, 0, None),
        "archived at once"
    );
    harness.store.sync().await.expect("syncs");
    assert!(
        harness.store.connection().borrow().is_online(),
        "no throttling bar"
    );

    // The account's own rate limit still pauses every call.
    let harness = Harness::synced().await;
    send_with(
        &harness,
        "draft00000000000027",
        Some(rate_limit(SignedDuration::from_mins(10), false)),
    )
    .await;
    assert!(
        matches!(
            harness.store.sync().await,
            Err(exo_store::Error::Throttled { .. })
        ),
        "sync waits for the account's pause"
    );
}

#[tokio::test]
async fn a_reply_being_sent_marks_its_row_until_it_goes() {
    let harness = Harness::synced().await;
    let thread = harness.all(&ListId::Inbox).await[2].id.clone();
    harness
        .store
        .save_draft(reply(&harness, &thread, "draft00000000000028").await)
        .await
        .expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    let later = Timestamp::now()
        .checked_add(SignedDuration::from_secs(20))
        .expect("in range");
    harness
        .store
        .send_draft("draft00000000000028", later)
        .await
        .expect("holds");
    let row = harness
        .summary(&ListId::Inbox, &thread)
        .await
        .expect("still listed");
    assert!(
        row.sending && row.drafts == 0,
        "offline, it says it sends when online"
    );
    harness
        .store
        .send_draft("draft00000000000028", Timestamp::now())
        .await
        .expect("due");
    harness.store.push_drafts().await.expect("sends");
    assert!(
        !harness
            .summary(&ListId::Inbox, &thread)
            .await
            .is_some_and(|row| row.sending),
        "sent"
    );
}

#[tokio::test]
async fn gmails_server_errors_fail_a_send_once_retrying_runs_out() {
    let harness = Harness::synced().await;
    let mut brief = sending_fault(500, "Backend Error", "backendError");
    brief["times"] = json!(2);
    let (failed, _) = send_with(&harness, "draft00000000000031", Some(brief)).await;
    assert!(
        failed.is_empty(),
        "a couple of errors are retried: {failed:?}"
    );
    assert_eq!(sent_with_id(&harness, "draft00000000000031@exo").await, 1);

    let (failed, thread) = send_with(
        &harness,
        "draft00000000000032",
        Some(sending_fault(500, "Backend Error", "backendError")),
    )
    .await;
    assert_eq!(
        failed,
        [DraftFailure::Send {
            id: "draft00000000000032".into(),
            reason: SendFailure::Temporary
        }]
    );
    assert_eq!(
        shown_failure(&harness, "draft00000000000032", &thread)
            .await
            .0,
        Some(SendFailure::Temporary)
    );
    assert_eq!(sent_with_id(&harness, "draft00000000000032@exo").await, 0);
}

#[tokio::test]
async fn a_send_gmail_made_but_kept_answering_with_errors_is_sent() {
    let harness = Harness::synced().await;
    let mut lost = sending_fault(503, "The service is currently unavailable.", "backendError");
    lost["after"] = json!(true);
    let (failed, _) = send_with(&harness, "draft00000000000033", Some(lost)).await;
    assert!(failed.is_empty(), "it went: {failed:?}");
    assert!(
        harness
            .store
            .draft("draft00000000000033")
            .await
            .expect("reads")
            .is_none(),
        "no longer a draft"
    );
    assert_eq!(sent_with_id(&harness, "draft00000000000033@exo").await, 1);
}

#[tokio::test]
async fn being_offline_never_fails_a_send() {
    let harness = Harness::synced().await;
    harness
        .store
        .save_draft(draft("draft00000000000041", "draft00000000000041@exo"))
        .await
        .expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    harness
        .store
        .send_draft("draft00000000000041", Timestamp::now())
        .await
        .expect("holds");
    // Nothing listens on a port just freed.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("a port")
        .port();
    let endpoints =
        gmail::Endpoints::mock(&format!("http://127.0.0.1:{port}/").parse().expect("a URL"));
    let offline = exo_store::Store::open_with(
        harness.dir.path(),
        support::ACCOUNT,
        endpoints,
        gmail::StaticToken("t".into()),
        support::options(),
    )
    .await
    .expect("opens");
    let report = offline.push_drafts().await.expect("pushes");
    assert!(report.failed.is_empty() && report.offline, "{report:?}");
    let draft = offline
        .draft("draft00000000000041")
        .await
        .expect("reads")
        .expect("kept");
    assert!(
        draft.send_at.is_some() && draft.failure.is_none(),
        "still held, to send once online"
    );
}

#[tokio::test]
async fn a_failed_draft_keeps_its_thread_on_top_of_the_inbox() {
    let harness = Harness::synced().await;
    let inbox = harness.all(&ListId::Inbox).await;
    // A reply to the oldest conversation in the Inbox.
    let oldest = inbox.last().expect("an inbox").id.clone();
    harness
        .store
        .save_draft(reply(&harness, &oldest, "draft00000000000051").await)
        .await
        .expect("saves");
    harness.store.push_drafts().await.expect("pushes");
    // Done, then sent and refused.
    harness
        .store
        .apply(std::slice::from_ref(&oldest), exo_store::Mutation::MarkDone)
        .await
        .expect("applies");
    harness.store.flush().await.expect("flushes");
    assert!(
        !harness.ids(&ListId::Inbox).await.contains(oldest.as_str()),
        "done"
    );
    harness
        .store
        .send_draft("draft00000000000051", Timestamp::now())
        .await
        .expect("holds");
    harness
        .fault(sending_fault(400, "Invalid To header", "invalidArgument"))
        .await;
    harness.store.push_drafts().await.expect("pushes");
    let inbox = harness.all(&ListId::Inbox).await;
    assert_eq!(
        inbox.first().map(|row| &row.id),
        Some(&oldest),
        "back in the Inbox, on top"
    );
    assert!(
        harness
            .ids(&ListId::Important)
            .await
            .contains(oldest.as_str()),
        "in Important"
    );
}
