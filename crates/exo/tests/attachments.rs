//! A conversation's files end to end on gmail-mock: the file notifications while a file
//! downloads and once it has, the alert offline, `Download all`'s zip, and Copy
//! Attachments pasted into a draft.

mod support;

use egui::{Event, Key, Modifiers, pos2};
use gmail::mime::{Attachment, Email, Mailbox};
use support::{Fixture, start};

/// ⌘ as the Mac sends it.
const CMD: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);
const ME: &str = "maya@brightmoor.co";
const DECK: &str = "Brightmoor_Board_Deck_Nov2026_v2.pdf";

/// Delivers `email` to the inbox and syncs.
fn deliver(app: &mut Fixture, email: &Email) {
    let delivery = serde_json::json!({ "raw": gmail::wire::encode_base64url(&email.to_rfc822()), "labelIds": ["INBOX", "UNREAD", "IMPORTANT"] });
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

/// Priya's venue photo and agenda, two files in one message.
fn offsite() -> Email {
    let file = |filename: &str, content_type: &str, data: &[u8]| Attachment {
        filename: filename.into(),
        content_type: content_type.into(),
        data: data.to_vec(),
        content_id: None,
    };
    Email {
        from: Some(Mailbox::new("Priya Raman", "priya@brightmoor.co")),
        to: vec![Mailbox::address(ME)],
        subject: "Offsite venue and agenda".into(),
        html: Some("<div>Here they are.</div>".into()),
        attachments: vec![
            file("agenda.pdf", "application/pdf", b"%PDF-1.4 agenda"),
            file("agenda.pdf", "application/pdf", b"%PDF-1.4 second"),
        ],
        message_id: Some("offsite-files@brightmoor.co".into()),
        ..Email::default()
    }
}

/// Opens the Board deck conversation, whose last message has the deck.
fn open_board_deck(app: &mut Fixture) {
    for _ in 0..5 {
        app.key(Key::J);
    }
    app.key(Key::Enter);
}

/// Opens the palette, types `query` and runs the first row.
fn palette(app: &mut Fixture, query: &str) {
    app.press(Key::K, CMD);
    app.harness
        .input_mut()
        .events
        .push(Event::Text(query.to_owned()));
    app.harness.run_steps(1);
    app.key(Key::Enter);
}

/// Clicks at `at` with ⌘ held.
fn cmd_click(app: &mut Fixture, at: egui::Pos2) {
    app.hover(at);
    app.harness
        .input_mut()
        .events
        .push(Event::ModifiersChanged(CMD));
    for pressed in [true, false] {
        app.harness.input_mut().events.push(Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: CMD,
        });
        app.harness.run_steps(1);
    }
    app.harness
        .input_mut()
        .events
        .push(Event::ModifiersChanged(Modifiers::NONE));
    app.settle();
}

/// Waits until `done` holds, at most five seconds.
fn wait_for(app: &mut Fixture, mut done: impl FnMut(&Fixture) -> bool) {
    for _ in 0..20 {
        if done(app) {
            return;
        }
        app.wait(0.25);
    }
}

