//! The window's tabs end to end, on one account: each keeps
//! its own page, the shell's keys and the strip's buttons, ⌘⇧T, the 20-tab limit and coming back
//! after a relaunch, and their links (`exo://…`): copied from the strip and with ⌃/, and opened.
//! Tabs on two accounts are in `sign_in.rs`.

mod support;

use egui::{Key, Modifiers, pos2};
use exo::desktop::{Desktop, Launcher, Opening, Placement};
use exo_store::{Filter, ListId};
use support::{Fixture, start};

const COMMAND: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);

fn tabs(app: &Fixture) -> (usize, usize) {
    (
        app.harness.state().tab_count(),
        app.harness.state().active_tab(),
    )
}

fn open_thread(app: &Fixture) -> Option<String> {
    app.mail().thread().map(|thread| thread.id().to_owned())
}

/// Sent, with its first conversation open; returns the conversation.
fn sent_conversation(app: &mut Fixture) -> String {
    app.key(Key::G);
    app.key(Key::T);
    assert_eq!(*app.mail().list(), ListId::Sent);
    app.key(Key::Enter);
    open_thread(app).expect("a conversation opens")
}

#[test]
fn a_new_tab_opens_home_and_each_tab_keeps_its_route() {
    let mut app = start("tabs-routes");
    let home = app.mail().list().clone();
    let opened = sent_conversation(&mut app);
    app.press(Key::T, COMMAND);
    assert_eq!(tabs(&app), (2, 1), "last, and on screen");
    assert_eq!((app.mail().list(), open_thread(&app)), (&home, None));
    let titles = app.harness.state().tab_titles();
    assert_eq!(
        titles[0], "Sent - Exo",
        "the tab left behind keeps its title"
    );
    assert!(titles[1].starts_with("Inbox"), "{titles:?}");

    app.press(Key::Num1, COMMAND);
    assert_eq!(tabs(&app), (2, 0));
    assert_eq!(
        (app.mail().list(), open_thread(&app)),
        (&ListId::Sent, Some(opened))
    );
    app.press(Key::Num2, COMMAND);
    assert_eq!((app.mail().list(), open_thread(&app)), (&home, None));
    app.press(Key::Num9, COMMAND);
    assert_eq!(tabs(&app), (2, 1), "there's no ninth tab");
}

#[test]
fn each_tab_keeps_its_own_page() {
    let mut app = start("tabs-pages");
    let opened = sent_conversation(&mut app);
    app.key(Key::R);
    assert_eq!(app.mail().thread_drafts(), 1, "a reply");
    app.press(Key::T, COMMAND);
    assert_eq!((open_thread(&app), app.mail().thread_drafts()), (None, 0));
    app.key(Key::X);
    assert!(app.mail().inbox().is_selecting());
    app.press(Key::Num1, COMMAND);
    assert_eq!(
        (open_thread(&app), app.mail().thread_drafts()),
        (Some(opened), 1),
        "the reply stays open in its tab"
    );
    assert!(
        !app.mail().inbox().is_selecting(),
        "the other tab's selection is its own"
    );
    app.press(Key::Num2, COMMAND);
    assert!(app.mail().inbox().is_selecting(), "and stays there");
}

#[test]
fn a_setting_changed_in_one_tab_reaches_the_others() {
    let mut app = start("tabs-settings");
    let before = app.mail().account().settings().show_sender_full_names;
    app.press(Key::T, COMMAND);
    app.mail_mut()
        .account_mut()
        .update_settings(|settings| settings.show_sender_full_names = !before);
    app.press(Key::Num1, COMMAND);
    assert_eq!(
        app.mail().account().settings().show_sender_full_names,
        !before
    );
    app.mail_mut()
        .account_mut()
        .update_settings(|settings| settings.show_sender_full_names = before);
    app.press(Key::Num2, COMMAND);
    assert_eq!(
        app.mail().account().settings().show_sender_full_names,
        before,
        "and back"
    );
}

#[test]
fn a_search_comes_back_with_its_tab() {
    let mut app = start("tabs-search");
    app.key(Key::Slash);
    app.type_text("potato");
    app.key(Key::Enter);
    assert_eq!(app.mail().search_query(), Some("potato"));
    app.press(Key::T, COMMAND);
    assert!(!app.mail().is_searching(), "the new tab is at home");
    assert_eq!(app.harness.state().tab_titles()[0], "Search: potato - Exo");
    app.press(Key::Num1, COMMAND);
    assert_eq!(app.mail().search_query(), Some("potato"));
    assert_eq!(app.subjects(), ["Thanksgiving 🦃"]);
}

