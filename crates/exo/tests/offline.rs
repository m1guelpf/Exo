//! The offline bar at the bottom of the window: what it says while Gmail can't be reached, rate
//! limits the account or says it has no mail.

mod support;

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use exo_ui::MessageBarColor;
use serde_json::json;
use support::start;

#[test]
fn the_bar_says_the_network_is_down() {
    let mut app = start("offline-network");
    assert_eq!(app.mail().status_bar(), None, "online, no bar");
    app.disconnect();
    // The OS says the network changed, and Gmail doesn't answer the check.
    app.mail().account().network_changed();
    app.settle();
    let bar = app.mail().status_bar().expect("the bar shows");
    assert_eq!((bar.title, bar.color), ("Offline", MessageBarColor::Blue));
    assert_eq!(
        bar.text(),
        "Good news: Exo works offline! Messages will send and receive when next online."
    );
}

#[test]
fn the_footer_counts_what_waits_for_the_network() {
    let mut app = start("offline-footer");
    assert_eq!(app.mail().footer_line(), None, "synced, and nothing waits");
    app.disconnect();
    app.mail().account().network_changed();
    app.settle();
    app.key(Key::E);
    assert_eq!(
        app.mail().footer_line(),
        Some("1 change to upload"),
        "Mark Done waits"
    );
    app.key(Key::E);
    assert_eq!(app.mail().footer_line(), Some("2 changes to upload"));
}

#[test]
fn a_conversation_opened_offline_fills_in_once_gmail_answers() {
    let mut app = start("offline-bodies");
    // Mail out of the inbox, stored for its rows alone, on top of Done.
    let gmail = app.gmail();
    let email = gmail::mime::Email {
        from: Some(gmail::mime::Mailbox::address("dana@contoso.example")),
        to: vec![gmail::mime::Mailbox::address("maya@brightmoor.co")],
        subject: "Quarterly numbers".into(),
        text: Some("The numbers are in.".into()),
        ..gmail::mime::Email::default()
    };
    let options = gmail::InsertOptions {
        label_ids: vec!["CATEGORY_UPDATES".into()],
        ..gmail::InsertOptions::default()
    };
    app.block_on(async {
        gmail
            .gmail()
            .insert_message(email.to_rfc822(), &options)
            .await
    })
    .expect("messages.insert");
    app.sync();
    app.disconnect();
    app.key(Key::G);
    app.key(Key::E);
    assert_eq!(app.cursor(), "Quarterly numbers");
    app.key(Key::Enter);
    let snapshot = app.mail().account().snapshot();
    let thread = snapshot
        .thread
        .as_ref()
        .expect("the conversation shows, its headers at once");
    assert_eq!(
        snapshot.bodiless,
        [thread.messages[0].id.clone()],
        "its body a placeholder"
    );
    // A reply would quote nothing: it doesn't start.
    app.key(Key::R);
    assert_eq!(
        app.mail().thread_drafts(),
        0,
        "no reply without the body it quotes"
    );

    // Gmail answers again: the body comes in without the conversation opening again.
    app.reconnect();
    app.sync();
    let snapshot = app.mail().account().snapshot();
    assert!(snapshot.bodiless.is_empty());
    let text = snapshot
        .thread
        .as_ref()
        .and_then(|thread| thread.messages[0].body.text.clone());
    assert!(
        text.is_some_and(|text| text.contains("The numbers are in.")),
        "the body is in"
    );
    app.key(Key::R);
    assert_eq!(app.mail().thread_drafts(), 1, "and a reply starts");
}

#[test]
fn the_bar_says_gmail_doesnt_answer() {
    let mut app = start("offline-gmail");
    app.disconnect();
    app.sync();
    let bar = app.mail().status_bar().expect("the bar shows");
    assert_eq!(bar.title, "Offline");
    assert_eq!(
        bar.text(),
        "Gmail is not responding right now. Exo will work offline and catch up when Gmail responds."
    );
}

#[test]
fn the_bar_says_gmail_is_throttling() {
    let mut app = start("offline-throttled");
    let at = jiff::Timestamp::now() + jiff::SignedDuration::from_mins(10);
    app.fault(Some(json!({ "path": "/gmail/v1/", "status": 429, "message": format!("User-rate limit exceeded.  Retry after {at}") })));
    app.sync();
    let bar = app
        .mail()
        .status_bar()
        .expect("the bar shows once a call gives up");
    assert_eq!((bar.title, bar.color), ("Note", MessageBarColor::Blue));
    assert_eq!(
        bar.text(),
        "Gmail is temporarily throttling requests. Exo will work offline and catch up when Gmail responds."
    );
}

#[test]
fn a_disabled_gmail_turns_the_bar_red_and_links_to_gmail() {
    let mut app = start("offline-disabled");
    app.fault(Some(json!({ "path": "/gmail/v1/", "status": 400, "message": "Mail service not enabled", "reason": "failedPrecondition", "times": 1000 })));
    app.sync();
    let bar = app.mail().status_bar().expect("the bar shows");
    assert_eq!((bar.title, bar.color), ("Error", MessageBarColor::Red));
    assert_eq!(
        bar.text(),
        "Your Gmail account has been disabled. Please open mail.google.com for more information."
    );

    // `mail.google.com`, at 1280×800 (390–478, 777–792).
    let link = Pos2::new(430.0, 785.0);
    app.harness
        .input_mut()
        .events
        .push(Event::PointerMoved(link));
    app.harness.run_steps(1);
    assert_eq!(
        app.harness.output().platform_output.cursor_icon,
        egui::CursorIcon::PointingHand
    );
    let mut opened = None;
    for pressed in [true, false] {
        app.harness.input_mut().events.push(Event::PointerButton {
            pos: link,
            button: PointerButton::Primary,
            pressed,
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
    assert_eq!(opened.as_deref(), Some("https://mail.google.com/?shxr=0"));
}