/// The names in `folder`.
fn saved(folder: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn a_download_says_so_and_offers_show_until_it_goes() {
    let mut app = start("attachments-notification");
    let folder = app.folder("downloads");
    exo::downloads::choose(&app.harness.ctx, folder.clone()).expect("chosen");
    open_board_deck(&mut app);
    cmd_click(&mut app, pos2(320.0, 320.0));
    wait_for(&mut app, |_| !saved(&folder).is_empty());
    assert_eq!(saved(&folder), [DECK]);
    assert_eq!(
        app.mail().file_notifications(),
        [DECK],
        "its notification stays once it's saved, with Show"
    );
    // It fades after 6 s and is gone at 8.
    app.wait(8.5);
    assert!(app.mail().file_notifications().is_empty());
}

#[test]
fn a_preview_lets_its_notification_go_once_it_opens() {
    let mut app = start("attachments-preview-notification");
    open_board_deck(&mut app);
    app.click(pos2(320.0, 320.0));
    wait_for(&mut app, |app| {
        app.previews()
            .read_dir()
            .is_ok_and(|mut entries| entries.next().is_some())
    });
    app.wait(2.0);
    assert!(
        app.mail().file_notifications().is_empty(),
        "cleared 1.5 s after the preview: {:?}",
        app.mail().file_notifications()
    );
}

#[test]
fn a_file_not_kept_says_offline_that_it_isnt_available() {
    let mut app = start("attachments-offline");
    open_board_deck(&mut app);
    app.disconnect();
    app.click(pos2(320.0, 320.0));
    wait_for(&mut app, |app| app.mail().alert().is_some());
    assert_eq!(
        app.mail().alert(),
        Some((
            "Sorry!",
            "This attachment is not available offline.\nPlease check your connection and try again."
        ))
    );
    assert!(
        app.mail().file_notifications().is_empty(),
        "no notification: the alert clears it"
    );
}

/// Where `Download all` is under the open message's first card: past the card and the gap
/// under it, where the pointer turns into a hand again.
fn download_all(app: &mut Fixture) -> egui::Pos2 {
    let (mut on_card, mut past_card) = (false, false);
    for step in 0..250 {
        let at = pos2(280.0, 250.0 + 2.0 * step as f32);
        let hand = app.hover(at) == egui::CursorIcon::PointingHand;
        if hand && past_card {
            return at + egui::vec2(0.0, 6.0);
        }
        on_card |= hand;
        past_card |= on_card && !hand;
    }
    panic!("no Download all");
}

#[test]
fn download_all_zips_a_messages_files_into_the_download_folder() {
    let mut app = start("attachments-download-all");
    let folder = app.folder("zip-downloads");
    exo::downloads::choose(&app.harness.ctx, folder.clone()).expect("chosen");
    deliver(&mut app, &offsite());
    while app.cursor() != "Offsite venue and agenda" {
        app.key(Key::K);
    }
    app.key(Key::Enter);
    assert_eq!(
        app.mail()
            .thread()
            .and_then(|thread| thread.detail())
            .map(|detail| detail.subject.clone())
            .as_deref(),
        Some("Offsite venue and agenda")
    );
    let at = download_all(&mut app);
    app.click(at);
    wait_for(&mut app, |_| !saved(&folder).is_empty());
    assert_eq!(saved(&folder), ["offsite_venue_and_agenda_1.zip"]);
    assert_eq!(
        app.mail().file_notifications(),
        ["offsite_venue_and_agenda_1.zip"]
    );
    let zip = folder.join("offsite_venue_and_agenda_1.zip");
    let listing = std::process::Command::new("unzip")
        .arg("-Z1")
        .arg(&zip)
        .output()
        .expect("unzip runs");
    assert_eq!(
        String::from_utf8_lossy(&listing.stdout)
            .lines()
            .collect::<Vec<_>>(),
        ["agenda.pdf", "agenda (1).pdf"],
        "a name taken gets a number"
    );
}

#[test]
fn copied_attachments_paste_into_a_draft() {
    let mut app = start("attachments-copy");
    exo::clipboard::capture(&app.harness.ctx);
    open_board_deck(&mut app);
    palette(&mut app, "Copy Attachments");
    assert_eq!(
        app.mail().hint_text().as_deref(),
        Some("Attachments copied to clipboard. Hit ⌘ V in a draft to paste them.")
    );
    app.key(Key::R);
    app.harness
        .input_mut()
        .events
        .push(Event::Paste(String::new()));
    app.harness.run_steps(1);
    app.settle();
    assert_eq!(
        app.mail().compose_attachments(),
        [DECK],
        "on the reply, by reference to the message's part"
    );
    assert_eq!(app.mail().compose_alert(), None);
}

#[test]
fn another_accounts_attachments_dont_paste() {
    let mut app = start("attachments-copy-other");
    exo::clipboard::capture(&app.harness.ctx);
    let copied = serde_json::json!({ "account": "bea@example.com", "attachments": [{ "message_id": "18f3a", "attachment_id": null, "part_id": "1", "name": "deck.pdf", "mime_type": "application/pdf", "size": 3 }] });
    exo::clipboard::capture_attachments(&app.harness.ctx, &copied.to_string());
    open_board_deck(&mut app);
    app.key(Key::R);
    app.harness
        .input_mut()
        .events
        .push(Event::Paste(String::new()));
    app.harness.run_steps(1);
    app.settle();
    assert!(app.mail().compose_attachments().is_empty());
    assert_eq!(
        app.mail().compose_alert(),
        Some(
            "Sorry! Attachments cannot be copy & pasted between email accounts. If you'd like to include attachments, first download them and then attach them to this draft."
        )
    );
}

/// Priya's venue photo, agenda and the inn's confirmation as a message.
fn venue() -> Email {
    let png = {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(60, 40, image::Rgba([70, 130, 190, 255]))
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("encoded");
        bytes
    };
    let file = |filename: &str, content_type: &str, data: &[u8]| Attachment {
        filename: filename.into(),
        content_type: content_type.into(),
        data: data.to_vec(),
        content_id: None,
    };
    let eml = b"From: Millbrook Inn <reservations@millbrookinn.example.com>\r\nSubject: Your room block is confirmed\r\nContent-Type: text/html\r\n\r\n<p>Dear Priya,</p>\r\n";
    Email {
        from: Some(Mailbox::new("Priya Raman", "priya@brightmoor.co")),
        to: vec![Mailbox::address(ME)],
        subject: "Offsite venue".into(),
        html: Some("<div>Here they are.</div>".into()),
        attachments: vec![
            file("lake.png", "image/png", &png),
            file("agenda.pdf", "application/pdf", b"%PDF-1.4 agenda"),
            file("Room block.eml", "message/rfc822", eml),
        ],
        message_id: Some("offsite-venue@brightmoor.co".into()),
        ..Email::default()
    }
}

#[test]
fn images_and_messages_open_in_the_viewer_and_move_with_the_arrows() {
    let mut app = start("attachments-viewer");
    deliver(&mut app, &venue());
    while app.cursor() != "Offsite venue" {
        app.key(Key::K);
    }
    app.key(Key::Enter);
    // The photo's card, the first under the body.
    let card = (0..60)
        .map(|step| pos2(280.0, 250.0 + 4.0 * step as f32))
        .find(|at| app.hover(*at) == egui::CursorIcon::PointingHand)
        .expect("a card");
    app.click(card + egui::vec2(0.0, 20.0));
    assert_eq!(app.mail().previewed(), Some("lake.png"));
    // The viewer has images and messages; the PDF goes to Quick Look.
    app.key(Key::ArrowRight);
    assert_eq!(app.mail().previewed(), Some("Room block.eml"));
    app.key(Key::ArrowRight);
    assert_eq!(
        app.mail().previewed(),
        Some("Room block.eml"),
        "the last stays"
    );
    app.key(Key::ArrowLeft);
    assert_eq!(app.mail().previewed(), Some("lake.png"));
    app.key(Key::Escape);
    assert_eq!(app.mail().previewed(), None);
    assert!(
        app.mail().thread().is_some(),
        "Escape closed the viewer, not the conversation"
    );
}

/// A file dropped on the window.
#[derive(Debug)]
struct Dropped(std::path::PathBuf);

impl egui::DroppedFile for Dropped {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|error| error.to_string())
    }
}

