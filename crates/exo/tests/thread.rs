//! The thread view driven by keys, end to end: opening and leaving threads, moving between
//! conversations and messages, and triage from inside a thread.

mod support;

use egui::{Key, Modifiers};
use support::{Fixture, start};

fn open_subject(app: &Fixture) -> Option<String> {
    app.mail()
        .thread()
        .and_then(|thread| thread.detail())
        .map(|detail| detail.subject.clone())
}

#[test]
fn enter_opens_the_thread_read_and_escape_goes_back() {
    let mut app = start("thread-open");
    app.key(Key::J);
    assert!(
        app.mail()
            .inbox()
            .cursor()
            .is_some_and(|thread| thread.unread),
        "the invite starts unread"
    );
    app.key(Key::Enter);
    let subject = open_subject(&app).expect("a thread is open");
    assert!(
        subject.starts_with("Invitation: Coffee with Grace"),
        "{subject}"
    );
    // Last message expanded and focused.
    let thread = app.mail().thread().expect("open");
    assert_eq!(thread.expanded(), [true]);
    assert!(thread.focused().is_some());
    app.key(Key::Escape);
    assert!(app.mail().thread().is_none(), "back to the list");
    assert!(
        app.cursor().starts_with("Invitation: Coffee with Grace"),
        "the cursor stays on the row"
    );
    assert!(
        !app.mail()
            .inbox()
            .cursor()
            .is_some_and(|thread| thread.unread),
        "opening marked it read"
    );
}

#[test]
fn j_and_k_move_between_conversations_without_leaving() {
    let mut app = start("thread-jk");
    app.key(Key::Enter);
    assert_eq!(
        open_subject(&app).as_deref(),
        Some("Design crit moved to 3pm")
    );
    app.key(Key::J);
    assert!(
        open_subject(&app)
            .is_some_and(|subject| subject.starts_with("Invitation: Coffee with Grace"))
    );
    app.key(Key::K);
    app.key(Key::K);
    assert!(
        app.mail().thread().is_none(),
        "past the first conversation, back to the list"
    );
}

#[test]
fn done_moves_on_to_the_next_conversation_and_undo_comes_back() {
    let mut app = start("thread-done");
    app.key(Key::Enter);
    app.key(Key::E);
    assert!(
        !app.subjects()
            .contains(&"Design crit moved to 3pm".to_owned()),
        "out of the inbox"
    );
    assert!(
        open_subject(&app)
            .is_some_and(|subject| subject.starts_with("Invitation: Coffee with Grace")),
        "auto-advance"
    );
    app.key(Key::Z);
    assert_eq!(
        open_subject(&app).as_deref(),
        Some("Design crit moved to 3pm"),
        "undo reopens it"
    );
    assert_eq!(app.subjects().len(), 24);
}

#[test]
fn mark_unread_goes_back_to_the_list() {
    let mut app = start("thread-unread");
    app.key(Key::Enter);
    app.key(Key::U);
    assert!(app.mail().thread().is_none());
    assert!(
        app.mail()
            .inbox()
            .cursor()
            .is_some_and(|thread| thread.unread)
    );
}

#[test]
fn messages_expand_collapse_and_show_their_headers() {
    let mut app = start("thread-messages");
    // Debrief: Senior Product Designer (Aiko), three messages, only the last one expanded.
    for _ in 0..8 {
        app.key(Key::J);
    }
    app.key(Key::Enter);
    let expanded = |app: &Fixture| app.mail().thread().expect("open").expanded();
    assert_eq!(expanded(&app), [false, false, true]);
    app.key(Key::P);
    app.key(Key::Enter);
    assert_eq!(
        expanded(&app),
        [false, true, true],
        "Enter expands the focused message"
    );
    app.key(Key::Escape);
    assert_eq!(
        expanded(&app),
        [false, false, true],
        "Escape collapses it first"
    );
    app.press(Key::O, Modifiers::SHIFT);
    assert_eq!(expanded(&app), [true, true, true], "⇧O expands everything");
    app.press(Key::O, Modifiers::SHIFT);
    assert_eq!(
        expanded(&app),
        [false, false, true],
        "and collapses all it can"
    );
    app.press(Key::H, Modifiers::SHIFT);
    assert!(app.mail().thread().expect("open").header_expanded(), "⇧H");
    app.key(Key::O);
    assert_eq!(
        expanded(&app),
        [false, true, true],
        "O expands the focused message"
    );
}

