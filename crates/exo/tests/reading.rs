//! Reading a conversation end to end on gmail-mock: Open Links & Attachments (`⌘O`), replies
//! that quote a selection, the Bcc reply-all warning, the palette's Google apps rows and View
//! Raw Email Data (`|`). Links never reach a browser: the tests read what the app asked egui to
//! open.

mod support;

use egui::{Event, Key, Modifiers, OutputCommand, pos2};
use gmail::mime::{Attachment, Email, Mailbox};
use support::{Fixture, start};

/// ⌘ as the Mac sends it.
const CMD: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);
const ME: &str = "maya@brightmoor.co";
const AGENDA: &str = "https://www.brightmoor.co/offsite/agenda/";

/// Delivers `email` to the inbox and syncs.
fn deliver(app: &mut Fixture, email: &Email) {
    deliver_with(app, email, &["INBOX", "UNREAD", "IMPORTANT"]);
}

/// Delivers `email` with `labels` and syncs.
fn deliver_with(app: &mut Fixture, email: &Email, labels: &[&str]) {
    let delivery = serde_json::json!({ "raw": gmail::wire::encode_base64url(&email.to_rfc822()), "labelIds": labels });
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
    app.sync();
}

/// Priya's links for the offsite: three links in the body and a file.
fn offsite() -> Email {
    let html = format!(
        "<div>The <a href=\"{AGENDA}\">offsite agenda</a> is up: <a href=\"https://millbrookinn.example.com/reservations?id=48213\">click here</a> for the rooms.</div><div>Trains: <a href=\"https://www.amtrak.com/\">www.amtrak.com</a></div>"
    );
    Email {
        from: Some(Mailbox::new("Priya Raman", "priya@brightmoor.co")),
        to: vec![Mailbox::address(ME)],
        subject: "Links for the offsite".into(),
        html: Some(html),
        attachments: vec![Attachment {
            filename: "budget.pdf".into(),
            content_type: "application/pdf".into(),
            data: b"%PDF-1.4 budget".to_vec(),
            content_id: None,
        }],
        message_id: Some("offsite-links@brightmoor.co".into()),
        ..Email::default()
    }
}

/// Puts the list's cursor on the conversation titled `subject`.
fn cursor_to(app: &mut Fixture, subject: &str) {
    while app.cursor() != subject {
        app.key(Key::K);
    }
}

/// Presses `key` with `modifiers` and returns the URL that frame (or the next few) opened.
fn press_opening(app: &mut Fixture, key: Key, modifiers: Modifiers) -> Option<String> {
    let mut opened = None;
    for pressed in [true, false] {
        app.harness.input_mut().events.push(Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        });
        app.harness.run_steps(1);
        opened = opened.or_else(|| {
            app.harness
                .output()
                .platform_output
                .commands
                .iter()
                .find_map(|command| match command {
                    OutputCommand::OpenUrl(open) => Some(open.url.clone()),
                    _ => None,
                })
        });
    }
    app.settle();
    opened
}

/// Opens the palette, types `query` and runs the first row; returns the URL it opened.
fn palette_opening(app: &mut Fixture, query: &str) -> Option<String> {
    app.press(Key::K, CMD);
    app.type_text(query);
    press_opening(app, Key::Enter, Modifiers::NONE)
}

#[test]
fn google_apps_open_as_the_account() {
    let mut app = start("reading-google-apps");
    assert_eq!(
        palette_opening(&mut app, "open google drive").as_deref(),
        Some("https://drive.google.com/drive/?authuser=maya%40brightmoor.co")
    );
    assert_eq!(
        palette_opening(&mut app, "open filter settings").as_deref(),
        Some("https://mail.google.com/mail/?shxr=0&authuser=maya%40brightmoor.co#settings/filters")
    );
    assert_eq!(
        palette_opening(&mut app, "open gmail").as_deref(),
        Some("https://mail.google.com/mail/?shxr=0&authuser=maya%40brightmoor.co")
    );
    // In a thread, the first row opens the message in view in Gmail.
    app.key(Key::Enter);
    let focused = app
        .mail()
        .thread()
        .and_then(|thread| thread.focused())
        .expect("a focused message")
        .to_owned();
    let url = palette_opening(&mut app, "open in gmail").expect("opened");
    assert!(
        url.starts_with("https://mail.google.com/mail/?shxr=0&_shn="),
        "{url}"
    );
    assert!(
        url.ends_with(&format!("&authuser=maya%40brightmoor.co#all/{focused}")),
        "{url}"
    );
}