#[test]
fn next_and_previous_wrap_round() {
    let mut app = start("tabs-cycle");
    app.press(Key::T, COMMAND);
    app.press(Key::T, COMMAND);
    assert_eq!(tabs(&app), (3, 2));
    app.press(Key::Tab, Modifiers::CTRL);
    assert_eq!(tabs(&app), (3, 0), "⌃Tab past the last");
    app.press(Key::Tab, Modifiers::CTRL | Modifiers::SHIFT);
    assert_eq!(tabs(&app), (3, 2), "⌃⇧Tab before the first");
    app.press(Key::ArrowLeft, COMMAND | Modifiers::ALT);
    assert_eq!(tabs(&app), (3, 1));
    app.press(Key::CloseBracket, COMMAND | Modifiers::SHIFT);
    assert_eq!(tabs(&app), (3, 2), "Window ▸ Next Tab");
}

#[test]
fn closing_a_tab_shows_its_neighbour_and_command_shift_t_brings_it_back() {
    let mut app = start("tabs-close");
    app.press(Key::T, COMMAND);
    let opened = sent_conversation(&mut app);
    app.press(Key::W, COMMAND);
    assert_eq!(
        tabs(&app),
        (1, 0),
        "its left neighbour, with none on its right"
    );
    assert_ne!(*app.mail().list(), ListId::Sent);
    app.press(Key::T, COMMAND | Modifiers::SHIFT);
    assert_eq!(tabs(&app), (2, 1));
    assert_eq!(
        (app.mail().list(), open_thread(&app)),
        (&ListId::Sent, Some(opened))
    );
    app.press(Key::W, COMMAND);
    app.press(Key::T, COMMAND | Modifiers::SHIFT);
    app.press(Key::T, COMMAND | Modifiers::SHIFT);
    assert_eq!(tabs(&app).0, 2, "only the last one closed comes back, once");
}

#[test]
fn the_strip_opens_switches_and_closes_tabs() {
    let mut app = start("tabs-strip");
    // `+` sits right of the pills, 8px in from the strip's 75px margin.
    app.click(pos2(1280.0 - 75.0 - 8.0 - 12.0, 20.0));
    assert_eq!(tabs(&app), (2, 1));
    app.key(Key::G);
    app.key(Key::T);
    // Two pills share the 1130px between the margins: each 541px after its 8px margin.
    app.click(pos2(83.0 + 270.0, 20.0));
    assert_eq!(tabs(&app), (2, 0));
    assert_ne!(*app.mail().list(), ListId::Sent);
    // The second pill's ×, 4px in from its right edge, shows while it's hovered.
    app.click(pos2(632.0 + 541.0 - 4.0 - 8.0, 20.0));
    assert_eq!(tabs(&app), (1, 0), "closed, and the tab on screen stays");
    app.press(Key::T, COMMAND | Modifiers::SHIFT);
    assert_eq!(*app.mail().list(), ListId::Sent);
}

#[test]
fn a_window_holds_twenty_tabs() {
    let mut app = start("tabs-limit");
    for _ in 0..25 {
        app.press(Key::T, COMMAND);
    }
    assert_eq!(tabs(&app), (20, 19));
}

