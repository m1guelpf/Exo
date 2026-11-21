//! The `gmail` client against the in-process mock: every Gmail endpoint group, history after
//! mutations, send and threading, drafts, settings, batch and admin delivery.

mod support;

use gmail::mime::{Attachment, Email, Mailbox};
use gmail::*;

fn inbox() -> ListQuery {
    ListQuery {
        label_ids: vec!["INBOX".into()],
        ..ListQuery::default()
    }
}

fn labels_of(message: &Message) -> Vec<&str> {
    message.label_ids.iter().map(String::as_str).collect()
}

#[tokio::test]
async fn labels_crud_and_counts() {
    let server = support::server().await;
    let client = support::client(&server);
    let gmail = client.gmail();

    let labels = gmail.labels().await.unwrap().labels;
    assert_eq!(labels[0].id.as_deref(), Some("CHAT"));
    assert!(
        labels
            .iter()
            .any(|l| l.name.as_deref() == Some("Projects/Atlas Launch") && l.color.is_some())
    );
    assert!(
        labels.iter().all(|l| l.messages_total.is_none()),
        "list omits counts"
    );

    let counts = gmail.label("INBOX").await.unwrap();
    let listed = gmail
        .threads(&ListQuery {
            max_results: Some(500),
            ..inbox()
        })
        .await
        .unwrap();
    assert_eq!(counts.threads_total, listed.result_size_estimate);
    assert!(counts.threads_unread.unwrap() > 0 && counts.threads_unread < counts.threads_total);

    let created = gmail
        .create_label(&Label {
            name: Some("Projects/Exo/Research".into()),
            color: Some(LabelColor {
                text_color: Some("#ffffff".into()),
                background_color: Some("#653e9b".into()),
            }),
            ..Label::default()
        })
        .await
        .unwrap();
    let id = created.id.clone().unwrap();
    assert!(id.starts_with("Label_"));
    assert_eq!(created.kind, Some(LabelType::User));
    assert_eq!(
        created.label_list_visibility,
        Some(LabelListVisibility::LabelShow)
    );

    let duplicate = gmail
        .create_label(&Label {
            name: Some("projects/exo/research".into()),
            ..Label::default()
        })
        .await;
    assert_eq!(duplicate.unwrap_err().status(), Some(409));
    let bad_color = gmail
        .create_label(&Label {
            name: Some("Off palette".into()),
            color: Some(LabelColor {
                text_color: Some("#123456".into()),
                background_color: Some("#ffffff".into()),
            }),
            ..Label::default()
        })
        .await;
    assert_eq!(bad_color.unwrap_err().status(), Some(400));
    assert_eq!(
        gmail
            .create_label(&Label {
                name: Some("Inbox".into()),
                ..Label::default()
            })
            .await
            .unwrap_err()
            .status(),
        Some(400)
    );

    let patched = gmail
        .patch_label(
            &id,
            &Label {
                label_list_visibility: Some(LabelListVisibility::LabelHide),
                ..Label::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(patched.name.as_deref(), Some("Projects/Exo/Research"));
    assert!(patched.color.is_some(), "patch keeps unspecified fields");
    let updated = gmail
        .update_label(
            &id,
            &Label {
                name: Some("Research".into()),
                ..Label::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.color, None, "update replaces the label");

    let message = gmail.messages(&inbox()).await.unwrap().messages[0]
        .id
        .clone()
        .unwrap();
    gmail
        .modify_message(
            &message,
            &ModifyMessageRequest {
                add_label_ids: vec![id.clone()],
                ..ModifyMessageRequest::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(gmail.label(&id).await.unwrap().messages_total, Some(1));
    gmail.delete_label(&id).await.unwrap();
    assert!(gmail.label(&id).await.unwrap_err().is_not_found());
    let after = gmail.message(&message, Format::Minimal, &[]).await.unwrap();
    assert!(
        !after.label_ids.contains(&id),
        "deleting a label removes it from messages"
    );
    assert_eq!(
        gmail.delete_label("INBOX").await.unwrap_err().status(),
        Some(400)
    );
}

#[tokio::test]
async fn threads_get_modify_trash_and_delete() {
    let server = support::server().await;
    let client = support::client(&server);
    let gmail = client.gmail();

    let listed = gmail.threads(&inbox()).await.unwrap();
    let first = &listed.threads[0];
    assert!(first.snippet.is_some() && first.history_id.is_some() && first.messages.is_empty());

    let long = gmail
        .threads(&ListQuery {
            q: Some("subject:\"Atlas launch checklist\"".into()),
            ..ListQuery::default()
        })
        .await
        .unwrap();
    let id = long.threads[0].id.clone().unwrap();
    let full = gmail.thread(&id, Format::Full, &[]).await.unwrap();
    assert!(full.messages.len() >= 15);
    let dates: Vec<i64> = full
        .messages
        .iter()
        .map(|m| m.internal_date.unwrap())
        .collect();
    assert!(
        dates.windows(2).all(|w| w[0] <= w[1]),
        "messages are oldest first"
    );
    assert_eq!(
        full.history_id,
        full.messages.iter().map(|m| m.history_id.unwrap()).max()
    );
    let reply = &full.messages[1];
    assert_eq!(reply.header("Subject"), Some("Re: Atlas launch checklist"));
    assert!(reply.header("In-Reply-To").is_some() && reply.header("References").is_some());

    let metadata = gmail
        .thread(&id, Format::Metadata, &["Subject", "From"])
        .await
        .unwrap();
    let headers: Vec<&str> = metadata.messages[0]
        .payload
        .as_ref()
        .unwrap()
        .headers
        .iter()
        .map(|h| h.name.as_str())
        .collect();
    assert_eq!(headers, ["Subject", "From"]);
    assert!(
        gmail.thread(&id, Format::Raw, &[]).await.is_err(),
        "threads.get has no raw format"
    );

    let archived = gmail
        .modify_thread(
            &id,
            &ModifyThreadRequest {
                remove_label_ids: vec!["INBOX".into()],
                add_label_ids: vec!["STARRED".into()],
            },
        )
        .await
        .unwrap();
    assert!(
        archived
            .messages
            .iter()
            .all(|m| !m.label_ids.contains(&"INBOX".to_owned())
                && m.label_ids.contains(&"STARRED".to_owned()))
    );

    let trashed = gmail.trash_thread(&id).await.unwrap();
    assert!(
        trashed
            .messages
            .iter()
            .all(|m| m.label_ids.contains(&"TRASH".to_owned()))
    );
    let query = ListQuery {
        q: Some("subject:\"Atlas launch checklist\"".into()),
        ..ListQuery::default()
    };
    assert!(
        gmail.threads(&query).await.unwrap().threads.is_empty(),
        "trash is excluded by default"
    );
    let with_trash = ListQuery {
        include_spam_trash: true,
        ..query.clone()
    };
    assert_eq!(gmail.threads(&with_trash).await.unwrap().threads.len(), 1);
    let in_trash = ListQuery {
        q: Some("in:trash subject:\"Atlas launch checklist\"".into()),
        ..ListQuery::default()
    };
    assert_eq!(
        gmail.threads(&in_trash).await.unwrap().threads.len(),
        1,
        "in:trash lifts the exclusion"
    );

    let untrashed = gmail.untrash_thread(&id).await.unwrap();
    assert!(
        untrashed
            .messages
            .iter()
            .all(|m| !m.label_ids.contains(&"TRASH".to_owned()))
    );
    gmail.delete_thread(&id).await.unwrap();
    assert!(
        gmail
            .thread(&id, Format::Minimal, &[])
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn messages_formats_and_mutations() {
    let server = support::server().await;
    let client = support::client(&server);
    let gmail = client.gmail();

    let both = ListQuery {
        label_ids: vec!["INBOX".into(), "UNREAD".into()],
        max_results: Some(500),
        ..ListQuery::default()
    };
    let unread_inbox = gmail.messages(&both).await.unwrap().messages;
    for item in &unread_inbox {
        assert!(
            item.label_ids.is_empty() && item.thread_id.is_some(),
            "list items carry id and threadId only"
        );
        let message = gmail
            .message(item.id.as_deref().unwrap(), Format::Minimal, &[])
            .await
            .unwrap();
        assert!(
            labels_of(&message).contains(&"INBOX") && labels_of(&message).contains(&"UNREAD"),
            "labelIds are ANDed"
        );
    }

    let id = unread_inbox[0].id.clone().unwrap();
    let full = gmail.message(&id, Format::Full, &[]).await.unwrap();
    let raw = gmail.message(&id, Format::Raw, &[]).await.unwrap();
    assert!(raw.payload.is_none() && full.raw.is_none());
    let bytes = raw.raw.unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains(&format!(
            "Message-ID: {}",
            full.header("Message-ID").unwrap()
        )),
        "raw and full agree"
    );
    assert_eq!(
        full.size_estimate,
        Some(u32::try_from(bytes.len()).unwrap())
    );

    let read = gmail
        .modify_message(
            &id,
            &ModifyMessageRequest {
                remove_label_ids: vec!["UNREAD".into()],
                ..ModifyMessageRequest::default()
            },
        )
        .await
        .unwrap();
    assert!(!labels_of(&read).contains(&"UNREAD"));
    let invalid = gmail
        .modify_message(
            &id,
            &ModifyMessageRequest {
                add_label_ids: vec!["Label_999".into()],
                ..ModifyMessageRequest::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(invalid.status(), Some(400));

    let ids: Vec<String> = unread_inbox
        .iter()
        .skip(1)
        .take(3)
        .filter_map(|m| m.id.clone())
        .collect();
    gmail
        .batch_modify_messages(&BatchModifyMessagesRequest {
            ids: ids.clone(),
            add_label_ids: vec!["STARRED".into()],
            remove_label_ids: vec!["UNREAD".into()],
            ..BatchModifyMessagesRequest::default()
        })
        .await
        .unwrap();
    for id in &ids {
        let message = gmail.message(id, Format::Minimal, &[]).await.unwrap();
        assert!(
            labels_of(&message).contains(&"STARRED") && !labels_of(&message).contains(&"UNREAD")
        );
    }

    let trashed = gmail.trash_message(&id).await.unwrap();
    assert!(labels_of(&trashed).contains(&"TRASH") && !labels_of(&trashed).contains(&"INBOX"));
    let untrashed = gmail.untrash_message(&id).await.unwrap();
    assert!(!labels_of(&untrashed).contains(&"TRASH") && !labels_of(&untrashed).contains(&"INBOX"));

    gmail.delete_message(&id).await.unwrap();
    assert!(
        gmail
            .message(&id, Format::Minimal, &[])
            .await
            .unwrap_err()
            .is_not_found()
    );
    gmail.batch_delete_messages(ids.clone()).await.unwrap();
    assert!(
        gmail
            .message(&ids[0], Format::Minimal, &[])
            .await
            .unwrap_err()
            .is_not_found()
    );
    assert_eq!(
        gmail
            .message("nope!", Format::Full, &[])
            .await
            .unwrap_err()
            .status(),
        Some(400)
    );
}

#[tokio::test]
async fn attachments_round_trip() {
    let server = support::server().await;
    let client = support::client(&server);
    let gmail = client.gmail();

    let hits = gmail
        .messages(&ListQuery {
            q: Some("filename:pdf".into()),
            ..ListQuery::default()
        })
        .await
        .unwrap();
    let message = gmail
        .message(hits.messages[0].id.as_deref().unwrap(), Format::Full, &[])
        .await
        .unwrap();
    let part = message
        .payload
        .as_ref()
        .unwrap()
        .walk()
        .find(|p| p.filename.as_deref().is_some_and(|f| f.ends_with(".pdf")))
        .unwrap();
    let body = part.body.as_ref().unwrap();
    let attachment = gmail
        .attachment(
            message.id.as_deref().unwrap(),
            body.attachment_id.as_deref().unwrap(),
        )
        .await
        .unwrap();
    let data = attachment.data.unwrap();
    assert!(data.starts_with(b"%PDF-"));
    assert_eq!(body.size, Some(u32::try_from(data.len()).unwrap()));

    let png = vec![0x89, b'P', b'N', b'G', 1, 2, 3];
    let email = Email {
        to: vec![Mailbox::address("rachel.kim@callowayfreight.com")],
        subject: "Chart".into(),
        text: Some("Attached.".into()),
        attachments: vec![Attachment {
            filename: "chart.png".into(),
            content_type: "image/png".into(),
            data: png.clone(),
            content_id: None,
        }],
        ..Email::default()
    };
    let sent = gmail.send(email.to_rfc822(), None).await.unwrap();
    let sent = gmail
        .message(sent.id.as_deref().unwrap(), Format::Full, &[])
        .await
        .unwrap();
    let part = sent
        .payload
        .as_ref()
        .unwrap()
        .walk()
        .find(|p| p.filename.as_deref() == Some("chart.png"))
        .unwrap();
    assert_eq!(part.mime_type.as_deref(), Some("image/png"));
    let fetched = gmail
        .attachment(
            sent.id.as_deref().unwrap(),
            part.body
                .as_ref()
                .unwrap()
                .attachment_id
                .as_deref()
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(fetched.data.unwrap(), png);
}

#[tokio::test]
async fn send_threads_replies_like_gmail() {
    let server = support::server().await;
    let client = support::client(&server);
    let gmail = client.gmail();

    let original = Email {
        to: vec![Mailbox::new("Rachel Kim", "rachel.kim@callowayfreight.com")],
        subject: "Cross-dock follow-up".into(),
        text: Some("Hi Rachel, following up on cross-dock transfers.".into()),
        html: Some("<p>Hi Rachel, following up on <b>cross-dock</b> transfers.</p>".into()),
        ..Email::default()
    };
    let sent = gmail.send(original.to_rfc822(), None).await.unwrap();
    assert_eq!(labels_of(&sent), ["SENT"]);
    let sent = gmail
        .message(sent.id.as_deref().unwrap(), Format::Full, &[])
        .await
        .unwrap();
    assert_eq!(
        sent.thread_id, sent.id,
        "a new thread is named after its first message"
    );
    assert_eq!(
        sent.header("From"),
        Some("Maya Okafor <maya@brightmoor.co>"),
        "Gmail fills in From"
    );
    let payload = sent.payload.as_ref().unwrap();
    assert_eq!(payload.mime_type.as_deref(), Some("multipart/alternative"));
    assert_eq!(payload.parts[1].part_id.as_deref(), Some("1"));

    let mut reply = Email {
        to: vec![Mailbox::new("Rachel Kim", "rachel.kim@callowayfreight.com")],
        subject: gmail::mime::reply_subject("Cross-dock follow-up"),
        text: Some("One more thing.".into()),
        ..Email::default()
    };
    reply.reply_to_message(&sent);
    let threaded = gmail
        .send(reply.to_rfc822(), sent.thread_id.as_deref())
        .await
        .unwrap();
    assert_eq!(threaded.thread_id, sent.thread_id);

    // Without matching threading headers Gmail starts a new thread despite the threadId.
    let unrelated = Email {
        to: vec![Mailbox::address("rachel.kim@callowayfreight.com")],
        subject: "Something else".into(),
        text: Some("Hi".into()),
        ..Email::default()
    };
    let separate = gmail
        .send(unrelated.to_rfc822(), sent.thread_id.as_deref())
        .await
        .unwrap();
    assert_ne!(separate.thread_id, sent.thread_id);

    let to_self = Email {
        to: vec![Mailbox::address("maya@brightmoor.co")],
        subject: "Note to self".into(),
        text: Some("Remember the radiator.".into()),
        ..Email::default()
    };
    let note = gmail.send(to_self.to_rfc822(), None).await.unwrap();
    assert!(labels_of(&note).contains(&"INBOX") && labels_of(&note).contains(&"SENT"));

    let no_recipients = Email {
        subject: "x".into(),
        text: Some("y".into()),
        ..Email::default()
    };
    let error = gmail
        .send(no_recipients.to_rfc822(), None)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, Error::Api(api) if api.message == "Recipient address required"),
        "{error}"
    );
}

#[tokio::test]
async fn insert_and_import_add_messages_without_sending() {
    let server = support::server().await;
    let client = support::client(&server);
    let gmail = client.gmail();
    let raw = b"From: Theo Lindqvist <theo@harborlightcoffee.com>\r\nTo: maya@brightmoor.co\r\nSubject: Signed order form\r\nDate: Mon, 16 Nov 2026 10:00:00 -0500\r\nMessage-ID: <order-form@harborlightcoffee.com>\r\n\r\nSigned copy attached in the portal.\r\n".to_vec();

    let inserted = gmail
        .insert_message(
            raw.clone(),
            &InsertOptions {
                label_ids: vec!["INBOX".into(), "UNREAD".into()],
                ..InsertOptions::default()
            },
        )
        .await
        .unwrap();
    let inserted = gmail
        .message(inserted.id.as_deref().unwrap(), Format::Minimal, &[])
        .await
        .unwrap();
    assert!(
        inserted.internal_date.unwrap() >= 1_794_924_000_000,
        "insert defaults to receivedTime"
    );

    let imported = gmail
        .import_message(raw, &InsertOptions::default())
        .await
        .unwrap();
    let imported = gmail
        .message(imported.id.as_deref().unwrap(), Format::Minimal, &[])
        .await
        .unwrap();
    assert_eq!(
        imported.internal_date,
        Some(1_794_841_200_000),
        "import defaults to the Date header"
    );
}

#[tokio::test]
async fn drafts_lifecycle() {
    let server = support::server().await;
    let client = support::client(&server);
    let gmail = client.gmail();

    let existing = gmail.drafts(&DraftQuery::default()).await.unwrap();
    assert_eq!(existing.result_size_estimate, Some(4));
    let found = gmail
        .drafts(&DraftQuery {
            q: Some("subject:invoice".into()),
            ..DraftQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(found.drafts.len(), 1);

    let email = |text: &str| Email {
        to: vec![Mailbox::address("felix@novakstudio.design")],
        subject: "Square crop".into(),
        text: Some(text.into()),
        ..Email::default()
    };
    let draft = gmail
        .create_draft(email("First pass").to_rfc822(), None)
        .await
        .unwrap();
    let id = draft.id.clone().unwrap();
    assert!(id.starts_with('r'));
    let first_message = draft.message.unwrap();
    assert_eq!(labels_of(&first_message), ["DRAFT"]);

    let updated = gmail
        .update_draft(&id, email("Second pass").to_rfc822(), None)
        .await
        .unwrap();
    assert_eq!(updated.id.as_deref(), Some(id.as_str()));
    assert_ne!(
        updated.message.as_ref().unwrap().id,
        first_message.id,
        "updates replace the message"
    );
    let fetched = gmail.draft(&id, Format::Raw).await.unwrap();
    assert!(
        String::from_utf8_lossy(fetched.message.unwrap().raw.as_deref().unwrap())
            .contains("Second pass")
    );

    let sent = gmail.send_draft(&id).await.unwrap();
    assert_eq!(labels_of(&sent), ["SENT"]);
    assert!(
        gmail
            .draft(&id, Format::Minimal)
            .await
            .unwrap_err()
            .is_not_found()
    );

    let doomed = gmail
        .create_draft(email("Never mind").to_rfc822(), None)
        .await
        .unwrap();
    gmail
        .delete_draft(doomed.id.as_deref().unwrap())
        .await
        .unwrap();
    assert_eq!(
        gmail
            .drafts(&DraftQuery::default())
            .await
            .unwrap()
            .result_size_estimate,
        Some(4)
    );
}

#[tokio::test]
async fn history_records_every_mutation() {
    let server = support::server().await;
    let client = support::client(&server);
    let gmail = client.gmail();

    let start = gmail.profile().await.unwrap().history_id.unwrap();
    let empty = gmail
        .history(&HistoryQuery {
            start_history_id: start,
            ..HistoryQuery::default()
        })
        .await
        .unwrap();
    assert!(empty.history.is_empty() && empty.next_page_token.is_none());
    assert_eq!(empty.history_id, Some(start));

    let message = gmail.messages(&inbox()).await.unwrap().messages[0].clone();
    let id = message.id.clone().unwrap();
    gmail
        .modify_message(
            &id,
            &ModifyMessageRequest {
                add_label_ids: vec!["STARRED".into()],
                remove_label_ids: vec!["INBOX".into()],
                ..ModifyMessageRequest::default()
            },
        )
        .await
        .unwrap();
    let email = Email {
        to: vec![Mailbox::address("jessmoreno.nyc@gmail.com")],
        subject: "hi".into(),
        text: Some("hey".into()),
        ..Email::default()
    };
    let sent = gmail.send(email.to_rfc822(), None).await.unwrap();
    gmail
        .delete_message(sent.id.as_deref().unwrap())
        .await
        .unwrap();

    let all = gmail
        .history(&HistoryQuery {
            start_history_id: start,
            ..HistoryQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(all.history.len(), 4);
    let ids: Vec<u64> = all.history.iter().map(|h| h.id.unwrap()).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]) && ids[0] > start);
    assert_eq!(all.history_id, gmail.profile().await.unwrap().history_id);
    let added = &all.history[0].labels_added[0];
    assert_eq!(added.label_ids, ["STARRED"]);
    assert!(
        added.message.label_ids.contains(&"STARRED".to_owned()),
        "labelIds are the full set after the change"
    );
    assert_eq!(all.history[1].labels_removed[0].label_ids, ["INBOX"]);
    assert_eq!(all.history[2].messages_added[0].message.id, sent.id);
    assert_eq!(all.history[3].messages_deleted[0].message.id, sent.id);
    assert!(all.history.iter().all(|h| h.messages.len() == 1));
    let modified = gmail.message(&id, Format::Minimal, &[]).await.unwrap();
    assert_eq!(
        modified.history_id, all.history[1].id,
        "a message's historyId is its last change"
    );

    let only_added = gmail
        .history(&HistoryQuery {
            start_history_id: start,
            history_types: vec![HistoryType::MessageAdded],
            ..HistoryQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(only_added.history.len(), 1);
    let starred = gmail
        .history(&HistoryQuery {
            start_history_id: start,
            label_id: Some("STARRED".into()),
            ..HistoryQuery::default()
        })
        .await
        .unwrap();
    assert!(starred.history.iter().all(|h| h.messages_added.is_empty()));

    let mut pages = 0;
    let mut seen = Vec::new();
    let mut token = None;
    loop {
        let page = gmail
            .history(&HistoryQuery {
                start_history_id: start,
                max_results: Some(1),
                page_token: token.clone(),
                ..HistoryQuery::default()
            })
            .await
            .unwrap();
        pages += 1;
        seen.extend(page.history.iter().map(|h| h.id.unwrap()));
        token = page.next_page_token;
        if token.is_none() {
            break;
        }
    }
    assert_eq!((pages, seen), (4, ids));

    let expired = gmail
        .history(&HistoryQuery {
            start_history_id: 1,
            ..HistoryQuery::default()
        })
        .await
        .unwrap_err();
    assert!(expired.is_not_found(), "{expired}");
    reqwest::Client::new()
        .post(format!("{}__mock/history/expire", server.url()))
        .send()
        .await
        .unwrap();
    assert!(
        gmail
            .history(&HistoryQuery {
                start_history_id: start,
                ..HistoryQuery::default()
            })
            .await
            .unwrap_err()
            .is_not_found()
    );
}

#[tokio::test]
async fn settings_send_as_vacation_and_filters() {
    let server = support::server().await;
    let client = support::client(&server);
    let gmail = client.gmail();

    let aliases = gmail.send_as_list().await.unwrap().send_as;
    let primary = &aliases[0];
    assert_eq!(
        (primary.is_primary, primary.is_default),
        (Some(true), Some(true))
    );
    assert!(
        primary
            .signature
            .as_deref()
            .unwrap()
            .contains("Head of Product")
    );
    let patched = gmail
        .patch_send_as(
            "maya@brightmoor.co",
            &SendAs {
                signature: Some("<div>— Maya</div>".into()),
                ..SendAs::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(patched.signature.as_deref(), Some("<div>— Maya</div>"));
    assert_eq!(patched.display_name.as_deref(), Some("Maya Okafor"));
    assert!(
        gmail
            .send_as("nobody@example.com")
            .await
            .unwrap_err()
            .is_not_found()
    );

    let vacation = gmail.vacation().await.unwrap();
    assert_eq!(vacation.enable_auto_reply, Some(false));
    let enabled = gmail
        .update_vacation(&VacationSettings {
            enable_auto_reply: Some(true),
            ..vacation
        })
        .await
        .unwrap();
    assert_eq!(gmail.vacation().await.unwrap(), enabled);
    // Given both bodies, Gmail keeps the HTML one.
    let both = gmail
        .update_vacation(&VacationSettings {
            response_body_plain_text: Some("Back Monday.".into()),
            response_body_html: Some("<div>Back Monday.</div>".into()),
            ..enabled
        })
        .await
        .unwrap();
    assert_eq!(both.response_body_plain_text, None);
    assert_eq!(
        gmail
            .vacation()
            .await
            .unwrap()
            .response_body_html
            .as_deref(),
        Some("<div>Back Monday.</div>")
    );

    let before = gmail.filters().await.unwrap().filter.len();
    let filter = Filter {
        // Clients may send an ID; Gmail treats it as output-only.
        id: Some("client-generated".into()),
        criteria: Some(FilterCriteria {
            from: Some("deals@fieldandfell.com".into()),
            ..FilterCriteria::default()
        }),
        action: Some(FilterAction {
            add_label_ids: vec!["TRASH".into()],
            ..FilterAction::default()
        }),
    };
    let created = gmail.create_filter(&filter).await.unwrap();
    assert_ne!(created.id.as_deref(), Some("client-generated"));
    let duplicate = gmail.create_filter(&filter).await.unwrap_err();
    assert!(matches!(&duplicate, Error::Api(api) if api.message == "Filter already exists"));
    assert_eq!(
        gmail.filter(created.id.as_deref().unwrap()).await.unwrap(),
        created
    );
    assert_eq!(gmail.filters().await.unwrap().filter.len(), before + 1);

    // New mail runs through filters.
    let delivered: Message = reqwest::Client::new()
        .post(format!("{}__mock/deliver", server.url()))
        .json(&serde_json::json!({"from": "Field & Fell <deals@fieldandfell.com>", "subject": "Flash sale", "text": "Today only"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(labels_of(&delivered).contains(&"TRASH"));

    gmail
        .delete_filter(created.id.as_deref().unwrap())
        .await
        .unwrap();
    assert!(
        gmail
            .filter(created.id.as_deref().unwrap())
            .await
            .unwrap_err()
            .is_not_found()
    );
    let no_criteria = Filter {
        action: filter.action.clone(),
        ..Filter::default()
    };
    assert_eq!(
        gmail
            .create_filter(&no_criteria)
            .await
            .unwrap_err()
            .status(),
        Some(400)
    );
}

#[tokio::test]
async fn batch_runs_calls_independently() {
    let server = support::server().await;
    let client = support::client(&server);
    let gmail = client.gmail();

    let ids: Vec<String> = gmail
        .messages(&inbox())
        .await
        .unwrap()
        .messages
        .iter()
        .take(4)
        .filter_map(|m| m.id.clone())
        .collect();
    let mut calls: Vec<_> = ids
        .iter()
        .map(|id| gmail.message(id, Format::Metadata, &["Subject"]))
        .collect();
    calls.push(gmail.message("0000000000000001", Format::Metadata, &[]));
    let results = gmail.batch(calls).await.unwrap();
    assert_eq!(results.len(), 5);
    for (result, id) in results.iter().zip(&ids) {
        let message = result.as_ref().unwrap();
        assert_eq!(message.id.as_ref(), Some(id));
        assert!(message.header("Subject").is_some());
    }
    assert!(results[4].as_ref().unwrap_err().is_not_found());

    let modifies: Vec<_> = ids
        .iter()
        .map(|id| {
            gmail.modify_message(
                id,
                &ModifyMessageRequest {
                    add_label_ids: vec!["STARRED".into()],
                    ..ModifyMessageRequest::default()
                },
            )
        })
        .collect();
    for result in gmail.batch(modifies).await.unwrap() {
        assert!(labels_of(&result.unwrap()).contains(&"STARRED"));
    }

    let too_many: Vec<_> = (0..=MAX_BATCH).map(|_| gmail.profile()).collect();
    assert!(matches!(gmail.batch(too_many).await, Err(Error::Batch(_))));
}

#[tokio::test]
async fn delivered_mail_arrives_threads_and_shows_in_history() {
    let server = support::server().await;
    let client = support::client(&server);
    let gmail = client.gmail();
    let admin = reqwest::Client::new();
    let start = gmail.profile().await.unwrap().history_id.unwrap();

    let thread = gmail
        .threads(&ListQuery {
            q: Some("subject:\"dinner thurs\"".into()),
            ..ListQuery::default()
        })
        .await
        .unwrap()
        .threads[0]
        .id
        .clone()
        .unwrap();
    let delivered: Message = admin
        .post(format!("{}__mock/deliver", server.url()))
        .json(&serde_json::json!({"from": "Jess Moreno <jessmoreno.nyc@gmail.com>", "threadId": thread, "text": "running 5 late!"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(delivered.thread_id.as_deref(), Some(thread.as_str()));
    let top = gmail.threads(&inbox()).await.unwrap().threads[0].clone();
    assert_eq!(
        top.id.as_deref(),
        Some(thread.as_str()),
        "new mail moves the thread to the top"
    );
    let full = gmail.thread(&thread, Format::Full, &[]).await.unwrap();
    assert_eq!(
        full.messages.last().unwrap().header("Subject"),
        Some("Re: dinner thurs?")
    );

    let history = gmail
        .history(&HistoryQuery {
            start_history_id: start,
            ..HistoryQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(
        history.history[0].messages_added[0].message.id,
        delivered.id
    );

    // Armed faults fail the next matching request, then clear.
    admin
        .post(format!("{}__mock/faults", server.url()))
        .json(&serde_json::json!({"path": "/gmail/v1/users/me/threads/", "status": 500, "reason": "responseTooLarge", "message": "Response too large"}))
        .send()
        .await
        .unwrap();
    let failed = gmail.thread(&thread, Format::Full, &[]).await.unwrap_err();
    assert!(
        matches!(&failed, Error::Api(api) if api.code == 500 && api.errors[0].reason == "responseTooLarge"),
        "{failed}"
    );
    assert!(gmail.thread(&thread, Format::Full, &[]).await.is_ok());
}

#[tokio::test]
async fn a_quota_refuses_what_the_minute_cant_hold() {
    let server = gmail_mock::MockServer::start(gmail_mock::Config {
        quota: Some(240),
        ..gmail_mock::Config::default()
    })
    .await
    .expect("mock starts");
    let client = support::client(&server);
    let gmail = client.gmail();
    let id = gmail.threads(&inbox()).await.unwrap().threads[0]
        .id
        .clone()
        .unwrap();
    // 10 for the list, then two full reads at 120 each: the second doesn't fit in 240.
    gmail.thread(&id, Format::Full, &[]).await.unwrap();
    let raw = support::Raw::new(&server);
    let response = raw
        .http
        .get(raw.url(&format!("/gmail/v1/users/me/threads/{id}")))
        .bearer_auth(gmail_mock::DEV_ACCESS_TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 429);
    let wait: u64 = response.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((55..=60).contains(&wait), "{wait}");
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("User-rate limit exceeded.  Retry after 20"),
        "{body}"
    );
    assert_eq!(body["error"]["errors"][0]["reason"], "rateLimitExceeded");
    // A cheaper read still fits, and each call of a batch is charged on its own.
    gmail.thread(&id, Format::Minimal, &[]).await.unwrap();
    let results = gmail
        .batch(vec![
            gmail.thread(&id, Format::Minimal, &[]),
            gmail.thread(&id, Format::Minimal, &[]),
        ])
        .await
        .unwrap();
    assert!(
        results[0].is_ok()
            && results[1]
                .as_ref()
                .is_err_and(|error| error.status() == Some(429))
    );
    let (_, quota) = raw.get("/__mock/quota").await;
    assert_eq!(
        (quota["lastMinute"].as_u64(), quota["refused"].as_u64()),
        (Some(210), Some(2))
    );
}

#[tokio::test]
async fn a_disconnected_mock_answers_nothing_and_comes_back_as_it_was() {
    let mut server = gmail_mock::MockServer::start(gmail_mock::Config::default())
        .await
        .expect("mock starts");
    let client = support::client(&server);
    let gmail = client.gmail();
    let id = gmail.threads(&inbox()).await.unwrap().threads[0]
        .id
        .clone()
        .unwrap();
    gmail
        .modify_thread(
            &id,
            &ModifyThreadRequest {
                add_label_ids: vec!["STARRED".into()],
                ..ModifyThreadRequest::default()
            },
        )
        .await
        .unwrap();
    // The client's pooled connection goes too, so the next call gets no answer at all.
    server.disconnect().await;
    let error = gmail.profile().await.expect_err("nobody answers");
    assert!(
        matches!(error, gmail::Error::Http(ref http) if http.status().is_none()),
        "{error:?}"
    );
    server.reconnect().await.expect("the port binds again");
    let thread = gmail.thread(&id, Format::Minimal, &[]).await.unwrap();
    assert!(
        thread
            .messages
            .iter()
            .any(|message| message.label_ids.iter().any(|label| label == "STARRED")),
        "the mailbox is as it was"
    );
}
