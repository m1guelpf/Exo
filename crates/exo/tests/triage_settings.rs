//! The triage settings end to end on gmail-mock: Auto Advance (its dialog, and where triage goes
//! next), Send + Mark Done with Undo send, the palette's RSVP rows and RSVP + Mark Done, and Show
//! Sender Full Names.

mod support;

use egui::{Event, Key, Modifiers};
use exo_store::AutoAdvance;
use support::{Fixture, start, start_with_settings};

const CMD: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);
const CRIT: &str = "Design crit moved to 3pm";
const INVITATION: &str = "Invitation: Coffee with Grace";

/// Opens the palette and runs the row `query` ranks first.
fn palette(app: &mut Fixture, query: &str) {
    app.press(Key::K, CMD);
    app.harness
        .input_mut()
        .events
        .push(Event::Text(query.to_owned()));
    app.harness.run_steps(1);
    app.key(Key::Enter);
}

fn open_subject(app: &Fixture) -> String {
    app.mail()
        .thread()
        .and_then(|thread| thread.detail())
        .map(|detail| detail.subject.clone())
        .unwrap_or_default()
}

fn listed(app: &Fixture, subject: &str) -> bool {
    app.subjects()
        .iter()
        .any(|listed| listed.starts_with(subject))
}

/// My answer to Grace's coffee invitation, as the mock's calendar has it.
fn calendar_answer(app: &Fixture) -> Option<String> {
    let gmail = app.gmail();
    let query = gmail::calendar::EventsQuery {
        q: Some("Coffee with Grace".into()),
        single_events: true,
        ..Default::default()
    };
    let found = app
        .block_on(
            gmail
                .calendar()
                .events("maya@brightmoor.co", &query)
                .into_future(),
        )
        .expect("lists");
    found
        .items
        .iter()
        .flat_map(|event| &event.attendees)
        .find(|attendee| attendee.email.as_deref() == Some("maya@brightmoor.co"))
        .and_then(|attendee| attendee.response_status.clone())
}

#[test]
fn the_auto_advance_dialog_chooses_from_its_list_and_keeps_the_choice() {
    let mut app = start("auto-advance-dialog");
    palette(&mut app, "auto advance settings");
    assert!(app.mail().settings_dialog_open());
    // ↓ opens the list on the choice, which it lists first; `go back to the list of
    // conversations` is next, by name.
    app.key(Key::ArrowDown);
    app.key(Key::ArrowDown);
    app.key(Key::Enter);
    assert_eq!(
        app.mail().account().settings().desktop_auto_advance,
        AutoAdvance::Off
    );
    assert!(
        app.mail().settings_dialog_open(),
        "a choice leaves the dialog up"
    );
    app.key(Key::Escape);
    assert!(!app.mail().settings_dialog_open());
    app.relaunch();
    assert_eq!(
        app.mail().account().settings().desktop_auto_advance,
        AutoAdvance::Off,
        "kept"
    );
}

#[test]
fn auto_advance_up_goes_to_the_newer_conversation() {
    let mut app = start_with_settings("auto-advance-up", &|settings| {
        settings.desktop_auto_advance = AutoAdvance::Up
    });
    app.key(Key::J);
    app.key(Key::Enter);
    assert!(open_subject(&app).starts_with(INVITATION));
    app.key(Key::E);
    assert!(!listed(&app, INVITATION), "marked done");
    assert_eq!(open_subject(&app), CRIT, "the conversation above opens");
    // In the list too: from `1:1 agenda`, the cursor goes up.
    app.key(Key::Escape);
    app.key(Key::J);
    assert_eq!(app.cursor(), "1:1 agenda");
    app.key(Key::E);
    assert_eq!(app.cursor(), CRIT);
}

#[test]
fn auto_advance_off_goes_back_to_the_list_with_the_cursor_below() {
    let mut app = start_with_settings("auto-advance-off", &|settings| {
        settings.desktop_auto_advance = AutoAdvance::Off
    });
    app.key(Key::Enter);
    assert_eq!(open_subject(&app), CRIT);
    app.key(Key::E);
    assert!(app.mail().thread().is_none(), "back to the list");
    assert!(
        app.cursor().starts_with(INVITATION),
        "on the conversation below"
    );
    app.key(Key::Z);
    assert_eq!(open_subject(&app), CRIT, "undo reopens it");
}