#[test]
fn clicking_the_participants_expands_the_header_and_its_chevron_folds_it() {
    let mut app = start("thread-header-click");
    app.key(Key::Enter);
    assert_eq!(
        open_subject(&app).as_deref(),
        Some("Design crit moved to 3pm")
    );
    // "Tom to Me & Priya", at 1280×800.
    assert_eq!(
        app.hover(egui::pos2(260.0, 125.0)),
        egui::CursorIcon::PointingHand
    );
    app.click(egui::pos2(260.0, 125.0));
    let thread = app.mail().thread().expect("open");
    assert!(thread.header_expanded(), "the header lists every recipient");
    assert_eq!(thread.expanded(), [true], "and the message stays open");
    // The full header is text, and its chevron (after the date) folds it.
    assert_eq!(app.hover(egui::pos2(300.0, 150.0)), egui::CursorIcon::Text);
    app.click(egui::pos2(540.0, 180.0));
    assert!(!app.mail().thread().expect("open").header_expanded());
}

#[test]
fn clicking_an_attachment_previews_it_from_the_cache() {
    let mut app = start("thread-attachment");
    for _ in 0..5 {
        app.key(Key::J);
    }
    app.key(Key::Enter);
    assert_eq!(open_subject(&app).as_deref(), Some("Board deck — draft 2"));
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
    // The deck's card, under the body.
    let card = egui::pos2(320.0, 320.0);
    assert_eq!(app.hover(card), egui::CursorIcon::PointingHand);
    app.click(card);
    assert!(
        cached.is_file(),
        "a click downloads the file into the preview cache"
    );
}

#[test]
fn triage_in_a_thread_acts_on_that_thread_not_the_list_selection() {
    let mut app = start("thread-selection");
    // Select the first conversation, then open the second.
    app.key(Key::X);
    app.key(Key::J);
    app.key(Key::Enter);
    assert!(
        open_subject(&app)
            .is_some_and(|subject| subject.starts_with("Invitation: Coffee with Grace"))
    );
    app.key(Key::E);
    let subjects = app.subjects();
    assert!(
        subjects.contains(&"Design crit moved to 3pm".to_owned()),
        "the selected row stays: {subjects:?}"
    );
    assert!(
        !subjects
            .iter()
            .any(|subject| subject.starts_with("Invitation: Coffee with Grace")),
        "the open thread is done"
    );
}

#[test]
fn triage_in_a_thread_another_device_archived_leaves_the_list_alone() {
    let mut app = start("thread-vanished");
    app.key(Key::Enter);
    let id = app.mail().thread().expect("open").id().to_owned();
    let gmail = app.gmail();
    let archive = gmail::ModifyThreadRequest {
        add_label_ids: Vec::new(),
        remove_label_ids: vec!["INBOX".into()],
    };
    app.block_on(async { gmail.gmail().modify_thread(&id, &archive).await })
        .expect("archived elsewhere");
    app.sync();
    assert!(
        !app.subjects()
            .contains(&"Design crit moved to 3pm".to_owned()),
        "the sync took it out of the list"
    );
    assert_eq!(
        open_subject(&app).as_deref(),
        Some("Design crit moved to 3pm"),
        "still open"
    );
    app.key(Key::E);
    let subjects = app.subjects();
    assert!(
        subjects
            .iter()
            .any(|subject| subject.starts_with("Invitation: Coffee with Grace")),
        "the row the cursor fell back to stays: {subjects:?}"
    );
    assert_eq!(subjects.len(), 23);
}

#[test]
fn the_cursor_follows_a_body_that_moves_under_a_still_pointer() {
    let mut app = start("thread-still-pointer");
    for _ in 0..5 {
        app.key(Key::J);
    }
    app.key(Key::Enter);
    app.press(Key::H, Modifiers::SHIFT);
    // On the second line of "Board deck — draft 2", under the full header.
    let at = egui::pos2(560.0, 247.0);
    assert_eq!(app.hover(at), egui::CursorIcon::Text);
    // The header folds and the body moves up: the pointer is now past the end of a shorter line.
    app.press(Key::H, Modifiers::SHIFT);
    assert_eq!(
        app.harness.output().platform_output.cursor_icon,
        egui::CursorIcon::Default
    );
}

