//! The `q` search language, ordering and pagination, checked against the fixture mailbox.

mod support;

use std::collections::BTreeSet;

use gmail::{Client, Format, Label, ListQuery, Message, ModifyMessageRequest};

/// Every message matching `q`, with metadata.
async fn search(client: &Client, q: &str) -> Vec<Message> {
    let gmail = client.gmail();
    let mut ids = Vec::new();
    let mut token = None;
    loop {
        let page = gmail
            .messages(&ListQuery {
                q: Some(q.into()),
                max_results: Some(500),
                page_token: token,
                ..ListQuery::default()
            })
            .await
            .unwrap_or_else(|error| panic!("{q}: {error}"));
        ids.extend(page.messages.into_iter().filter_map(|m| m.id));
        token = page.next_page_token;
        if token.is_none() {
            break;
        }
    }
    let mut messages = Vec::new();
    for chunk in ids.chunks(100) {
        let calls = chunk
            .iter()
            .map(|id| gmail.message(id, Format::Metadata, &[]))
            .collect();
        for result in gmail.batch(calls).await.unwrap() {
            messages.push(result.unwrap());
        }
    }
    messages
}

fn header(message: &Message, name: &str) -> String {
    message.header(name).unwrap_or_default().to_lowercase()
}

fn has(message: &Message, label: &str) -> bool {
    message.label_ids.iter().any(|l| l == label)
}

fn ids(messages: &[Message]) -> BTreeSet<String> {
    messages.iter().filter_map(|m| m.id.clone()).collect()
}

async fn label_id(client: &Client, name: &str) -> String {
    client
        .gmail()
        .labels()
        .await
        .unwrap()
        .labels
        .into_iter()
        .find(|l| l.name.as_deref() == Some(name))
        .and_then(|l| l.id)
        .unwrap()
}

/// Runs `q` and checks that there are at least `min` results, all satisfying `check`.
async fn expect(
    client: &Client,
    q: &str,
    min: usize,
    check: impl Fn(&Message) -> bool,
) -> Vec<Message> {
    let results = search(client, q).await;
    assert!(
        results.len() >= min,
        "{q}: {} results, expected at least {min}",
        results.len()
    );
    for message in &results {
        assert!(
            check(message),
            "{q}: unexpected match {:?} {:?}",
            message.header("Subject"),
            message.label_ids
        );
    }
    results
}

#[tokio::test]
async fn address_and_subject_operators() {
    let server = support::server().await;
    let client = support::client(&server);

    let rachel = expect(&client, "from:rachel.kim@callowayfreight.com", 2, |m| {
        header(m, "From").contains("rachel.kim@")
    })
    .await;
    let domain = expect(&client, "from:callowayfreight.com", 3, |m| {
        header(m, "From").contains("@callowayfreight.com")
    })
    .await;
    assert!(ids(&domain).is_superset(&ids(&rachel)));
    expect(&client, r#"from:"Rachel Kim""#, 2, |m| {
        header(m, "From").contains("rachel")
    })
    .await;
    expect(&client, "to:priya", 3, |m| {
        [header(m, "To"), header(m, "Cc"), header(m, "Bcc")]
            .iter()
            .any(|h| h.contains("priya@"))
    })
    .await;
    expect(&client, "cc:daniel@brightmoor.co", 1, |m| {
        header(m, "Cc").contains("daniel@")
    })
    .await;
    expect(&client, "from:me", 20, |m| {
        has(m, "SENT") || has(m, "DRAFT")
    })
    .await;
    expect(&client, "subject:(MSA redlines)", 2, |m| {
        header(m, "Subject").contains("msa") && header(m, "Subject").contains("redlines")
    })
    .await;
    expect(&client, r#"subject:"launch checklist""#, 15, |m| {
        header(m, "Subject").contains("launch checklist")
    })
    .await;
    expect(&client, "list:productbrief.news", 5, |m| {
        header(m, "List-Id").contains("productbrief")
    })
    .await;
    expect(&client, "deliveredto:maya@brightmoor.co", 100, |m| {
        !has(m, "SENT")
    })
    .await;
    assert!(
        search(&client, "from:rach").await.is_empty(),
        "words match whole tokens, not prefixes"
    );

    let one = &rachel[0];
    let id = one.header("Message-ID").unwrap();
    let found = expect(&client, &format!("rfc822msgid:{id}"), 1, |m| m.id == one.id).await;
    assert_eq!(found.len(), 1);
}

