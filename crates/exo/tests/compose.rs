//! Compose end to end: drafts saved to Gmail, sending with undo, discarding, replies in their
//! thread, and pasting a copied message, on gmail-mock.

mod support;

use egui::{Key, Modifiers};
use gmail::{DraftQuery, Format, ListQuery};
use support::{Fixture, start, start_with_silent_people};

const COMMAND: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);

/// Gmail's drafts as (subject, thread id).
fn gmail_drafts(app: &Fixture) -> Vec<(String, String)> {
    let gmail = app.gmail();
    app.block_on(async {
        let drafts = gmail
            .gmail()
            .drafts(&DraftQuery::default())
            .await
            .expect("drafts.list")
            .drafts;
        let mut out = Vec::new();
        for draft in drafts {
            let id = draft.id.expect("an id");
            let full = gmail
                .gmail()
                .draft(&id, Format::Full)
                .await
                .expect("drafts.get");
            let message = full.message.expect("a message");
            out.push((
                message.header("Subject").unwrap_or_default().to_owned(),
                message.thread_id.unwrap_or_default(),
            ));
        }
        out
    })
}

/// Subjects of Gmail's sent messages.
fn sent_subjects(app: &Fixture) -> Vec<String> {
    let gmail = app.gmail();
    app.block_on(async {
        let query = ListQuery {
            label_ids: vec!["SENT".into()],
            max_results: Some(500),
            ..ListQuery::default()
        };
        let list = gmail
            .gmail()
            .messages(&query)
            .await
            .expect("messages.list")
            .messages;
        let mut out = Vec::new();
        for message in list {
            let full = gmail
                .gmail()
                .message(&message.id.expect("id"), Format::Metadata, &["Subject"])
                .await
                .expect("messages.get");
            out.push(full.header("Subject").unwrap_or_default().to_owned());
        }
        out
    })
}

/// Writes a new message to Priya: `c`, the address, the subject, the body.
fn write_new_message(app: &mut Fixture, subject: &str) {
    app.key(Key::C);
    assert!(app.mail().compose_pane_open(), "C opens a new message");
    app.type_text("priya@brightmoor.co");
    app.key(Key::Enter);
    app.key(Key::Tab);
    app.type_text(subject);
    app.key(Key::Tab);
    app.type_text("Hi there");
}

#[test]
fn a_new_message_is_kept_as_a_gmail_draft_and_sends_with_undo_send() {
    let mut app = start("compose-send");
    write_new_message(&mut app, "Lunch plans");
    assert_eq!(
        app.mail().compose_focus(),
        Some((
            vec!["priya@brightmoor.co".to_owned()],
            "Lunch plans".to_owned()
        ))
    );
    app.wait(1.5);
    assert!(
        in_gmail(|| gmail_drafts(&app)
            .iter()
            .any(|(subject, _)| subject == "Lunch plans")),
        "saved to Gmail"
    );

    app.press(Key::Enter, COMMAND);
    assert!(
        !app.mail().compose_pane_open(),
        "sending goes back to the list"
    );
    assert!(
        !sent_subjects(&app).contains(&"Lunch plans".to_owned()),
        "held for undo"
    );
    // Instant Send (⌘⇧Z on the toast) sends it now.
    app.press(Key::Z, COMMAND.plus(Modifiers::SHIFT));
    app.wait(0.5);
    assert!(
        sent_subjects(&app).contains(&"Lunch plans".to_owned()),
        "sent"
    );
    assert!(
        !gmail_drafts(&app)
            .iter()
            .any(|(subject, _)| subject == "Lunch plans"),
        "no longer a draft"
    );
}

#[test]
fn undo_send_takes_the_message_back_into_compose() {
    let mut app = start("compose-undo");
    write_new_message(&mut app, "Undo me");
    app.press(Key::Enter, COMMAND);
    app.key(Key::Z);
    assert!(app.mail().compose_pane_open(), "the draft reopens");
    assert_eq!(
        app.mail()
            .compose_focus()
            .map(|(_, subject)| subject)
            .as_deref(),
        Some("Undo me")
    );
    assert!(!sent_subjects(&app).contains(&"Undo me".to_owned()));
    assert!(
        gmail_drafts(&app)
            .iter()
            .any(|(subject, _)| subject == "Undo me"),
        "still a draft"
    );
}

#[test]
fn sending_without_a_subject_asks_first() {
    let mut app = start("compose-subject");
    app.key(Key::C);
    app.type_text("priya@brightmoor.co");
    app.key(Key::Enter);
    app.key(Key::Tab);
    app.key(Key::Tab);
    app.type_text("Hi");
    app.press(Key::Enter, COMMAND);
    assert!(app.mail().compose_pane_open(), "the alert holds the send");
    // Enter picks "Send anyway".
    app.key(Key::Enter);
    assert!(!app.mail().compose_pane_open(), "sent anyway");
}

#[test]
fn discarding_removes_the_gmail_draft_and_undo_brings_it_back() {
    let mut app = start("compose-discard");
    write_new_message(&mut app, "Discard me");
    app.wait(1.5);
    assert!(in_gmail(|| gmail_drafts(&app)
        .iter()
        .any(|(subject, _)| subject == "Discard me")));
    app.press(Key::Comma, COMMAND.plus(Modifiers::SHIFT));
    assert!(!app.mail().compose_pane_open());
    assert!(
        !gmail_drafts(&app)
            .iter()
            .any(|(subject, _)| subject == "Discard me"),
        "gone from Gmail"
    );
    app.key(Key::Z);
    assert!(app.mail().compose_pane_open(), "undo reopens it");
    assert!(
        gmail_drafts(&app)
            .iter()
            .any(|(subject, _)| subject == "Discard me"),
        "and saves it again"
    );
}

#[test]
fn a_reply_is_a_draft_in_its_thread_and_an_untouched_one_leaves_nothing() {
    let mut app = start("compose-reply");
    let before = gmail_drafts(&app).len();
    app.key(Key::Enter);
    app.key(Key::R);
    assert_eq!(
        app.mail().compose_focus().map(|(to, _)| to),
        Some(vec!["tom@brightmoor.co".to_owned()]),
        "a reply to the sender"
    );
    app.key(Key::Escape);
    assert!(app.mail().compose_focus().is_none());
    app.wait(1.5);
    assert_eq!(
        gmail_drafts(&app).len(),
        before,
        "an untouched reply is dropped, never saved"
    );

    app.key(Key::R);
    app.type_text("Sounds good");
    app.wait(1.5);
    let thread = app
        .mail()
        .thread()
        .map(|thread| thread.id().to_owned())
        .expect("the thread is open");
    let drafts = gmail_drafts(&app);
    assert!(
        drafts
            .iter()
            .any(|(subject, id)| subject == "Re: Design crit moved to 3pm" && *id == thread),
        "the reply draft is in its thread: {drafts:?}"
    );
    // Leaving keeps it; the row reads as a draft.
    app.key(Key::Escape);
    app.key(Key::Escape);
    let row = app
        .mail()
        .inbox()
        .threads()
        .iter()
        .find(|row| row.id == thread)
        .cloned()
        .expect("still listed");
    assert_eq!(row.drafts, 1, "Draft to … in the list");
}

