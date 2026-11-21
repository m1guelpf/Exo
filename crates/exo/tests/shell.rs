//! The shell's own keys end to end: zoom, and back and forward.

mod support;

use egui::{Key, Modifiers};
use exo_store::ListId;
use support::start;

const COMMAND: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);

#[test]
fn command_equals_zooms_by_chromiums_steps_and_command_zero_goes_back() {
    let mut app = start("zoom");
    app.press(Key::Equals, COMMAND);
    assert_eq!(app.harness.ctx.zoom_factor(), 1.2);
    app.press(Key::Equals, COMMAND);
    assert!((app.harness.ctx.zoom_factor() - 1.44).abs() < 1e-5);
    // ⌘⇧= isn't one of the shell's keys, nor is egui's own zoom on.
    app.press(Key::Equals, COMMAND | Modifiers::SHIFT);
    assert!((app.harness.ctx.zoom_factor() - 1.44).abs() < 1e-5);
    app.press(Key::Num0, COMMAND);
    assert_eq!(app.harness.ctx.zoom_factor(), 1.0);
    app.press(Key::Minus, COMMAND);
    assert!((app.harness.ctx.zoom_factor() - 1.0 / 1.2).abs() < 1e-5);
}

#[test]
fn back_and_forward_walk_lists_and_conversations() {
    let mut app = start("history");
    let home = app.mail().list().clone();
    app.key(Key::Enter);
    let opened = app
        .mail()
        .thread()
        .map(|thread| thread.id().to_owned())
        .expect("a conversation opens");
    // Escape goes back to where it was opened from, so forward opens it again.
    app.key(Key::Escape);
    assert!(app.mail().thread().is_none());
    app.press(Key::CloseBracket, COMMAND);
    assert_eq!(
        app.mail().thread().map(|thread| thread.id().to_owned()),
        Some(opened)
    );
    app.press(Key::OpenBracket, COMMAND);
    assert!(app.mail().thread().is_none(), "back to the list");
    // Lists too: ⌘← is ⌘[.
    app.key(Key::G);
    app.key(Key::T);
    assert_eq!(*app.mail().list(), ListId::Sent);
    app.press(Key::ArrowLeft, COMMAND);
    assert_eq!(*app.mail().list(), home);
    app.press(Key::ArrowRight, COMMAND);
    assert_eq!(*app.mail().list(), ListId::Sent);
}

#[test]
fn back_does_nothing_while_typing() {
    let mut app = start("history-typing");
    app.key(Key::G);
    app.key(Key::T);
    app.key(Key::C);
    // ⌘[ in a new message's body is the editor's (Decrease Indent), not back.
    app.key(Key::Tab);
    app.key(Key::Tab);
    app.press(Key::OpenBracket, COMMAND);
    assert_eq!(*app.mail().list(), ListId::Sent);
}
