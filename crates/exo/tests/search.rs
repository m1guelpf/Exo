//! Search and filters driven by keys, end to end: gmail-mock, a fresh store, the bar.

mod support;

use egui::{Key, Modifiers};
use exo_store::Filter;
use support::start;

#[test]
fn slash_searches_and_escape_backs_out_step_by_step() {
    let mut app = start("search");
    app.key(Key::Slash);
    assert!(app.mail().is_searching());
    assert!(app.subjects().is_empty(), "an empty search shows no rows");
    app.type_text("potato");
    app.key(Key::Enter);
    assert_eq!(app.mail().search_query(), Some("potato"));
    assert_eq!(app.subjects(), ["Thanksgiving 🦃"]);
    assert!(
        !app.mail().search_bar_focused(),
        "the results take the keyboard"
    );
    let row = &app.mail().inbox().threads()[0];
    assert!(row.messages > 1);
    let marks = row.search.as_ref().expect("a search row");
    assert_eq!(
        marks
            .snippet
            .iter()
            .map(|range| &row.snippet[range.clone()])
            .collect::<Vec<_>>(),
        ["potato"]
    );

    // Escape gives the bar the keyboard, then (with no suggestions open) leaves search.
    app.key(Key::Escape);
    assert!(app.mail().search_bar_focused());
    app.key(Key::Escape);
    assert!(!app.mail().is_searching());
    assert_eq!(app.subjects().len(), 24, "back to the inbox");
}

#[test]
fn a_pause_in_typing_runs_the_search_ahead_for_enter() {
    let mut app = start("search-instant");
    app.key(Key::Slash);
    // Typing settles for longer than the pause.
    app.type_text("kitchen");
    assert_eq!(app.mail().search_query(), None, "nothing shows yet");
    // Enter shows what Gmail answered then (a page, all it has): it isn't asked again.
    app.disconnect();
    app.key(Key::Enter);
    assert_eq!(app.subjects(), ["Offsite catering — Hudson, Dec 9–11"]);
    let snapshot = app.mail().account().snapshot();
    let results = snapshot.search.as_ref().expect("results");
    assert!(results.finished && !results.offline, "{results:?}");
}

#[test]
fn escape_then_slash_replaces_the_search() {
    let mut app = start("search-escape-slash");
    app.key(Key::Slash);
    app.type_text("offsite");
    app.key(Key::Enter);
    assert!(!app.mail().inbox().threads().is_empty());
    app.key(Key::Escape);
    assert!(app.mail().search_bar_focused());
    assert!(
        app.mail().inbox().cursor().is_none(),
        "the results lose their cursor while the bar has the keyboard"
    );
    app.type_text("/");
    assert_eq!(
        app.mail().search_text(),
        Some("/"),
        "Escape selected the search, which `/` replaces"
    );
}

#[test]
fn an_empty_search_says_so_and_keeps_the_keyboard() {
    let mut app = start("search-none");
    app.key(Key::Slash);
    app.type_text("zzqx");
    app.key(Key::Enter);
    assert!(app.subjects().is_empty());
    assert!(
        app.mail().search_bar_focused(),
        "focus waits for a row that never comes"
    );
}

#[test]
fn opening_a_result_remembers_the_search() {
    let mut app = start("search-history");
    app.key(Key::Slash);
    app.type_text("potato");
    app.key(Key::Enter);
    app.key(Key::Enter);
    assert!(app.mail().thread().is_some(), "the result opens");
    let history = &app.mail().account().settings().search_history;
    assert_eq!(
        history
            .iter()
            .map(|item| item.value.as_str())
            .collect::<Vec<_>>(),
        ["potato"]
    );
}

#[test]
fn shift_u_filters_the_inbox_and_escape_clears_it() {
    let mut app = start("filter");
    app.press(Key::U, Modifiers::SHIFT);
    assert_eq!(app.mail().filter(), Some(Filter::Unread));
    let unread = app.subjects();
    assert!(!unread.is_empty() && unread.len() < 24);
    assert!(
        app.mail()
            .inbox()
            .threads()
            .iter()
            .all(|thread| thread.unread)
    );
    // Read rows stay while the filter is on (sticky rows).
    app.key(Key::U);
    assert_eq!(app.subjects(), unread);
    app.key(Key::Escape);
    assert_eq!(app.mail().filter(), None);
    assert_eq!(app.subjects().len(), 24);
}

#[test]
fn a_filter_becomes_a_search_token() {
    let mut app = start("filter-search");
    // Search opens over a filtered list with the filter's token.
    app.press(Key::S, Modifiers::SHIFT);
    app.key(Key::Slash);
    assert_eq!(app.mail().search_text(), Some("is:starred "));
    app.key(Key::Escape);
    assert!(!app.mail().is_searching());
    app.key(Key::Escape);
    assert_eq!(app.mail().filter(), None, "Escape clears the filter");
    // In search, ⇧U adds `is:unread` to the query and runs it.
    app.key(Key::Slash);
    app.type_text("potato");
    app.key(Key::Enter);
    app.press(Key::U, Modifiers::SHIFT);
    assert_eq!(app.mail().search_query(), Some("potato is:unread "));
    app.press(Key::U, Modifiers::SHIFT);
    assert_eq!(app.mail().search_query(), Some("potato "));
}

#[test]
fn tab_completes_an_operator_and_a_label() {
    let mut app = start("search-tab");
    app.key(Key::Slash);
    app.type_text("fr");
    app.key(Key::Tab);
    assert_eq!(app.mail().search_text(), Some("from:"));
    app.type_text("emekao@gmail.com");
    app.key(Key::Enter);
    assert!(
        app.subjects().contains(&"Thanksgiving 🦃".to_owned()),
        "{:?}",
        app.subjects()
    );
    app.key(Key::Escape);
    app.press(Key::A, Modifiers::COMMAND);
    app.type_text("in:fam");
    app.key(Key::Tab);
    assert_eq!(app.mail().search_text(), Some("in:family "));
}

#[test]
fn a_copied_subject_pastes_into_search_as_its_search() {
    let mut app = start("search-paste-subject");
    exo::clipboard::capture(&app.harness.ctx);
    app.key(Key::Enter);
    app.press(Key::K, Modifiers::MAC_CMD.plus(Modifiers::COMMAND));
    app.type_text("Copy Subject");
    app.key(Key::Enter);
    let subject = app
        .mail()
        .thread()
        .and_then(|thread| thread.detail())
        .map(|detail| detail.subject.clone())
        .expect("an open thread");
    let search = format!("subject:\"{subject}\"");
    assert_eq!(
        exo::clipboard::captured_search(&app.harness.ctx),
        Some((subject.clone(), search.clone()))
    );

    // `/`, then the paste egui makes of ⌘V: the subject's text.
    app.key(Key::Slash);
    app.type_text("from:me ");
    app.harness
        .input_mut()
        .events
        .push(egui::Event::Paste(subject.clone()));
    app.harness.run_steps(1);
    assert_eq!(
        app.mail().search_text(),
        Some(format!("from:me {search}").as_str())
    );

    // After `subject:`, the text alone.
    app.press(Key::A, Modifiers::MAC_CMD.plus(Modifiers::COMMAND));
    app.type_text("subject:\"");
    app.harness
        .input_mut()
        .events
        .push(egui::Event::Paste(subject.clone()));
    app.harness.run_steps(1);
    assert_eq!(
        app.mail().search_text(),
        Some(format!("subject:\"{subject}").as_str())
    );
}