#[test]
fn tabs_come_back_after_a_relaunch() {
    let dir = std::env::temp_dir().join(format!("exo-tabs-state-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a folder");
    let keep = |app: &mut Fixture| {
        let ctx = app.harness.ctx.clone();
        app.harness.state_mut().keep_tabs(&ctx, &dir);
        app.settle();
    };
    let mut app = start("tabs-relaunch");
    keep(&mut app);
    let opened = sent_conversation(&mut app);
    app.press(Key::T, COMMAND);
    app.press(Key::T, COMMAND);
    app.press(Key::Num1, COMMAND);
    app.relaunch();
    keep(&mut app);
    assert_eq!(tabs(&app), (3, 0));
    assert_eq!(
        (app.mail().list(), open_thread(&app)),
        (&ListId::Sent, Some(opened))
    );
    app.press(Key::Num3, COMMAND);
    assert!(open_thread(&app).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Back to the list from a conversation (`g u`).
fn leave_conversation(app: &mut Fixture) {
    app.key(Key::G);
    app.key(Key::U);
    assert!(open_thread(app).is_none());
}

/// What the last copy put on the clipboard, as text; its HTML is the same.
fn copied(app: &Fixture) -> Option<String> {
    let (html, text) = exo::clipboard::captured(&app.harness.ctx)?;
    assert_eq!(html, text, "`text/html` and `text/plain`");
    Some(text)
}

#[test]
fn the_strips_copy_button_copies_its_tabs_link() {
    let mut app = start("tabs-copy-button");
    exo::clipboard::capture(&app.harness.ctx);
    let opened = sent_conversation(&mut app);
    app.press(Key::T, COMMAND);
    // The first pill's copy button, left of its ×: 31px in from its right edge at 624.
    app.click(pos2(624.0 - 31.0, 20.0));
    assert_eq!(
        copied(&app),
        Some(format!(
            "exo://maya@brightmoor.co/label/sent/thread/{opened}"
        )),
        "the tab clicked, not the one on screen"
    );
    assert_eq!(tabs(&app), (2, 1));
    assert_eq!(
        app.mail().hint_text().as_deref(),
        Some("Link copied to clipboard. Hit ctralt / to do this instantly!"),
        "on the tab on screen"
    );
    app.click(pos2(1173.0 - 31.0, 20.0));
    assert_eq!(
        copied(&app).as_deref(),
        Some("exo://maya@brightmoor.co"),
        "the tab on screen, at home"
    );
}

#[test]
fn control_slash_copies_where_the_tab_is() {
    let mut app = start("tabs-copy-key");
    exo::clipboard::capture(&app.harness.ctx);
    let opened = sent_conversation(&mut app);
    app.press(Key::Slash, Modifiers::CTRL);
    let link = format!("exo://maya@brightmoor.co/label/sent/thread/{opened}");
    assert_eq!(copied(&app).as_ref(), Some(&link));
    assert_eq!(
        app.mail().hint_text(),
        Some(format!("\"{link}\" copied to clipboard."))
    );
    leave_conversation(&mut app);
    app.press(Key::U, Modifiers::SHIFT);
    assert_eq!(app.mail().filter(), Some(Filter::Unread));
    app.press(Key::Slash, Modifiers::CTRL);
    assert_eq!(
        copied(&app).as_deref(),
        Some("exo://maya@brightmoor.co/label/sent?filters=unread"),
        "the list's filter"
    );
    // The palette's row for the conversation under the cursor.
    app.press(Key::K, COMMAND);
    app.type_text("Copy Private Link to Conversation");
    app.key(Key::Enter);
    assert!(
        copied(&app).is_some_and(|link| link.starts_with("exo://maya@brightmoor.co/thread/")),
        "{:?}",
        copied(&app)
    );
}

#[test]
fn a_link_opens_in_its_accounts_tab() {
    let mut app = start("tabs-open-link");
    let launcher = Launcher::default();
    let desktop = Desktop::new(
        &app.harness.ctx,
        launcher.clone(),
        Placement::load(&app.folder("window")),
        Opening::Shown,
    );
    app.harness.state_mut().set_desktop(desktop);
    let opened = sent_conversation(&mut app);
    leave_conversation(&mut app);
    app.press(Key::T, COMMAND);
    launcher.open_url(&format!(
        "exo://maya@brightmoor.co/label/sent/thread/{opened}?filters=unread"
    ));
    app.settle();
    assert_eq!(tabs(&app), (2, 1), "the tab on screen shows its account");
    assert_eq!(
        (app.mail().list(), open_thread(&app), app.mail().filter()),
        (&ListId::Sent, Some(opened), Some(Filter::Unread))
    );
    launcher.open_url("exo://maya@brightmoor.co/search/potato");
    app.settle();
    assert_eq!(app.mail().search_query(), Some("potato"));
    assert_eq!(app.subjects(), ["Thanksgiving 🦃"]);
    launcher.open_url("exo://maya@brightmoor.co/calendar/week/2026-12-02");
    app.settle();
    assert_eq!(
        app.mail().calendar_week(),
        jiff::civil::Date::new(2026, 12, 2).ok()
    );
    launcher.open_url("exo://maya@brightmoor.co/label/no-such-label");
    app.settle();
    assert_eq!(
        (app.mail().calendar_week(), app.mail().is_searching()),
        (None, false)
    );
    assert_eq!(
        *app.mail().list(),
        ListId::Inbox,
        "a slug nothing has: home"
    );
    assert_eq!(tabs(&app), (2, 1));
}