#[test]
fn cmd_o_opens_the_focused_messages_links_and_closes_again() {
    let mut app = start("reading-open-links");
    deliver(&mut app, &offsite());
    cursor_to(&mut app, "Links for the offsite");
    app.key(Key::Enter);
    app.press(Key::O, CMD);
    assert!(app.mail().open_links_shown());
    app.press(Key::O, CMD);
    assert!(!app.mail().open_links_shown(), "⌘O again closes it");
    app.press(Key::O, CMD);
    app.type_text("agenda");
    assert_eq!(
        press_opening(&mut app, Key::Enter, Modifiers::NONE).as_deref(),
        Some(AGENDA)
    );
    assert!(!app.mail().open_links_shown(), "the row closes the list");
}

#[test]
fn resting_on_a_link_in_the_focused_message_hints_at_cmd_o() {
    let mut app = start("reading-open-hint");
    deliver(&mut app, &offsite());
    cursor_to(&mut app, "Links for the offsite");
    app.key(Key::Enter);
    let line = (140..400)
        .step_by(4)
        .map(|y| pos2(260.0, y as f32))
        .find(|at| app.hover(*at) == egui::CursorIcon::Text)
        .expect("the body's first line")
        + egui::vec2(0.0, 4.0);
    let link = (0..60)
        .map(|step| pos2(260.0 + 8.0 * step as f32, line.y))
        .find(|at| app.hover(*at) == egui::CursorIcon::PointingHand)
        .expect("a link on the first line");
    app.hover(link);
    app.wait(0.4);
    assert_eq!(
        app.mail().hint_text().as_deref(),
        Some("Hit ⌘ O to open links instantly!")
    );
}

#[test]
fn cmd_o_previews_a_file_as_a_click_on_its_card_does() {
    let mut app = start("reading-open-file");
    for _ in 0..5 {
        app.key(Key::J);
    }
    app.key(Key::Enter);
    let detail = app
        .mail()
        .thread()
        .and_then(|thread| thread.detail())
        .expect("loaded");
    let attachment = detail
        .messages
        .last()
        .and_then(|message| message.attachments.first())
        .expect("the deck")
        .clone();
    let cached = app
        .previews()
        .join(format!("{}-{}", attachment.message_id, attachment.part_id))
        .join(&attachment.filename);
    app.press(Key::O, CMD);
    app.key(Key::Enter);
    app.settle();
    assert!(
        cached.is_file(),
        "the file's row downloads it into the preview cache"
    );
}

#[test]
fn cmd_enter_on_a_link_opens_it_and_keeps_the_list() {
    let mut app = start("reading-open-links-cmd");
    deliver(&mut app, &offsite());
    cursor_to(&mut app, "Links for the offsite");
    app.key(Key::Enter);
    app.press(Key::O, CMD);
    app.type_text("agenda");
    assert_eq!(
        press_opening(&mut app, Key::Enter, CMD).as_deref(),
        Some(AGENDA)
    );
    assert!(app.mail().open_links_shown(), "⌘ leaves the list open");
}

#[test]
fn shift_enter_on_a_file_downloads_it() {
    let mut app = start("reading-open-file-shift");
    let folder = app.folder("downloads");
    exo::downloads::choose(&app.harness.ctx, folder.clone()).expect("chosen");
    deliver(&mut app, &offsite());
    cursor_to(&mut app, "Links for the offsite");
    app.key(Key::Enter);
    app.press(Key::O, CMD);
    app.type_text("budget");
    app.press(Key::Enter, Modifiers::SHIFT);
    app.settle();
    assert!(
        folder.join("budget.pdf").is_file(),
        "saved to the download folder, not previewed"
    );
    assert!(!app.mail().open_links_shown());
}