#[test]
fn pasting_a_copied_message_starts_a_draft_of_it() {
    let mut app = start("compose-paste");
    exo::clipboard::capture(&app.harness.ctx);
    app.key(Key::Enter);
    app.wait(0.5);
    app.press(Key::C, COMMAND);
    app.key(Key::Escape);
    let html = exo::clipboard::captured(&app.harness.ctx)
        .map(|(html, _)| html)
        .expect("copied");
    assert!(!html.is_empty());
    app.harness
        .input_mut()
        .events
        .push(egui::Event::Paste("Heads up".into()));
    app.harness.run_steps(1);
    app.settle();
    assert!(
        app.mail().compose_pane_open(),
        "⌘V outside a field makes a draft"
    );
    assert_eq!(
        app.mail()
            .compose_focus()
            .map(|(_, subject)| subject)
            .as_deref(),
        Some("Design crit moved to 3pm"),
        "of the copied message"
    );
}

/// The files on Gmail's draft with `subject`, as (name, bytes).
fn draft_files(app: &Fixture, subject: &str) -> Vec<(String, u64)> {
    fn walk(part: &gmail::MessagePart, out: &mut Vec<(String, u64)>) {
        if let Some(name) = part.filename.as_deref().filter(|name| !name.is_empty()) {
            out.push((
                name.to_owned(),
                part.body
                    .as_ref()
                    .and_then(|body| body.size)
                    .unwrap_or(0)
                    .into(),
            ));
        }
        for child in &part.parts {
            walk(child, out);
        }
    }
    let gmail = app.gmail();
    app.block_on(async {
        for draft in gmail
            .gmail()
            .drafts(&DraftQuery::default())
            .await
            .expect("drafts.list")
            .drafts
        {
            let full = gmail
                .gmail()
                .draft(&draft.id.expect("an id"), Format::Full)
                .await
                .expect("drafts.get");
            let message = full.message.expect("a message");
            if message.header("Subject") == Some(subject) {
                let mut out = Vec::new();
                walk(message.payload.as_ref().expect("a payload"), &mut out);
                return out;
            }
        }
        Vec::new()
    })
}

#[test]
fn a_forward_carries_the_original_files_into_its_gmail_draft() {
    let mut app = start("compose-forward-files");
    for _ in 0..5 {
        app.key(Key::J);
    }
    app.key(Key::Enter);
    app.key(Key::F);
    app.type_text("priya@brightmoor.co");
    app.key(Key::Enter);
    app.wait(1.5);
    let files = draft_files(&app, "Fwd: Board deck — draft 2");
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(files[0].0, "Brightmoor_Board_Deck_Nov2026_v2.pdf");
    assert!(files[0].1 > 0, "with its bytes");
}

#[test]
fn a_draft_left_without_its_files_at_a_quit_gets_them_after_the_relaunch() {
    let mut app = start("compose-files-relaunch");
    for _ in 0..5 {
        app.key(Key::J);
    }
    let thread = app.mail().inbox().threads()[5].id.clone();
    let gmail = app.gmail();
    let messages: Vec<String> = app
        .block_on(async {
            gmail
                .gmail()
                .thread(&thread, Format::Minimal, &[])
                .await
                .expect("threads.get")
                .messages
        })
        .into_iter()
        .filter_map(|message| message.id)
        .collect();
    // Gmail won't hand the deck over.
    for message in &messages {
        app.fault(Some(serde_json::json!({"method": "GET", "path": format!("/gmail/v1/users/me/messages/{message}"), "status": 503, "times": 100_000})));
    }
    app.key(Key::Enter);
    app.key(Key::F);
    app.type_text("priya@brightmoor.co");
    app.key(Key::Enter);
    app.wait(1.5);
    let subject = "Fwd: Board deck — draft 2";
    assert!(
        draft_files(&app, subject).is_empty(),
        "not written without its file"
    );
    // Quit and opened again, the file still out of reach; then Gmail hands it over.
    app.relaunch();
    app.fault(None);
    app.sync();
    assert!(
        in_gmail(|| draft_files(&app, subject)
            .iter()
            .any(|(name, _)| name == "Brightmoor_Board_Deck_Nov2026_v2.pdf")),
        "written with its file, without another edit"
    );
}

#[test]
fn typing_part_of_a_name_suggests_people_and_enter_takes_the_first() {
    let mut app = start("compose-suggestions");
    app.key(Key::C);
    app.wait(0.5);
    app.type_text("raman");
    app.wait(0.5);
    app.key(Key::Enter);
    assert_eq!(
        app.mail().compose_focus().map(|(to, _)| to),
        Some(vec!["priya@brightmoor.co".to_owned()]),
        "Priya, found by her last name"
    );
}

#[test]
fn suggestions_come_from_mail_without_waiting_for_google() {
    let mut app = start_with_silent_people("compose-silent-people");
    app.key(Key::C);
    app.wait(0.5);
    app.type_text("raman");
    app.wait(0.5);
    app.key(Key::Enter);
    assert_eq!(
        app.mail().compose_focus().map(|(to, _)| to),
        Some(vec!["priya@brightmoor.co".to_owned()]),
        "Priya, from stored mail"
    );
}

#[test]
fn a_reply_pops_out_and_closing_it_keeps_the_draft_in_its_thread() {
    let mut app = start("compose-popout-reply");
    app.key(Key::Enter);
    app.press(Key::R, Modifiers::SHIFT);
    assert!(app.mail().compose_popped_out(), "⇧R pops the reply out");
    assert_eq!(
        app.mail().compose_focus().map(|(to, _)| to),
        Some(vec!["tom@brightmoor.co".to_owned()]),
        "with the keyboard"
    );
    app.type_text("Sounds good");
    app.key(Key::Escape);
    assert!(
        !app.mail().compose_popped_out(),
        "Escape closes the pop-out"
    );
    assert!(app.mail().compose_focus().is_none());
    app.wait(1.5);
    assert!(
        gmail_drafts(&app)
            .iter()
            .any(|(subject, _)| subject == "Re: Design crit moved to 3pm"),
        "the draft is kept"
    );
}

#[test]
fn a_new_message_pops_out_and_back_in() {
    let mut app = start("compose-popout-new");
    write_new_message(&mut app, "Pop me");
    app.press(Key::P, COMMAND.plus(Modifiers::SHIFT));
    assert!(
        app.mail().compose_popped_out() && !app.mail().compose_pane_open(),
        "⌘⇧P pops it out of its pane"
    );
    assert_eq!(
        app.mail()
            .compose_focus()
            .map(|(_, subject)| subject)
            .as_deref(),
        Some("Pop me")
    );
    app.press(Key::D, COMMAND);
    assert!(
        app.mail().compose_focus().is_none(),
        "⌘D gives the app the keyboard"
    );
    app.press(Key::D, COMMAND);
    assert!(app.mail().compose_focus().is_some(), "and back");
    app.press(Key::P, COMMAND.plus(Modifiers::SHIFT));
    assert!(
        !app.mail().compose_popped_out() && app.mail().compose_pane_open(),
        "⌘⇧P pops it back into its pane"
    );

    app.key(Key::Escape);
    app.key(Key::Escape);
    app.press(Key::C, Modifiers::SHIFT);
    assert!(
        app.mail().compose_popped_out() && !app.mail().compose_pane_open(),
        "⇧C writes a new message popped out"
    );
}

/// Gives the account a join date.
fn joined(app: &mut Fixture, at: &str) {
    let joined = Some(at.parse().expect("a timestamp"));
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
}

