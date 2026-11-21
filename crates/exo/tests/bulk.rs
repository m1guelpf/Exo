//! Select-all and bulk actions (⌘⇧A, ⌘A, then triage), end to end on gmail-mock.

mod support;

use egui::{Key, Modifiers};
use gmail::ListQuery;
use support::{Fixture, start};

/// ⌘ as the Mac sends it.
const CMD: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);

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

/// Done, with everything selected: more conversations than are loaded.
fn select_all_of_done(name: &str) -> Fixture {
    let mut app = start(name);
    app.key(Key::G);
    app.key(Key::E);
    app.press(Key::A, CMD.plus(Modifiers::SHIFT));
    assert!(app.mail().inbox().is_selecting());
    assert!(
        !app.mail().inbox().is_exact(),
        "Done has more than the rows loaded"
    );
    assert!(app.mail().inbox().count() < count(&app, "-in:inbox"));
    app
}

#[test]
fn a_select_all_reaches_conversations_not_loaded_and_z_puts_them_back() {
    let mut app = select_all_of_done("bulk-star");
    let loaded = app.mail().inbox().count();
    assert_eq!(count(&app, "-in:inbox is:starred"), 0);
    app.key(Key::S);
    assert_eq!(count(&app, "-in:inbox is:starred"), 0, "it asks first");
    app.key(Key::Enter);
    assert!(!app.mail().inbox().is_selecting(), "the selection clears");
    let starred = count(&app, "-in:inbox is:starred");
    assert!(
        starred > loaded,
        "{starred} starred, past the {loaded} rows loaded"
    );
    app.key(Key::Z);
    assert_eq!(count(&app, "-in:inbox is:starred"), 0);
}

#[test]
fn cancelling_the_alert_changes_nothing() {
    let mut app = select_all_of_done("bulk-cancel");
    app.key(Key::S);
    app.key(Key::Escape);
    assert_eq!(count(&app, "-in:inbox is:starred"), 0);
    assert!(app.mail().inbox().is_selecting(), "the selection stays");
}

#[test]
fn removing_every_label_needs_the_rows_loaded() {
    let mut app = select_all_of_done("bulk-remove-all");
    let labelled = count(&app, "-in:inbox has:userlabels");
    app.press(Key::Y, Modifiers::SHIFT);
    assert_eq!(count(&app, "-in:inbox has:userlabels"), labelled);
    assert!(app.mail().inbox().is_selecting());
}

#[test]
fn select_all_from_here_leaves_the_rows_above() {
    let mut app = start("bulk-from-here");
    let inbox = count(&app, "in:inbox");
    app.key(Key::J);
    app.key(Key::J);
    app.press(Key::A, CMD);
    // Every row is loaded, so this is acted on row by row.
    assert!(app.mail().inbox().is_exact());
    app.key(Key::E);
    assert_eq!(count(&app, "in:inbox"), 2, "the two rows above stay");
    app.key(Key::Z);
    assert_eq!(count(&app, "in:inbox"), inbox);
}

/// Runs `command` from the palette.
fn palette(app: &mut Fixture, command: &str) {
    app.press(Key::K, CMD);
    app.type_text(command);
    app.key(Key::Enter);
}

#[test]
fn get_me_to_zero_marks_old_mail_done_keeping_unread_and_undo_brings_it_back() {
    let mut app = start("get-me-to-zero");
    let inbox = count(&app, "in:inbox");
    let unread = count(&app, "in:inbox is:unread");
    palette(&mut app, "get me to zero");
    // The age list, up to `1 day`.
    app.key(Key::Enter);
    app.key(Key::ArrowUp);
    app.key(Key::ArrowUp);
    app.key(Key::Enter);
    // `More options…`, then `Keep unread emails`.
    app.key(Key::Tab);
    app.key(Key::Enter);
    app.key(Key::Enter);
    // `Let's go!`, then the alert's `Got it`.
    for _ in 0..3 {
        app.key(Key::Tab);
    }
    app.key(Key::Enter);
    assert_eq!(count(&app, "in:inbox"), inbox, "nothing until Got it");
    app.key(Key::Enter);
    let left = count(&app, "in:inbox");
    assert!(left < inbox, "old mail is done");
    assert_eq!(
        count(&app, "in:inbox is:unread"),
        unread,
        "unread mail stays"
    );
    assert!(count(&app, "in:inbox newer_than:1d") <= left);
    app.key(Key::Z);
    assert_eq!(count(&app, "in:inbox"), inbox);
}

#[test]
fn bulk_actions_undoes_get_me_to_zero() {
    let mut app = start("bulk-actions");
    let inbox = count(&app, "in:inbox");
    palette(&mut app, "get me to zero");
    // `More options…`, `Do this later`, `Let's go!`.
    for _ in 0..3 {
        app.key(Key::Tab);
    }
    app.key(Key::Enter);
    app.key(Key::Enter);
    assert!(count(&app, "in:inbox") < inbox);
    palette(&mut app, "bulk actions");
    // The row's Undo, then `Undo action`'s Undo.
    app.key(Key::Enter);
    app.key(Key::Enter);
    assert_eq!(count(&app, "in:inbox"), inbox);
}

#[test]
fn get_me_to_zero_dates_its_cutoff_from_now_not_from_when_exo_opened() {
    let mut app = start("get-me-to-zero-now");
    // A month after Exo opened, a week ago is after every conversation in the inbox.
    let later = gmail_mock::Config::default()
        .now
        .checked_add(jiff::SignedDuration::from_hours(30 * 24))
        .expect("in range");
    app.set_now(later);
    palette(&mut app, "get me to zero");
    // `More options…`, `Do this later`, `Let's go!`, then the alert's `Got it`.
    for _ in 0..3 {
        app.key(Key::Tab);
    }
    app.key(Key::Enter);
    app.key(Key::Enter);
    assert_eq!(
        count(&app, "in:inbox"),
        0,
        "everything is over a week old by now"
    );
}
