//! A spellchecked one-line field, as Chromium checks an `<input>`: when marks show while typing,
//! idle, unfocused and focused again, and what a right-click selects and offers.

use std::ops::Range;

use egui::{Color32, Event, Id, Key, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::Harness;
use exo_editor::{ContextMenu, FieldSpelling, Spelling};
use exo_ui::{Family, Font, TextStyle};

const LINE: Rect = Rect::from_min_max(pos2(20.0, 20.0), pos2(320.0, 40.0));
const STYLE: TextStyle = TextStyle::new(Font::new(Family::Sans, 400), 14.0, 20.0);

/// Knows a few words; anything else made of letters is misspelled.
#[derive(Clone, Default)]
struct Words;

impl Spelling for Words {
    fn check(&mut self, text: &str) -> Vec<Range<usize>> {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        let mut start = None;
        for at in 0..=chars.len() {
            match (chars.get(at).is_some_and(|ch| ch.is_alphabetic()), start) {
                (true, None) => start = Some(at),
                (false, Some(from)) => {
                    let word: String = chars[from..at].iter().collect::<String>().to_lowercase();
                    if !["i", "saw", "the", "cat", "a", "and", "dog"].contains(&word.as_str()) {
                        out.push(from..at);
                    }
                    start = None;
                }
                _ => {}
            }
        }
        out
    }

    fn guesses(&mut self, _text: &str, _word: Range<usize>) -> Vec<String> {
        vec!["the".into(), "tea".into()]
    }

    fn learn(&mut self, _word: &str) {}

    fn ignore(&mut self, _word: &str) {}
}

struct State {
    text: String,
    spelling: FieldSpelling,
    ready: bool,
    menus: Vec<ContextMenu>,
}

fn harness(text: &str) -> Harness<'static, State> {
    let mut spelling = FieldSpelling::default();
    spelling.set_engine(Box::new(Words));
    let mut harness = Harness::builder()
        .with_size(vec2(400.0, 200.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            move |ui, state: &mut State| {
                if !std::mem::replace(&mut state.ready, true) {
                    exo_ui::install(ui.ctx()).expect("the system fonts load");
                    return;
                }
                state.spelling.show(
                    ui,
                    Id::new("field"),
                    &mut state.text,
                    LINE,
                    &STYLE,
                    Color32::BLACK,
                    Color32::LIGHT_BLUE,
                );
                state.menus.extend(state.spelling.take_menu());
            },
            State {
                text: text.to_owned(),
                spelling,
                ready: false,
                menus: Vec::new(),
            },
        );
    harness.run_steps(2);
    harness
}

fn click(harness: &mut Harness<'_, State>, at: Pos2, button: PointerButton) {
    harness.input_mut().events.push(Event::PointerMoved(at));
    harness.input_mut().events.push(Event::PointerButton {
        pos: at,
        button,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(1);
    harness.input_mut().events.push(Event::PointerButton {
        pos: at,
        button,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(1);
}

/// Types `text` a character a frame.
fn type_text(harness: &mut Harness<'_, State>, text: &str) {
    for ch in text.chars() {
        harness.input_mut().events.push(Event::Text(ch.to_string()));
        harness.run_steps(1);
    }
}

/// The marks shown, as `(start, end)` characters.
fn marks(harness: &Harness<'_, State>) -> Vec<(usize, usize)> {
    harness
        .state()
        .spelling
        .marks()
        .iter()
        .map(|mark| (mark.start, mark.end))
        .collect()
}

/// Where character `index` of the field's text is on screen, mid-line.
fn at_char(harness: &Harness<'_, State>, index: usize) -> Pos2 {
    pos2(
        exo_ui::field_x(&harness.ctx, &harness.state().text, LINE, &STYLE, index) + 2.0,
        LINE.center().y,
    )
}

#[test]
fn a_word_typed_is_marked_once_finished_or_a_second_after_typing_stops() {
    let mut harness = harness("");
    click(&mut harness, LINE.center(), PointerButton::Primary);
    type_text(&mut harness, "I saw teh");
    assert!(marks(&harness).is_empty(), "not while it's being typed");
    type_text(&mut harness, " ");
    assert_eq!(marks(&harness), [(6, 9)], "a space checks the field");
    type_text(&mut harness, "cat and a dgo");
    assert_eq!(marks(&harness), [(6, 9)]);
    harness.run_steps(50);
    assert_eq!(marks(&harness), [(6, 9)], "under a second later");
    harness.run_steps(12);
    assert_eq!(
        marks(&harness),
        [(6, 9), (20, 23)],
        "a second after typing stopped, the whole field"
    );
}

#[test]
fn marks_go_with_the_keyboard_and_come_back_with_an_edit() {
    let mut harness = harness("");
    click(&mut harness, LINE.center(), PointerButton::Primary);
    type_text(&mut harness, "I saw teh cat ");
    assert_eq!(marks(&harness), [(6, 9)]);
    click(&mut harness, pos2(380.0, 150.0), PointerButton::Primary);
    assert!(
        marks(&harness).is_empty(),
        "gone once it loses the keyboard"
    );
    click(&mut harness, LINE.center(), PointerButton::Primary);
    harness.run_steps(90);
    assert!(
        marks(&harness).is_empty(),
        "focusing it again checks nothing it checked"
    );
    harness.input_mut().events.push(Event::Key {
        key: Key::End,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(1);
    type_text(&mut harness, " ");
    assert_eq!(marks(&harness), [(6, 9)], "an edit does");
}

#[test]
fn text_it_has_never_checked_is_marked_when_it_takes_the_keyboard() {
    let mut harness = harness("I saw teh cat");
    assert!(marks(&harness).is_empty(), "not without the keyboard");
    click(&mut harness, LINE.center(), PointerButton::Primary);
    assert_eq!(marks(&harness), [(6, 9)]);
}

#[test]
fn a_right_click_selects_the_word_and_offers_its_corrections() {
    let mut harness = harness("I saw teh cat");
    click(&mut harness, LINE.center(), PointerButton::Primary);
    let teh = at_char(&harness, 7);
    click(&mut harness, teh, PointerButton::Secondary);
    let menu = harness.state().menus.last().cloned().expect("a menu");
    assert_eq!(menu.selection, "teh");
    let misspelling = menu.misspelling.expect("the misspelling");
    assert_eq!(
        (misspelling.range.clone(), misspelling.guesses.clone()),
        (6..9, vec!["the".to_owned(), "tea".to_owned()])
    );
    let ctx = harness.ctx.clone();
    let state = harness.state_mut();
    let mut text = state.text.clone();
    state.spelling.correct(&ctx, &mut text, &misspelling, "the");
    assert_eq!(text, "I saw the cat");
    assert!(state.spelling.marks().is_empty(), "checked again at once");

    // Over a selection, the selection stays.
    let mut harness = self::harness("I saw teh cat");
    click(&mut harness, LINE.center(), PointerButton::Primary);
    harness.input_mut().events.push(Event::Key {
        key: Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    });
    harness.run_steps(1);
    let saw = at_char(&harness, 3);
    click(&mut harness, saw, PointerButton::Secondary);
    assert_eq!(
        harness
            .state()
            .menus
            .last()
            .map(|menu| menu.selection.as_str()),
        Some("I saw teh cat")
    );
}
