//! Split inboxes end to end: the default splits, creating one from Split Inbox Settings,
//! the invalid-definition and discard alerts, Tab between tabs, Escape going home, Important •
//! Other, and the split library's templates (a made-up library, `support/split-library.json`).

mod support;

use egui::{Event, Key, Modifiers, Pos2};
use exo_store::ListId;
use support::{Fixture, start_new_account as start};

/// ⌘ on a Mac, as egui-winit reports it.
const CMD: Modifiers = Modifiers {
    mac_cmd: true,
    command: true,
    ..Modifiers::NONE
};
/// Split Inbox Settings at 1280×800: its Inactive tab, and the middle of its first row (rows
/// are 52px apart).
const INACTIVE_TAB: Pos2 = Pos2::new(476.0, 302.0);
fn row(index: usize, inactive: bool) -> Pos2 {
    Pos2::new(
        600.0,
        if inactive { 354.0 } else { 358.0 } + 52.0 * index as f32,
    )
}

fn typing(app: &mut Fixture, text: &str) {
    app.harness
        .input_mut()
        .events
        .push(Event::Text(text.to_owned()));
    app.harness.run_steps(1);
}

/// Opens the palette and runs the row `query` ranks first.
fn palette(app: &mut Fixture, query: &str) {
    app.press(Key::K, CMD);
    typing(app, query);
    app.key(Key::Enter);
}

/// Opens New Split Inbox from Split Inbox Settings and fills it in.
fn new_split(app: &mut Fixture, name: &str, query: &str) {
    palette(app, "split inbox settings");
    assert!(app.mail().settings_dialog_open());
    // Enter is Add Split Inbox, which opens the form with the name focused.
    app.key(Key::Enter);
    typing(app, name);
    app.key(Key::Tab);
    typing(app, query);
}

fn split_named(app: &Fixture, name: &str) -> Option<exo_store::SplitInbox> {
    app.mail()
        .account()
        .settings()
        .splits()
        .find(|split| split.name() == name)
        .cloned()
}

fn tab_names(app: &Fixture) -> Vec<String> {
    let settings = app.mail().account().settings();
    settings
        .split_entries()
        .iter()
        .filter(|entry| !entry.is_disabled())
        .map(|entry| entry.name().to_owned())
        .collect()
}

#[test]
fn a_new_account_has_important_other_and_the_default_splits() {
    let app = start("splits-defaults");
    assert_eq!(*app.mail().list(), ListId::Important);
    assert_eq!(tab_names(&app), ["Important", "Other"]);
    let off: Vec<(String, bool)> = app
        .mail()
        .account()
        .settings()
        .splits()
        .map(|split| (split.name().to_owned(), split.is_disabled))
        .collect();
    let off: Vec<(&str, bool)> = off
        .iter()
        .map(|(name, disabled)| (name.as_str(), *disabled))
        .collect();
    assert_eq!(
        off,
        [
            ("Starred", true),
            ("VIP", true),
            ("Team", true),
            ("Calendar", true),
            ("News", true)
        ]
    );
}

#[test]
fn a_new_split_takes_its_threads_and_shows_them() {
    let mut app = start("splits-create");
    let important_before = app.subjects();
    new_split(&mut app, "Calloway", "from:callowayfreight.com");
    app.key(Key::Enter);
    let split = split_named(&app, "Calloway").expect("created");
    assert!(
        !app.mail().settings_dialog_open(),
        "creating closes the dialog"
    );
    assert_eq!(
        *app.mail().list(),
        ListId::Split(split.id.clone()),
        "and shows the new split"
    );
    let taken = app.subjects();
    assert!(
        !taken.is_empty() && taken.iter().all(|subject| subject.contains("Atlas beta")),
        "{taken:?}"
    );
    assert!(
        app.mail()
            .account()
            .settings()
            .splits_engagement
            .first_split_enabled_at
            .is_some(),
        "Add Split is gone from the header"
    );
    // With only Important and Other shown, a new split goes before them, which keeps them together.
    assert_eq!(tab_names(&app), ["Calloway", "Important", "Other"]);

    // Tab moves between the tabs, wrapping around.
    app.key(Key::Tab);
    assert_eq!(*app.mail().list(), ListId::Important);
    let important = app.subjects();
    assert!(
        taken.iter().all(|subject| !important.contains(subject)),
        "Important leaves out what the split takes"
    );
    app.key(Key::Tab);
    assert_eq!(*app.mail().list(), ListId::Other);
    let other = app.subjects();
    assert!(
        taken.iter().all(|subject| !other.contains(subject)),
        "and so does Other"
    );
    assert!(
        important.len() < important_before.len(),
        "the split took some of Important's"
    );
    app.key(Key::Tab);
    assert_eq!(*app.mail().list(), ListId::Split(split.id));
    app.press(Key::Tab, Modifiers::SHIFT);
    assert_eq!(*app.mail().list(), ListId::Other);
}

