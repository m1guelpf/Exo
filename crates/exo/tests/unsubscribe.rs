//! Unsubscribe end to end on gmail-mock: `⌘U` and its picker, the
//! confirmation, each way of unsubscribing (a one-click POST to the mock's own endpoint, a
//! `mailto:` sent from the account, a web link opened), the Mark Done and Trash all rows with
//! Undo, the offer to block when there's nothing to unsubscribe from, and offline.

mod support;

use egui::{Key, Modifiers};
use gmail::mime::{Email, Mailbox};
use gmail::{Format, ListQuery};
use serde_json::Value;
use support::{Fixture, start, start_with_settings};

/// ⌘ as the Mac sends it.
const CMD: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);
const ME: &str = "maya@brightmoor.co";
const SENDER: &str = "deals@shop.example";

/// The mock's one-click endpoint for `list`.
fn endpoint(app: &Fixture, list: &str) -> String {
    format!("{}__mock/unsubscribe/{list}", app.mock_url())
}

/// Delivers a newsletter from [`SENDER`] titled `subject`, with `List-Unsubscribe: {header}`
/// (and one-click when `one_click`), and syncs. The app's clock moves to the mock's, as the two
/// run together outside tests: a bulk action is on mail up to now.
fn deliver(app: &mut Fixture, subject: &str, header: Option<&str>, one_click: bool) {
    let mut headers = vec![
        ("Delivered-To".to_owned(), ME.to_owned()),
        (
            "List-Id".to_owned(),
            "Shop Deals <deals.shop.example>".to_owned(),
        ),
    ];
    headers.extend(header.map(|header| ("List-Unsubscribe".to_owned(), header.to_owned())));
    if one_click {
        headers.push((
            "List-Unsubscribe-Post".to_owned(),
            "List-Unsubscribe=One-Click".to_owned(),
        ));
    }
    let email = Email {
        from: Some(Mailbox::new("Shop Deals", SENDER)),
        to: vec![Mailbox::address(ME)],
        subject: subject.to_owned(),
        text: Some(format!("{subject}: this week's deals.")),
        message_id: Some(format!("{}@shop.example", subject.replace(' ', "-"))),
        headers,
        ..Email::default()
    };
    let delivery = serde_json::json!({ "raw": gmail::wire::encode_base64url(&email.to_rfc822()), "labelIds": ["INBOX", "UNREAD", "CATEGORY_PROMOTIONS"] });
    let url = app.mock_url().join("__mock/deliver").expect("a URL");
    app.block_on(async {
        reqwest::Client::new()
            .post(url)
            .json(&delivery)
            .send()
            .await?
            .error_for_status()
    })
    .expect("delivered");
    let clock = app.mock_url().join("__mock/clock").expect("a URL");
    let clock: Value = app
        .block_on(async { reqwest::get(clock).await?.json().await })
        .expect("the mock's clock");
    let now: jiff::Timestamp = clock["now"]
        .as_str()
        .and_then(|now| now.parse().ok())
        .expect("a time");
    app.set_now(now.to_zoned(jiff::tz::TimeZone::get("America/New_York").expect("a zone")));
    app.sync();
}

/// Puts the list's cursor on the conversation titled `subject`.
fn cursor_to(app: &mut Fixture, subject: &str) {
    while app.cursor() != subject {
        app.key(Key::K);
    }
}

/// The one-click unsubscribes the mock took.
fn unsubscribes(app: &Fixture) -> Vec<Value> {
    let url = app.mock_url().join("__mock/unsubscribes").expect("a URL");
    let list: Value = app
        .block_on(async { reqwest::get(url).await?.json().await })
        .expect("the mock answers");
    list.as_array().cloned().unwrap_or_default()
}

/// How many threads Gmail finds for `q`.
fn count(app: &Fixture, q: &str) -> usize {
    let gmail = app.gmail();
    let query = ListQuery {
        q: Some(q.into()),
        max_results: Some(500),
        ..ListQuery::default()
    };
    app.block_on(async { gmail.gmail().threads(&query).await })
        .expect("threads.list")
        .threads
        .len()
}