#[test]
fn share_availabilitys_tip_shows_a_week_after_joining_until_its_closed() {
    let mut app = start("compose-availability-tip");
    joined(&mut app, "2026-11-12T12:00:00Z");
    app.key(Key::Enter);
    app.key(Key::R);
    assert!(
        !app.mail().compose_availability_tip(),
        "joined five days ago"
    );
    joined(&mut app, "2026-11-01T12:00:00Z");
    assert!(
        app.mail().compose_availability_tip(),
        "joined over a week ago"
    );
    // Its close icon, at the right of its title, under the reply's calendar icon at 1280×800.
    app.click(egui::pos2(859.0, 382.0));
    assert!(
        app.mail()
            .account()
            .settings()
            .has_shown_share_availability_tooltip,
        "closed for good"
    );
    assert!(!app.mail().compose_availability_tip());
}

#[test]
fn what_is_typed_after_command_d_out_of_a_pop_out_is_the_apps() {
    let mut app = start("compose-popout-cycle");
    app.press(Key::C, Modifiers::SHIFT);
    app.press(Key::D, COMMAND);
    app.type_text("g");
    app.press(Key::D, COMMAND);
    app.key(Key::Enter);
    assert_eq!(
        app.mail().compose_focus().map(|(to, _)| to),
        Some(Vec::new()),
        "the `g` went to the app, not the pop-out's To"
    );
}

#[test]
fn a_pop_out_taller_than_the_window_scrolls_its_body_to_the_caret() {
    let mut app = start("compose-popout-tall");
    app.press(Key::C, Modifiers::SHIFT);
    app.key(Key::Tab);
    app.key(Key::Tab);
    for line in 1..=30 {
        app.type_text(&format!("line {line}"));
        app.key(Key::Enter);
    }
    // At 800px the form stops at `100vh - 120px` under the 41px strip, its footer (68px) on the
    // window's bottom: the body scrolls between the subject and the footer, the caret in view.
    let caret = app
        .mail()
        .compose_caret()
        .expect("the body has the keyboard");
    assert!(
        caret.top() > 800.0 - 639.0 && caret.bottom() <= 800.0 - 4.0 - 68.0,
        "the caret ({caret:?}) shows above the footer"
    );
}

#[test]
fn a_reply_taller_than_the_window_scrolls_its_body_and_keeps_its_top_in_view() {
    let mut app = start("compose-reply-tall");
    app.key(Key::Enter);
    app.wait(0.5);
    app.key(Key::R);
    for line in 1..=40 {
        app.type_text(&format!("line {line}"));
        app.key(Key::Enter);
    }
    // At 800px the reply stops at `100vh - 116px` under the 41px strip (643px): the thread keeps
    // its bottom in view and the body scrolls to the caret, so its header shows too (the form
    // spans 125–768).
    let caret = app
        .mail()
        .compose_caret()
        .expect("the body has the keyboard");
    assert!(
        caret.top() > 125.0 && caret.bottom() < 768.0 - 52.0,
        "the caret ({caret:?}) shows above the footer"
    );
    // The wheel over its body scrolls the body, not the thread.
    let thread = app
        .mail()
        .thread()
        .map(|thread| thread.scroll())
        .expect("the thread is open");
    app.hover(egui::pos2(400.0, 400.0));
    app.harness
        .input_mut()
        .events
        .push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 100.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
    app.wait(0.5);
    assert!(
        app.mail()
            .compose_caret()
            .is_some_and(|moved| moved.top() > caret.top()),
        "the body scrolled up"
    );
    assert_eq!(
        app.mail().thread().map(|thread| thread.scroll()),
        Some(thread),
        "the thread stayed"
    );
}

#[test]
fn a_header_past_300px_scrolls_its_recipients_and_leaves_room_for_the_body() {
    let mut app = start("compose-many-recipients");
    app.press(Key::C, Modifiers::SHIFT);
    for at in 1..=40 {
        app.type_text(&format!("person{at}@example.com,"));
    }
    app.key(Key::Tab);
    app.key(Key::Tab);
    app.type_text("Hi all");
    // The pop-out's form (639px from 157) keeps its header to 300px, so the subject and the
    // body come right under it, above the footer.
    let caret = app
        .mail()
        .compose_caret()
        .expect("the body has the keyboard");
    assert!(
        caret.top() > 157.0 + 300.0 - 12.0 && caret.bottom() < 800.0 - 4.0 - 68.0,
        "the caret ({caret:?}) shows under the header"
    );
}

/// A header of Gmail's draft with `subject`.
fn draft_header(app: &Fixture, subject: &str, name: &str) -> Option<String> {
    let gmail = app.gmail();
    app.block_on(async {
        for draft in gmail
            .gmail()
            .drafts(&DraftQuery::default())
            .await
            .expect("drafts.list")
            .drafts
        {
            let full = gmail
                .gmail()
                .draft(&draft.id.expect("an id"), Format::Full)
                .await
                .expect("drafts.get");
            let message = full.message.expect("a message");
            if message.header("Subject") == Some(subject) {
                return message.header(name).map(str::to_owned);
            }
        }
        None
    })
}

#[test]
fn changing_a_replys_subject_makes_it_a_new_message_in_its_own_thread() {
    let mut app = start("compose-subject-fork");
    app.key(Key::Enter);
    app.wait(0.5);
    let thread = app
        .mail()
        .thread()
        .map(|thread| thread.id().to_owned())
        .expect("the thread is open");
    app.press(Key::O, COMMAND.plus(Modifiers::SHIFT));
    app.type_text("Sounds good");
    app.press(Key::S, COMMAND.plus(Modifiers::SHIFT));
    assert!(
        !app.mail().compose_popped_out(),
        "⌘⇧S only focuses a reply's subject"
    );
    app.key(Key::Enter);
    assert!(
        app.mail().compose_popped_out(),
        "Enter on it pops out a new message"
    );
    app.type_text("Crit notes");
    assert_eq!(
        app.mail().compose_focus(),
        Some((
            vec!["tom@brightmoor.co".to_owned()],
            "Crit notes".to_owned()
        )),
        "the subject was selected, recipients kept"
    );
    app.wait(1.5);
    let drafts = gmail_drafts(&app);
    let forked = drafts
        .iter()
        .find(|(subject, _)| subject == "Crit notes")
        .expect("saved to Gmail");
    assert_ne!(forked.1, thread, "in a thread of its own");
    assert_eq!(
        draft_header(&app, "Crit notes", "In-Reply-To"),
        None,
        "answering nothing"
    );
    assert!(
        !drafts
            .iter()
            .any(|(subject, _)| subject == "Re: Design crit moved to 3pm"),
        "the reply is gone: {drafts:?}"
    );
}

#[test]
fn subjects_take_unicode_replacements_and_backspace_undoes_one() {
    let mut app = start("compose-subject-arrows");
    app.key(Key::C);
    app.key(Key::Tab);
    app.type_text("a -> b ... c");
    assert_eq!(
        app.mail()
            .compose_focus()
            .map(|(_, subject)| subject)
            .as_deref(),
        Some("a → b … c")
    );
    app.type_text(" =>");
    app.key(Key::Backspace);
    assert_eq!(
        app.mail()
            .compose_focus()
            .map(|(_, subject)| subject)
            .as_deref(),
        Some("a → b … c =>"),
        "Backspace right after one brings back what was typed"
    );
}