#[test]
fn tab_held_outside_the_inbox_goes_to_the_first_tab() {
    let mut app = start("splits-tab-outside");
    new_split(&mut app, "Calloway", "from:callowayfreight.com");
    app.key(Key::Enter);
    app.key(Key::G);
    app.key(Key::T);
    assert_eq!(*app.mail().list(), ListId::Sent);
    // Held past the repeat delay, the highlight would step along the tabs in the inbox; here
    // every step is to the first tab.
    let tab = |pressed| Event::Key {
        key: Key::Tab,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    app.harness.input_mut().events.push(tab(true));
    app.harness.run_steps(40);
    app.harness.input_mut().events.push(tab(false));
    app.settle();
    assert_eq!(tab_names(&app)[0], "Calloway");
    assert_eq!(
        *app.mail().list(),
        ListId::Split(split_named(&app, "Calloway").expect("created").id)
    );
}

#[test]
fn escape_goes_home_and_backspace_goes_back() {
    let mut app = start("splits-escape");
    new_split(&mut app, "Calloway", "from:callowayfreight.com");
    app.key(Key::Enter);
    let split = ListId::Split(split_named(&app, "Calloway").expect("created").id);
    app.key(Key::Tab);
    app.key(Key::Tab);
    assert_eq!(*app.mail().list(), ListId::Other);
    // Backspace goes back a list.
    app.key(Key::Backspace);
    assert_eq!(*app.mail().list(), ListId::Important);
    app.key(Key::Tab);
    app.key(Key::Tab);
    assert_eq!(*app.mail().list(), split);
    // Escape goes to the start of the back stack, which lists don't open: home.
    app.key(Key::Escape);
    assert_eq!(*app.mail().list(), ListId::Important);
    app.key(Key::Escape);
    assert_eq!(*app.mail().list(), ListId::Important, "and stays there");
}

#[test]
fn a_default_split_keeps_its_name_and_turns_on_from_its_settings() {
    let mut app = start("splits-news");
    palette(&mut app, "split inbox settings");
    app.click(INACTIVE_TAB);
    // News is the last of the defaults: Starred, VIP, Team, Calendar, News.
    app.click(row(4, true));
    app.key(Key::Tab);
    typing(&mut app, "label:Newsletters");
    let news = split_named(&app, "News").expect("seeded");
    assert!(news.is_disabled);
    // `Turn On Split Inbox`, Edit Split Inbox's primary action for a split that's off.
    app.click(Pos2::new(806.0, 573.0));
    let news = split_named(&app, "News").expect("kept");
    assert!(!news.is_disabled, "turned on");
    assert_eq!(news.query(), "label:Newsletters");
    assert!(
        !app.mail().settings_dialog_open(),
        "turning on closes the dialog"
    );
    assert_eq!(*app.mail().list(), ListId::Split(news.id));
}

#[test]
fn invalid_definitions_and_unsaved_changes_ask_first() {
    let mut app = start("splits-alerts");
    new_split(&mut app, "Budget", "budget");
    app.key(Key::Enter);
    assert!(
        split_named(&app, "Budget").is_none(),
        "text can't define a split"
    );
    // `Sorry!` takes the keyboard: Enter is its OK.
    assert!(app.mail().settings_dialog_open());
    app.key(Key::Enter);
    assert!(
        app.mail().settings_dialog_open(),
        "OK goes back to the form"
    );
    assert!(split_named(&app, "Budget").is_none());

    // Escape with a name typed asks to discard; Discard goes back to the list.
    app.key(Key::Escape);
    app.key(Key::Enter);
    assert!(
        app.mail().settings_dialog_open(),
        "back on Split Inbox Settings"
    );
    app.key(Key::Escape);
    assert!(!app.mail().settings_dialog_open());
}

#[test]
fn important_and_other_turn_off_and_on_from_the_settings() {
    let mut app = start("splits-io");
    palette(&mut app, "split inbox settings");
    // Important's row asks first.
    app.click(row(0, false));
    assert!(
        !app.mail().account().settings().unified_inbox,
        "not before Confirm"
    );
    app.key(Key::Enter);
    assert!(app.mail().account().settings().unified_inbox);
    assert_eq!(*app.mail().list(), ListId::Inbox, "the inbox shows Inbox");
    // They're off now: the Inactive tab's first row turns them back on.
    app.click(INACTIVE_TAB);
    app.click(row(0, true));
    assert!(!app.mail().account().settings().unified_inbox);
    assert_eq!(*app.mail().list(), ListId::Important);
    app.key(Key::Escape);
    app.key(Key::G);
    app.key(Key::O);
    assert_eq!(*app.mail().list(), ListId::Other, "g o is Other");
    app.key(Key::G);
    app.key(Key::I);
    assert_eq!(*app.mail().list(), ListId::Important, "and g i Important");
}

#[test]
fn dragging_a_row_by_its_handle_reorders_the_tabs() {
    let mut app = start("splits-reorder");
    new_split(&mut app, "Calloway", "from:callowayfreight.com");
    app.key(Key::Enter);
    assert_eq!(tab_names(&app), ["Calloway", "Important", "Other"]);
    palette(&mut app, "split inbox settings");
    // Calloway's handle, the first row's (its handle at 834,344 28×28), dragged onto Other's.
    let handle = Pos2::new(848.0, 358.0);
    let button = egui::PointerButton::Primary;
    app.harness
        .input_mut()
        .events
        .push(Event::PointerMoved(handle));
    app.harness.run_steps(1);
    app.harness.input_mut().events.push(Event::PointerButton {
        pos: handle,
        button,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    app.harness.run_steps(1);
    for y in [370.0, 400.0, 430.0, 450.0, 462.0] {
        app.harness
            .input_mut()
            .events
            .push(Event::PointerMoved(Pos2::new(848.0, y)));
        app.harness.run_steps(1);
    }
    app.harness.input_mut().events.push(Event::PointerButton {
        pos: Pos2::new(848.0, 462.0),
        button,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    app.harness.run_steps(1);
    app.settle();
    assert_eq!(tab_names(&app), ["Important", "Other", "Calloway"]);
}

/// The library at 1280×800: the middle of a card's preview in the first row of the section on
/// top, by column, and the sidebar's second category.
fn card(column: usize) -> Pos2 {
    Pos2::new(100.0 + 224.0 + 8.0 + 276.0 * column as f32 + 132.0, 350.0)
}
const SECOND_CATEGORY: Pos2 = Pos2::new(218.0, 193.0);

/// Opens the library from Split Inbox Settings' Add Split Inbox.
fn library(app: &mut Fixture) {
    app.with_sample_split_library();
    palette(app, "split inbox settings");
    app.key(Key::Enter);
}

#[test]
fn a_template_from_the_library_turns_its_split_on() {
    let mut app = start("splits-library-template");
    library(&mut app);
    // `Sample Tools`: GitHub's card.
    app.click(SECOND_CATEGORY);
    app.wait(1.0);
    app.click(card(0));
    // GitHub's senders aren't the user's to edit: Enter is `Turn on Split Inbox`.
    app.key(Key::Enter);
    let github = split_named(&app, "GitHub").expect("created from the template");
    assert_eq!(
        (
            github.query(),
            github.template_id.as_deref(),
            github.is_disabled
        ),
        (
            "from:notifications@github.com OR from:noreply@github.com",
            Some("splitTemplate_github"),
            false
        )
    );
    assert!(
        github.hides_when_empty(),
        "a template's split hides when empty unless unchecked"
    );
    assert!(!app.mail().settings_dialog_open());
    assert_eq!(*app.mail().list(), ListId::Split(github.id));
}

#[test]
fn an_empty_split_offers_to_hide_empty_splits_and_a_new_hidden_one_says_so() {
    let mut app = start("splits-hidden-when-empty");
    // An empty split that shows while empty offers to hide them all.
    new_split(&mut app, "Nothing", "from:nobody@example.org");
    app.key(Key::Enter);
    app.settle();
    assert_eq!(
        app.mail().toast(),
        Some("Split 'Nothing' created."),
        "it shows while empty, so it doesn't say so"
    );
    assert!(app.mail().inbox().at_inbox_zero() && app.mail().inbox().view().nudge);
    // `Try it out`, at the card's bottom right; then `Turn on for all splits`.
    app.click(Pos2::new(327.0, 286.0));
    assert!(
        app.mail().settings_dialog_open()
            && app
                .mail()
                .account()
                .settings()
                .nudge_completed("hideEmptySplit")
    );
    app.key(Key::Enter);
    assert!(
        split_named(&app, "Nothing")
            .expect("created")
            .hides_when_empty()
            && app.mail().account().settings().hides_other_when_empty()
    );
    assert_eq!(app.mail().toast(), Some("Split Inboxes updated."));
    // `z` is Undo, which shows them while empty again.
    app.key(Key::Escape);
    app.key(Key::Z);
    assert!(
        !split_named(&app, "Nothing")
            .expect("kept")
            .hides_when_empty()
            && !app.mail().account().settings().hides_other_when_empty()
    );

    // GitHub's template split, which hides when empty, has nothing in it: it says so.
    library(&mut app);
    app.click(SECOND_CATEGORY);
    app.wait(1.0);
    app.click(card(0));
    app.key(Key::Enter);
    app.settle();
    assert_eq!(
        app.mail().toast(),
        Some("This Split Inbox is hidden when empty.")
    );
}

/// VIP's template at 1280×800: its `Definition` heading.
const DEFINITION: Pos2 = Pos2::new(424.0, 357.0);

#[test]
fn vip_picks_senders_from_the_address_book_and_refuses_what_isnt_an_address() {
    let mut app = start("splits-library-vip");
    library(&mut app);
    app.click(card(1));
    // The address book, read for the picker.
    app.wait(0.5);
    // The picker has the keyboard. What isn't an address isn't taken, and Escape clears it
    // rather than closing the dialog.
    app.type_text("nope");
    app.key(Key::Enter);
    app.key(Key::Escape);
    assert!(app.mail().settings_dialog_open(), "Escape cleared the text");
    // Someone from the address book, then the domain that fits, which comes first.
    app.type_text("lena");
    app.key(Key::Enter);
    app.type_text("brightmoor");
    app.key(Key::Enter);
    // An address typed in full: Tab takes it.
    app.type_text("zed@nowhere.org");
    app.key(Key::Tab);
    // Enter in the empty picker does nothing; off it, it's `Turn on Split Inbox`.
    app.key(Key::Enter);
    assert!(
        split_named(&app, "VIP").is_some_and(|vip| vip.is_disabled),
        "Enter in the picker doesn't turn it on"
    );
    app.click(DEFINITION);
    app.key(Key::Enter);
    let vip = split_named(&app, "VIP").expect("seeded");
    assert_eq!(
        (
            vip.query(),
            vip.is_disabled,
            vip.leave_threads_in_important_other
        ),
        (
            "from:zed@nowhere.org OR from:brightmoor.co OR from:lena@brightmoor.co",
            false,
            true
        )
    );
    assert!(app.mail().settings_dialog_open(), "back on the library");
}

#[test]
fn a_built_in_split_and_important_other_open_their_templates() {
    let mut app = start("splits-library-builtins");
    app.with_sample_split_library();
    palette(&mut app, "split inbox settings");
    // Team (Inactive, the third default) lists the account's domain; Enter turns it on.
    app.click(INACTIVE_TAB);
    app.click(row(2, true));
    app.key(Key::Enter);
    let team = split_named(&app, "Team").expect("seeded");
    assert_eq!(
        (team.query(), team.is_disabled),
        ("from:brightmoor.co to:me", false)
    );

    // Important's row opens Important • Other's template: turning it off asks first.
    palette(&mut app, "split inbox settings");
    app.click(row(1, false));
    app.key(Key::Enter);
    assert!(
        !app.mail().account().settings().unified_inbox,
        "not before Confirm"
    );
    app.key(Key::Enter);
    assert!(app.mail().account().settings().unified_inbox);
    assert_eq!(*app.mail().list(), ListId::Inbox);
}

/// The open sub-palette's prompt and rows.
fn sub_rows(app: &Fixture) -> (String, Vec<String>) {
    app.mail()
        .sub_palette_rows()
        .expect("a sub-palette is open")
}

#[test]
fn new_split_inbox_from_the_palette_offers_the_conversations_definitions_then_a_name() {
    let mut app = start("splits-from-palette");
    palette(&mut app, "new split inbox");
    assert_eq!(
        sub_rows(&app),
        (
            "New Split Inbox".to_owned(),
            [
                "New Split Inbox with sender: tom@brightmoor.co",
                "New Split Inbox with domain: brightmoor.co",
                "New Split Inbox with recipients: maya@brightmoor.co, priya@brightmoor.co",
                "New Split Inbox with subject: Design crit moved to 3pm",
                "New Split Inbox from scratch",
            ]
            .map(str::to_owned)
            .to_vec()
        ),
        "the rows for the inbox's first conversation"
    );
    app.key(Key::Enter);
    assert_eq!(sub_rows(&app), ("Split Inbox".to_owned(), Vec::new()));
    typing(&mut app, "Inbox");
    assert_eq!(sub_rows(&app).1, ["Please try another name"]);
    for _ in 0..5 {
        app.key(Key::Backspace);
    }
    typing(&mut app, "Tom");
    assert_eq!(sub_rows(&app).1, ["Create Split Inbox “Tom”"]);
    app.key(Key::Enter);
    let split = split_named(&app, "Tom").expect("created");
    assert_eq!(split.query(), "from:tom@brightmoor.co");
    assert_eq!(
        *app.mail().list(),
        ListId::Split(split.id.clone()),
        "and shown"
    );
    assert_eq!(app.mail().toast_text(), Some("Split 'Tom' created."));

    // The same name and definition again goes to it (from Tom's conversations, now in it).
    palette(&mut app, "new split inbox");
    app.key(Key::Enter);
    typing(&mut app, "tom");
    assert_eq!(sub_rows(&app).1, ["Go to “Tom” Split Inbox"]);
}

#[test]
fn a_subject_is_edited_before_the_split_is_named() {
    let mut app = start("splits-from-subject");
    palette(&mut app, "new split inbox");
    typing(&mut app, "subject");
    app.key(Key::Enter);
    assert_eq!(
        sub_rows(&app),
        (
            "Edit Subject".to_owned(),
            vec!["Subject: “Design crit moved to 3pm”".to_owned()]
        )
    );
    for _ in 0.."moved to 3pm".len() {
        app.key(Key::Backspace);
    }
    app.key(Key::Enter);
    typing(&mut app, "Crits");
    app.key(Key::Enter);
    assert_eq!(
        split_named(&app, "Crits").expect("created").query(),
        "subject:\"Design crit \""
    );
}

#[test]
fn escape_from_edit_subject_goes_back_to_the_main_palette() {
    let mut app = start("splits-subject-escape");
    palette(&mut app, "new split inbox");
    typing(&mut app, "subject");
    app.key(Key::Enter);
    // The first Escape takes the subject out, the second goes back to the main palette, as from
    // the definitions.
    app.key(Key::Escape);
    assert_eq!(sub_rows(&app).0, "Edit Subject");
    app.key(Key::Escape);
    assert_eq!(app.mail().sub_palette_rows(), None);
    typing(&mut app, "new split inbox");
    app.key(Key::Enter);
    assert_eq!(
        sub_rows(&app).0,
        "New Split Inbox",
        "the main palette had the keyboard"
    );
}

#[test]
fn add_to_split_inbox_extends_a_splits_definition() {
    let mut app = start("splits-add-to");
    new_split(&mut app, "Calloway", "from:callowayfreight.com");
    app.key(Key::Enter);
    app.key(Key::G);
    app.key(Key::I);
    palette(&mut app, "add to split inbox");
    assert_eq!(
        sub_rows(&app).1[0],
        "Add sender to Split Inbox: tom@brightmoor.co"
    );
    app.key(Key::Enter);
    let (prompt, splits) = sub_rows(&app);
    assert_eq!(prompt, "Add to Split Inbox");
    assert!(
        splits.contains(&"Calloway".to_owned()) && !splits.contains(&"Calendar".to_owned()),
        "{splits:?}"
    );
    typing(&mut app, "calloway");
    app.key(Key::Enter);
    assert_eq!(
        split_named(&app, "Calloway").expect("still there").query(),
        "from:callowayfreight.com OR from:tom@brightmoor.co"
    );
    assert_eq!(app.mail().toast_text(), Some("Split 'Calloway' updated."));
}

#[test]
fn a_name_taken_by_another_definition_asks_before_updating_that_split() {
    let mut app = start("splits-update-existing");
    new_split(&mut app, "Calloway", "from:callowayfreight.com");
    app.key(Key::Enter);
    app.key(Key::G);
    app.key(Key::I);
    palette(&mut app, "new split inbox");
    app.key(Key::Enter);
    typing(&mut app, "Calloway");
    app.key(Key::Enter);
    assert_eq!(
        app.mail().alert(),
        Some((
            "Update existing Split Inbox?",
            "\"Calloway\" is already a Split Inbox. Would you like to update it?"
        ))
    );
    // No goes back to the name.
    app.key(Key::Escape);
    assert_eq!(sub_rows(&app).1, ["Create Split Inbox “Calloway”"]);
    app.key(Key::Enter);
    app.key(Key::Enter);
    let split = split_named(&app, "Calloway").expect("updated");
    assert_eq!(split.query(), "from:tom@brightmoor.co");
    assert_eq!(*app.mail().list(), ListId::Split(split.id));
}

#[test]
fn a_services_mail_offers_its_split_through_the_new_split_dialog() {
    let mut app = start("splits-service");
    let delivery = serde_json::json!({ "from": "GitHub <notifications@github.com>", "subject": "[brightmoor/atlas] Fix the export timeout (PR #482)", "text": "Review requested.", "labelIds": ["INBOX", "UNREAD", "IMPORTANT"] });
    let url = app.mock_url().join("__mock/deliver").expect("a URL");
    app.block_on(async {
        reqwest::Client::new()
            .post(url)
            .json(&delivery)
            .send()
            .await?
            .error_for_status()
    })
    .expect("delivered");
    app.sync();
    let github = |app: &Fixture| {
        app.subjects()
            .iter()
            .any(|subject| subject.starts_with("[brightmoor/atlas]"))
    };
    if !github(&app) {
        app.key(Key::G);
        app.key(Key::O);
    }
    assert!(github(&app), "{:?}", app.subjects());
    for _ in 0..20 {
        if app.cursor().starts_with("[brightmoor/atlas]") {
            break;
        }
        app.key(Key::K);
    }
    app.press(Key::K, CMD);
    typing(&mut app, "new split inbox github");
    // The conversation is read for its rows.
    app.wait(0.3);
    app.key(Key::Enter);
    assert!(
        app.mail()
            .new_split_dialog_query()
            .is_some_and(|query| query == "from:notifications@github.com")
    );
    typing(&mut app, "Inbox");
    app.key(Key::Enter);
    assert!(
        app.mail().new_split_dialog_query().is_some(),
        "a built-in split's name is refused"
    );
    for _ in 0..5 {
        app.key(Key::Backspace);
    }
    typing(&mut app, "GitHub");
    app.key(Key::Tab);
    typing(&mut app, "PR");
    app.key(Key::Enter);
    assert!(app.mail().new_split_dialog_query().is_none());
    let split = split_named(&app, "GitHub").expect("created");
    assert_eq!(
        split.query(),
        "from:notifications@github.com subject:\"PR\""
    );
    assert_eq!(*app.mail().list(), ListId::Split(split.id));
}

#[test]
fn new_split_inbox_from_scratch_opens_the_form_and_closing_it_closes_the_dialog() {
    let mut app = start("splits-from-scratch");
    palette(&mut app, "new split inbox");
    typing(&mut app, "scratch");
    app.key(Key::Enter);
    assert!(app.mail().settings_dialog_open());
    app.key(Key::Escape);
    assert!(
        !app.mail().settings_dialog_open(),
        "not back to Split Inbox Settings: it opened from the palette"
    );
}

#[test]
fn vip_split_senders_without_the_library_opens_new_split_inbox() {
    let mut app = start("splits-vip-no-library");
    palette(&mut app, "vip split senders");
    assert!(app.mail().settings_dialog_open());
    // The form, with the name focused.
    typing(&mut app, "Bosses");
    app.key(Key::Tab);
    typing(&mut app, "from:callowayfreight.com");
    app.key(Key::Enter);
    assert!(split_named(&app, "Bosses").is_some());
}

#[test]
fn a_sender_row_takes_the_keyboard_and_backspace_or_enter_removes_it() {
    let mut app = start("splits-library-rows");
    library(&mut app);
    app.click(card(1));
    app.wait(0.5);
    app.type_text("zed@nowhere.org");
    app.key(Key::Tab);
    app.type_text("amy@nowhere.org");
    app.key(Key::Tab);
    app.type_text("bo@nowhere.org");
    app.key(Key::Tab);
    // Off the empty picker, Tab goes to the first row; Backspace removes it and gives the
    // keyboard back to the picker, from which Tab reaches the (new) first row again.
    app.key(Key::Tab);
    app.key(Key::Backspace);
    app.key(Key::Tab);
    app.key(Key::Enter);
    app.click(DEFINITION);
    app.key(Key::Enter);
    assert_eq!(
        split_named(&app, "VIP").expect("seeded").query(),
        "from:zed@nowhere.org"
    );
}
