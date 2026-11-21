//! Recipient fields end to end on gmail-mock: how suggestions rank, removing a suggestion,
//! dragging chips, the remove commands and Exclude from Auto Bcc.

mod support;

use egui::{Key, Modifiers};
use support::{Fixture, start_with_settings};

const COMMAND: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);

/// The app on an account opted out of the out-of-office nudge, which on some days would open
/// over the thread.
fn start(name: &str) -> Fixture {
    start_with_settings(name, &|settings| {
        settings.out_of_office_event_detection.opted_out_at = Some("2026-11-01T00:00:00Z".into())
    })
}

/// A Reply All to the first conversation (Tom's `Design crit moved to 3pm`) with its To focused
/// (⌘⇧O), and `query` typed there.
fn suggest(app: &mut Fixture, query: &str) -> Vec<String> {
    app.key(Key::Enter);
    app.press(Key::O, COMMAND.plus(Modifiers::SHIFT));
    app.type_text(query);
    app.wait(0.3);
    app.mail().compose_suggestions()
}

#[test]
fn a_replys_suggestions_start_with_the_thread_you_included() {
    let mut app = start("recipients-thread-first");
    let found = suggest(&mut app, "brightmoor");
    // The thread's people by address (`maya` < `priya` < `tom`), then the address book's.
    assert_eq!(
        found[..3],
        [
            "Maya Okafor maya@brightmoor.co",
            "Priya Raman priya@brightmoor.co",
            "Tom Becker tom@brightmoor.co"
        ]
    );
}

fn recipients(app: &Fixture) -> [Vec<String>; 3] {
    app.mail()
        .compose_recipients()
        .expect("a draft has the keyboard")
}

fn palette(app: &mut Fixture, query: &str) {
    app.press(Key::K, COMMAND);
    app.type_text(query);
    app.key(Key::Enter);
    app.settle();
}

#[test]
fn the_remove_commands_take_a_group_off_the_draft() {
    let mut app = start("recipients-remove-commands");
    suggest(&mut app, "carlos@pemberton-cpa.com");
    app.key(Key::Enter);
    assert_eq!(
        recipients(&app)[0],
        ["tom@brightmoor.co", "carlos@pemberton-cpa.com"]
    );
    palette(&mut app, "remove external");
    assert_eq!(
        recipients(&app),
        [
            vec!["tom@brightmoor.co".to_owned()],
            vec!["priya@brightmoor.co".to_owned()],
            Vec::new()
        ],
        "Carlos is the one outside brightmoor.co"
    );
    palette(&mut app, "remove recipients from brightmoor.co");
    assert_eq!(
        recipients(&app),
        [Vec::<String>::new(), Vec::new(), Vec::new()]
    );
    app.press(Key::O, COMMAND.plus(Modifiers::SHIFT));
    app.press(Key::Z, COMMAND);
    assert_eq!(recipients(&app)[0], ["tom@brightmoor.co"], "one undo step");
}

#[test]
fn compose_to_writes_to_the_contact_pane_s_person() {
    let mut app = start("recipients-compose-to");
    app.settle();
    palette(&mut app, "compose to tom");
    assert!(app.mail().compose_pane_open(), "a new message");
    assert_eq!(
        recipients(&app)[0],
        ["tom@brightmoor.co"],
        "to Tom, whom the pane shows for the first conversation"
    );
}

#[test]
fn exclude_from_auto_bcc_adds_the_sender_or_their_domain_once() {
    let mut app = start("recipients-exclude-auto-bcc");
    app.settle();
    palette(&mut app, "exclude from auto bcc");
    app.key(Key::Enter);
    app.settle();
    assert_eq!(
        app.mail().toast_text(),
        Some("Added to Auto Bcc Exclusions: tom@brightmoor.co")
    );
    palette(&mut app, "exclude from auto bcc");
    app.key(Key::Enter);
    app.settle();
    assert_eq!(
        app.mail().toast_text(),
        Some("Already in Auto Bcc Exclusions: tom@brightmoor.co")
    );
    palette(&mut app, "exclude from auto bcc");
    app.type_text("domain");
    app.key(Key::Enter);
    app.settle();
    assert_eq!(
        app.mail().toast_text(),
        Some("Added to Auto Bcc Exclusions: brightmoor.co")
    );
}

