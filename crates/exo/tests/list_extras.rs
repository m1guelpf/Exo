//! The list's smaller pieces, end to end on gmail-mock: Trash on UK keyboards, the Snippets
//! folder in the folder drawer, the contact pane's Mail header, the rows' right-click menu, and
//! the list header's keyboard.

mod support;

use std::sync::{Arc, Mutex};

use egui::{Event, Key, Modifiers, Pos2};
use exo::desktop::context_menu::MenuItem;
use exo::inbox::HeaderFocus;
use exo_store::ListId;
use support::{Fixture, start, start_new_account};

/// The middle of the list's rows at 1280×800.
fn row(index: usize) -> Pos2 {
    Pos2::new(400.0, 137.0 + 36.0 * index as f32)
}

/// The menus shown: each item's title and whether it was on.
type Menus = Arc<Mutex<Vec<Vec<(String, bool)>>>>;

/// Has the app's context menus choose the item titled `choose`, and returns what they offered.
fn choose_in_menus(app: &mut Fixture, choose: &'static str) -> Menus {
    let shown = Arc::new(Mutex::new(Vec::new()));
    let seen = shown.clone();
    app.mail_mut().set_context_menu(move |items, _| {
        let titles: Vec<(String, bool)> = items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Item { title, enabled, .. } => Some((title.clone(), *enabled)),
                MenuItem::Separator => None,
            })
            .collect();
        seen.lock().expect("not poisoned").push(titles);
        items
            .iter()
            .position(|item| matches!(item, MenuItem::Item { title, .. } if title == choose))
    });
    shown
}

/// Presses `key` with the text it types, as egui-winit reports a key on a keyboard whose layout
/// types `text` with it.
fn press_typing(app: &mut Fixture, key: Key, modifiers: Modifiers, text: &str) {
    let event = |pressed| Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    };
    app.harness
        .input_mut()
        .events
        .extend([event(true), Event::Text(text.to_owned())]);
    app.harness.run_steps(1);
    app.harness.input_mut().events.push(event(false));
    app.settle();
}

#[test]
fn the_key_that_types_hash_on_a_uk_keyboard_trashes() {
    let mut app = start("uk-trash");
    let first = app.cursor();
    // A US keyboard's backslash and ⌥3 (`£`) do nothing.
    press_typing(&mut app, Key::Backslash, Modifiers::NONE, "\\");
    press_typing(&mut app, Key::Num3, Modifiers::ALT, "£");
    app.wait(1.0);
    assert_eq!(app.cursor(), first);
    // A UK PC keyboard's `#` key, by Enter.
    press_typing(&mut app, Key::Backslash, Modifiers::NONE, "#");
    app.wait(1.0);
    assert!(!app.subjects().contains(&first), "{first} is in the Trash");
    assert_eq!(app.mail().toast_text(), Some("Moved to Trash."));
    // A UK Mac's ⌥3.
    let second = app.cursor();
    press_typing(&mut app, Key::Num3, Modifiers::ALT, "#");
    app.wait(1.0);
    assert!(
        !app.subjects().contains(&second),
        "{second} is in the Trash"
    );
}

#[test]
fn the_folder_drawer_goes_to_snippets_and_opens_on_it_there() {
    let mut app = start("drawer-snippets");
    app.key(Key::ArrowLeft);
    // Inbox, Starred, Drafts, Sent, Done, Scheduled, Reminders, Snippets.
    for _ in 0..7 {
        app.key(Key::ArrowDown);
    }
    app.key(Key::Enter);
    assert!(app.mail().snippets_folder_open());
    // Opened again, its cursor is on Snippets: Enter keeps the folder.
    app.key(Key::ArrowLeft);
    app.key(Key::Enter);
    assert!(app.mail().snippets_folder_open());
}

#[test]
fn the_contact_panes_mail_header_searches_their_address() {
    let mut app = start("pane-mail-header");
    // The first row's contact pane at 1280×800: its Mail header.
    app.click(Pos2::new(1100.0, 174.0));
    assert_eq!(
        app.mail().search_query().map(str::trim),
        Some("tom@brightmoor.co")
    );
}

