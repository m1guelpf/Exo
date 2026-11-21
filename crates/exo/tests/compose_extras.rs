//! Compose's extras end to end on gmail-mock: Instant Intro, Auto Bcc, recipient chips edited in
//! place and moved, the `:` emoji and `+` mention dropdowns, and their settings dialogs.

mod support;

use egui::{Key, Modifiers};
use exo_store::EmojiSkinTone;
use support::{Fixture, start_with_settings};

const COMMAND: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);

/// The app on an account opted out of the out-of-office nudge, which on some days would open
/// over the thread.
fn start(name: &str) -> Fixture {
    start_with(name, &|_| {})
}

fn start_with(name: &str, change: &dyn Fn(&mut exo_store::Settings)) -> Fixture {
    start_with_settings(name, &|settings| {
        settings.out_of_office_event_detection.opted_out_at = Some("2026-11-01T00:00:00Z".into());
        change(settings);
    })
}

fn recipients(app: &Fixture) -> [Vec<String>; 3] {
    app.mail()
        .compose_recipients()
        .expect("a draft has the keyboard")
}

fn body(app: &Fixture) -> String {
    app.mail()
        .compose_text()
        .expect("a draft has the keyboard")
        .0
}

fn palette(app: &mut Fixture, query: &str) {
    app.press(Key::K, COMMAND);
    app.type_text(query);
    app.key(Key::Enter);
    app.settle();
}

/// Opens the first conversation: Tom's `Design crit moved to 3pm`, to the account and Priya.
fn open_intro(app: &mut Fixture) {
    app.key(Key::Enter);
    app.settle();
}

#[test]
fn instant_intro_moves_the_introducer_to_bcc_and_thanks_them() {
    let mut app = start("intro-thread");
    open_intro(&mut app);
    app.press(Key::I, COMMAND.plus(Modifiers::SHIFT));
    app.settle();
    let [to, cc, bcc] = recipients(&app);
    assert_eq!(bcc, ["tom@brightmoor.co"], "Tom to Bcc");
    assert!(
        !to.contains(&"tom@brightmoor.co".to_owned())
            && !cc.contains(&"tom@brightmoor.co".to_owned())
    );
    assert!(
        to.contains(&"priya@brightmoor.co".to_owned()),
        "Priya stays, moved up to To"
    );
    assert_eq!(body(&app).lines().next(), Some("Thanks Tom! (to BCC)"));
    assert_eq!(
        app.mail().hint_text().as_deref(),
        Some("Not quite right? Hit ⌘ shift I again!")
    );
    // Again: Priya instead, Tom back.
    app.press(Key::I, COMMAND.plus(Modifiers::SHIFT));
    app.settle();
    let [to, _, bcc] = recipients(&app);
    assert_eq!(
        (to, bcc),
        (
            vec!["tom@brightmoor.co".to_owned()],
            vec!["priya@brightmoor.co".to_owned()]
        )
    );
    assert_eq!(body(&app).lines().next(), Some("Thanks Priya! (to BCC)"));
}

#[test]
fn instant_intro_uses_the_line_saved_in_its_dialog() {
    let mut app = start("intro-settings");
    palette(&mut app, "instant intro settings");
    assert!(app.mail().settings_dialog_open());
    // The line is in the input: replace it, and Enter saves.
    app.press(Key::A, COMMAND);
    app.type_text("Merci {first_name}!");
    app.key(Key::Enter);
    app.settle();
    assert!(!app.mail().settings_dialog_open());
    assert_eq!(
        app.mail().account().settings().custom_instant_intro,
        "Merci {first_name}!"
    );
    open_intro(&mut app);
    app.press(Key::I, COMMAND.plus(Modifiers::SHIFT));
    app.settle();
    assert_eq!(body(&app).lines().next(), Some("Merci Tom!"));
}