#[test]
fn a_removed_suggestion_stays_out_until_undone_or_written_to() {
    let mut app = start("recipients-remove-suggestion");
    app.key(Key::C);
    app.type_text("pa");
    app.wait(0.3);
    let before = app.mail().compose_suggestions();
    assert_eq!(
        before.first().map(String::as_str),
        Some("Paul Durand pdurand@westbrook-realty.com")
    );
    app.mail_mut().compose_remove_suggestion(0);
    app.wait(0.1);
    assert_eq!(
        app.mail().toast_text(),
        Some("Removed pdurand@westbrook-realty.com.")
    );
    assert_eq!(
        app.mail().compose_suggestions(),
        before[1..],
        "the others move up"
    );
    assert!(
        app.mail()
            .account()
            .settings()
            .excluded_contacts
            .contains_key("pdurand@westbrook-realty.com")
    );
    // ⌘Z in the field right after brings it back, typed text and all.
    app.press(Key::Z, COMMAND);
    assert_eq!(app.mail().compose_suggestions(), before);
    assert_eq!(app.mail().toast_text(), None);
    app.mail_mut().compose_remove_suggestion(0);
    app.type_text("u");
    app.wait(0.3);
    assert!(
        app.mail().compose_suggestions().is_empty(),
        "`pau` is only Paul"
    );
    // Sending to him takes him off the list of removed ones.
    for _ in 0..3 {
        app.key(Key::Backspace);
    }
    app.type_text("pdurand@westbrook-realty.com");
    app.key(Key::Enter);
    app.key(Key::Tab);
    app.type_text("Hi");
    app.key(Key::Tab);
    app.type_text("Hello");
    app.press(Key::Enter, COMMAND);
    assert!(app.mail().account().settings().excluded_contacts.is_empty());
}

#[test]
fn arrows_at_the_edge_of_typed_text_finish_it() {
    let mut app = start("recipients-arrows");
    app.key(Key::C);
    app.type_text("ann@x.com");
    app.key(Key::ArrowLeft);
    assert_eq!(
        recipients(&app)[0],
        Vec::<String>::new(),
        "mid-text, ← moves the caret"
    );
    app.key(Key::ArrowRight);
    app.key(Key::ArrowRight);
    assert_eq!(
        recipients(&app)[0],
        ["ann@x.com"],
        "→ at the end makes it a chip"
    );
    assert_eq!(
        app.mail().compose_text().map(|(_, typed)| typed),
        Some(String::new()),
        "the caret is after it"
    );
    app.type_text("bob@x.com");
    for _ in 0.."bob@x.com".len() {
        app.key(Key::ArrowLeft);
    }
    app.key(Key::ArrowLeft);
    assert_eq!(
        recipients(&app)[0],
        ["ann@x.com", "bob@x.com"],
        "← at the start does too"
    );
    app.key(Key::Backspace);
    assert_eq!(recipients(&app)[0], ["ann@x.com"], "and selects it");
}

#[test]
fn the_contact_pane_shows_a_replys_selected_chip() {
    let mut app = start("recipients-pane-chip");
    app.key(Key::Enter);
    app.wait(0.3);
    assert_eq!(app.mail().contact_pane_email(), Some("tom@brightmoor.co"));
    app.press(Key::C, COMMAND.plus(Modifiers::SHIFT));
    app.key(Key::ArrowLeft);
    app.wait(0.1);
    assert_eq!(
        app.mail().contact_pane_email(),
        Some("priya@brightmoor.co"),
        "Cc's chip, selected"
    );
    app.key(Key::ArrowRight);
    app.wait(0.1);
    assert_eq!(
        app.mail().contact_pane_email(),
        Some("priya@brightmoor.co"),
        "and it stays once the caret's back"
    );
    app.type_text("carlos@pemberton-cpa.com");
    app.key(Key::Enter);
    app.wait(0.1);
    assert_eq!(
        app.mail().contact_pane_email(),
        Some("carlos@pemberton-cpa.com"),
        "whom the reply just got"
    );
}

#[test]
fn the_contact_pane_shows_a_popped_out_drafts_new_recipient_until_the_cursor_moves() {
    let mut app = start("recipients-pane-popout");
    app.wait(0.3);
    assert_eq!(
        app.mail().contact_pane_email(),
        Some("tom@brightmoor.co"),
        "the cursor's conversation"
    );
    app.press(Key::C, Modifiers::SHIFT);
    app.type_text("carlos@pemberton-cpa.com");
    app.key(Key::Enter);
    app.wait(0.1);
    assert_eq!(
        app.mail().contact_pane_email(),
        Some("carlos@pemberton-cpa.com"),
        "whom the draft just got, over the list"
    );
    // The pointer resting on Grace's row (the second, left of the pop-out's card).
    app.hover(egui::pos2(200.0, 208.0));
    app.wait(0.3);
    assert_eq!(
        app.mail().contact_pane_email(),
        Some("grace@juniperpartners.vc"),
        "the cursor moved"
    );
}

