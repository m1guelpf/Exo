//! The Move and Label pickers and the label keys (`V`, `L`, `Y`, `[`, `]`, `⇧Y`), end to end
//! on gmail-mock.

mod support;

use egui::{Key, Modifiers};
use support::{Fixture, start};

const FIRST: &str = "Design crit moved to 3pm";
/// The ninth row, labelled Hiring.
const HIRING: &str = "Debrief: Senior Product Designer (Aiko)";

/// The chips on the inbox row with `subject`.
fn chips(app: &Fixture, subject: &str) -> Vec<String> {
    app.mail()
        .inbox()
        .threads()
        .iter()
        .find(|thread| thread.subject == subject)
        .map(|thread| thread.labels.clone())
        .unwrap_or_default()
}

/// The names of the account's labels, as Gmail has them.
fn gmail_labels(app: &Fixture) -> Vec<String> {
    let gmail = app.gmail();
    app.block_on(async { gmail.gmail().labels().await })
        .expect("lists labels")
        .labels
        .into_iter()
        .filter_map(|label| label.name)
        .collect()
}

#[test]
fn moving_to_a_label_takes_the_thread_out_of_the_inbox_and_z_brings_it_back() {
    let mut app = start("move");
    app.key(Key::V);
    app.type_text("clients");
    app.key(Key::Enter);
    assert!(
        !app.subjects().contains(&FIRST.to_owned()),
        "moved out of the inbox"
    );
    assert!(
        app.cursor().starts_with("Invitation: Coffee with Grace"),
        "the cursor moves down"
    );
    app.key(Key::Z);
    assert!(
        app.subjects().contains(&FIRST.to_owned()),
        "back in the inbox"
    );
    assert_eq!(app.cursor(), FIRST);
}

#[test]
fn a_new_label_is_created_only_once_confirmed() {
    let mut app = start("create");
    app.key(Key::L);
    app.type_text("Errands");
    // The footer is the only row.
    app.key(Key::Enter);
    app.key(Key::Escape);
    assert!(
        !gmail_labels(&app).contains(&"Errands".to_owned()),
        "Cancel creates nothing"
    );

    app.key(Key::L);
    app.type_text("Errands");
    app.key(Key::Enter);
    app.key(Key::Enter);
    assert!(gmail_labels(&app).contains(&"Errands".to_owned()));
    assert!(
        chips(&app, FIRST).contains(&"Errands".to_owned()),
        "and the thread has it"
    );
}

#[test]
fn y_marks_done_in_the_inbox_and_close_bracket_goes_up() {
    let mut app = start("y");
    app.key(Key::J);
    app.key(Key::J);
    let third = app.cursor();
    app.key(Key::CloseBracket);
    assert!(!app.subjects().contains(&third), "out of the inbox");
    assert!(
        app.cursor().starts_with("Invitation: Coffee with Grace"),
        "the cursor goes up"
    );
    app.key(Key::Y);
    assert!(
        !app.subjects()
            .iter()
            .any(|subject| subject.starts_with("Invitation: Coffee with Grace"))
    );
}

#[test]
fn shift_y_removes_every_label_and_marks_done() {
    let mut app = start("remove-all");
    assert!(chips(&app, HIRING).contains(&"Hiring".to_owned()));
    while app.cursor() != HIRING {
        app.key(Key::J);
    }
    app.press(Key::Y, Modifiers::SHIFT);
    assert!(!app.subjects().contains(&HIRING.to_owned()), "marked done");
    app.key(Key::Z);
    assert!(
        chips(&app, HIRING).contains(&"Hiring".to_owned()),
        "undo puts the label back"
    );
}

#[test]
fn moving_the_open_thread_opens_the_next_conversation() {
    let mut app = start("move-open");
    app.key(Key::Enter);
    let first = app
        .mail()
        .thread()
        .map(|thread| thread.id().to_owned())
        .expect("a thread is open");
    app.key(Key::V);
    app.type_text("projects");
    app.key(Key::Enter);
    let open = app
        .mail()
        .thread()
        .map(|thread| thread.id().to_owned())
        .expect("the next one opens");
    assert_ne!(open, first);
    assert!(!app.subjects().contains(&FIRST.to_owned()));
}

#[test]
fn moving_a_search_result_keeps_it_listed() {
    let mut app = start("move-search");
    app.key(Key::Slash);
    app.type_text("potato");
    app.key(Key::Enter);
    assert_eq!(app.subjects(), ["Thanksgiving 🦃"]);
    app.key(Key::V);
    app.type_text("receipts");
    app.key(Key::Enter);
    assert_eq!(
        app.subjects(),
        ["Thanksgiving 🦃"],
        "results don't slide out"
    );
    assert!(chips(&app, "Thanksgiving 🦃").contains(&"Receipts".to_owned()));
}

/// The mock's filters, as `(from, added labels)`.
fn gmail_filters(app: &Fixture) -> Vec<(String, Vec<String>)> {
    let gmail = app.gmail();
    let list = app
        .block_on(async { gmail.gmail().filters().await })
        .expect("lists filters")
        .filter;
    list.into_iter()
        .map(|filter| {
            (
                filter
                    .criteria
                    .and_then(|criteria| criteria.from)
                    .unwrap_or_default(),
                filter
                    .action
                    .map(|action| action.add_label_ids)
                    .unwrap_or_default(),
            )
        })
        .collect()
}

#[test]
fn blocking_from_the_spam_picker_trashes_the_sender_until_undone() {
    const SENDER: &str = "updates@parcelpoint.com";
    let mut app = start("block");
    for _ in 0..3 {
        app.key(Key::J);
    }
    let parcel = app.cursor();
    app.press(Key::Num1, Modifiers::SHIFT);
    // `Mark Spam, and block updates@parcelpoint.com`, then the alert's Block.
    app.key(Key::ArrowDown);
    app.key(Key::Enter);
    assert!(
        gmail_filters(&app).iter().all(|(from, _)| from != SENDER),
        "nothing until confirmed"
    );
    app.key(Key::Enter);
    assert!(!app.subjects().contains(&parcel), "trashed");
    assert!(gmail_filters(&app).contains(&(SENDER.to_owned(), vec!["TRASH".to_owned()])));
    assert_eq!(app.mail().account().settings().blocked_senders, [SENDER]);

    app.key(Key::Z);
    assert!(app.subjects().contains(&parcel), "back in the inbox");
    assert!(
        gmail_filters(&app).iter().all(|(from, _)| from != SENDER),
        "the filter is gone"
    );
    assert!(app.mail().account().settings().blocked_senders.is_empty());
}