#[test]
fn a_dropped_file_is_on_its_way_until_gmail_has_it() {
    let mut app = start("attachments-upload");
    let notes = app.folder("drops").join("notes.txt");
    std::fs::create_dir_all(notes.parent().expect("a folder")).expect("a folder");
    std::fs::write(&notes, "agenda").expect("written");
    app.key(Key::Enter);
    app.key(Key::R);
    app.harness
        .input_mut()
        .dropped_files
        .push(std::sync::Arc::new(Dropped(notes)));
    app.harness.run_steps(2);
    assert_eq!(
        app.mail().compose_uploading(),
        ["notes.txt"],
        "on its way before the draft is even saved"
    );
    wait_for(&mut app, |app| app.mail().compose_uploading().is_empty());
    assert!(
        app.mail().compose_uploading().is_empty(),
        "Gmail's draft has it"
    );
    assert_eq!(app.mail().compose_attachments(), ["notes.txt"]);
}

/// Priya forwarding Daniel's kickoff notes as a message, with a link in its body.
fn kickoff() -> Email {
    let eml = b"From: Daniel Ortiz <daniel@northwind.example.com>\r\nTo: Maya Okafor <maya@brightmoor.co>\r\nSubject: Northwind kickoff\r\nDate: Tue, 17 Nov 2026 07:40:00 -0500\r\nContent-Type: text/html\r\n\r\n<p>Thanks all for making the time on Monday.</p><p><a href=\"https://example.com/planning-pack\">Planning pack</a></p>\r\n";
    Email {
        from: Some(Mailbox::new("Priya Raman", "priya@brightmoor.co")),
        to: vec![Mailbox::address(ME)],
        subject: "Fwd: Northwind kickoff".into(),
        html: Some("<div>Daniel's notes.</div>".into()),
        attachments: vec![Attachment {
            filename: "Northwind kickoff.eml".into(),
            content_type: "message/rfc822".into(),
            data: eml.to_vec(),
            content_id: None,
        }],
        message_id: Some("kickoff-forward@brightmoor.co".into()),
        ..Email::default()
    }
}

