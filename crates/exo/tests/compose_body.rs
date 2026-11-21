//! The compose body's keys and helpers on gmail-mock: moving on to the quote's `…`, the link
//! popover, pasting.

mod support;

use egui::{Key, Modifiers};
use support::start;

#[test]
fn down_at_the_end_of_a_reply_moves_to_the_ellipsis_and_enter_opens_it() {
    let mut app = start("body-ellipsis");
    app.key(Key::Enter);
    app.key(Key::R);
    // Not a greeting, which autocomplete would finish with Tom's name.
    app.type_text("Sounds good");
    assert!(
        !app.mail()
            .compose_body()
            .expect("a draft")
            .contains("gmail_quote")
    );
    app.key(Key::ArrowDown);
    app.key(Key::Enter);
    let body = app.mail().compose_body().expect("a draft");
    assert!(
        body.find("Sounds good")
            .is_some_and(|at| body.find("gmail_quote").is_some_and(|quote| at < quote)),
        "Enter on the `…` revealed the quote: {body}"
    );
    // Back in the body, ⌘↑ then ↓ is an ordinary move.
    app.press(Key::ArrowUp, Modifiers::MAC_CMD.plus(Modifiers::COMMAND));
    app.key(Key::ArrowDown);
    assert!(app.mail().compose_body().is_some(), "still in the draft");
}