/// The alert showing, its title and text, the `<wbr>`s in its text left out.
fn alert(app: &Fixture) -> Option<(String, String)> {
    app.mail()
        .alert()
        .map(|(title, text)| (title.to_owned(), text.replace('\u{200B}', "")))
}

/// The open thread's subject.
fn open_subject(app: &Fixture) -> Option<String> {
    app.mail()
        .thread()
        .and_then(|thread| thread.detail())
        .map(|detail| detail.subject.clone())
}

#[test]
fn a_one_click_unsubscribe_posts_to_the_list_after_confirming() {
    let mut app = start("unsubscribe-one-click");
    let link = endpoint(&app, "shop.example/u?id=7");
    deliver(&mut app, "Weekend sale", Some(&format!("<{link}>")), true);
    cursor_to(&mut app, "Weekend sale");
    app.key(Key::Enter);
    app.press(Key::U, CMD);
    // The picker's first row, `Unsubscribe`, then its alert.
    app.key(Key::Enter);
    assert_eq!(
        alert(&app),
        Some((
            "Unsubscribe".into(),
            "Would you like to unsubscribe from deals@shop.example?".into()
        ))
    );
    assert!(unsubscribes(&app).is_empty(), "nothing before Confirm");
    app.key(Key::Enter);
    assert_eq!(app.mail().toast(), Some("Unsubscribed."));
    let posted = unsubscribes(&app);
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0]["list"], "shop.example/u");
    assert_eq!(posted[0]["query"], "id=7");
    assert_eq!(
        posted[0]["contentType"],
        "application/x-www-form-urlencoded"
    );
    assert_eq!(posted[0]["body"], "List-Unsubscribe=One-Click");
    assert_eq!(
        posted[0]["credentials"],
        serde_json::json!([]),
        "no cookies or credentials"
    );
    assert_eq!(
        open_subject(&app).as_deref(),
        Some("Weekend sale"),
        "a lone Unsubscribe doesn't move on"
    );
}

#[test]
fn a_mailto_unsubscribe_sends_the_request_from_the_account() {
    let mut app = start("unsubscribe-mailto");
    deliver(
        &mut app,
        "Daily digest",
        Some("<mailto:leave-123@lists.shop.example?subject=unsubscribe%20123&body=ignored>"),
        false,
    );
    cursor_to(&mut app, "Daily digest");
    // From the list: the conversation's messages are read for it.
    app.press(Key::U, CMD);
    app.key(Key::Enter);
    app.key(Key::Enter);
    assert_eq!(app.mail().toast(), Some("Unsubscribed."));
    app.wait(1.0);

    let gmail = app.gmail();
    let sent = app.block_on(async {
        let query = ListQuery {
            q: Some("to:leave-123@lists.shop.example".into()),
            label_ids: vec!["SENT".into()],
            ..ListQuery::default()
        };
        let listed = gmail
            .gmail()
            .messages(&query)
            .await
            .expect("messages.list")
            .messages;
        let mut full = Vec::new();
        for message in listed {
            full.push(
                gmail
                    .gmail()
                    .message(&message.id.expect("an id"), Format::Full, &[])
                    .await
                    .expect("messages.get"),
            );
        }
        full
    });
    let [message] = &sent[..] else {
        panic!("one request in Sent: {sent:?}")
    };
    assert_eq!(message.header("Subject"), Some("unsubscribe 123"));
    assert!(
        message
            .header("To")
            .is_some_and(|to| to.trim_matches(['<', '>']) == "leave-123@lists.shop.example"),
        "{:?}",
        message.header("To")
    );
    assert!(
        message.header("From").is_some_and(|from| from.contains(ME)),
        "{:?}",
        message.header("From")
    );
    assert_eq!(message.header("Cc"), None);
    let text = message
        .payload
        .as_ref()
        .and_then(|payload| {
            payload
                .walk()
                .find(|part| part.mime_type.as_deref() == Some("text/plain"))
        })
        .and_then(|part| part.body.as_ref()?.data.clone())
        .expect("a text part");
    let text = String::from_utf8(text).expect("UTF-8");
    assert!(
        text.contains(
            "This is an unsubscribe request sent from Exo on behalf of maya@brightmoor.co"
        ),
        "{text:?}"
    );
    assert!(!text.contains("ignored"), "the link's body isn't sent");
}