#[test]
fn cmd_o_in_the_list_reads_the_conversation_for_its_links() {
    let mut app = start("reading-open-list");
    deliver(&mut app, &offsite());
    cursor_to(&mut app, "Links for the offsite");
    app.press(Key::O, CMD);
    assert!(
        app.mail().thread().is_none(),
        "the conversation stays closed"
    );
    app.wait(0.5);
    app.type_text("amtrak");
    assert_eq!(
        press_opening(&mut app, Key::Enter, Modifiers::NONE).as_deref(),
        Some("https://www.amtrak.com/")
    );
}

#[test]
fn cmd_o_in_all_mail_lists_an_archived_conversations_links() {
    let mut app = start("reading-open-archived");
    // Outside the inbox lists, whose first conversations sync keeps bodies for: when its bodies
    // aren't stored, the list fetches them (`Store::load_bodies`).
    deliver_with(&mut app, &offsite(), &[]);
    app.key(Key::G);
    app.key(Key::A);
    cursor_to(&mut app, "Links for the offsite");
    app.press(Key::O, CMD);
    app.wait(0.5);
    app.type_text("agenda");
    assert_eq!(
        press_opening(&mut app, Key::Enter, Modifiers::NONE).as_deref(),
        Some(AGENDA)
    );
}

#[test]
fn reply_all_to_a_message_you_got_as_bcc_warns() {
    let mut app = start("reading-bcc");
    let email = Email {
        from: Some(Mailbox::new("Daniel Reyes", "daniel@brightmoor.co")),
        to: vec![Mailbox::new("Grace Liu", "grace@northwind.vc")],
        cc: vec![Mailbox::new("Marcus Hale", "marcus@brightmoor.co")],
        bcc: vec![Mailbox::new("Maya Okafor", ME)],
        subject: "Intro: Grace and Calloway ops".into(),
        text: Some("Grace, meet the Calloway ops team.".into()),
        ..Email::default()
    };
    deliver(&mut app, &email);
    cursor_to(&mut app, "Intro: Grace and Calloway ops");
    app.key(Key::Enter);
    app.key(Key::R);
    assert_eq!(
        app.mail().hint_text(),
        None,
        "a reply goes to the sender alone"
    );
    // Escape leaves the empty reply, which goes.
    app.key(Key::Escape);
    assert_eq!(app.mail().compose_focus(), None);
    app.key(Key::A);
    assert_eq!(
        app.mail().hint_text().as_deref(),
        Some("Careful, you were on BCC! Reply instead of Reply All? esc R")
    );
}

