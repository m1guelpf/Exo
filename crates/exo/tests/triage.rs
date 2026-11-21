//! Discarding drafts from the list (⌘⇧,) and unblocking senders (Blocked Senders), end to end
//! on gmail-mock.

mod support;

use egui::{Key, Modifiers};
use gmail::{DraftQuery, Format};
use support::{Fixture, start};

/// ⌘ as the Mac sends it.
const CMD: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);

/// Gmail's drafts' subjects.
fn gmail_drafts(app: &Fixture) -> Vec<String> {
    let gmail = app.gmail();
    app.block_on(async {
        let mut subjects = Vec::new();
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
            subjects.push(
                full.message
                    .and_then(|message| message.header("Subject").map(str::to_owned))
                    .unwrap_or_default(),
            );
        }
        subjects
    })
}

/// Writes a reply in the cursor's conversation and goes back to the list; returns its subject.
fn reply_draft(app: &mut Fixture) -> String {
    let subject = format!("Re: {}", app.cursor());
    app.key(Key::Enter);
    app.key(Key::R);
    app.type_text("Sounds good");
    app.wait(1.5);
    app.key(Key::Escape);
    app.key(Key::Escape);
    subject
}

/// The drafts the cursor's row shows.
fn row_drafts(app: &Fixture) -> usize {
    app.mail()
        .inbox()
        .cursor()
        .map_or(0, |thread| thread.drafts)
}

#[test]
fn command_shift_comma_discards_the_rows_draft_and_z_reopens_it() {
    let mut app = start("discard-listed");
    let subject = reply_draft(&mut app);
    assert!(gmail_drafts(&app).contains(&subject));
    assert_eq!(row_drafts(&app), 1);
    app.press(Key::Comma, CMD.plus(Modifiers::SHIFT));
    app.wait(1.5);
    assert!(!gmail_drafts(&app).contains(&subject), "gone from Gmail");
    assert_eq!(
        row_drafts(&app),
        0,
        "the row isn't a draft any more, before a sync too"
    );
    app.key(Key::Z);
    assert!(
        app.mail().compose_focus().is_some(),
        "the draft opens again"
    );
    app.wait(1.5);
    assert!(gmail_drafts(&app).contains(&subject), "and is saved again");
}

#[test]
fn command_shift_comma_in_a_thread_discards_its_draft_when_none_has_the_keyboard() {
    let mut app = start("discard-in-thread");
    let subject = reply_draft(&mut app);
    // Back in the thread, the messages have the keyboard, not the draft.
    app.key(Key::Enter);
    assert!(app.mail().compose_focus().is_none());
    app.press(Key::Comma, CMD.plus(Modifiers::SHIFT));
    app.wait(1.5);
    assert_eq!(app.mail().toast_text(), Some("Draft discarded."));
    assert!(!gmail_drafts(&app).contains(&subject), "gone from Gmail");
}

#[test]
fn several_selected_drafts_go_together_and_come_back_together() {
    let mut app = start("discard-several");
    let first = reply_draft(&mut app);
    app.key(Key::J);
    let second = reply_draft(&mut app);
    app.key(Key::K);
    app.key(Key::X);
    app.key(Key::J);
    app.key(Key::X);
    app.press(Key::Comma, CMD.plus(Modifiers::SHIFT));
    app.wait(1.5);
    let drafts = gmail_drafts(&app);
    assert!(!drafts.contains(&first) && !drafts.contains(&second));
    app.key(Key::Z);
    app.wait(1.5);
    let drafts = gmail_drafts(&app);
    assert!(
        drafts.contains(&first) && drafts.contains(&second),
        "{drafts:?}"
    );
}

/// The mock's filters' senders.
fn filtered(app: &Fixture) -> Vec<String> {
    let gmail = app.gmail();
    let filters = app
        .block_on(async { gmail.gmail().filters().await })
        .expect("filters.list")
        .filter;
    filters
        .into_iter()
        .filter_map(|filter| filter.criteria.and_then(|criteria| criteria.from))
        .collect()
}

#[test]
fn unblocking_takes_the_sender_off_the_list_and_deletes_its_filter() {
    let mut app = start("unblock");
    app.press(Key::K, CMD);
    app.type_text("block sender");
    app.key(Key::Enter);
    // `Block sender`'s Block.
    app.key(Key::Enter);
    let sender = "tom@brightmoor.co".to_owned();
    assert_eq!(
        app.mail().account().settings().blocked_senders,
        std::slice::from_ref(&sender)
    );
    assert!(filtered(&app).contains(&sender));

    app.press(Key::K, CMD);
    app.type_text("blocked senders");
    app.key(Key::Enter);
    assert!(app.mail().settings_dialog_open());
    // The row's Unblock, then the alert's.
    app.key(Key::Enter);
    app.key(Key::Enter);
    assert!(app.mail().account().settings().blocked_senders.is_empty());
    assert!(!filtered(&app).contains(&sender));
}