#[tokio::test]
async fn label_state_and_category_operators() {
    let server = support::server().await;
    let client = support::client(&server);
    let calloway = label_id(&client, "Clients/Calloway").await;
    let atlas = label_id(&client, "Projects/Atlas Launch").await;

    expect(&client, "label:clients-calloway", 3, |m| has(m, &calloway)).await;
    let quoted = expect(&client, r#"label:"Projects/Atlas Launch""#, 10, |m| {
        has(m, &atlas)
    })
    .await;
    assert_eq!(
        ids(&quoted),
        ids(&search(&client, "label:projects-atlas-launch").await)
    );
    expect(&client, "in:inbox is:unread", 10, |m| {
        has(m, "INBOX") && has(m, "UNREAD")
    })
    .await;
    expect(&client, "in:inbox is:read", 5, |m| {
        has(m, "INBOX") && !has(m, "UNREAD")
    })
    .await;
    expect(&client, "is:starred", 2, |m| has(m, "STARRED")).await;
    expect(&client, "is:important", 50, |m| has(m, "IMPORTANT")).await;
    expect(&client, "-in:inbox from:jessmoreno.nyc@gmail.com", 1, |m| {
        !has(m, "INBOX")
    })
    .await;
    expect(&client, "category:promotions", 10, |m| {
        has(m, "CATEGORY_PROMOTIONS")
    })
    .await;
    expect(&client, "category:primary", 50, |m| {
        has(m, "CATEGORY_PERSONAL")
    })
    .await;
    expect(&client, "category:social", 5, |m| has(m, "CATEGORY_SOCIAL")).await;
    expect(&client, "has:userlabels", 10, |m| {
        m.label_ids.iter().any(|l| l.starts_with("Label_"))
    })
    .await;
    expect(&client, "has:nouserlabels", 100, |m| {
        !m.label_ids.iter().any(|l| l.starts_with("Label_"))
    })
    .await;
    assert!(search(&client, "is:snoozed").await.is_empty());

    // Spam and trash only appear when asked for.
    assert!(
        search(&client, "is:unread")
            .await
            .iter()
            .all(|m| !has(m, "SPAM") && !has(m, "TRASH"))
    );
    expect(&client, "in:spam", 5, |m| has(m, "SPAM")).await;
    expect(&client, "in:trash", 1, |m| has(m, "TRASH")).await;
    let everything = search(&client, "in:anywhere").await;
    assert_eq!(everything.len(), server_total(&client).await);
}

async fn server_total(client: &Client) -> usize {
    client
        .gmail()
        .profile()
        .await
        .unwrap()
        .messages_total
        .unwrap() as usize
}