#[test]
fn auto_bcc_rides_along_unless_everyone_is_excluded() {
    let mut app = start_with("auto-bcc", &|settings| {
        settings.auto_bcc = vec!["CRM <crm@bcc.example.org>".into()];
        settings.auto_bcc_exclusions = vec!["brightmoor.co".into()];
    });
    app.key(Key::C);
    app.settle();
    assert_eq!(
        recipients(&app)[2],
        ["crm@bcc.example.org"],
        "a new message gets it"
    );
    app.type_text("jess@example.com");
    app.key(Key::Enter);
    app.settle();
    assert_eq!(recipients(&app)[2], ["crm@bcc.example.org"]);
    app.key(Key::Escape);
    app.key(Key::Escape);
    app.settle();
    // A reply to Tom and Priya, both at the excluded domain: it comes out again.
    open_intro(&mut app);
    app.key(Key::A);
    app.settle();
    assert!(recipients(&app)[2].is_empty(), "everyone is excluded");
    assert_eq!(app.mail().toast_text(), Some("Auto Bcc address removed"));
}

#[test]
fn the_auto_bcc_dialog_adds_addresses_and_excludes_your_domain() {
    let mut app = start("auto-bcc-settings");
    palette(&mut app, "auto bcc settings");
    assert!(app.mail().settings_dialog_open());
    app.type_text("123@bcc.hubspot.com, nope");
    app.key(Key::Enter);
    app.settle();
    let settings = app.mail().account().settings();
    assert_eq!(settings.auto_bcc, ["123@bcc.hubspot.com"]);
    assert_eq!(
        app.mail().toast_text(),
        Some("The following invalid Auto Bcc addresses were not added: 'nope'.")
    );
}

#[test]
fn chips_edit_in_place_move_between_fields_and_undo() {
    let mut app = start("chips");
    app.key(Key::C);
    app.type_text("ann@example.com");
    app.key(Key::Enter);
    app.type_text("bob@example.com");
    app.key(Key::Enter);
    app.settle();
    // ← selects Bob; Enter edits him in place; the text is all selected, so typing replaces it.
    app.key(Key::ArrowLeft);
    app.key(Key::Enter);
    app.settle();
    assert_eq!(
        app.mail().compose_text().map(|(_, input)| input).as_deref(),
        Some("bob@example.com")
    );
    app.type_text("Cy <cy@example.com>");
    app.key(Key::Enter);
    app.settle();
    assert_eq!(recipients(&app)[0], ["ann@example.com", "cy@example.com"]);
    // Cy is selected after the edit: ⌥↓ moves him to Cc.
    app.press(Key::ArrowDown, Modifiers::ALT);
    app.settle();
    assert_eq!(
        (recipients(&app)[0].clone(), recipients(&app)[1].clone()),
        (
            vec!["ann@example.com".to_owned()],
            vec!["cy@example.com".to_owned()]
        )
    );
    // ⌘Z in a list undoes it.
    app.press(Key::Z, COMMAND);
    app.settle();
    assert_eq!(recipients(&app)[0], ["ann@example.com", "cy@example.com"]);
    // Backspace in the empty input removes the last chip at once.
    app.press(Key::O, COMMAND.plus(Modifiers::SHIFT));
    app.key(Key::Backspace);
    app.settle();
    assert_eq!(recipients(&app)[0], ["ann@example.com"]);
}

#[test]
fn emoji_by_name_and_the_tone_hint() {
    let mut app = start("emoji");
    open_intro(&mut app);
    app.key(Key::R);
    app.settle();
    app.type_text("Ship it :rocke");
    app.settle();
    app.key(Key::Enter);
    app.settle();
    assert_eq!(body(&app), "Ship it 🚀");
    // `:name:` takes the emoji named exactly that; the first one with tones sets `NONE` and hints.
    app.type_text(" :wave");
    app.settle();
    app.press(Key::Semicolon, Modifiers::SHIFT);
    app.settle();
    assert_eq!(body(&app), "Ship it 🚀 👋");
    assert_eq!(
        app.mail().account().settings().emoji_skin_tone,
        Some(EmojiSkinTone::None)
    );
    assert_eq!(
        app.mail().hint_text().as_deref(),
        Some("Want to personalize your emoji? Hit ⌘ K → Emoji Settings!")
    );
}

