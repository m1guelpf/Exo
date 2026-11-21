//! The folder drawer, hovering rows, the contact pane's conversations and ⌘C, end to end on
//! gmail-mock.

mod support;

use egui::{Key, Modifiers};
use support::start;

const COMMAND: Modifiers = Modifiers {
    mac_cmd: true,
    command: true,
    ..Modifiers::NONE
};

#[test]
fn left_opens_the_folder_drawer_and_its_keys_pick_a_folder() {
    let mut app = start("folders");
    app.key(Key::ArrowLeft);
    // The drawer has the keyboard: J moves its cursor, not the list's.
    app.key(Key::J);
    assert_eq!(app.cursor(), "Design crit moved to 3pm");
    app.key(Key::Enter);
    let starred = app.mail().inbox().threads();
    assert!(
        !starred.is_empty() && starred.iter().all(|thread| thread.starred),
        "Starred, the second folder"
    );

    // Escape closes it and gives the keys back to the list.
    app.key(Key::ArrowLeft);
    app.key(Key::Escape);
    let first = app.cursor();
    app.key(Key::J);
    assert_ne!(app.cursor(), first);

    // End goes to the last label.
    app.key(Key::ArrowLeft);
    app.key(Key::End);
    app.key(Key::Enter);
    let last = app.mail().inbox().threads();
    assert!(
        last.iter()
            .all(|thread| thread.label_ids.iter().any(|id| id.starts_with("Label_"))),
        "a label's list"
    );
}

#[test]
fn the_folders_button_opens_the_drawer_and_a_click_outside_closes_it() {
    let mut app = start("folders-button");
    app.click(egui::pos2(62.0, 84.0));
    app.key(Key::J);
    assert_eq!(
        app.cursor(),
        "Design crit moved to 3pm",
        "the drawer has the keys"
    );
    // The list under the drawer's layer doesn't get the click.
    app.click(egui::pos2(700.0, 400.0));
    assert!(app.mail().thread().is_none());
    app.key(Key::J);
    assert_ne!(
        app.cursor(),
        "Design crit moved to 3pm",
        "the list has them back"
    );
}

#[test]
fn hovering_a_label_chip_puts_the_cursor_on_its_row() {
    let mut app = start("hover-chip");
    // The `hiring` chip of Zara and Tom's debrief, at 1280×800.
    app.hover(egui::pos2(298.0, 473.0));
    assert_eq!(app.cursor(), "Debrief: Senior Product Designer (Aiko)");
}

#[test]
fn a_row_under_a_popped_out_draft_doesnt_take_the_pointer() {
    let mut app = start("hover-under-popout");
    app.press(Key::C, egui::Modifiers::SHIFT);
    let cursor = app.cursor();
    // Over the pop-out's body, which covers a row at 1280×800.
    app.hover(egui::pos2(800.0, 628.0));
    app.wait(0.3);
    assert_eq!(
        app.cursor(),
        cursor,
        "the card has the pointer, not the row under it"
    );
    // Off the card, the row under the pointer takes it.
    app.hover(egui::pos2(400.0, 628.0));
    app.wait(0.3);
    assert_ne!(app.cursor(), cursor);
}

#[test]
fn the_contact_pane_follows_a_row_while_the_pointer_keeps_moving_on_it() {
    let mut app = start("pane-follows");
    // Down and right from the first row onto Priya's 1:1 agenda (the third, at 1280×800), then
    // drifting right along it towards the contact pane without ever stopping, as a hand does:
    // heading for the right pane, the rows wait 100 ms before taking the cursor.
    let path = (0..16)
        .map(|step| egui::pos2(300.0 + 8.0 * step as f32, 150.0 + 4.0 * step as f32))
        .chain((0..30).map(|step| egui::pos2(428.0 + 2.0 * step as f32, 210.0)));
    for at in path {
        app.harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(at));
        app.harness.run_steps(1);
        // The account answers on its own threads.
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        app.cursor(),
        "1:1 agenda",
        "the row took the cursor 100 ms in"
    );
    let snapshot = app.mail().account().snapshot();
    assert_eq!(
        snapshot.contact.as_ref().map(|(email, _)| email.as_str()),
        Some("priya@brightmoor.co"),
        "and the pane followed 100 ms later"
    );
}

#[test]
fn the_contact_pane_lists_conversations_from_local_mail() {
    let app = start("contact");
    let snapshot = app.mail().account().snapshot();
    let (email, threads) = snapshot.contact.clone().expect("the cursor's contact");
    assert_eq!(email, "tom@brightmoor.co");
    assert!(threads.len() > 1, "{threads:?}");
    assert!(
        threads.windows(2).all(|pair| pair[0].date >= pair[1].date),
        "newest first"
    );
}

#[test]
fn command_c_copies_the_message_then_the_conversation() {
    let mut app = start("copy");
    exo::clipboard::capture(&app.harness.ctx);
    // Zara and Tom's debrief, three messages.
    for _ in 0..8 {
        app.key(Key::J);
    }
    app.key(Key::Enter);
    app.press(Key::C, COMMAND);
    let (html, text) = exo::clipboard::captured(&app.harness.ctx).expect("a copy");
    assert!(
        text.starts_with(
            "From: Maya Okafor <maya@brightmoor.co>\nTo:   Tom Becker <tom@brightmoor.co>\n"
        ),
        "{text}"
    );
    assert!(
        text.contains("\nSubject: Re: Debrief: Senior Product Designer (Aiko)\n\n"),
        "{text}"
    );
    assert!(!text.contains("------"), "one message");
    assert!(
        html.starts_with(
            "<div><div><strong>From:</strong> Maya Okafor &lt;maya@brightmoor.co&gt;</div>"
        ),
        "{html}"
    );

    app.press(Key::C, COMMAND);
    let (html, text) = exo::clipboard::captured(&app.harness.ctx).expect("a copy");
    assert_eq!(
        text.matches("\n\n------\n\n").count(),
        2,
        "the conversation's three messages"
    );
    assert_eq!(html.matches("<br><hr>").count(), 2);
}
