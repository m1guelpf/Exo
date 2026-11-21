//! Find in page (⌘F) end to end: the field's keys, what it finds on the page and in bodies, and
//! the find pasteboard.

mod support;

use std::sync::{Arc, Mutex};

use egui::{Key, Modifiers};
use support::{Fixture, start};

fn count(app: &Fixture) -> Option<(usize, usize)> {
    exo_ui::find::count(&app.harness.ctx)
}

fn find(app: &mut Fixture, text: &str) {
    app.press(Key::F, Modifiers::COMMAND);
    app.type_text(text);
}

#[test]
fn command_f_finds_the_pages_text_and_enter_steps_through_it() {
    let mut app = start("find-steps");
    // The list's subjects and snippets, and the contact pane's.
    find(&mut app, "the");
    let (current, total) = count(&app).expect("searched");
    assert!(total > 2, "{total}");
    assert_eq!(current, 1, "from the first match");
    app.key(Key::Enter);
    assert_eq!(count(&app), Some((2, total)));
    app.press(Key::Enter, Modifiers::SHIFT);
    app.press(Key::Enter, Modifiers::SHIFT);
    assert_eq!(
        count(&app),
        Some((total, total)),
        "back past the first wraps to the last"
    );
    app.press(Key::G, Modifiers::COMMAND);
    assert_eq!(count(&app), Some((1, total)), "⌘G steps on, wrapping");
    app.type_text("zzz");
    assert_eq!(count(&app), Some((0, 0)), "nothing matches");
}

#[test]
fn the_field_takes_the_keys_until_escape_closes_it() {
    let mut app = start("find-keys");
    let before = app.subjects();
    find(&mut app, "agenda");
    assert!(count(&app).is_some());
    // E would mark the conversation done; the field has the keyboard.
    app.key(Key::E);
    assert_eq!(app.subjects(), before, "the page got no key");
    app.key(Key::Escape);
    assert_eq!(count(&app), None, "closing stops the search");
    app.key(Key::E);
    assert_ne!(app.subjects(), before, "the page has the keyboard again");
}

#[test]
fn reopening_keeps_the_text_without_searching_until_asked() {
    let mut app = start("find-reopen");
    find(&mut app, "agenda");
    app.key(Key::Escape);
    app.press(Key::F, Modifiers::COMMAND);
    assert_eq!(count(&app), None, "closing the bar forgot the result");
    app.key(Key::Enter);
    assert!(
        count(&app).is_some_and(|(current, _)| current == 1),
        "{:?}",
        count(&app)
    );
}

#[test]
fn the_text_to_find_is_shared_through_the_find_pasteboard() {
    let mut app = start("find-pasteboard");
    let pasteboard = Arc::new(Mutex::new(String::new()));
    exo::find::share_find_text(
        &app.harness.ctx,
        exo::find::Pasteboard::Memory(pasteboard.clone()),
    );
    find(&mut app, "agenda");
    assert_eq!(
        *pasteboard.lock().expect("lock"),
        "agenda",
        "typing shares it"
    );
    // Another app finds something else; coming back adopts it, without searching.
    "crit".clone_into(&mut pasteboard.lock().expect("lock"));
    app.sync();
    assert_eq!(count(&app), None);
    app.press(Key::G, Modifiers::COMMAND);
    let (current, total) = count(&app).expect("⌘G searches for it");
    assert!(current == 1 && total >= 1, "{current}/{total}");
}

#[test]
fn a_match_in_a_body_out_of_view_scrolls_into_view() {
    let mut app = start("find-body");
    let paragraphs: String = (1..=80)
        .map(|line| format!("<p>Line {line} of the notes.</p>"))
        .collect();
    let html = format!("<div>{paragraphs}<p>The zebracorn is at the end.</p></div>");
    let deliver = app.mock_url().join("__mock/deliver").expect("a URL");
    let message = serde_json::json!({ "from": "Ann Lee <ann@example.com>", "subject": "Long notes", "html": html });
    app.block_on(async {
        reqwest::Client::new()
            .post(deliver)
            .json(&message)
            .send()
            .await?
            .error_for_status()
    })
    .expect("delivered");
    app.sync();
    while app.cursor() != "Long notes" {
        let before = app.cursor();
        app.key(Key::K);
        assert_ne!(app.cursor(), before, "the new conversation is at the top");
    }
    app.key(Key::Enter);
    assert_eq!(app.mail().thread().expect("open").scroll(), 0.0);
    find(&mut app, "zebracorn");
    app.settle();
    assert_eq!(count(&app), Some((1, 1)), "found in the body");
    let scroll = app.mail().thread().expect("open").scroll();
    assert!(scroll > 1000.0, "scrolled down to it: {scroll}");
}

#[test]
fn a_match_past_a_widened_columns_edge_scrolls_it_sideways() {
    let mut app = start("find-wide");
    // A 1200px-wide sale mail whose last column, past the column's right edge,
    // is a link.
    let html = "<table width=\"1200\" cellpadding=\"0\" cellspacing=\"0\"><tr>\
                <td width=\"1000\" style=\"height:80px\">Black Friday</td>\
                <td width=\"200\"><a href=\"https://example.com/deal\" style=\"display:block;height:80px\">Zebracorn deal</a></td>\
                </tr></table>";
    let message = serde_json::json!({ "from": "Design Shop <hello@designshop.example>", "subject": "Black Friday is here", "html": html });
    let deliver = app.mock_url().join("__mock/deliver").expect("a URL");
    app.block_on(async {
        reqwest::Client::new()
            .post(deliver)
            .json(&message)
            .send()
            .await?
            .error_for_status()
    })
    .expect("delivered");
    app.sync();
    while app.cursor() != "Black Friday is here" {
        app.key(Key::K);
    }
    app.key(Key::Enter);
    let at = egui::pos2(800.0, 200.0);
    assert_eq!(
        app.hover(at),
        egui::CursorIcon::Default,
        "the first cell's empty end"
    );
    find(&mut app, "zebracorn");
    app.settle();
    assert_eq!(count(&app), Some((1, 1)), "found in the body");
    assert_eq!(
        app.hover(at),
        egui::CursorIcon::PointingHand,
        "the column scrolled to the link"
    );
}