#[test]
fn a_web_link_opens_in_the_browser_without_a_toast() {
    let mut app = start("unsubscribe-web");
    deliver(
        &mut app,
        "Prefs",
        Some("<https://shop.example/preferences>"),
        false,
    );
    cursor_to(&mut app, "Prefs");
    app.press(Key::U, CMD);
    app.key(Key::Enter);
    let mut opened = None;
    for pressed in [true, false] {
        app.harness.input_mut().events.push(egui::Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        app.harness.run_steps(1);
        opened = opened.or_else(|| {
            app.harness
                .output()
                .platform_output
                .commands
                .iter()
                .find_map(|command| match command {
                    egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
                    _ => None,
                })
        });
    }
    app.settle();
    assert_eq!(opened.as_deref(), Some("https://shop.example/preferences"));
    assert_eq!(
        app.mail().toast(),
        None,
        "the browser does the unsubscribing"
    );
}

#[test]
fn with_nothing_to_unsubscribe_from_it_offers_to_block() {
    let mut app = start("unsubscribe-block");
    // A colleague's mail, with no unsubscribe anywhere.
    app.press(Key::U, CMD);
    let (title, text) = alert(&app).expect("an alert");
    assert_eq!(title, "Block instead?");
    assert!(text.starts_with("There isn't an unsubscribe link here.\n\nWould you like to block instead? This will trash all messages, including any future messages, from "), "{text}");
    app.key(Key::Escape);
    assert_eq!(alert(&app), None);
}

#[test]
fn a_crafted_mailto_is_never_sent_and_counts_as_no_link() {
    let mut app = start("unsubscribe-crafted");
    deliver(
        &mut app,
        "Crafted",
        Some("<mailto:leave@lists.shop.example?bcc=someone@else.example>"),
        false,
    );
    cursor_to(&mut app, "Crafted");
    app.press(Key::U, CMD);
    assert_eq!(
        alert(&app).map(|(title, _)| title).as_deref(),
        Some("Block instead?")
    );
    app.key(Key::Escape);
    assert_eq!(count(&app, "in:sent to:leave@lists.shop.example"), 0);
}

#[test]
fn unsubscribe_and_mark_done_all_moves_on_and_undo_brings_them_back() {
    let mut app = start("unsubscribe-mark-done");
    let link = endpoint(&app, "shop.example/u");
    for subject in ["Deal one", "Deal two", "Deal three"] {
        deliver(&mut app, subject, Some(&format!("<{link}>")), true);
    }
    assert_eq!(count(&app, "in:inbox from:deals@shop.example"), 3);
    cursor_to(&mut app, "Deal three");
    app.press(Key::U, CMD);
    app.key(Key::ArrowDown);
    app.key(Key::Enter);
    assert_eq!(alert(&app), Some(("Unsubscribe & Mark Done".into(), "Would you like to unsubscribe and Mark Done all existing messages from deals@shop.example?".into())));
    app.key(Key::Enter);
    assert_eq!(
        app.mail().toast(),
        Some("Unsubscribed, and Marking Done all from deals@shop.example.")
    );
    assert_ne!(app.cursor(), "Deal three", "from the inbox it moves on");
    app.sync();
    assert_eq!(count(&app, "in:inbox from:deals@shop.example"), 0);
    assert_eq!(unsubscribes(&app).len(), 1);
    assert!(
        !app.subjects()
            .iter()
            .any(|subject| subject.starts_with("Deal ")),
        "{:?}",
        app.subjects()
    );

    app.key(Key::Z);
    assert_eq!(app.mail().toast(), Some("Unsubscribed."));
    app.sync();
    assert_eq!(
        count(&app, "in:inbox from:deals@shop.example"),
        3,
        "back in the inbox"
    );
}

#[test]
fn unsubscribe_and_trash_all_from_a_thread_opens_the_next_conversation() {
    let mut app = start("unsubscribe-trash");
    let link = endpoint(&app, "shop.example/u");
    for subject in ["Offer one", "Offer two"] {
        deliver(&mut app, subject, Some(&format!("<{link}>")), true);
    }
    cursor_to(&mut app, "Offer two");
    app.key(Key::Enter);
    app.press(Key::U, CMD);
    app.key(Key::ArrowDown);
    app.key(Key::ArrowDown);
    app.key(Key::Enter);
    assert_eq!(alert(&app), Some(("Unsubscribe & Trash".into(), "Would you like to unsubscribe and Trash all existing messages from deals@shop.example?".into())));
    app.key(Key::Enter);
    assert_eq!(
        app.mail().toast(),
        Some("Unsubscribed, and Trashing all from deals@shop.example.")
    );
    let open = open_subject(&app);
    assert!(
        open.is_some() && open.as_deref() != Some("Offer two"),
        "the next conversation opens: {open:?}"
    );
    app.sync();
    assert_eq!(count(&app, "in:trash from:deals@shop.example"), 2);
}

#[test]
fn with_auto_advance_off_trash_all_goes_back_to_the_list() {
    let mut app = start_with_settings("unsubscribe-trash-off", &|settings| {
        settings.desktop_auto_advance = exo_store::AutoAdvance::Off
    });
    let link = endpoint(&app, "shop.example/u");
    deliver(&mut app, "Offer", Some(&format!("<{link}>")), true);
    cursor_to(&mut app, "Offer");
    app.key(Key::Enter);
    app.press(Key::U, CMD);
    app.key(Key::ArrowDown);
    app.key(Key::ArrowDown);
    app.key(Key::Enter);
    app.key(Key::Enter);
    assert_eq!(
        app.mail().toast(),
        Some("Unsubscribed, and Trashing all from deals@shop.example.")
    );
    assert_eq!(open_subject(&app), None, "back to the list");
}

#[test]
fn the_picker_blocks_the_senders_domain() {
    let mut app = start("unsubscribe-block-domain");
    let link = endpoint(&app, "shop.example/u");
    deliver(&mut app, "Domain deal", Some(&format!("<{link}>")), true);
    cursor_to(&mut app, "Domain deal");
    app.press(Key::U, CMD);
    for _ in 0..3 {
        app.key(Key::ArrowDown);
    }
    app.key(Key::Enter);
    assert_eq!(
        alert(&app),
        Some((
            "Block domain".into(),
            "Trash all messages, including any future messages, from shop.example?".into()
        ))
    );
    app.key(Key::Escape);
    assert!(unsubscribes(&app).is_empty());
}

#[test]
fn offline_the_unsubscribe_waits_through_a_relaunch_and_goes_once_back() {
    let mut app = start("unsubscribe-offline");
    let link = endpoint(&app, "shop.example/later");
    deliver(&mut app, "Later", Some(&format!("<{link}>")), true);
    cursor_to(&mut app, "Later");
    app.key(Key::Enter);
    app.disconnect();
    app.mail().account().network_changed();
    app.settle();
    assert!(app.mail().status_bar().is_some(), "offline");
    app.press(Key::U, CMD);
    app.key(Key::Enter);
    app.key(Key::Enter);
    assert_eq!(
        app.mail().toast(),
        Some("Unsubscribe will happen when online.")
    );

    // Quitting meanwhile doesn't lose it.
    app.relaunch();
    app.reconnect();
    let started = std::time::Instant::now();
    while unsubscribes(&app).is_empty() && started.elapsed() < std::time::Duration::from_secs(20) {
        app.wait(0.5);
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let posted = unsubscribes(&app);
    assert_eq!(posted.len(), 1, "posted once back online");
    assert_eq!(posted[0]["list"], "shop.example/later");
}