/// Runs frames until one puts `want` on egui's output, at most two seconds' worth, the first
/// with `events` in.
fn output(
    app: &mut Fixture,
    events: Vec<Event>,
    want: impl Fn(&egui::OutputCommand) -> Option<String>,
) -> Option<String> {
    let mut events = events.into_iter().peekable();
    let mut found = None;
    for _ in 0..120 {
        app.harness.input_mut().events.extend(events.next());
        app.harness.run_steps(1);
        found = found.or_else(|| {
            app.harness
                .output()
                .platform_output
                .commands
                .iter()
                .find_map(&want)
        });
        if found.is_some() && events.peek().is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    found
}

/// ⌘C, and what it put on the clipboard.
fn copy(app: &mut Fixture) -> Option<String> {
    let key = |pressed| Event::Key {
        key: Key::C,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: CMD,
    };
    output(app, vec![key(true), key(false)], |command| match command {
        egui::OutputCommand::CopyText(text) => Some(text.clone()),
        _ => None,
    })
}

#[test]
fn a_message_in_the_viewer_selects_copies_and_opens_its_links() {
    let mut app = start("attachments-viewer-text");
    deliver(&mut app, &kickoff());
    while app.cursor() != "Fwd: Northwind kickoff" {
        app.key(Key::K);
    }
    app.key(Key::Enter);
    let card = (0..60)
        .map(|step| pos2(280.0, 250.0 + 4.0 * step as f32))
        .find(|at| app.hover(*at) == egui::CursorIcon::PointingHand)
        .expect("a card");
    app.click(card + egui::vec2(0.0, 20.0));
    assert_eq!(app.mail().previewed(), Some("Northwind kickoff.eml"));
    app.wait(0.5);
    // At 1280×800 the card's text starts 312px in; its subject's line is at 141, the body at 301.
    app.drag(pos2(312.0, 157.0), pos2(700.0, 157.0));
    assert_eq!(
        copy(&mut app).as_deref(),
        Some("Northwind kickoff"),
        "the subject selects"
    );
    app.drag(pos2(313.0, 312.0), pos2(1000.0, 318.0));
    let copied = copy(&mut app);
    assert!(
        copied
            .as_deref()
            .is_some_and(|text| text.starts_with("Thanks all for making the time")),
        "the body selects, and the subject's selection went: {copied:?}"
    );
    let link = (0..40)
        .map(|step| pos2(330.0, 301.0 + 4.0 * step as f32))
        .find(|at| app.hover(*at) == egui::CursorIcon::PointingHand)
        .expect("the link");
    let button = |pressed| Event::PointerButton {
        pos: link,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let opened = output(
        &mut app,
        vec![button(true), button(false)],
        |command| match command {
            egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
            _ => None,
        },
    );
    assert_eq!(opened.as_deref(), Some("https://example.com/planning-pack"));
    assert_eq!(
        app.mail().previewed(),
        Some("Northwind kickoff.eml"),
        "a web link leaves the viewer open"
    );
}