#[test]
fn a_rows_right_click_menu_acts_on_it_or_on_the_selection_its_in() {
    let mut app = start("row-menu");
    let shown = choose_in_menus(&mut app, "Mark done");
    let subjects = app.subjects();
    // The second row, right-clicked: it takes the cursor, and Mark done takes it away.
    app.right_click(row(1));
    app.wait(1.0);
    let menus = shown.lock().expect("not poisoned").clone();
    let titles: Vec<&str> = menus[0].iter().map(|(title, _)| title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Mark done",
            "Remind me",
            "Star",
            "Label",
            "Reply",
            "Reply all",
            "Forward",
            "Mark read",
            "Trash"
        ],
        "for an unread conversation"
    );
    assert!(
        !app.subjects().contains(&subjects[1]),
        "{} is done",
        subjects[1]
    );
    assert!(app.subjects().contains(&subjects[0]));
    // Two selected rows: the menu, on one of them, acts on both.
    app.key(Key::Home);
    app.key(Key::X);
    app.key(Key::J);
    app.key(Key::X);
    let (first, second) = (app.subjects()[0].clone(), app.subjects()[1].clone());
    app.right_click(row(0));
    app.wait(1.0);
    let menus = shown.lock().expect("not poisoned").clone();
    assert!(
        menus[1]
            .iter()
            .any(|(title, enabled)| title == "Reply" && !enabled),
        "several can't be replied to"
    );
    assert!(!app.subjects().contains(&first) && !app.subjects().contains(&second));
}

#[test]
fn up_from_the_first_row_gives_the_header_the_keyboard() {
    let mut app = start_new_account("header-focus");
    // K stays in the list; ↑ leaves it for Important's tab.
    app.key(Key::K);
    assert!(!app.mail().inbox().header_focused());
    app.key(Key::ArrowUp);
    assert_eq!(app.mail().inbox().header_focus(), Some(HeaderFocus::Tab(0)));
    // → to Other, Enter shows it, and the list has the keyboard again.
    app.key(Key::ArrowRight);
    app.key(Key::Enter);
    assert_eq!(*app.mail().list(), ListId::Other);
    assert!(!app.mail().inbox().header_focused());
    // ↓ comes back down; ← past the folders button opens the drawer, which takes the keyboard.
    app.key(Key::ArrowUp);
    app.key(Key::ArrowDown);
    assert!(!app.mail().inbox().header_focused());
    app.key(Key::ArrowUp);
    app.key(Key::ArrowLeft);
    assert_eq!(
        app.mail().inbox().header_focus(),
        Some(HeaderFocus::Folders)
    );
    app.key(Key::ArrowLeft);
    assert!(app.mail().folders_open() && !app.mail().inbox().header_focused());
}

const CMD: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);

#[test]
fn command_c_in_the_list_copies_the_cursors_conversation_without_opening_it() {
    let mut app = start("list-copy");
    exo::clipboard::capture(&app.harness.ctx);
    // A conversation of one message copies as that message, which ⌘V makes a draft of.
    let first = app.mail().inbox().cursor().cloned().expect("a cursor");
    assert_eq!(first.messages, 1);
    app.press(Key::C, CMD);
    assert!(app.mail().thread().is_none(), "the list stays");
    let (html, text) = exo::clipboard::captured(&app.harness.ctx).expect("copied");
    assert!(
        text.starts_with("From: ") && text.contains(&format!("Subject: {}", first.subject)),
        "{text}"
    );
    assert!(html.contains("<strong>Subject:</strong>"));
    assert_eq!(
        app.mail().hint_text().as_deref(),
        Some("Message copied to clipboard. Hit ⌘ V to use as new draft.")
    );

    // One of several messages copies as the conversation.
    let at = app
        .mail()
        .inbox()
        .threads()
        .iter()
        .position(|thread| thread.messages > 1 && !thread.draft_only)
        .expect("a longer conversation");
    for _ in 0..at {
        app.key(Key::J);
    }
    app.press(Key::C, CMD);
    let (_, text) = exo::clipboard::captured(&app.harness.ctx).expect("copied");
    assert!(text.contains("\n\n------\n\n"), "{text}");
    assert_eq!(
        app.mail().hint_text().as_deref(),
        Some("Conversation copied to clipboard.")
    );

    // Not with several selected.
    app.key(Key::X);
    app.key(Key::J);
    app.key(Key::X);
    exo::clipboard::capture(&app.harness.ctx);
    app.press(Key::C, CMD);
    assert!(exo::clipboard::captured(&app.harness.ctx).is_none());
}