#[test]
fn a_reply_quotes_the_text_selected_in_the_focused_message() {
    let mut app = start("reading-quote");
    let html = "<div>Can you send the <b>board deck</b> by Friday?</div><div>Thanks</div>";
    let delivery =
        serde_json::json!({ "from": "Ann Lee <ann@example.com>", "subject": "Deck", "html": html });
    let deliver = app.mock_url().join("__mock/deliver").expect("a URL");
    app.block_on(async {
        reqwest::Client::new()
            .post(deliver)
            .json(&delivery)
            .send()
            .await?
            .error_for_status()
    })
    .expect("delivered");
    app.sync();
    cursor_to(&mut app, "Deck");
    app.key(Key::Enter);
    // The body's first line: the first point down the column where the cursor is the I-beam.
    let x = 260.0;
    let line = (140..400)
        .step_by(4)
        .map(|y| pos2(x, y as f32))
        .find(|at| app.hover(*at) == egui::CursorIcon::Text)
        .expect("the body's first line")
        + egui::vec2(0.0, 4.0);
    // A triple click selects the first paragraph.
    app.hover(line);
    for _ in 0..3 {
        for pressed in [true, false] {
            app.harness.input_mut().events.push(Event::PointerButton {
                pos: line,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            app.harness.run_steps(1);
        }
    }
    app.settle();
    app.key(Key::R);
    app.wait(1.5);
    let gmail = app.gmail();
    let body = || {
        app.block_on(async {
            for draft in gmail
                .gmail()
                .drafts(&gmail::DraftQuery::default())
                .await
                .expect("drafts.list")
                .drafts
            {
                let full = gmail
                    .gmail()
                    .draft(&draft.id.expect("an id"), gmail::Format::Raw)
                    .await
                    .expect("drafts.get");
                let raw = String::from_utf8_lossy(
                    &full
                        .message
                        .and_then(|message| message.raw)
                        .unwrap_or_default(),
                )
                .into_owned();
                if raw.contains("Deck") {
                    return raw;
                }
            }
            String::new()
        })
    };
    let started = std::time::Instant::now();
    while !body().contains("<blockquote") && started.elapsed() < std::time::Duration::from_secs(10)
    {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let raw = body().replace("=\r\n", "");
    assert!(
        raw.contains("<blockquote"),
        "the reply quotes the selection: {raw}"
    );
    assert!(
        raw.contains("board deck</b>") && raw.contains("by Friday?"),
        "with its formatting: {raw}"
    );
    assert!(
        !raw.contains("Thanks</blockquote>"),
        "and nothing past it: {raw}"
    );
}

#[test]
fn a_reply_without_a_selection_quotes_nothing() {
    let mut app = start("reading-no-quote");
    app.key(Key::Enter);
    app.key(Key::R);
    app.wait(1.5);
    let gmail = app.gmail();
    let drafts = app.block_on(async {
        gmail
            .gmail()
            .drafts(&gmail::DraftQuery::default())
            .await
            .expect("drafts.list")
            .drafts
    });
    for draft in drafts {
        let full = app.block_on(async {
            gmail
                .gmail()
                .draft(&draft.id.expect("an id"), gmail::Format::Raw)
                .await
                .expect("drafts.get")
        });
        let raw = String::from_utf8_lossy(
            &full
                .message
                .and_then(|message| message.raw)
                .unwrap_or_default(),
        )
        .into_owned();
        assert!(
            !raw.contains("padding: 0 0 0 20px"),
            "no quote of a selection: {raw}"
        );
    }
}

#[test]
fn pipe_shows_the_conversation_as_gmail_sends_it_until_escape() {
    let mut app = start("reading-raw");
    app.key(Key::Enter);
    let thread = app
        .mail()
        .thread()
        .map(|thread| thread.id().to_owned())
        .expect("open");
    app.press(Key::Pipe, Modifiers::SHIFT);
    let started = std::time::Instant::now();
    while app
        .mail()
        .raw_source()
        .is_some_and(|(text, _)| text.is_empty())
        && started.elapsed() < std::time::Duration::from_secs(10)
    {
        app.wait(0.05);
    }
    let (text, selected) = app.mail().raw_source().expect("the viewer is open");
    assert!(
        text.starts_with(&format!("{{\n    \"id\": \"{thread}\",\n")),
        "{text}"
    );
    assert!(
        text.contains("\n        {\n            \"id\": "),
        "four spaces a level: {text}"
    );
    assert!(!text.contains("&#39;"), "snippets decoded: {text}");
    assert!(selected, "its text is selected once it's in");
    let source = text.to_owned();
    // ⌘C copies it all.
    let mut copied = None;
    for pressed in [true, false] {
        app.harness.input_mut().events.push(Event::Key {
            key: Key::C,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: CMD,
        });
        app.harness.run_steps(1);
        copied = copied.or_else(|| {
            app.harness
                .output()
                .platform_output
                .commands
                .iter()
                .find_map(|command| match command {
                    OutputCommand::CopyText(text) => Some(text.clone()),
                    _ => None,
                })
        });
    }
    assert_eq!(copied.as_deref(), Some(source.as_str()));
    app.key(Key::Escape);
    assert!(app.mail().raw_source().is_none());
    assert!(
        app.mail().thread().is_some(),
        "Escape closes the viewer, not the thread"
    );
}

#[test]
fn the_palette_offers_raw_email_data_on_a_conversation() {
    let mut app = start("reading-raw-palette");
    app.press(Key::K, CMD);
    app.type_text("view raw email data");
    app.key(Key::Enter);
    assert!(
        app.mail().raw_source().is_some(),
        "the list's highlighted conversation"
    );
}

#[test]
fn the_conversation_the_cursor_rests_on_gets_its_bodies_before_it_opens() {
    let mut app = start("reading-prefetch");
    let email = Email {
        from: Some(Mailbox::address("dana@contoso.example")),
        to: vec![Mailbox::address(ME)],
        subject: "Quarterly numbers".into(),
        text: Some("The numbers are in.".into()),
        ..Email::default()
    };
    // Out of the inbox, so sync stores it for its row alone.
    deliver_with(&mut app, &email, &["CATEGORY_UPDATES"]);
    app.key(Key::G);
    app.key(Key::E);
    assert_eq!(app.cursor(), "Quarterly numbers");
    // Opened with Gmail gone, it has its body already.
    app.disconnect();
    app.key(Key::Enter);
    let snapshot = app.mail().account().snapshot();
    assert!(
        snapshot.bodiless.is_empty(),
        "fetched while the cursor rested on it"
    );
    let text = snapshot
        .thread
        .as_ref()
        .and_then(|thread| thread.messages[0].body.text.clone());
    assert!(text.is_some_and(|text| text.contains("The numbers are in.")));
}

/// Priya asks to meet on a day the body names.
fn planning() -> Email {
    Email {
        from: Some(Mailbox::new("Priya Raman", "priya@brightmoor.co")),
        to: vec![
            Mailbox::address(ME),
            Mailbox::new("Tom Becker", "tom@brightmoor.co"),
        ],
        subject: "Planning sync".into(),
        html: Some("<div>Can we meet on Thursday, Nov 26 about the plan?</div>".into()),
        message_id: Some("planning-sync@brightmoor.co".into()),
        ..Email::default()
    }
}

#[test]
fn resting_on_a_date_shows_its_day_and_a_click_starts_an_event_for_the_conversation() {
    let mut app = start("reading-date");
    deliver(&mut app, &planning());
    cursor_to(&mut app, "Planning sync");
    app.key(Key::Enter);
    let line = (140..400)
        .step_by(4)
        .map(|y| pos2(260.0, y as f32))
        .find(|at| app.hover(*at) == egui::CursorIcon::Text)
        .expect("the body's first line")
        + egui::vec2(0.0, 4.0);
    let date = (0..80)
        .map(|step| pos2(260.0 + 6.0 * step as f32, line.y))
        .find(|at| app.hover(*at) == egui::CursorIcon::PointingHand)
        .expect("the date on the first line");
    app.hover(date);
    app.wait(0.4);
    assert_eq!(
        app.mail()
            .calendar_day()
            .map(|day| day.to_string())
            .as_deref(),
        Some("2026-11-26"),
        "the day shows in the right pane"
    );
    app.click(date);
    let (title, guests) = app
        .mail()
        .calendar_form_values()
        .expect("a new event's form");
    assert_eq!(title, "Planning sync");
    assert_eq!(
        guests,
        ["priya@brightmoor.co", "tom@brightmoor.co"],
        "everyone but me"
    );
}

#[test]
fn a_one_click_action_gets_a_palette_row() {
    let mut app = start("reading-action");
    let html = r#"<html><head><script type="application/ld+json">{"@context":"http://schema.org","@type":"EmailMessage","potentialAction":{"@type":"ViewAction","target":"https://shop.example.com/orders/1042","name":"View order"}}</script></head><body><p>Your desk shipped.</p></body></html>"#;
    let email = Email {
        from: Some(Mailbox::new("Oakline", "orders@shop.example.com")),
        to: vec![Mailbox::address(ME)],
        subject: "Your order has shipped".into(),
        html: Some(html.into()),
        ..Email::default()
    };
    deliver(&mut app, &email);
    cursor_to(&mut app, "Your order has shipped");
    app.key(Key::Enter);
    app.wait(0.2);
    assert_eq!(
        palette_opening(&mut app, "view order").as_deref(),
        Some("https://shop.example.com/orders/1042")
    );
}
