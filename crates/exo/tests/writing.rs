//! The Writing and Advanced settings at work: Backtick as Escape and Convert Hyphen to Bulleted
//! List in compose, the signature on replies, and where attachments download to, on gmail-mock.

mod support;

use egui::{Event, Key, Modifiers};
use gmail::{DraftQuery, Format};
use support::{Fixture, start, start_with_settings};

const COMMAND: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);

/// Presses `key` with `modifiers` as a keyboard does: the key, then the text it types.
fn type_key(app: &mut Fixture, key: Key, modifiers: Modifiers, text: &str) {
    let event = |pressed| Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    };
    let input = app.harness.input_mut();
    input.events.extend([event(true), Event::Text(text.into())]);
    app.harness.run_steps(1);
    app.harness.input_mut().events.push(event(false));
    app.settle();
}

/// A new message to Priya with the keyboard in its body.
fn new_message(app: &mut Fixture) {
    app.key(Key::C);
    app.type_text("priya@brightmoor.co");
    app.key(Key::Enter);
    app.key(Key::Tab);
    app.type_text("Ticks");
    app.key(Key::Tab);
    app.type_text("Run ");
}

#[test]
fn a_backtick_types_in_compose_unless_it_is_escape() {
    let mut app = start("writing-backtick-off");
    new_message(&mut app);
    type_key(&mut app, Key::Backtick, Modifiers::NONE, "`");
    assert!(
        app.mail()
            .compose_body()
            .is_some_and(|body| body.contains("Run `")),
        "typed: {:?}",
        app.mail().compose_body()
    );
    assert!(app.mail().compose_focus().is_some(), "still writing");

    let mut app = start_with_settings("writing-backtick-on", &|settings| {
        settings.enable_backtick_as_escape = Some(true)
    });
    new_message(&mut app);
    // ⌥⇧` types one.
    type_key(
        &mut app,
        Key::Backtick,
        Modifiers::ALT | Modifiers::SHIFT,
        "`",
    );
    assert!(
        app.mail()
            .compose_body()
            .is_some_and(|body| body.contains("Run `") && !body.contains("``")),
        "{:?}",
        app.mail().compose_body()
    );
    type_key(&mut app, Key::Backtick, Modifiers::NONE, "`");
    assert_eq!(
        app.mail().compose_focus(),
        None,
        "a backtick leaves the draft, as Escape does"
    );
}

#[test]
fn a_backtick_in_the_subject_types_but_closes_the_palette() {
    let mut app = start("writing-backtick-subject");
    app.key(Key::C);
    app.type_text("priya@brightmoor.co");
    app.key(Key::Enter);
    app.key(Key::Tab);
    type_key(&mut app, Key::Backtick, Modifiers::NONE, "`");
    assert_eq!(
        app.mail()
            .compose_focus()
            .map(|(_, subject)| subject)
            .as_deref(),
        Some("`"),
        "compose's Escape leaves it to the subject"
    );
    app.press(Key::K, COMMAND);
    type_key(&mut app, Key::Backtick, Modifiers::NONE, "`");
    assert!(
        app.mail().compose_focus().is_some(),
        "the palette closed back to the draft"
    );
}

#[test]
fn a_hyphen_starts_a_list_only_with_the_setting() {
    let mut app = start("writing-hyphen-off");
    new_message(&mut app);
    app.key(Key::Enter);
    app.type_text("- milk");
    assert!(
        app.mail()
            .compose_body()
            .is_some_and(|body| !body.contains("<ul") && body.contains("- milk")),
        "{:?}",
        app.mail().compose_body()
    );

    let mut app = start_with_settings("writing-hyphen-on", &|settings| {
        settings.convert_hyphen_to_bullet_list = true
    });
    new_message(&mut app);
    app.key(Key::Enter);
    app.type_text("- milk");
    assert!(
        app.mail()
            .compose_body()
            .is_some_and(|body| body.contains("<ul")
                && body.contains("milk")
                && !body.contains("- milk")),
        "{:?}",
        app.mail().compose_body()
    );
}

/// The raw message of Gmail's draft that has `text`, its quoted-printable lines joined.
fn draft_with(app: &Fixture, text: &str) -> Option<String> {
    let gmail = app.gmail();
    app.block_on(async {
        for draft in gmail
            .gmail()
            .drafts(&DraftQuery::default())
            .await
            .expect("drafts.list")
            .drafts
        {
            let full = gmail
                .gmail()
                .draft(&draft.id.expect("an id"), Format::Raw)
                .await
                .expect("drafts.get");
            let raw = String::from_utf8_lossy(
                &full
                    .message
                    .and_then(|message| message.raw)
                    .unwrap_or_default(),
            )
            .replace("=\r\n", "");
            if raw.contains(text) {
                return Some(raw);
            }
        }
        None
    })
}

/// Replies to the first conversation with `text`, and waits for Gmail to have the draft.
fn reply(app: &mut Fixture, text: &str) -> String {
    app.key(Key::Enter);
    app.key(Key::R);
    app.type_text(text);
    for _ in 0..20 {
        app.wait(0.5);
        if let Some(raw) = draft_with(app, text) {
            return raw;
        }
    }
    panic!("Gmail never had the reply");
}

#[test]
fn replies_carry_the_signature_unless_turned_off() {
    let mut app = start("writing-signature-on");
    let raw = reply(&mut app, "Sounds good");
    assert!(
        raw.contains("Head of Product"),
        "the alias's signature: {raw}"
    );

    let mut app = start_with_settings("writing-signature-off", &|settings| {
        settings.include_signature_on_replies = Some(false)
    });
    let raw = reply(&mut app, "Sounds good");
    assert!(
        !raw.contains("Head of Product") && raw.contains("gmail_signature"),
        "an empty signature block: {raw}"
    );
}

#[test]
fn a_download_goes_to_the_folder_chosen() {
    let mut app = start("writing-downloads");
    let folder = app.folder("chosen-downloads");
    std::fs::create_dir_all(&folder).expect("a folder");
    exo::downloads::choose(&app.harness.ctx, folder.clone()).expect("chosen");
    for _ in 0..5 {
        app.key(Key::J);
    }
    app.key(Key::Enter);
    // ⌘-click on the deck's card downloads it rather than previewing it.
    let card = egui::pos2(320.0, 320.0);
    app.hover(card);
    app.harness
        .input_mut()
        .events
        .push(Event::ModifiersChanged(COMMAND));
    for pressed in [true, false] {
        app.harness.input_mut().events.push(Event::PointerButton {
            pos: card,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: COMMAND,
        });
        app.harness.run_steps(1);
    }
    app.harness
        .input_mut()
        .events
        .push(Event::ModifiersChanged(Modifiers::NONE));
    for _ in 0..20 {
        app.wait(0.25);
        if std::fs::read_dir(&folder)
            .expect("readable")
            .next()
            .is_some()
        {
            break;
        }
    }
    let saved: Vec<_> = std::fs::read_dir(&folder)
        .expect("readable")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert!(saved[0].ends_with(".pdf"), "{saved:?}");
}