#[test]
fn a_reply_arriving_keeps_the_bodies_laid_out_and_the_scroll_where_it_was() {
    let mut app = start("thread-reply-arrives");
    for _ in 0..6 {
        app.key(Key::J);
    }
    app.key(Key::Enter);
    let thread = app.mail().thread().expect("open").id().to_owned();
    app.key(Key::Space);
    let scroll = app.mail().thread().expect("open").scroll();
    assert!(scroll > 50.0, "down the newsletter: {scroll}");
    let deliver = app.mock_url().join("__mock/deliver").expect("a URL");
    let reply = format!(
        r#"{{"threadId":"{thread}","from":"Ann Lee <ann@example.com>","text":"Thanks for this."}}"#
    );
    app.block_on(async {
        reqwest::Client::new()
            .post(deliver)
            .header("content-type", "application/json")
            .body(reply)
            .send()
            .await?
            .error_for_status()
    })
    .expect("delivered");
    app.sync();
    let open = app.mail().thread().expect("still open");
    assert_eq!(open.expanded(), [true], "the reply waits behind the bar");
    assert!(open.has_new_messages());
    assert_eq!(
        open.scroll(),
        scroll,
        "the newsletter didn't blank and drop the scroll"
    );
    app.press(Key::N, Modifiers::SHIFT);
    let open = app.mail().thread().expect("still open");
    assert_eq!(
        open.expanded(),
        [true, true],
        "⇧N shows it, expanded as the last message"
    );
    assert!(!open.has_new_messages());
    assert_eq!(
        open.focused(),
        open.messages().last().map(|message| message.id.as_str()),
        "and focuses it"
    );
}

/// Delivers a reply from Ann into thread `thread` with `labels`, and syncs.
fn deliver_reply(app: &mut Fixture, thread: &str, labels: &[&str]) {
    let deliver = app.mock_url().join("__mock/deliver").expect("a URL");
    let reply = serde_json::json!({ "threadId": thread, "from": "Ann Lee <ann@example.com>", "text": "Thanks for this.", "labelIds": labels });
    app.block_on(async {
        reqwest::Client::new()
            .post(deliver)
            .json(&reply)
            .send()
            .await?
            .error_for_status()
    })
    .expect("delivered");
    app.sync();
}

#[test]
fn opening_a_conversation_expands_its_unread_messages_then_marks_it_read() {
    let mut app = start("thread-unread-expanded");
    let deliver = app.mock_url().join("__mock/deliver").expect("a URL");
    let first = serde_json::json!({ "from": "Ann Lee <ann@example.com>", "subject": "Two unread", "text": "The first.", "labelIds": ["INBOX", "UNREAD"] });
    let delivered: serde_json::Value = app
        .block_on(async {
            reqwest::Client::new()
                .post(deliver)
                .json(&first)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await
        })
        .expect("delivered");
    let thread = delivered["threadId"].as_str().expect("a thread").to_owned();
    deliver_reply(&mut app, &thread, &["INBOX", "UNREAD"]);
    while app.cursor() != "Two unread" {
        app.key(Key::K);
    }
    app.key(Key::Enter);
    app.settle();
    assert_eq!(
        app.mail().thread().expect("open").expanded(),
        [true, true],
        "both were unread"
    );
    assert!(
        !app.mail()
            .inbox()
            .cursor()
            .is_some_and(|thread| thread.unread),
        "and opening marked it read"
    );
}

#[test]
fn down_past_the_last_message_reaches_the_new_messages_bar_and_enter_shows_them() {
    use exo_commands::Region;
    let mut app = start("thread-new-messages-bar");
    app.key(Key::Enter);
    let thread = app.mail().thread().expect("open").id().to_owned();
    deliver_reply(&mut app, &thread, &["INBOX", "UNREAD"]);
    let region = |app: &Fixture| app.mail().thread().expect("open").region();
    app.key(Key::ArrowDown);
    assert_eq!(
        region(&app),
        Region::ThreadMessagesNewMessages,
        "↓ past the only message"
    );
    app.key(Key::ArrowUp);
    assert_eq!(
        region(&app),
        Region::ThreadMessagesNewMessages,
        "↑ stays: the message it'd go back to is the first"
    );
    app.key(Key::Enter);
    let open = app.mail().thread().expect("open");
    assert_eq!(
        (open.expanded(), open.region()),
        (vec![true, true], Region::ThreadMessages)
    );
    assert!(
        !app.mail()
            .inbox()
            .cursor()
            .is_some_and(|thread| thread.unread),
        "showing it marked the conversation read"
    );
}