#[test]
fn auto_advance_goes_from_where_the_open_conversation_was_before_it_moved_up() {
    let mut app = start("auto-advance-anchor");
    app.key(Key::J);
    app.key(Key::J);
    app.key(Key::Enter);
    assert_eq!(open_subject(&app), "1:1 agenda");
    // Priya writes again: the conversation goes to the top while it's open.
    let thread = app
        .mail()
        .thread()
        .map(|view| view.id().to_owned())
        .expect("open");
    let delivery = serde_json::json!({ "from": "Priya Raman <priya@brightmoor.co>", "threadId": thread, "text": "One more thing." });
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
    assert_eq!(
        app.subjects().first().map(String::as_str),
        Some("1:1 agenda"),
        "on top"
    );
    app.key(Key::E);
    assert_eq!(
        open_subject(&app),
        "Out for delivery: your package from Field & Fell",
        "the one that was below it, not the one now below it"
    );
}

#[test]
fn send_and_mark_done_marks_a_reply_done_and_undo_send_brings_it_back() {
    let mut app = start("send-and-done");
    palette(&mut app, "enable send + mark done");
    assert!(app.mail().account().settings().send_and_done);
    assert_eq!(
        app.mail().hint_text().as_deref(),
        Some("Enabled Send + Mark Done")
    );
    app.key(Key::Enter);
    app.key(Key::R);
    app.type_text("Works for me");
    // ⌘ Enter is Send & Mark Done now.
    app.press(Key::Enter, CMD);
    assert!(!listed(&app, CRIT), "marked done with the send");
    assert!(open_subject(&app).starts_with(INVITATION), "Auto Advance");
    app.key(Key::Z);
    app.settle();
    assert!(listed(&app, CRIT), "back in the inbox");
    assert_eq!(
        app.mail().toast(),
        Some("Sending undone and conversation moved to Inbox.")
    );
    assert!(app.mail().compose_focus().is_some(), "the reply reopens");
    // Out of the draft, whose own `Send & Mark Done` would rank first.
    app.key(Key::Escape);
    palette(&mut app, "disable send + mark done");
    assert!(!app.mail().account().settings().send_and_done);
}

#[test]
fn the_palette_answers_the_open_invitation() {
    let mut app = start("rsvp-palette");
    app.key(Key::J);
    app.key(Key::Enter);
    app.settle();
    assert_eq!(app.mail().invite_answer(), Some("needsAction"));
    palette(&mut app, "rsvp maybe");
    app.settle();
    assert_eq!(app.mail().toast(), Some("Responded Maybe."));
    assert_eq!(
        app.mail().invite_answer(),
        Some("tentative"),
        "the card shows it"
    );
    assert_eq!(calendar_answer(&app).as_deref(), Some("tentative"));
    assert!(open_subject(&app).starts_with(INVITATION), "and stays");
}

#[test]
fn rsvp_and_mark_done_marks_the_invitation_done_and_undo_brings_it_back() {
    let mut app = start("rsvp-and-done");
    palette(&mut app, "enable rsvp + mark done");
    assert!(app.mail().account().settings().rsvp_and_done);
    app.key(Key::J);
    app.key(Key::Enter);
    app.settle();
    // The card's first button, from the keyboard.
    app.key(Key::Tab);
    app.key(Key::Enter);
    app.settle();
    assert_eq!(app.mail().toast(), Some("Responded Yes and Marked Done."));
    assert_eq!(calendar_answer(&app).as_deref(), Some("accepted"));
    assert!(!listed(&app, INVITATION), "marked done");
    assert_eq!(open_subject(&app), "1:1 agenda", "Auto Advance");
    app.key(Key::Z);
    assert!(listed(&app, INVITATION), "undone");
    assert!(open_subject(&app).starts_with(INVITATION));
    // From the palette, its rows say so.
    palette(&mut app, "rsvp no and mark done");
    app.settle();
    assert_eq!(app.mail().toast(), Some("Responded No and Marked Done."));
    assert_eq!(calendar_answer(&app).as_deref(), Some("declined"));
}

#[test]
fn show_sender_full_names_toggles_from_the_palette() {
    let mut app = start("sender-full-names");
    palette(&mut app, "show sender full names");
    assert!(app.mail().account().settings().show_sender_full_names);
    palette(&mut app, "hide sender full names");
    assert!(!app.mail().account().settings().show_sender_full_names);
}
