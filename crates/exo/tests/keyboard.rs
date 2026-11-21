//! The app driven by keys (and the odd click), end to end: gmail-mock, a fresh store, the
//! commands, the toasts and their undo.

mod support;

use egui::{Event, Key, Modifiers, Pos2};
use exo_store::ListId;
use support::start;

#[test]
fn e_marks_done_and_z_undoes_it() {
    let mut app = start("done");
    assert_eq!(
        app.subjects().len(),
        24,
        "24: Gmail-web drafts only show in Drafts"
    );
    assert_eq!(app.cursor(), "Design crit moved to 3pm");
    app.key(Key::E);
    assert!(
        !app.subjects()
            .contains(&"Design crit moved to 3pm".to_owned()),
        "out of the inbox"
    );
    assert!(
        app.cursor().starts_with("Invitation: Coffee with Grace"),
        "the cursor moves down"
    );
    app.key(Key::Z);
    assert_eq!(app.subjects().len(), 24, "back in the inbox");
    assert_eq!(
        app.cursor(),
        "Design crit moved to 3pm",
        "the cursor goes back to it"
    );
}

#[test]
fn g_e_shows_done_and_escape_goes_back() {
    let mut app = start("go");
    app.key(Key::G);
    app.key(Key::E);
    let done = app.subjects();
    assert!(!done.is_empty());
    assert!(
        app.mail()
            .inbox()
            .threads()
            .iter()
            .all(|thread| thread.done)
    );
    app.key(Key::Escape);
    assert_eq!(app.subjects().len(), 24, "back to the inbox");
}

#[test]
fn s_stars_and_the_star_survives_a_trip_through_gmail() {
    let mut app = start("star");
    assert!(
        !app.mail()
            .inbox()
            .cursor()
            .is_some_and(|thread| thread.starred)
    );
    app.key(Key::S);
    assert!(
        app.mail()
            .inbox()
            .cursor()
            .is_some_and(|thread| thread.starred)
    );
    // `g s` lists it.
    app.key(Key::G);
    app.key(Key::S);
    assert!(
        app.subjects()
            .contains(&"Design crit moved to 3pm".to_owned())
    );
}

#[test]
fn unstarring_in_starred_keeps_the_rows_until_the_list_changes() {
    let mut app = start("unstar");
    app.key(Key::G);
    app.key(Key::S);
    let starred = app.subjects();
    assert_eq!(starred.len(), 3);
    // Every one of them, kept in the list: no row leaves, the cursor stays put, and the stars go.
    for (at, subject) in starred.iter().enumerate() {
        assert_eq!(&app.cursor(), subject);
        app.key(Key::S);
        assert_eq!(app.subjects(), starred, "{subject:?} stays");
        assert_eq!(&app.cursor(), subject, "the cursor doesn't move");
        assert!(!app.mail().inbox().threads()[at].starred);
        app.key(Key::J);
    }
    // Starred again, and unstarred, it still stays.
    app.key(Key::S);
    assert!(
        app.mail()
            .inbox()
            .cursor()
            .is_some_and(|thread| thread.starred)
    );
    app.key(Key::S);
    assert_eq!(app.subjects(), starred);
    // Marking it read or unread keeps it too; marking it done dismisses it.
    app.press(Key::U, Modifiers::NONE);
    assert_eq!(app.subjects(), starred);
    app.key(Key::E);
    assert_eq!(app.subjects(), starred[..2], "done, it goes");
    // Opened, a conversation starred and unstarred stays open.
    assert_eq!(app.cursor(), starred[1]);
    app.key(Key::Enter);
    app.key(Key::S);
    app.key(Key::S);
    assert!(app.mail().thread().is_some(), "no auto-advance");
    // Gone once the list changes.
    app.key(Key::Escape);
    app.key(Key::G);
    app.key(Key::I);
    app.key(Key::G);
    app.key(Key::S);
    assert!(app.subjects().is_empty(), "{:?}", app.subjects());
}

#[test]
fn searching_lets_go_of_the_unstarred_rows() {
    let mut app = start("unstar-search");
    app.key(Key::G);
    app.key(Key::S);
    let starred = app.subjects();
    app.key(Key::S);
    assert_eq!(app.subjects(), starred);
    // Opening search clears the list's sticky rows.
    app.key(Key::Slash);
    app.key(Key::Escape);
    assert_eq!(app.subjects(), starred[1..]);
}

#[test]
fn x_selects_and_shift_hash_trashes_the_selection() {
    let mut app = start("trash");
    app.key(Key::X);
    app.key(Key::J);
    app.key(Key::X);
    assert_eq!(app.mail().inbox().selection().len(), 2);
    app.press(Key::Num3, Modifiers::SHIFT);
    assert_eq!(app.subjects().len(), 22);
    assert!(app.mail().inbox().selection().is_empty());
}

#[test]
fn all_mail_backfills_as_the_list_reaches_its_end() {
    let mut app = start("more");
    app.key(Key::G);
    app.key(Key::A);
    let first = app.subjects().len();
    for _ in 0..5 {
        app.key(Key::End);
    }
    let later = app.subjects().len();
    assert!(later > first, "{first} rows, then {later}");
}

#[test]
fn the_done_button_of_a_done_row_marks_it_not_done() {
    let mut app = start("toggle");
    app.key(Key::G);
    app.key(Key::E);
    let first = app.cursor();
    // The cursor row's purple check, at 1280×800.
    app.click(Pos2::new(860.0, 137.0));
    assert!(
        !app.subjects().contains(&first),
        "{first:?} went back to the inbox"
    );
}

#[test]
fn going_to_done_and_straight_back_never_shows_inbox_zero() {
    let mut app = start("back");
    // `g e` then `g i` in one frame, before the account has caught up with either.
    for key in [Key::G, Key::E, Key::G, Key::I] {
        for pressed in [true, false] {
            app.harness.input_mut().events.push(Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Modifiers::NONE,
            });
        }
    }
    app.harness.run_steps(1);
    assert!(
        !app.mail().inbox().at_inbox_zero(),
        "the frame drew Inbox Zero"
    );
    app.settle();
    assert_eq!(app.subjects().len(), 24);
}

#[test]
fn shifted_g_sequences_go_to_their_lists_through_the_shift_key() {
    // FastFrame's egui reports Shift as a key of its own, pressed between `g` and `!`; it used
    // to end the sequence, so `!` opened Mark Spam, `#` trashed and `⇧M` muted.
    for (key, list) in [
        (Key::Exclamationmark, ListId::Spam),
        (Key::Num3, ListId::Trash),
        (Key::M, ListId::Muted),
    ] {
        let mut app = start(&format!("g-shift-{key:?}"));
        let inbox = app.subjects();
        app.key(Key::G);
        app.press(Key::ShiftLeft, Modifiers::SHIFT);
        app.press(key, Modifiers::SHIFT);
        assert_eq!(*app.mail().list(), list, "g then shift+{key:?}");
        app.key(Key::Escape);
        assert_eq!(
            app.subjects(),
            inbox,
            "g then shift+{key:?} left the inbox alone"
        );
    }
}