#[test]
fn a_message_in_trash_folds_into_its_marker_which_the_cursor_skips() {
    let mut app = start("thread-trashed");
    let thread = app.mail().inbox().cursor().expect("a row").id.clone();
    deliver_reply(&mut app, &thread, &["TRASH"]);
    app.key(Key::Enter);
    let open = app.mail().thread().expect("open");
    assert_eq!(
        open.expanded(),
        [true, false],
        "the last message out of Trash is the expanded one"
    );
    let first = open.focused().map(str::to_owned);
    app.key(Key::N);
    assert_eq!(
        app.mail()
            .thread()
            .expect("open")
            .focused()
            .map(str::to_owned),
        first,
        "N doesn't land on the marker"
    );
    app.press(Key::O, Modifiers::SHIFT);
    assert_eq!(
        app.mail().thread().expect("open").expanded(),
        [true, true],
        "⇧O shows it"
    );
    app.key(Key::N);
    assert_ne!(
        app.mail()
            .thread()
            .expect("open")
            .focused()
            .map(str::to_owned),
        first,
        "and then N does"
    );
}

#[test]
fn the_keyboard_reaches_the_header_and_the_back_gutter() {
    use exo_commands::Region;
    let mut app = start("thread-header-keys");
    app.key(Key::Enter);
    let region = |app: &Fixture| app.mail().thread().expect("open").region();
    app.key(Key::ArrowUp);
    assert_eq!(
        region(&app),
        Region::ThreadMessagesHeader,
        "↑ from the first message"
    );
    app.key(Key::ArrowDown);
    assert_eq!(region(&app), Region::ThreadMessages, "↓ back");
    app.key(Key::ArrowUp);
    // Without a status line nothing in the header has the keyboard yet: ← goes to the gutter.
    app.key(Key::ArrowLeft);
    assert_eq!(region(&app), Region::ActionPane);
    app.key(Key::ArrowRight);
    assert_eq!(
        region(&app),
        Region::ThreadMessagesHeader,
        "→ goes back where it came from"
    );
    app.key(Key::Escape);
    assert_eq!(region(&app), Region::ThreadMessages);
    app.key(Key::ArrowLeft);
    assert_eq!(region(&app), Region::ActionPane, "← from the messages");
    app.key(Key::ArrowRight);
    assert_eq!(region(&app), Region::ThreadMessages);
    app.key(Key::ArrowLeft);
    app.key(Key::Enter);
    assert!(
        app.mail().thread().is_none(),
        "Enter in the gutter goes back"
    );
}

#[test]
fn enter_on_a_header_control_clicks_it() {
    let mut app = start("thread-header-enter");
    app.key(Key::Enter);
    let subject = open_subject(&app);
    app.key(Key::ArrowUp);
    // → focuses the first control, Done.
    app.key(Key::ArrowRight);
    app.key(Key::Enter);
    assert_ne!(
        open_subject(&app),
        subject,
        "marked done, on to the next conversation"
    );
    assert!(
        !app.subjects()
            .contains(&"Design crit moved to 3pm".to_owned())
    );
}

#[test]
fn the_cursor_is_an_ibeam_over_text_an_arrow_elsewhere_and_a_hand_over_whole_buttons() {
    let mut app = start("thread-cursors");
    for _ in 0..6 {
        app.key(Key::J);
    }
    app.key(Key::Enter);
    assert!(open_subject(&app).is_some_and(|subject| subject.starts_with("Tuesday, Nov 17")));
    // The Morning Ledger: a paragraph, the empty end of the headline's line, and the
    // "Read today's edition" button, in its padding.
    assert_eq!(app.hover(egui::pos2(400.0, 419.0)), egui::CursorIcon::Text);
    assert_eq!(
        app.hover(egui::pos2(832.0, 368.0)),
        egui::CursorIcon::Default
    );
    assert_eq!(
        app.hover(egui::pos2(310.0, 625.0)),
        egui::CursorIcon::PointingHand
    );
}