#[tokio::test]
async fn attachment_date_and_size_operators() {
    let server = support::server().await;
    let client = support::client(&server);

    expect(&client, "has:attachment filename:pdf", 4, |_| true).await;
    expect(&client, "filename:csv", 2, |_| true).await;
    expect(&client, "filename:invite.ics", 3, |m| {
        header(m, "Subject").starts_with("invitation:")
    })
    .await;

    let midnight = |day: i8| {
        jiff::civil::date(2026, 11, day)
            .to_zoned(jiff::tz::TimeZone::get("America/New_York").unwrap())
            .unwrap()
            .timestamp()
            .as_millisecond()
    };
    let (from, to) = (midnight(10), midnight(12));
    expect(&client, "after:2026/11/10 before:2026/11/12", 5, |m| {
        (from..to).contains(&m.internal_date.unwrap())
    })
    .await;
    let epoch = expect(
        &client,
        &format!("after:{} before:{}", from / 1000, to / 1000),
        5,
        |_| true,
    )
    .await;
    assert_eq!(
        ids(&epoch),
        ids(&search(&client, "after:2026/11/10 before:2026/11/12").await)
    );

    let now = jiff::Timestamp::from_second(1_794_924_000).unwrap();
    let day = 86_400_000;
    expect(&client, "newer_than:2d", 10, |m| {
        m.internal_date.unwrap() >= now.as_millisecond() - 2 * day - 60_000
    })
    .await;
    expect(&client, "older_than:60d", 10, |m| {
        m.internal_date.unwrap() < now.as_millisecond() - 59 * day
    })
    .await;
    expect(&client, "larger:5K", 20, |m| {
        m.size_estimate.unwrap() > 5 * 1024
    })
    .await;
    expect(&client, "smaller:3K", 5, |m| {
        m.size_estimate.unwrap() < 3 * 1024
    })
    .await;
}