#[test]
fn a_new_subjects_colon_picks_an_emoji_and_escape_closes_it() {
    let mut app = start("compose-subject-pickers");
    app.key(Key::C);
    app.key(Key::Tab);
    app.type_text("Ship it :rocke");
    app.settle();
    app.key(Key::Enter);
    app.settle();
    let subject = |app: &Fixture| app.mail().compose_focus().map(|(_, subject)| subject);
    assert_eq!(
        subject(&app).as_deref(),
        Some("Ship it 🚀"),
        "Enter took the emoji, not the body"
    );
    app.type_text(" :sm");
    app.settle();
    app.key(Key::Escape);
    app.settle();
    app.key(Key::Enter);
    assert_eq!(
        subject(&app).as_deref(),
        Some("Ship it 🚀 :sm"),
        "Escape closed it, and Enter went on to the body"
    );
    assert!(app.mail().compose_body().is_some());
}

#[test]
fn tab_from_the_body_reaches_send_and_the_reply_shows_in_its_thread_at_once() {
    let mut app = start("compose-tab-send");
    app.key(Key::Enter);
    app.key(Key::R);
    app.type_text("Sounds good");
    app.key(Key::Tab);
    app.type_text("x");
    app.key(Key::Enter);
    assert!(app.mail().compose_focus().is_none(), "Enter on Send sends");
    let shown = |app: &Fixture| {
        app.mail()
            .thread()
            .map(|thread| {
                thread
                    .messages()
                    .iter()
                    .map(|message| (message.id.clone(), message.snippet.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let (sending, snippet) = shown(&app).last().cloned().expect("messages");
    assert!(
        sending.starts_with("sending-"),
        "the reply shows while it's held: {:?}",
        shown(&app)
    );
    assert!(
        snippet.starts_with("Sounds good"),
        "typing on Send typed nothing: {snippet}"
    );
    assert_eq!(
        app.mail().thread().and_then(|thread| thread.focused()),
        Some(sending.as_str()),
        "focused"
    );
    // Instant Send: Gmail's copy takes its place, focus and all.
    app.press(Key::Z, COMMAND.plus(Modifiers::SHIFT));
    app.wait(0.5);
    app.sync();
    let messages = shown(&app);
    let (last, _) = messages.last().cloned().expect("messages");
    assert!(
        !messages.iter().any(|(id, _)| id.starts_with("sending-")),
        "{messages:?}"
    );
    assert_eq!(
        app.mail().thread().and_then(|thread| thread.focused()),
        Some(last.as_str())
    );
    assert!(sent_subjects(&app).contains(&"Re: Design crit moved to 3pm".to_owned()));
}

#[test]
fn the_from_list_chooses_among_send_as_aliases() {
    let mut app = start("compose-from");
    write_new_message(&mut app, "From check");
    app.press(Key::F, COMMAND.plus(Modifiers::SHIFT));
    app.key(Key::ArrowDown);
    app.key(Key::Enter);
    app.wait(1.5);
    let from = || draft_header(&app, "From check", "From").unwrap_or_default();
    assert!(
        in_gmail(|| from().contains("hello@mayaokafor.com")),
        "{}",
        from()
    );
}

#[test]
fn formatting_keys_reach_the_body() {
    let mut app = start("compose-format");
    write_new_message(&mut app, "Bold check");
    app.press(Key::B, COMMAND);
    app.type_text(" loud");
    app.wait(1.5);
    let gmail = app.gmail();
    let html = || {
        app.block_on(async {
            for draft in gmail
                .gmail()
                .drafts(&DraftQuery::default())
                .await
                .expect("drafts.list")
                .drafts
            {
                let full = gmail
                    .gmail()
                    .draft(&draft.id.expect("an id"), Format::Raw)
                    .await
                    .expect("drafts.get");
                let raw = String::from_utf8_lossy(
                    &full
                        .message
                        .and_then(|message| message.raw)
                        .unwrap_or_default(),
                )
                .into_owned();
                if raw.contains("Bold check") {
                    return raw;
                }
            }
            String::new()
        })
    };
    assert!(
        in_gmail(|| html().contains("loud</b>")),
        "⌘B made the rest bold: {}",
        html()
    );
}

#[test]
fn a_draft_is_dated_when_it_is_written_not_when_exo_opened() {
    // The fixture's app opened at the mock's clock (`Config::now`), not this moment.
    let mut app = start("compose-date");
    write_new_message(&mut app, "Dated");
    app.wait(1.5);
    assert!(
        in_gmail(|| draft_header(&app, "Dated", "Date").is_some()),
        "saved to Gmail"
    );
    let date = draft_header(&app, "Dated", "Date").expect("a Date header");
    let date = jiff::fmt::rfc2822::parse(&date)
        .expect("an RFC 2822 date")
        .timestamp();
    let off = jiff::Timestamp::now().duration_since(date).abs();
    assert!(
        off < jiff::SignedDuration::from_mins(5),
        "dated {date}, {off:?} from now"
    );
}

/// The raw message of Gmail's draft with `subject`.
fn draft_raw(app: &Fixture, subject: &str) -> String {
    let gmail = app.gmail();
    app.block_on(async {
        for draft in gmail
            .gmail()
            .drafts(&DraftQuery::default())
            .await
            .expect("drafts.list")
            .drafts
        {
            let full = gmail
                .gmail()
                .draft(&draft.id.expect("an id"), Format::Raw)
                .await
                .expect("drafts.get");
            let raw = String::from_utf8_lossy(
                &full
                    .message
                    .and_then(|message| message.raw)
                    .unwrap_or_default(),
            )
            .into_owned();
            if raw.contains(&format!("Subject: {subject}")) {
                return raw;
            }
        }
        String::new()
    })
}

#[test]
fn quitting_saves_what_was_typed_in_the_last_second() {
    let mut app = start("compose-quit-typing");
    write_new_message(&mut app, "Last words");
    app.wait(1.5);
    // A key, then quitting before the once-a-second save comes round.
    app.harness
        .input_mut()
        .events
        .push(egui::Event::Text("!".into()));
    app.harness.run_steps(1);
    app.relaunch();
    let raw = draft_raw(&app, "Last words");
    assert!(
        raw.contains("Hi there!"),
        "the last key is in Gmail's draft: {raw}"
    );
}

#[test]
fn a_draft_goes_to_gmail_at_most_every_5_s_and_its_last_edit_once_they_are_up() {
    let mut app = start("compose-pace");
    write_new_message(&mut app, "Paced");
    app.wait(1.5);
    assert!(in_gmail(|| draft_raw(&app, "Paced").contains("Hi there")));
    let written = std::time::Instant::now();
    // Saved on the device a second after the key, to Gmail once 5 s have passed since its last
    // write, by itself.
    app.type_text(" again");
    app.wait(1.5);
    assert!(
        in_gmail(|| draft_raw(&app, "Paced").contains("Hi there again")),
        "written by itself"
    );
    assert!(
        written.elapsed() > std::time::Duration::from_secs(4),
        "{:?} after the write before",
        written.elapsed()
    );
}

/// An account alone on a fresh gmail-mock, its store in a folder named after `name`.
struct LoneAccount {
    runtime: tokio::runtime::Runtime,
    server: gmail_mock::MockServer,
    dir: std::path::PathBuf,
}

impl LoneAccount {
    fn new(name: &str) -> Self {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let server = runtime
            .block_on(gmail_mock::MockServer::start(gmail_mock::Config::default()))
            .expect("mock starts");
        let dir = std::env::temp_dir().join(format!("exo-{name}-{}", std::process::id()));
        Self {
            runtime,
            server,
            dir,
        }
    }

    /// Opens the account, as launching Exo does.
    fn open(&self) -> exo::account::Account {
        let tokens = std::sync::Arc::new(gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()));
        let credentials = exo::account::Credentials {
            email: "maya@brightmoor.co".into(),
            endpoints: self.server.endpoints(),
            tokens,
        };
        exo::account::Account::open(&self.dir, credentials, egui::Context::default())
            .expect("opens")
    }

    /// Saves a new message to Priya saying `body`, with Message-ID `rfc822_id`.
    fn save(account: &exo::account::Account, rfc822_id: &str, body: &str) {
        let to = exo_store::Address {
            name: None,
            email: "priya@brightmoor.co".into(),
        };
        let html = format!("<div>{body}</div>");
        let record = exo_store::DraftRecord {
            action: exo_store::DraftAction::Compose,
            from: Some(exo_store::Address {
                name: None,
                email: "maya@brightmoor.co".into(),
            }),
            to: vec![to],
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: "Sent on quit".into(),
            body: html.clone(),
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
        let email = gmail::mime::Email {
            from: Some(gmail::mime::Mailbox::address("maya@brightmoor.co")),
            to: vec![gmail::mime::Mailbox::address("priya@brightmoor.co")],
            subject: "Sent on quit".into(),
            html: Some(html),
            message_id: Some(rfc822_id.into()),
            ..gmail::mime::Email::default()
        };
        let draft = exo_store::Draft {
            id: "draft00000000000099".into(),
            thread: None,
            adopted: None,
            record,
            raw: String::new(),
            complete: false,
            sending: false,
        };
        account.save_draft(draft, email);
    }

    /// Saves a new message to Priya with Message-ID `rfc822_id` and holds it for sending at `at`.
    fn send(account: &exo::account::Account, rfc822_id: &str, at: jiff::Timestamp) {
        Self::save(account, rfc822_id, "Bye");
        account.send_draft("draft00000000000099", at);
    }

    /// The text of Gmail's draft with Message-ID `rfc822_id`, if there is one.
    fn draft_text(&self, rfc822_id: &str) -> String {
        let gmail = gmail::Client::new(
            self.server.endpoints(),
            gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()),
        );
        let query = DraftQuery {
            q: Some(format!("rfc822msgid:{rfc822_id}")),
            ..DraftQuery::default()
        };
        self.runtime.block_on(async {
            let Some(id) = gmail
                .gmail()
                .drafts(&query)
                .await
                .expect("drafts.list")
                .drafts
                .into_iter()
                .find_map(|draft| draft.id)
            else {
                return String::new();
            };
            let draft = gmail
                .gmail()
                .draft(&id, Format::Full)
                .await
                .expect("drafts.get");
            draft
                .message
                .and_then(|message| message.snippet)
                .unwrap_or_default()
        })
    }

    /// The message of Gmail's draft with Message-ID `rfc822_id`, if there is one.
    fn draft_message(&self, rfc822_id: &str) -> Option<String> {
        let gmail = gmail::Client::new(
            self.server.endpoints(),
            gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()),
        );
        let query = DraftQuery {
            q: Some(format!("rfc822msgid:{rfc822_id}")),
            ..DraftQuery::default()
        };
        let drafts = self
            .runtime
            .block_on(async { gmail.gmail().drafts(&query).await })
            .expect("drafts.list")
            .drafts;
        drafts
            .into_iter()
            .find_map(|draft| draft.message.and_then(|message| message.id))
    }

    /// How many sent messages Gmail has with Message-ID `rfc822_id`.
    fn sent(&self, rfc822_id: &str) -> usize {
        let gmail = gmail::Client::new(
            self.server.endpoints(),
            gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()),
        );
        let query = ListQuery {
            q: Some(format!("rfc822msgid:{rfc822_id}")),
            label_ids: vec!["SENT".into()],
            ..ListQuery::default()
        };
        self.runtime
            .block_on(async { gmail.gmail().messages(&query).await })
            .expect("messages.list")
            .messages
            .len()
    }

    /// Arms a gmail-mock fault, or with `None` clears every one.
    fn fault(&self, fault: Option<serde_json::Value>) {
        let url = self.server.url().join("/__mock/faults").expect("a URL");
        let client = reqwest::Client::new();
        let request = match fault {
            Some(fault) => client.post(url).json(&fault),
            None => client.delete(url),
        };
        self.runtime
            .block_on(request.send())
            .expect("the mock takes it");
    }
}

impl Drop for LoneAccount {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn quitting_right_after_send_still_sends() {
    // The account alone, closed the moment Send is asked for: before its worker has published
    // the held send.
    let lone = LoneAccount::new("quit-send");
    let account = lone.open();
    LoneAccount::send(
        &account,
        "quit.1@brightmoor.co",
        jiff::Timestamp::now() + jiff::SignedDuration::from_secs(20),
    );
    drop(account);
    assert_eq!(
        lone.sent("quit.1@brightmoor.co"),
        1,
        "a held send goes before Exo quits"
    );
}

#[test]
fn quitting_within_the_pace_writes_the_latest_version() {
    let lone = LoneAccount::new("quit-paced");
    let account = lone.open();
    assert!(wait_for(30, || account.is_idle()), "the first sync");
    LoneAccount::save(&account, "quit.2@brightmoor.co", "First");
    assert!(
        wait_for(10, || lone
            .draft_text("quit.2@brightmoor.co")
            .contains("First")),
        "the first write goes at once"
    );
    // An edit whose write waits out the pace, then quitting.
    LoneAccount::save(&account, "quit.2@brightmoor.co", "First and last");
    assert!(wait_for(10, || account.is_idle()), "saved");
    drop(account);
    assert!(
        lone.draft_text("quit.2@brightmoor.co")
            .contains("First and last"),
        "written on the way out: {}",
        lone.draft_text("quit.2@brightmoor.co")
    );
}

#[test]
fn quitting_after_saving_a_draft_unchanged_leaves_gmails_as_it_is() {
    let lone = LoneAccount::new("quit-unchanged");
    let account = lone.open();
    assert!(wait_for(30, || account.is_idle()), "the first sync");
    LoneAccount::save(&account, "quit.3@brightmoor.co", "Unchanged");
    assert!(
        wait_for(10, || lone
            .draft_text("quit.3@brightmoor.co")
            .contains("Unchanged")),
        "written"
    );
    let message = lone.draft_message("quit.3@brightmoor.co");
    // Saved again with nothing changed, its message dated a second later, then quitting.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    LoneAccount::save(&account, "quit.3@brightmoor.co", "Unchanged");
    assert!(wait_for(10, || account.is_idle()), "saved");
    assert!(
        account.snapshot().drafts.iter().all(|draft| draft.synced),
        "Gmail has it already"
    );
    drop(account);
    assert_eq!(
        lone.draft_message("quit.3@brightmoor.co"),
        message,
        "not written on the way out"
    );
}

#[test]
fn a_send_held_when_google_signs_the_account_out_fails_as_logged_out() {
    let lone = LoneAccount::new("signed-out-send");
    let account = lone.open();
    // Google stops taking the account's token: closing the account (as the sign-out does) sends
    // what it holds, which fails.
    lone.fault(Some(
        serde_json::json!({"path": "/", "status": 401, "times": 100_000}),
    ));
    LoneAccount::send(
        &account,
        "signed-out.1@brightmoor.co",
        jiff::Timestamp::now() + jiff::SignedDuration::from_secs(20),
    );
    drop(account);
    lone.fault(None);
    // Signed in again: a failed draft, `Send failed: logged out of Google`, which goes only when
    // the user sends it again (not late, by itself).
    let account = lone.open();
    let started = std::time::Instant::now();
    let failure = || {
        account
            .snapshot()
            .drafts
            .iter()
            .find(|draft| draft.id == "draft00000000000099")
            .and_then(|draft| draft.failure)
    };
    while (failure().is_none() || account.snapshot().drafts.iter().any(|draft| !draft.synced))
        && started.elapsed() < std::time::Duration::from_secs(10)
    {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(failure(), Some(exo_store::SendFailure::LoggedOut));
    std::thread::sleep(std::time::Duration::from_millis(500));
    let sent = lone.sent("signed-out.1@brightmoor.co");
    drop(account);
    assert_eq!(sent, 0, "it waits for the user");
}

#[test]
fn a_reply_gmail_wont_send_stays_in_its_thread_as_a_failed_draft() {
    let mut app = start("compose-failed");
    app.key(Key::Enter);
    let thread = app
        .mail()
        .thread()
        .map(|view| view.id().to_owned())
        .expect("a thread");
    app.key(Key::R);
    app.type_text("Sounds good");
    app.fault(Some(serde_json::json!({"method": "POST", "path": "/gmail/v1/users/me/drafts/send", "status": 413, "message": "Request Entity Too Large", "times": 10})));
    app.press(Key::Enter, COMMAND);
    app.press(Key::Z, COMMAND.plus(Modifiers::SHIFT));
    app.wait(1.0);
    assert_eq!(
        app.mail().compose_alert(),
        Some("Message failed to send because it was too large."),
        "sent this session: the alert says so"
    );
    // See Message: the failed draft in its thread, a message rather than a draft to edit.
    app.key(Key::Enter);
    app.settle();
    let failure = |app: &Fixture| {
        app.mail()
            .thread()
            .and_then(|view| view.messages().last().and_then(|message| message.failure))
    };
    assert_eq!(failure(&app), Some(exo_store::SendFailure::TooLarge));
    assert!(
        app.mail().compose_focus().is_none(),
        "not reopened in compose"
    );
    let row = |app: &Fixture| {
        app.mail()
            .inbox()
            .threads()
            .iter()
            .position(|row| row.id == thread)
            .map(|at| (at, app.mail().inbox().threads()[at].failure))
    };
    assert_eq!(
        row(&app),
        Some((0, Some(exo_store::SendFailure::TooLarge))),
        "its row says so, on top"
    );
    // Nothing sends it, a relaunch included.
    app.fault(None);
    app.relaunch();
    app.settle();
    assert_eq!(
        row(&app),
        Some((0, Some(exo_store::SendFailure::TooLarge))),
        "still failed after a relaunch"
    );
    assert!(!sent_subjects(&app).contains(&"Re: Design crit moved to 3pm".to_owned()));
}

/// Waits for Gmail's drafts to show `done`: the save that typing a message ends with reaches
/// Gmail up to 5 s after the draft's first write.
fn in_gmail(done: impl FnMut() -> bool) -> bool {
    wait_for(10, done)
}

/// Waits up to `seconds` for `done`.
fn wait_for(seconds: u64, mut done: impl FnMut() -> bool) -> bool {
    let started = std::time::Instant::now();
    while !done() {
        if started.elapsed() > std::time::Duration::from_secs(seconds) {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    true
}

#[test]
fn a_draft_saved_without_its_files_goes_to_gmail_once_they_can_be_had() {
    let lone = LoneAccount::new("files-retry");
    let account = lone.open();
    assert!(wait_for(30, || account.is_idle()), "the first sync");
    // A message with a file, to forward.
    let gmail = gmail::Client::new(
        lone.server.endpoints(),
        gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into()),
    );
    let (message, part) = lone.runtime.block_on(async {
        let query = ListQuery {
            q: Some("has:attachment".into()),
            ..ListQuery::default()
        };
        let listed = gmail
            .gmail()
            .messages(&query)
            .await
            .expect("messages.list")
            .messages;
        let id = listed
            .first()
            .and_then(|message| message.id.clone())
            .expect("mail with a file");
        let full = gmail
            .gmail()
            .message(&id, Format::Full, &[])
            .await
            .expect("messages.get");
        let part = full
            .payload
            .as_ref()
            .and_then(|payload| {
                payload
                    .walk()
                    .find(|part| {
                        part.filename
                            .as_deref()
                            .is_some_and(|name| !name.is_empty())
                    })
                    .cloned()
            })
            .expect("a file");
        (id, part)
    });
    let file = exo_store::DraftAttachment {
        uuid: "a7c1f9e2-0000-4000-8000-000000000001".into(),
        name: part.filename.clone().unwrap_or_default(),
        mime_type: part.mime_type.clone().unwrap_or_default(),
        size: part
            .body
            .as_ref()
            .and_then(|body| body.size)
            .unwrap_or_default()
            .into(),
        content_id: None,
        source: exo_store::AttachmentSource::Gmail {
            message_id: message.clone(),
            attachment_id: part
                .body
                .as_ref()
                .and_then(|body| body.attachment_id.clone()),
            part_id: part.part_id.clone().unwrap_or_default(),
        },
    };
    let record = exo_store::DraftRecord {
        action: exo_store::DraftAction::Compose,
        from: Some(exo_store::Address {
            name: None,
            email: "maya@brightmoor.co".into(),
        }),
        to: vec![exo_store::Address {
            name: None,
            email: "priya@brightmoor.co".into(),
        }],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: "Fwd: the file".into(),
        body: "<div>Here it is</div>".into(),
        quoted_content: None,
        quoted_inlined: false,
        in_reply_to: None,
        in_reply_to_rfc822: None,
        references: Vec::new(),
        rfc822_id: "files-retry.1@brightmoor.co".into(),
        fingerprint: String::new(),
        attachments: vec![file],
        send_later: None,
        reminder: None,
    };
    let email = gmail::mime::Email {
        from: Some(gmail::mime::Mailbox::address("maya@brightmoor.co")),
        to: vec![gmail::mime::Mailbox::address("priya@brightmoor.co")],
        subject: "Fwd: the file".into(),
        html: Some("<div>Here it is</div>".into()),
        message_id: Some("files-retry.1@brightmoor.co".into()),
        ..gmail::mime::Email::default()
    };
    // Gmail is out of reach: the file can't be fetched, and neither can history.
    lone.fault(Some(serde_json::json!({"method": "GET", "path": format!("/gmail/v1/users/me/messages/{message}"), "status": 503, "times": 100_000})));
    lone.fault(Some(serde_json::json!({"method": "GET", "path": "/gmail/v1/users/me/history", "status": 400, "times": 100_000})));
    let draft = exo_store::Draft {
        id: "draft00000000000098".into(),
        thread: None,
        adopted: None,
        record,
        raw: String::new(),
        complete: false,
        sending: false,
    };
    account.save_draft(draft, email);
    // A sync round that fails, as offline, then off screen (switched away from).
    account.set_presence(exo::account::Presence::Visible);
    account.set_presence(exo::account::Presence::Focused);
    assert!(
        wait_for(60, || account.is_idle()),
        "the save and the failed round"
    );
    account.set_presence(exo::account::Presence::Background);
    let unwritten = || account.snapshot().drafts.iter().any(|draft| !draft.synced);
    assert!(unwritten(), "the draft isn't in Gmail");
    let drafts = |gmail: &gmail::Client| {
        lone.runtime.block_on(async {
            gmail
                .gmail()
                .drafts(&DraftQuery {
                    q: Some("rfc822msgid:files-retry.1@brightmoor.co".into()),
                    ..DraftQuery::default()
                })
                .await
                .expect("drafts.list")
                .drafts
        })
    };
    assert!(drafts(&gmail).is_empty(), "not written without its file");

    // Back online: the next round (5 s after a failed one) saves it again, with the file.
    lone.fault(None);
    assert!(
        wait_for(30, || !unwritten() && account.is_idle()),
        "written to Gmail by itself, off screen too"
    );
    let [written] = drafts(&gmail).try_into().expect("one Gmail draft");
    let written = lone.runtime.block_on(async {
        gmail
            .gmail()
            .draft(written.id.as_deref().expect("an id"), Format::Full)
            .await
            .expect("drafts.get")
    });
    let files: Vec<String> = written
        .message
        .and_then(|message| message.payload)
        .map(|payload| {
            payload
                .walk()
                .filter_map(|part| part.filename.clone())
                .filter(|name| !name.is_empty())
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(files, [part.filename.unwrap_or_default()], "with its file");
}

#[test]
fn a_reply_sent_offline_waits_for_the_network_where_it_shows() {
    let mut app = start("compose-offline-send");
    app.key(Key::Enter);
    let thread = app
        .mail()
        .thread()
        .map(|view| view.id().to_owned())
        .expect("a thread");
    app.key(Key::R);
    app.type_text("Sounds good");
    app.disconnect();
    app.mail().account().network_changed();
    app.settle();
    assert!(app.mail().status_bar().is_some(), "offline");
    app.press(Key::Enter, COMMAND);
    app.settle();
    // `Send: when online` in its thread and on its row, which draw from these.
    let sending = app
        .mail()
        .thread()
        .and_then(|view| view.messages().last().map(|message| message.sending));
    assert_eq!(sending, Some(true), "the reply shows as it'll go");
    let row = app
        .mail()
        .inbox()
        .threads()
        .iter()
        .find(|row| row.id == thread)
        .map(|row| row.sending);
    assert_eq!(row, Some(true), "its row knows a send waits");
}

/// In Gmail web, a reply to the first conversation with a file, synced and the conversation
/// opened: its thread and the draft's message.
fn open_gmail_web_draft_with_a_file(app: &mut Fixture) -> (String, String) {
    let thread = app.mail().inbox().threads()[0].id.clone();
    let gmail = app.gmail();
    let message = app.block_on(async {
        let detail = gmail
            .gmail()
            .thread(
                &thread,
                Format::Metadata,
                &["Message-ID", "References", "Subject"],
            )
            .await
            .expect("threads.get");
        let last = detail.messages.last().expect("a message");
        let mut email = gmail::mime::Email {
            to: vec![gmail::mime::Mailbox::address("tom@brightmoor.co")],
            subject: gmail::mime::reply_subject(last.header("Subject").unwrap_or_default()),
            text: Some("Notes attached".into()),
            attachments: vec![gmail::mime::Attachment {
                filename: "notes.txt".into(),
                content_type: "text/plain".into(),
                data: b"agenda".to_vec(),
                content_id: None,
            }],
            ..gmail::mime::Email::default()
        };
        email.reply_to_message(last);
        let draft = gmail
            .gmail()
            .create_draft(email.to_rfc822(), Some(&thread))
            .await
            .expect("drafts.create");
        draft
            .message
            .and_then(|message| message.id)
            .expect("its message")
    });
    app.sync();
    app.key(Key::Enter);
    (thread, message)
}

#[test]
fn a_draft_written_in_gmail_keeps_its_files_once_exo_saves_it() {
    let mut app = start("compose-adopt-files");
    open_gmail_web_draft_with_a_file(&mut app);
    // R takes up the draft on the message it answers; typing makes Exo save it to Gmail.
    app.key(Key::R);
    app.type_text(" Thanks!");
    app.wait(1.5);
    let files = draft_files(&app, "Re: Design crit moved to 3pm");
    assert_eq!(
        files
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["notes.txt"],
        "Exo's save kept the draft's file"
    );
}

#[test]
fn a_gmail_draft_taken_over_offline_stays_one_draft_with_its_files() {
    let mut app = start("compose-adopt-offline");
    let (thread, message) = open_gmail_web_draft_with_a_file(&mut app);
    // Offline for the draft's file and for Gmail's drafts, Exo takes the draft up.
    app.fault(Some(serde_json::json!({"method": "GET", "path": format!("/gmail/v1/users/me/messages/{message}"), "status": 404, "times": 1000})));
    app.fault(Some(serde_json::json!({"method": "GET", "path": "/gmail/v1/users/me/drafts", "status": 404, "times": 1000})));
    app.key(Key::R);
    app.type_text(" Thanks!");
    app.wait(1.5);
    // The thread closed and opened again before Gmail has the edit.
    app.key(Key::Escape);
    app.key(Key::Escape);
    assert!(app.mail().thread().is_none(), "back to the list");
    app.key(Key::Enter);
    assert_eq!(app.mail().thread_drafts(), 1, "taken up once");
    // Back online, the next save writes Gmail's draft, file and all.
    app.fault(None);
    app.key(Key::R);
    app.type_text(" Bye");
    app.wait(1.5);
    let drafts: Vec<_> = gmail_drafts(&app)
        .into_iter()
        .filter(|(_, id)| *id == thread)
        .collect();
    assert_eq!(drafts.len(), 1, "one Gmail draft: {drafts:?}");
    let subject = drafts[0].0.clone();
    assert_eq!(
        draft_files(&app, &subject)
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>(),
        ["notes.txt"],
        "with its file"
    );
    assert!(
        draft_raw(&app, &subject).contains("Thanks!"),
        "and the edit made offline"
    );
}

#[test]
fn a_gmail_draft_saved_while_its_files_cant_be_had_keeps_them() {
    let mut app = start("compose-adopt-no-files");
    let (_, message) = open_gmail_web_draft_with_a_file(&mut app);
    let subject = "Re: Design crit moved to 3pm";
    // Gmail answers, but not with the draft's file.
    app.fault(Some(serde_json::json!({"method": "GET", "path": format!("/gmail/v1/users/me/messages/{message}"), "status": 404, "times": 1000})));
    app.key(Key::R);
    app.type_text(" Thanks!");
    app.wait(1.5);
    assert_eq!(
        draft_files(&app, subject)
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>(),
        ["notes.txt"],
        "Gmail's draft kept its file"
    );
    // Once the file can be had, the next save writes the edit with it.
    app.fault(None);
    app.type_text(" Bye");
    app.wait(1.5);
    assert_eq!(
        draft_files(&app, subject)
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>(),
        ["notes.txt"]
    );
    assert!(
        draft_raw(&app, subject).contains("Thanks! Bye"),
        "with the edits"
    );
}

/// The Gmail quota units `act` costs, counted from when the app is quiet and Gmail has every
/// draft it holds (what came before, such as a save's refresh, isn't counted) to when it's that
/// way again (a save that waits out its pace included: 30 s with files). Sync's `history.list`
/// isn't counted ([`Fixture::units`]).
fn cost(app: &mut Fixture, act: impl FnOnce(&mut Fixture)) -> u64 {
    let synced = |app: &mut Fixture| {
        let account = app.mail().account();
        assert!(
            wait_for(40, || account.is_idle()
                && account.snapshot().drafts.iter().all(|draft| draft.synced)),
            "Gmail has every draft"
        );
        app.settle();
    };
    synced(app);
    let before = app.units();
    act(app);
    // Compose saves within the second.
    app.wait(1.5);
    synced(app);
    app.units() - before
}

#[test]
fn leaving_a_new_reply_forward_or_message_untouched_costs_nothing() {
    let mut app = start("compose-untouched");
    app.key(Key::Enter);
    let mut costs = Vec::new();
    for key in [Key::R, Key::A, Key::F] {
        costs.push((
            format!("{key:?}, Escape"),
            cost(&mut app, |app| {
                app.key(key);
                app.key(Key::Escape);
            }),
        ));
    }
    costs.push((
        "R, Escape twice (closing the thread)".into(),
        cost(&mut app, |app| {
            app.key(Key::R);
            app.key(Key::Escape);
            app.key(Key::Escape);
        }),
    ));
    costs.push((
        "C, Escape".into(),
        cost(&mut app, |app| {
            app.key(Key::C);
            app.key(Key::Escape);
        }),
    ));
    assert!(
        costs.iter().all(|(_, units)| *units == 0),
        "no Gmail draft is made for an untouched one: {costs:?}"
    );
}

#[test]
fn a_reply_opened_and_left_without_edits_is_not_written_again() {
    let mut app = start("compose-unchanged-reply");
    app.key(Key::Enter);
    let thread = app
        .mail()
        .thread()
        .map(|view| view.id().to_owned())
        .expect("a thread");
    app.key(Key::R);
    app.type_text("Sounds good");
    app.key(Key::Escape);
    assert!(in_gmail(
        || draft_raw(&app, "Re: Design crit moved to 3pm").contains("Sounds good")
    ));
    let mut costs = Vec::new();
    costs.push((
        "R, Escape",
        cost(&mut app, |app| {
            app.key(Key::R);
            app.key(Key::Escape);
        }),
    ));
    costs.push((
        "Escape twice, Enter, R, Escape",
        cost(&mut app, |app| {
            app.key(Key::Escape);
            app.key(Key::Escape);
            app.key(Key::Enter);
            app.key(Key::R);
            app.key(Key::Escape);
        }),
    ));
    // Relaunched, the first write would go at once.
    app.relaunch();
    costs.push((
        "relaunched: Enter, R, Escape",
        cost(&mut app, |app| {
            app.key(Key::Enter);
            assert_eq!(
                app.mail().thread().map(|view| view.id().to_owned()),
                Some(thread.clone())
            );
            app.key(Key::R);
            app.key(Key::Escape);
        }),
    ));
    assert!(
        costs.iter().all(|(_, units)| *units == 0),
        "Gmail has the draft already: {costs:?}"
    );
    assert!(draft_raw(&app, "Re: Design crit moved to 3pm").contains("Sounds good"));
}

/// The message Gmail's draft with `subject` holds.
fn draft_message(app: &Fixture, subject: &str) -> Option<String> {
    let gmail = app.gmail();
    app.block_on(async {
        for draft in gmail
            .gmail()
            .drafts(&DraftQuery::default())
            .await
            .expect("drafts.list")
            .drafts
        {
            let full = gmail
                .gmail()
                .draft(&draft.id.expect("an id"), Format::Full)
                .await
                .expect("drafts.get");
            let message = full.message.expect("a message");
            if message.header("Subject") == Some(subject) {
                return message.id;
            }
        }
        None
    })
}

/// Opens the draft-only conversation `subject` from the list: its draft in the new-message pane,
/// To focused.
fn open_draft_row(app: &mut Fixture, subject: &str) {
    for key in [Key::K, Key::J] {
        for _ in 0..30 {
            if app.cursor() == subject {
                break;
            }
            app.key(key);
        }
    }
    assert_eq!(app.cursor(), subject, "{:?}", app.subjects());
    app.key(Key::Enter);
    assert!(
        app.mail().compose_pane_open(),
        "the draft opens in the pane"
    );
}

#[test]
fn a_new_message_opened_and_left_without_edits_is_not_written_again() {
    let mut app = start("compose-unchanged-new");
    write_new_message(&mut app, "Untouched");
    app.wait(1.5);
    assert!(in_gmail(
        || draft_raw(&app, "Untouched").contains("Hi there")
    ));
    let mut costs = Vec::new();
    // The first Escape leaves the form, the second the pane.
    costs.push((
        "Escape twice",
        cost(&mut app, |app| {
            app.key(Key::Escape);
            app.key(Key::Escape);
        }),
    ));
    // Tab commits what's typed in To, here nothing.
    costs.push((
        "open, Tab, Tab, Escape twice",
        cost(&mut app, |app| {
            open_draft_row(app, "Untouched");
            app.key(Key::Tab);
            app.key(Key::Tab);
            app.key(Key::Escape);
            app.key(Key::Escape);
        }),
    ));
    assert!(
        costs.iter().all(|(_, units)| *units == 0),
        "Gmail has the draft already: {costs:?}"
    );
    // Quitting saves the open draft at once, which leaves Gmail's as it is.
    let message = draft_message(&app, "Untouched");
    open_draft_row(&mut app, "Untouched");
    app.key(Key::Tab);
    app.relaunch();
    assert!(
        app.mail()
            .account()
            .snapshot()
            .drafts
            .iter()
            .all(|draft| draft.synced)
    );
    assert_eq!(
        draft_message(&app, "Untouched"),
        message,
        "not written again on the way out"
    );
}

#[test]
fn a_gmail_web_draft_opened_and_left_untouched_stays_as_it_was() {
    let mut app = start("compose-adopt-untouched");
    let (thread, message) = open_gmail_web_draft_with_a_file(&mut app);
    let mut costs = Vec::new();
    // R takes the draft up in compose.
    costs.push((
        "R, Escape",
        cost(&mut app, |app| {
            app.key(Key::R);
            app.key(Key::Escape);
        }),
    ));
    costs.push((
        "Escape twice, Enter, R, Escape",
        cost(&mut app, |app| {
            app.key(Key::Escape);
            app.key(Key::Escape);
            app.key(Key::Enter);
            app.key(Key::R);
            app.key(Key::Escape);
        }),
    ));
    assert!(
        costs.iter().all(|(_, units)| *units == 0),
        "not taken over without an edit: {costs:?}"
    );
    let gmail = app.gmail();
    let messages: Vec<String> = app
        .block_on(async {
            gmail
                .gmail()
                .drafts(&DraftQuery::default())
                .await
                .expect("drafts.list")
                .drafts
        })
        .into_iter()
        .filter_map(|draft| draft.message)
        .filter(|draft| draft.thread_id.as_deref() == Some(thread.as_str()))
        .filter_map(|draft| draft.id)
        .collect();
    assert_eq!(messages, [message], "Gmail web's own version");
}
