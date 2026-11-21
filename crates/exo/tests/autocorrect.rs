//! Autocorrect and autocomplete in compose, end to end on gmail-mock: the palette's toggles and
//! their hints, a reply corrected as it's typed, `Learn` from the suggestion pill, Tab taking
//! ghost text, and the Autocorrect dialog.

mod support;

use std::{cell::RefCell, collections::HashSet, ops::Range, rc::Rc};

use egui::{Event, Key, Modifiers};
use support::{Fixture, start, start_with_settings};

const CMD: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);

/// Knows a few words; anything else made of letters is misspelled. `teh` corrects to `the`, and
/// `yest` completes to `yesterday`.
#[derive(Clone, Default)]
struct Words {
    learned: Rc<RefCell<HashSet<String>>>,
}

impl exo_editor::Spelling for Words {
    fn check(&mut self, text: &str) -> Vec<Range<usize>> {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        let mut from = None;
        for at in 0..=chars.len() {
            match (chars.get(at).is_some_and(|ch| ch.is_alphabetic()), from) {
                (true, None) => from = Some(at),
                (false, Some(start)) => {
                    let word: String = chars[start..at].iter().collect();
                    if !["I", "saw", "the", "Hi", "yesterday"].contains(&word.as_str())
                        && !self.learned.borrow().contains(&word)
                    {
                        out.push(start..at);
                    }
                    from = None;
                }
                _ => {}
            }
        }
        out
    }

    fn guesses(&mut self, _text: &str, _word: Range<usize>) -> Vec<String> {
        Vec::new()
    }

    fn learn(&mut self, word: &str) {
        self.learned.borrow_mut().insert(word.to_owned());
    }

    fn ignore(&mut self, _word: &str) {}

    fn correction(&mut self, text: &str, word: Range<usize>) -> Option<String> {
        (text
            .chars()
            .skip(word.start)
            .take(word.len())
            .collect::<String>()
            == "teh")
            .then(|| "the".to_owned())
    }

    fn completions(&mut self, text: &str, word: Range<usize>) -> Vec<String> {
        let partial: String = text.chars().skip(word.start).take(word.len()).collect();
        ["yesterday"]
            .into_iter()
            .filter(|word| word.starts_with(&partial))
            .map(str::to_owned)
            .collect()
    }
}

/// Opens the palette and runs the row `query` ranks first.
fn palette(app: &mut Fixture, query: &str) {
    app.press(Key::K, CMD);
    app.harness
        .input_mut()
        .events
        .push(Event::Text(query.to_owned()));
    app.harness.run_steps(1);
    app.key(Key::Enter);
}

/// A reply to the first conversation, its body's spelling checked by `words`.
fn reply(app: &mut Fixture, words: &Words) {
    let words = words.clone();
    app.mail_mut().set_spelling(move || Box::new(words.clone()));
    app.key(Key::Enter);
    app.key(Key::R);
}

#[test]
fn the_palette_turns_autocorrect_on_and_a_reply_is_corrected_as_its_typed() {
    let mut app = start("autocorrect-palette");
    let words = Words::default();
    app.mail_mut()
        .set_spelling(move || Box::new(Words::default()));
    assert!(
        !app.mail().autocorrect(),
        "a test's spellchecker doesn't correct until asked"
    );
    palette(&mut app, "enable autocorrect");
    assert_eq!(
        app.mail().hint_text().as_deref(),
        Some("Enabled Autocorrect")
    );
    assert_eq!(
        app.mail().account().settings().typing_assistance.enabled,
        Some(true)
    );
    reply(&mut app, &words);
    app.type_text("I saw teh ");
    assert_eq!(
        app.mail().compose_body().expect("a draft"),
        "<div class=\"\">I saw the&nbsp;</div>"
    );

    app.key(Key::Escape);
    palette(&mut app, "disable autocorrect");
    assert_eq!(
        app.mail().hint_text().as_deref(),
        Some("Disabled Autocorrect")
    );
    assert!(!app.mail().autocorrect());
}

#[test]
fn learning_a_word_from_the_pill_hints_it() {
    let mut app = start_with_settings("autocorrect-learn", &|settings| {
        settings.typing_assistance.enabled = Some(true)
    });
    let words = Words::default();
    reply(&mut app, &words);
    app.type_text("Hi qwzx ");
    app.key(Key::ArrowLeft);
    app.key(Key::ArrowLeft);
    app.wait(0.5);
    // ↓ chooses its only item, `Learn "qwzx"`.
    app.key(Key::ArrowDown);
    app.key(Key::Enter);
    assert!(words.learned.borrow().contains("qwzx"));
    assert_eq!(app.mail().hint_text().as_deref(), Some("Learned \"qwzx\""));
    assert_eq!(
        app.mail().compose_body().expect("a draft"),
        "<div class=\"\">Hi qwzx&nbsp;</div>",
        "Enter was the pill's"
    );
}

#[test]
fn tab_takes_autocompletes_ghost_text_and_the_palette_turns_it_off() {
    let mut app = start("autocomplete-tab");
    reply(&mut app, &Words::default());
    app.type_text("yest");
    app.key(Key::Tab);
    assert_eq!(
        app.mail().compose_body().expect("a draft"),
        "<div class=\"\">yesterday</div>"
    );
    assert!(
        app.mail().compose_caret().is_some(),
        "the body keeps the keyboard"
    );

    app.key(Key::Escape);
    palette(&mut app, "disable autocomplete");
    assert_eq!(
        app.mail().hint_text().as_deref(),
        Some("Disabled Autocomplete")
    );
    assert!(!app.mail().account().settings().autocomplete());
}

#[test]
fn autocorrect_settings_opens_its_dialog() {
    let mut app = start("autocorrect-dialog");
    palette(&mut app, "autocorrect settings");
    assert!(app.mail().settings_dialog_open());
    app.key(Key::Escape);
    assert!(!app.mail().settings_dialog_open());
}

#[test]
fn the_out_of_office_message_is_corrected_and_completed_as_a_body_is() {
    let mut app = start_with_settings("autocorrect-ooo", &|settings| {
        settings.typing_assistance.enabled = Some(true)
    });
    let gmail = app.gmail();
    let responder = gmail::VacationSettings {
        enable_auto_reply: Some(true),
        response_subject: Some("Away".into()),
        response_body_html: Some("<div>Back soon</div>".into()),
        ..gmail::VacationSettings::default()
    };
    app.block_on(async { gmail.gmail().update_vacation(&responder).await })
        .expect("the mock takes it");
    app.mail_mut()
        .set_spelling(move || Box::new(Words::default()));
    palette(&mut app, "out of office");
    // The dialog at 1280×800: the subject's line from (397, 312); Tab goes on to the message.
    app.click(egui::Pos2::new(800.0, 322.0));
    app.key(Key::Tab);
    app.key(Key::End);
    app.type_text(" teh ");
    assert!(
        app.mail()
            .out_of_office_form()
            .is_some_and(|form| form.body.contains("Back soon the")),
        "{:?}",
        app.mail().out_of_office_form().map(|form| &form.body)
    );
    app.type_text("yest");
    app.key(Key::Tab);
    assert!(
        app.mail()
            .out_of_office_form()
            .is_some_and(|form| form.body.contains("Back soon the yesterday")),
        "Tab takes the ghost text: {:?}",
        app.mail().out_of_office_form().map(|form| &form.body)
    );
}