#[tokio::test]
async fn boolean_structure_and_free_text() {
    let server = support::server().await;
    let client = support::client(&server);

    let jess = ids(&search(&client, "from:jessmoreno.nyc@gmail.com").await);
    let liam = ids(&search(&client, "from:liam.oconnell@fastmail.com").await);
    let union: BTreeSet<String> = jess.union(&liam).cloned().collect();
    assert_eq!(
        ids(&search(
            &client,
            "from:jessmoreno.nyc@gmail.com OR from:liam.oconnell@fastmail.com"
        )
        .await),
        union
    );
    assert_eq!(
        ids(&search(
            &client,
            "{from:jessmoreno.nyc@gmail.com from:liam.oconnell@fastmail.com}"
        )
        .await),
        union
    );
    assert_eq!(
        ids(&search(
            &client,
            "from:{jessmoreno.nyc@gmail.com, liam.oconnell@fastmail.com}"
        )
        .await),
        union
    );
    expect(
        &client,
        "(from:priya OR from:hiro) subject:incident",
        1,
        |m| header(m, "Subject").contains("incident"),
    )
    .await;
    assert!(
        search(
            &client,
            "from:jessmoreno.nyc@gmail.com -from:jessmoreno.nyc@gmail.com"
        )
        .await
        .is_empty()
    );

    expect(&client, "sweet potato", 1, |_| true).await;
    let phrase = search(&client, r#""cross-dock transfers""#).await;
    assert!(!phrase.is_empty());
    let raw = client
        .gmail()
        .message(phrase[0].id.as_deref().unwrap(), Format::Raw, &[])
        .await
        .unwrap()
        .raw
        .unwrap();
    assert!(
        String::from_utf8_lossy(&raw)
            .to_lowercase()
            .contains("cross-dock")
    );
    assert!(
        search(&client, "foo:bar").await.is_empty(),
        "unknown operators are plain text, not errors"
    );

    // Split and contact-pane queries.
    expect(
        &client,
        r#"-in:DRAFT -in:CHAT in:inbox from:"callowayfreight.com" to:{maya@brightmoor.co}"#,
        1,
        |m| has(m, "INBOX") && header(m, "From").contains("callowayfreight.com"),
    )
    .await;
    let rachel = "rachel.kim@callowayfreight.com";
    expect(&client, &format!("(from:{{{rachel}}} OR to:{{{rachel}}} OR cc:{{{rachel}}} OR bcc:{{{rachel}}}) AND -in:DRAFT"), 3, |m| !has(m, "DRAFT")).await;
}

#[tokio::test]
async fn label_names_with_brackets_and_slashes() {
    let server = support::server().await;
    let client = support::client(&server);
    let gmail = client.gmail();
    let label = gmail
        .create_label(&Label {
            name: Some("[Superhuman]/ru".into()),
            ..Label::default()
        })
        .await
        .unwrap();
    let message = search(&client, "in:inbox is:read").await[0]
        .id
        .clone()
        .unwrap();
    gmail
        .modify_message(
            &message,
            &ModifyMessageRequest {
                add_label_ids: vec![label.id.clone().unwrap()],
                ..ModifyMessageRequest::default()
            },
        )
        .await
        .unwrap();
    let hits = search(&client, "in:[Superhuman]/ru").await;
    assert_eq!(ids(&hits), BTreeSet::from([message.clone()]));
    let unread_or_ru = ids(&search(&client, "label:INBOX (is:UNREAD OR in:[Superhuman]/ru)").await);
    assert!(unread_or_ru.contains(&message) && unread_or_ru.len() > 1);
}

#[tokio::test]
async fn threads_match_on_any_message_and_sort_by_latest() {
    let server = support::server().await;
    let client = support::client(&server);
    let gmail = client.gmail();

    // Jordan replied in a thread Theo started.
    let threads = gmail
        .threads(&ListQuery {
            q: Some("from:jordan subject:harborlight".into()),
            ..ListQuery::default()
        })
        .await
        .unwrap();
    let thread = gmail
        .thread(
            threads.threads[0].id.as_deref().unwrap(),
            Format::Metadata,
            &["From"],
        )
        .await
        .unwrap();
    assert!(thread.messages[0].header("From").unwrap().contains("theo@"));

    let mut latest = Vec::new();
    let mut token = None;
    loop {
        let page = gmail
            .threads(&ListQuery {
                max_results: Some(60),
                page_token: token,
                ..ListQuery::default()
            })
            .await
            .unwrap();
        for item in &page.threads {
            let thread = gmail
                .thread(item.id.as_deref().unwrap(), Format::Minimal, &[])
                .await
                .unwrap();
            let newest = thread
                .messages
                .iter()
                .filter(|m| !has(m, "SPAM") && !has(m, "TRASH"))
                .map(|m| m.internal_date.unwrap())
                .max()
                .unwrap();
            latest.push(newest);
            assert_eq!(
                item.snippet,
                thread.messages.last().unwrap().snippet,
                "list snippet is the latest message's"
            );
        }
        token = page.next_page_token;
        if token.is_none() {
            break;
        }
    }
    assert!(
        latest.windows(2).all(|w| w[0] >= w[1]),
        "threads sort by their latest message"
    );
    assert!(latest.len() > 200);
}

#[tokio::test]
async fn pagination_defaults_caps_and_tokens() {
    let server = support::server().await;
    let client = support::client(&server);
    let gmail = client.gmail();

    let first = gmail.messages(&ListQuery::default()).await.unwrap();
    assert_eq!(first.messages.len(), 100, "maxResults defaults to 100");
    let total = first.result_size_estimate.unwrap() as usize;
    assert!(total > 300);
    let capped = gmail
        .messages(&ListQuery {
            max_results: Some(1000),
            ..ListQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(
        capped.messages.len(),
        500.min(total),
        "maxResults is capped at 500"
    );

    let mut seen = Vec::new();
    let mut dates = Vec::new();
    let mut token = None;
    loop {
        let page = gmail
            .messages(&ListQuery {
                max_results: Some(37),
                page_token: token,
                ..ListQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(page.result_size_estimate, Some(total as u32));
        for item in page.messages {
            let message = gmail
                .message(item.id.as_deref().unwrap(), Format::Minimal, &[])
                .await
                .unwrap();
            dates.push(message.internal_date.unwrap());
            seen.push(item.id.unwrap());
        }
        token = page.next_page_token;
        if token.is_none() {
            break;
        }
    }
    assert_eq!(seen.len(), total);
    assert_eq!(
        seen.iter().collect::<BTreeSet<_>>().len(),
        total,
        "no duplicates across pages"
    );
    assert!(
        dates.windows(2).all(|w| w[0] >= w[1]),
        "newest first by internalDate"
    );

    let bad = gmail
        .messages(&ListQuery {
            page_token: Some("garbage".into()),
            ..ListQuery::default()
        })
        .await
        .unwrap_err();
    assert_eq!(bad.status(), Some(400));
}