#[test]
fn a_body_too_wide_for_the_column_widens_it_and_the_column_scrolls_sideways() {
    use exo_commands::Region;
    let mut app = start("thread-wide");
    // A 1200px-wide sale mail whose last column is a link.
    let html = "<table width=\"1200\" cellpadding=\"0\" cellspacing=\"0\"><tr>\
                <td width=\"1000\" style=\"height:80px\">Black Friday</td>\
                <td width=\"200\"><a href=\"https://example.com/deal\" style=\"display:block;height:80px\">Deal</a></td>\
                </tr></table>";
    let delivery = serde_json::json!({ "from": "Design Shop <hello@designshop.example>", "subject": "Black Friday is here", "html": html });
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
    while app.cursor() != "Black Friday is here" {
        app.key(Key::K);
    }
    app.key(Key::Enter);
    assert_eq!(open_subject(&app).as_deref(), Some("Black Friday is here"));
    // The column now runs from the back gutter to 2rem from the pane's right, its first cell's
    // empty end under the pointer, and the link past the right edge.
    let at = egui::pos2(800.0, 200.0);
    assert_eq!(app.hover(at), egui::CursorIcon::Default);
    let region = |app: &Fixture| app.mail().thread().expect("open").region();
    // → scrolls the whole column 175px at a time, as far as it goes: the link comes into view.
    for _ in 0..3 {
        app.key(Key::ArrowRight);
    }
    assert_eq!(
        app.hover(at),
        egui::CursorIcon::PointingHand,
        "over the link"
    );
    // ← scrolls back, and only at the left edge goes to the gutter.
    for _ in 0..3 {
        app.key(Key::ArrowLeft);
        assert_eq!(region(&app), Region::ThreadMessages);
    }
    assert_eq!(app.hover(at), egui::CursorIcon::Default);
    app.key(Key::ArrowLeft);
    assert_eq!(region(&app), Region::ActionPane, "← at the left edge");
    app.key(Key::ArrowRight);
    // A sideways swipe scrolls the column too; the thread itself doesn't move up or down.
    let scroll = app.mail().thread().expect("open").scroll();
    app.harness
        .input_mut()
        .events
        .push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(-600.0, 0.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
    app.harness.run_steps(1);
    assert_eq!(
        app.hover(at),
        egui::CursorIcon::PointingHand,
        "over the link"
    );
    assert_eq!(app.mail().thread().expect("open").scroll(), scroll);
}

#[test]
fn a_new_accounts_focused_message_labels_its_reply_buttons() {
    let mut app = start("thread-reply-labels");
    app.key(Key::Enter);
    // Tom to Me & Priya. Labelled, Reply all's label is where the participants' row ends without
    // labels (reply buttons have labels in the account's first week).
    let label = egui::pos2(725.0, 133.0);
    let replies = |app: &mut Fixture, joined: &str| {
        let joined: Option<jiff::Timestamp> =
            (!joined.is_empty()).then(|| joined.parse().expect("a timestamp"));
        let accounts = app
            .mail()
            .accounts()
            .iter()
            .map(|account| exo_auth::Account {
                joined,
                ..account.clone()
            })
            .collect();
        app.mail_mut().set_accounts(accounts);
        app.settle();
        app.click(label);
        let to = app.mail().compose_focus().map(|(to, _)| to);
        if to.is_some() {
            app.key(Key::Escape);
        }
        to
    };
    // The mock's now is 9:00 New York on 17 November.
    assert_eq!(
        replies(&mut app, ""),
        None,
        "no join date: not a new account"
    );
    assert_eq!(
        replies(&mut app, "2026-11-11T15:00:00Z"),
        Some(vec!["tom@brightmoor.co".to_owned()]),
        "joined six days ago: Reply all"
    );
    assert_eq!(
        replies(&mut app, "2026-11-09T13:00:00Z"),
        None,
        "joined over a week ago"
    );
}

/// Presses the primary button `count` times in a row at `at` (a double or triple click).
fn multi_click(app: &mut Fixture, at: egui::Pos2, count: usize) {
    app.hover(at);
    for _ in 0..count {
        for pressed in [true, false] {
            let button = egui::PointerButton::Primary;
            app.harness
                .input_mut()
                .events
                .push(egui::Event::PointerButton {
                    pos: at,
                    button,
                    pressed,
                    modifiers: Modifiers::NONE,
                });
            app.harness.run_steps(1);
        }
    }
    app.settle();
}

/// ⌘C, and the text it put on the clipboard (in whichever frame the body worker answered).
fn copy(app: &mut Fixture) -> Option<String> {
    let command = Modifiers {
        mac_cmd: true,
        command: true,
        ..Modifiers::NONE
    };
    let mut copied = None;
    for frame in 0..200 {
        if frame < 2 {
            let pressed = frame == 0;
            app.harness.input_mut().events.push(egui::Event::Key {
                key: Key::C,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: command,
            });
        }
        app.harness.run_steps(1);
        copied = copied.or_else(|| {
            app.harness
                .output()
                .platform_output
                .commands
                .iter()
                .find_map(|command| match command {
                    egui::OutputCommand::CopyText(text) => Some(text.clone()),
                    _ => None,
                })
        });
        if copied.is_some() && frame >= 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    app.settle();
    copied
}

#[test]
fn double_and_triple_clicks_select_a_word_and_a_paragraph_to_copy() {
    let mut app = start("thread-multi-click");
    let html =
        "<div>Supercalifragilisticexpialidocious and more</div><div>A second paragraph</div>";
    let delivery = serde_json::json!({ "from": "Ann Lee <ann@example.com>", "subject": "Words", "html": html });
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
    while app.cursor() != "Words" {
        app.key(Key::K);
    }
    app.key(Key::Enter);
    // The body's first line: the first point down the column, under the header's selectable
    // subject, where the cursor is the I-beam.
    let x = 300.0;
    let line = (140..400)
        .step_by(4)
        .map(|y| egui::pos2(x, y as f32))
        .find(|at| app.hover(*at) == egui::CursorIcon::Text)
        .expect("the body's first line");
    let at = line + egui::vec2(0.0, 4.0);
    multi_click(&mut app, at, 2);
    assert_eq!(
        copy(&mut app).as_deref(),
        Some("Supercalifragilisticexpialidocious")
    );
    // A triple-clicked block runs to the next one's start, so it copies with its line break, as
    // in Chromium.
    multi_click(&mut app, at, 3);
    assert_eq!(
        copy(&mut app).as_deref(),
        Some("Supercalifragilisticexpialidocious and more\n")
    );
    // A single click clears it: ⌘C then copies the message, as it does with nothing selected.
    app.click(at);
    assert_ne!(
        copy(&mut app).as_deref(),
        Some("Supercalifragilisticexpialidocious and more\n")
    );
}

/// Presses at `from`, drags to `to` and lets go, as a mouse does.
fn drag(app: &mut Fixture, from: egui::Pos2, to: egui::Pos2) {
    app.hover(from);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    app.harness.input_mut().events.push(button(from, true));
    app.harness.run_steps(1);
    for step in 1..=10 {
        app.harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(
                from + (to - from) * step as f32 / 10.0,
            ));
        app.harness.run_steps(1);
    }
    app.harness.input_mut().events.push(button(to, false));
    app.harness.run_steps(1);
    app.settle();
}

#[test]
fn the_subject_selects_copies_and_suggests_copy_subject() {
    let mut app = start("thread-subject-select");
    app.key(Key::Enter);
    assert_eq!(
        open_subject(&app).as_deref(),
        Some("Design crit moved to 3pm")
    );
    // Across "Design crit" on the subject's line.
    drag(&mut app, egui::pos2(246.0, 84.0), egui::pos2(330.0, 84.0));
    assert_eq!(
        app.mail().hint_text().as_deref(),
        Some("To copy the subject faster, hit ⌘ K → Copy Subject!")
    );
    let copied = copy(&mut app).expect("copied");
    assert!(
        "Design crit moved to 3pm".starts_with(&copied) && copied.len() > 4,
        "{copied}"
    );
}

#[test]
fn a_full_headers_addresses_select_and_suggest_copy_recipients() {
    let mut app = start("thread-recipients-select");
    app.key(Key::Enter);
    app.press(Key::H, Modifiers::SHIFT);
    // Along the From row's name and address.
    drag(&mut app, egui::pos2(310.0, 130.0), egui::pos2(480.0, 130.0));
    assert_eq!(
        app.mail().hint_text().as_deref(),
        Some("To copy recipients faster, hit ⌘ K → Copy Recipients!")
    );
    let copied = copy(&mut app).expect("copied");
    assert!(copied.contains("Tom"), "{copied}");
}