#[test]
fn chips_drag_between_fields_in_one_undo_step() {
    let mut app = start("recipients-drag");
    app.key(Key::Enter);
    app.press(Key::O, COMMAND.plus(Modifiers::SHIFT));
    app.wait(0.2);
    // Tom's chip in To (where it sits at 1280×800) onto Bcc.
    app.drag(egui::pos2(340.0, 215.0), egui::pos2(320.0, 280.0));
    assert_eq!(
        recipients(&app),
        [
            Vec::<String>::new(),
            vec!["priya@brightmoor.co".to_owned()],
            vec!["tom@brightmoor.co".to_owned()]
        ]
    );
    assert_eq!(
        app.mail().hint_text().as_deref(),
        Some("Next time, hit ⌘ shift I to instantly move contacts to BCC!"),
        "a reply's Bcc suggests Instant Intro"
    );
    app.press(Key::Z, COMMAND);
    assert_eq!(
        recipients(&app),
        [
            vec!["tom@brightmoor.co".to_owned()],
            vec!["priya@brightmoor.co".to_owned()],
            Vec::new()
        ],
        "one undo step"
    );
    // Within a list, a chip lands where the cursor is.
    app.drag(egui::pos2(350.0, 247.0), egui::pos2(305.0, 215.0));
    assert_eq!(
        recipients(&app)[0],
        ["priya@brightmoor.co", "tom@brightmoor.co"],
        "Priya before Tom"
    );
}

#[test]
fn command_slash_pops_the_draft_out_and_searches() {
    let mut app = start("recipients-pop-search");
    app.key(Key::Enter);
    app.key(Key::R);
    app.type_text("Sounds good");
    app.press(Key::Slash, COMMAND);
    assert!(app.mail().compose_popped_out(), "popped out");
    assert_eq!(app.mail().search_text(), Some(""), "an empty bar");
    assert!(
        app.mail().search_bar_focused() && app.mail().compose_focus().is_none(),
        "with the keyboard"
    );
    app.type_text("atlas");
    assert_eq!(app.mail().search_text(), Some("atlas"));
}

#[test]
fn the_back_caret_beside_a_pop_out_goes_back() {
    let mut app = start("recipients-back-caret");
    app.key(Key::Enter);
    app.press(Key::R, Modifiers::SHIFT);
    assert!(app.mail().compose_popped_out());
    app.click(egui::pos2(64.0, 82.0));
    assert!(app.mail().thread().is_none(), "back to the list");
    assert!(
        app.mail().compose_popped_out(),
        "the draft stays popped out"
    );
}

#[test]
fn share_availabilitys_menu_filters_as_you_type() {
    let mut app = start("recipients-availability-menu");
    app.key(Key::Enter);
    app.key(Key::R);
    app.type_text("Hi ");
    app.press(Key::A, COMMAND.plus(Modifiers::SHIFT));
    assert_eq!(
        app.mail().compose_availability_query(),
        Some((String::new(), true))
    );
    app.type_text("fre");
    assert_eq!(
        app.mail().compose_availability_query(),
        Some(("fre".to_owned(), true)),
        "`fre` is in Insert free times"
    );
    app.type_text("zz");
    assert_eq!(
        app.mail().compose_availability_query(),
        Some(("frezz".to_owned(), false)),
        "no row, still open"
    );
    app.type_text("zzz");
    assert_eq!(
        app.mail().compose_availability_query(),
        None,
        "five characters without a row close it"
    );
    assert_eq!(
        app.mail()
            .compose_text()
            .map(|(body, _)| body)
            .as_deref()
            .map(str::trim_end),
        Some("Hi frezzzzz"),
        "keeping the text"
    );
    // Escape closes it too, keeping what's typed; Backspace at its start does.
    app.press(Key::A, COMMAND.plus(Modifiers::SHIFT));
    app.key(Key::Backspace);
    assert_eq!(app.mail().compose_availability_query(), None);
    assert_eq!(
        app.mail()
            .compose_text()
            .map(|(body, _)| body)
            .as_deref()
            .map(str::trim_end),
        Some("Hi frezzzzz"),
        "the Backspace only closed it"
    );
}