#[test]
fn the_emoji_dialog_picks_a_tone_the_picker_uses() {
    let mut app = start("emoji-settings");
    palette(&mut app, "emoji settings");
    assert_eq!(
        app.mail().account().settings().emoji_skin_tone,
        Some(EmojiSkinTone::None),
        "opening sets none"
    );
    app.key(Key::ArrowRight);
    app.key(Key::ArrowRight);
    app.key(Key::Enter);
    app.settle();
    assert!(!app.mail().settings_dialog_open());
    assert_eq!(
        app.mail().account().settings().emoji_skin_tone,
        Some(EmojiSkinTone::MediumLight)
    );
    open_intro(&mut app);
    app.key(Key::R);
    app.settle();
    app.type_text(":thumbs_up");
    app.settle();
    app.key(Key::Enter);
    app.settle();
    assert_eq!(body(&app), "👍🏼");
}

#[test]
fn a_new_subjects_mention_shows_no_contacts() {
    // The subject finds the trigger but never shows its contacts.
    let mut app = start("subject-mentions");
    app.key(Key::C);
    app.key(Key::Tab);
    app.type_text("Lunch with +pri");
    app.settle();
    app.key(Key::Enter);
    app.settle();
    let subject = app.mail().compose_focus().map(|(_, subject)| subject);
    assert_eq!(
        subject.as_deref(),
        Some("Lunch with +pri"),
        "Enter went on to the body"
    );
    assert!(recipients(&app)[0].is_empty());
}

#[test]
fn a_plus_mention_adds_them_to_to() {
    let mut app = start("mentions");
    open_intro(&mut app);
    app.key(Key::R);
    app.settle();
    assert_eq!(recipients(&app)[0], ["tom@brightmoor.co"]);
    app.type_text("Looping in +pri");
    app.settle();
    app.key(Key::Enter);
    app.settle();
    assert_eq!(body(&app), "Looping in +Priya");
    assert_eq!(
        recipients(&app)[0],
        ["tom@brightmoor.co", "priya@brightmoor.co"]
    );
    // `−` takes someone off.
    app.type_text(" and -tom");
    app.settle();
    app.key(Key::Enter);
    app.settle();
    assert_eq!(body(&app), "Looping in +Priya and \u{2212}Tom");
    assert_eq!(recipients(&app)[0], ["priya@brightmoor.co"]);
}

/// Lets `text` go in the middle of what has the keyboard, as the Mac's drag would from another app.
fn drop_on_focused(app: &mut Fixture, text: &str) {
    let ctx = app.harness.ctx.clone();
    let focused = ctx
        .memory(|memory| memory.focused())
        .expect("something has the keyboard");
    let at = ctx
        .read_response(focused)
        .expect("drawn last frame")
        .rect
        .center();
    exo::desktop::drag::simulate_drop(exo_ui::text_drag::Dropped {
        at,
        html: None,
        text: text.into(),
        source: None,
        copy: true,
    });
    app.settle();
}

#[test]
fn text_dragged_in_goes_into_the_subject_and_the_body_where_it_is_let_go() {
    let mut app = start("drop-text");
    app.key(Key::C);
    app.key(Key::Tab);
    // An `<input>` takes a drop's line breaks as spaces.
    drop_on_focused(&mut app, "Lunch\nplans");
    assert_eq!(
        app.mail()
            .compose_focus()
            .map(|(_, subject)| subject)
            .as_deref(),
        Some("Lunch plans")
    );
    app.key(Key::Enter);
    app.settle();
    drop_on_focused(&mut app, "See you there");
    let body = app.mail().compose_body().expect("a draft");
    assert!(body.contains("See you there"), "{body}");
}
