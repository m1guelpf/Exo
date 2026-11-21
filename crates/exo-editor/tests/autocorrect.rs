//! Autocorrect and autocomplete in the widget: words corrected or marked as they're finished, the
//! suggestion pill and its keys, the marks following edits, and ghost text from the spellchecker's
//! completions.

use std::{cell::RefCell, collections::HashSet, ops::Range, rc::Rc};

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, UiBuilder, pos2, vec2};
use egui_kittest::Harness;
use exo_editor::{Editor, EditorStyle, MarkedWord, Named, People, Spelling, SuggestionKind};

const ORIGIN: Pos2 = pos2(20.0, 20.0);

/// Knows a few words; anything else made of letters is misspelled. `teh` corrects to `the`,
/// `omw` is a text replacement, and `yest` completes to `yesterday`.
#[derive(Clone, Default)]
struct Words {
    learned: Rc<RefCell<HashSet<String>>>,
}

const KNOWN: &[&str] = &[
    "i",
    "saw",
    "the",
    "cat",
    "a",
    "dog",
    "and",
    "on",
    "my",
    "way",
    "yesterday",
    "x",
    "it",
    "yes",
];

impl Spelling for Words {
    fn check(&mut self, text: &str) -> Vec<Range<usize>> {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        let mut start = None;
        for at in 0..=chars.len() {
            match (chars.get(at).is_some_and(|ch| ch.is_alphabetic()), start) {
                (true, None) => start = Some(at),
                (false, Some(from)) => {
                    let as_typed: String = chars[from..at].iter().collect();
                    let word = as_typed.to_lowercase();
                    // A day's name is capitalised.
                    if as_typed != "Monday"
                        && !KNOWN.contains(&word.as_str())
                        && !self.learned.borrow().contains(&word)
                    {
                        out.push(from..at);
                    }
                    start = None;
                }
                _ => {}
            }
        }
        out
    }

    fn guesses(&mut self, text: &str, word: Range<usize>) -> Vec<String> {
        match text
            .chars()
            .skip(word.start)
            .take(word.len())
            .collect::<String>()
            .as_str()
        {
            "teh" => vec!["the".into(), "tea".into(), "ten".into(), "tech".into()],
            "dgo" => vec!["dog".into(), "dug".into()],
            "monday" => vec!["Monday".into()],
            _ => Vec::new(),
        }
    }

    fn learn(&mut self, word: &str) {
        self.learned.borrow_mut().insert(word.to_lowercase());
    }

    fn ignore(&mut self, _word: &str) {}

    fn replacement(&mut self, word: &str) -> Option<String> {
        (word == "omw").then(|| "On my way!".to_owned())
    }

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
        ["yesterday", "yes"]
            .into_iter()
            .filter(|word| word.starts_with(&partial.to_lowercase()))
            .map(str::to_owned)
            .collect()
    }
}

struct State {
    editor: Editor,
    ready: bool,
    learned: Vec<String>,
}

fn harness(body: &str, words: &Words) -> Harness<'static, State> {
    let mut editor = Editor::from_html(body);
    editor.set_spelling(Box::new(words.clone()));
    editor.set_autocorrect(true);
    editor.set_autocomplete(true);
    let mut harness = Harness::builder()
        .with_size(vec2(400.0, 300.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            move |ui, state: &mut State| {
                if !std::mem::replace(&mut state.ready, true) {
                    exo_ui::install(ui.ctx()).expect("the system fonts load");
                    return;
                }
                ui.scope_builder(
                    UiBuilder::new().max_rect(Rect::from_min_size(ORIGIN, vec2(300.0, 200.0))),
                    |ui| {
                        let out = state.editor.ui(ui, &EditorStyle::default());
                        state.learned.extend(out.learned);
                    },
                );
            },
            State {
                editor,
                ready: false,
                learned: Vec::new(),
            },
        );
    harness.run_steps(2);
    // Focus, the caret at the end.
    let at = pos2(290.0, 30.0);
    harness.input_mut().events.push(Event::PointerMoved(at));
    harness.input_mut().events.push(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(1);
    harness.input_mut().events.push(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(1);
    harness
}

fn type_text(harness: &mut Harness<'_, State>, text: &str) {
    for ch in text.chars() {
        if ch == '\n' {
            key(harness, Key::Enter, Modifiers::NONE);
        } else {
            harness.input_mut().events.push(Event::Text(ch.to_string()));
            harness.run_steps(1);
        }
    }
}

fn key(harness: &mut Harness<'_, State>, key: Key, modifiers: Modifiers) {
    harness.input_mut().events.push(Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    });
    harness.run_steps(1);
}

/// Lets the caret rest past the 300ms debounce.
fn rest(harness: &mut Harness<'_, State>) {
    harness.run_steps(24);
}

fn text(harness: &Harness<'_, State>) -> String {
    harness
        .state()
        .editor
        .document()
        .blocks
        .iter()
        .map(|block| block.text())
        .collect::<Vec<_>>()
        .join("\n")
}

fn marks(harness: &mut Harness<'_, State>) -> Vec<MarkedWord> {
    harness.state_mut().editor.autocorrect_marks()
}

fn corrected(block: usize, range: Range<usize>, from: &str, dotted: bool) -> MarkedWord {
    MarkedWord {
        block,
        range,
        corrected_from: Some(from.into()),
        dotted,
    }
}

fn misspelled(block: usize, range: Range<usize>) -> MarkedWord {
    MarkedWord {
        block,
        range,
        corrected_from: None,
        dotted: true,
    }
}

const CMD: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);

#[test]
fn a_finished_word_is_corrected_marked_or_left() {
    let mut harness = harness("", &Words::default());
    type_text(&mut harness, "I saw teh");
    assert_eq!(text(&harness), "I saw teh", "not while it's being typed");
    type_text(&mut harness, " ");
    assert_eq!(text(&harness), "I saw the ");
    assert_eq!(marks(&mut harness), [corrected(0, 6..9, "teh", true)]);
    type_text(&mut harness, "a dgo ");
    assert_eq!(text(&harness), "I saw the a dgo ");
    assert_eq!(
        marks(&mut harness),
        [corrected(0, 6..9, "teh", false), misspelled(0, 12..15)],
        "the correction's dots go once the caret leaves it"
    );
    assert_eq!(
        harness.state().editor.marked_misspellings(),
        [],
        "the body's own spellcheck is off"
    );
}

#[test]
fn a_correction_is_its_own_undo_step() {
    let mut harness = harness("", &Words::default());
    type_text(&mut harness, "I saw teh ");
    key(&mut harness, Key::Z, CMD);
    assert_eq!(text(&harness), "I saw teh ");
    assert_eq!(marks(&mut harness), []);
}

#[test]
fn hyphens_enter_punctuation_and_replacements_finish_words() {
    let mut harness = harness("", &Words::default());
    type_text(&mut harness, "teh-teh\nomw ");
    assert_eq!(text(&harness), "the-the\nOn my way! ");
    type_text(&mut harness, "teh.");
    assert_eq!(
        text(&harness),
        "the-the\nOn my way! teh.",
        "punctuation waits for typing to pause"
    );
    rest(&mut harness);
    assert_eq!(text(&harness), "the-the\nOn my way! the.");
    assert_eq!(
        marks(&mut harness).last(),
        Some(&corrected(1, 11..14, "teh", true))
    );
}

#[test]
fn names_are_left_alone() {
    let mut harness = harness("", &Words::default());
    harness.state_mut().editor.set_names(["Priya".to_owned()]);
    type_text(&mut harness, "priya ");
    assert_eq!(marks(&mut harness), []);
}

#[test]
fn marks_follow_the_edits_around_them() {
    let mut harness = harness("", &Words::default());
    type_text(&mut harness, "a dgo cat");
    key(&mut harness, Key::Home, Modifiers::NONE);
    type_text(&mut harness, "x \n");
    assert_eq!(text(&harness), "x \na dgo cat");
    assert_eq!(
        marks(&mut harness),
        [misspelled(1, 2..5)],
        "a line and a word before it"
    );
    // Into the word: its mark goes, and the word as it reads now is checked again, its dots
    // waiting for typing to pause.
    key(&mut harness, Key::ArrowRight, Modifiers::NONE);
    key(&mut harness, Key::ArrowRight, Modifiers::NONE);
    key(&mut harness, Key::ArrowRight, Modifiers::NONE);
    type_text(&mut harness, "o");
    assert_eq!(
        marks(&mut harness),
        [MarkedWord {
            block: 1,
            range: 2..6,
            corrected_from: None,
            dotted: false
        }]
    );
}

#[test]
fn the_pill_undoes_a_correction() {
    let mut harness = harness("", &Words::default());
    type_text(&mut harness, "teh ");
    key(&mut harness, Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(
        harness.state().editor.suggestions(),
        None,
        "not until the caret rests"
    );
    rest(&mut harness);
    let pill = harness
        .state()
        .editor
        .suggestions()
        .cloned()
        .expect("the pill");
    assert_eq!(
        (pill.kind, pill.items.as_slice(), pill.learn, pill.selected),
        (SuggestionKind::Undo, &["teh".to_owned()][..], false, None)
    );
    key(&mut harness, Key::ArrowDown, Modifiers::NONE);
    assert_eq!(
        harness
            .state()
            .editor
            .suggestions()
            .and_then(|pill| pill.selected),
        Some(0)
    );
    key(&mut harness, Key::Enter, Modifiers::NONE);
    assert_eq!(text(&harness), "teh ");
    assert_eq!(marks(&mut harness), [misspelled(0, 0..3)]);
    // Undone, it isn't corrected again.
    key(&mut harness, Key::End, Modifiers::NONE);
    type_text(&mut harness, "teh ");
    assert_eq!(text(&harness), "teh teh ");
}

#[test]
fn the_pill_offers_corrections_and_learns_a_word() {
    let words = Words::default();
    let mut harness = harness("", &words);
    type_text(&mut harness, "a dgo and dgo ");
    for _ in 0..10 {
        key(&mut harness, Key::ArrowLeft, Modifiers::NONE);
    }
    rest(&mut harness);
    let pill = harness
        .state()
        .editor
        .suggestions()
        .cloned()
        .expect("the pill");
    assert_eq!(
        (pill.kind, pill.items.as_slice(), pill.learn),
        (
            SuggestionKind::Multiple,
            &["dog".to_owned(), "dug".to_owned()][..],
            true
        )
    );
    // ←/→ go round the corrections and Learn; Escape puts it away until the caret leaves.
    key(&mut harness, Key::ArrowDown, Modifiers::NONE);
    key(&mut harness, Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(
        harness
            .state()
            .editor
            .suggestions()
            .and_then(|pill| pill.selected),
        Some(2)
    );
    key(&mut harness, Key::Escape, Modifiers::NONE);
    rest(&mut harness);
    assert_eq!(harness.state().editor.suggestions(), None);
    key(&mut harness, Key::Home, Modifiers::NONE);
    for _ in 0..3 {
        key(&mut harness, Key::ArrowRight, Modifiers::NONE);
    }
    rest(&mut harness);
    key(&mut harness, Key::ArrowDown, Modifiers::NONE);
    key(&mut harness, Key::Enter, Modifiers::NONE);
    assert_eq!(text(&harness), "a dog and dgo ");
    assert_eq!(
        marks(&mut harness),
        [corrected(0, 2..5, "dgo", true), misspelled(0, 10..13)]
    );

    // Learning the other: its marks go everywhere, the correction's too
    // (`[data-incorrect="dgo"]`), and the app hears of it.
    key(&mut harness, Key::End, Modifiers::NONE);
    key(&mut harness, Key::ArrowLeft, Modifiers::NONE);
    rest(&mut harness);
    key(&mut harness, Key::ArrowDown, Modifiers::NONE);
    key(&mut harness, Key::ArrowLeft, Modifiers::NONE);
    key(&mut harness, Key::Tab, Modifiers::NONE);
    assert!(words.learned.borrow().contains("dgo"));
    assert_eq!(harness.state().learned, ["dgo"]);
    assert_eq!(marks(&mut harness), []);
}

#[test]
fn a_misspelling_being_typed_offers_its_correction() {
    let mut harness = harness("", &Words::default());
    type_text(&mut harness, "teh");
    rest(&mut harness);
    let pill = harness
        .state()
        .editor
        .suggestions()
        .cloned()
        .expect("the pill");
    assert_eq!(
        (pill.kind, pill.items.as_slice()),
        (SuggestionKind::Single, &["the".to_owned()][..])
    );
    // Turned down, the word is marked and left as typed.
    key(&mut harness, Key::Escape, Modifiers::NONE);
    assert_eq!(marks(&mut harness), [misspelled(0, 0..3)]);
    type_text(&mut harness, " ");
    assert_eq!(text(&harness), "teh ");
}

#[test]
fn autocomplete_finishes_the_word_at_the_end_of_a_line() {
    let mut harness = harness("", &Words::default());
    type_text(&mut harness, "ye");
    assert!(
        !harness.state().editor.wants_tab(),
        "not under three letters"
    );
    type_text(&mut harness, "s");
    assert!(
        harness.state().editor.wants_tab(),
        "yes…: the likeliest completion"
    );
    type_text(&mut harness, "te");
    // Typing its letters takes them; Backspace gives them back.
    key(&mut harness, Key::Backspace, Modifiers::NONE);
    key(&mut harness, Key::Tab, Modifiers::NONE);
    assert_eq!(text(&harness), "yesterday");
    // Not mid-line.
    key(&mut harness, Key::Home, Modifiers::NONE);
    type_text(&mut harness, "yest ");
    assert_eq!(text(&harness), "yest yesterday");
    assert!(!harness.state().editor.wants_tab());
}

#[test]
fn turning_autocorrect_off_drops_its_marks_and_brings_spellcheck_back() {
    let mut harness = harness("", &Words::default());
    type_text(&mut harness, "a dgo ");
    assert_eq!(marks(&mut harness).len(), 1);
    harness.state_mut().editor.set_autocorrect(false);
    harness.run_steps(1);
    assert_eq!(marks(&mut harness), []);
    assert_eq!(harness.state().editor.marked_misspellings(), [(0, 2..5)]);
    type_text(&mut harness, "teh ");
    assert_eq!(text(&harness), "a dgo teh ", "nothing is corrected");
}

/// Priya Raman in To, Tom Becker in Cc, both named.
fn priya_and_tom() -> People {
    let named = |first: &str, last: &str| {
        Some(Named {
            first: first.into(),
            last: last.into(),
            full: format!("{first} {last}"),
        })
    };
    People {
        from: None,
        to: vec![named("Priya", "Raman")],
        cc: vec![named("Tom", "Becker")],
        bcc: Vec::new(),
    }
}

fn ghost(harness: &Harness<'_, State>) -> Option<String> {
    harness.state().editor.ghost_text().map(str::to_owned)
}

#[test]
fn autocomplete_greets_the_people_a_draft_is_to_on_its_first_line() {
    let mut harness = harness("", &Words::default());
    harness.state_mut().editor.set_people(priya_and_tom());
    type_text(&mut harness, "H");
    assert_eq!(
        ghost(&harness),
        None,
        "Hey, Hi or Hello: until the greeting's first word is typed, nothing"
    );
    type_text(&mut harness, "i");
    assert_eq!(ghost(&harness).as_deref(), Some(" Priya"));
    type_text(&mut harness, " ");
    assert_eq!(
        ghost(&harness).as_deref(),
        Some("Priya"),
        "typing it takes it"
    );
    type_text(&mut harness, "P");
    assert_eq!(ghost(&harness).as_deref(), Some("riya"));
    key(&mut harness, Key::Tab, Modifiers::NONE);
    assert_eq!(text(&harness), "Hi Priya");
    type_text(&mut harness, " a");
    assert_eq!(
        ghost(&harness).as_deref(),
        Some("nd Tom"),
        "with those it copies"
    );
    key(&mut harness, Key::Escape, Modifiers::NONE);
    key(&mut harness, Key::Enter, Modifiers::NONE);
    type_text(&mut harness, "Hi");
    assert_eq!(ghost(&harness), None, "the first line only");
}

#[test]
fn autocomplete_finishes_the_names_of_the_drafts_people() {
    let mut harness = harness("<div>Hello there</div><div><br></div>", &Words::default());
    harness.state_mut().editor.set_people(priya_and_tom());
    type_text(&mut harness, "Ask T");
    assert_eq!(ghost(&harness), None, "one letter: nothing yet");
    type_text(&mut harness, "o");
    assert_eq!(ghost(&harness).as_deref(), Some("m"));
    type_text(&mut harness, "m and Be");
    assert_eq!(ghost(&harness).as_deref(), Some("cker"), "a last name");
    type_text(&mut harness, "cker or pr");
    assert_eq!(ghost(&harness), None, "only a capitalised word");
    type_text(&mut harness, " - Ma");
    assert_eq!(ghost(&harness), None);
}

fn press(harness: &mut Harness<'_, State>, key_: Key, times: usize) {
    for _ in 0..times {
        key(harness, key_, Modifiers::NONE);
    }
}

#[test]
fn a_capitalisation_is_made_once_a_paragraph_and_never_again_once_undone_by_hand() {
    let mut harness = harness("", &Words::default());
    type_text(&mut harness, "I saw it on monday and monday ");
    assert_eq!(
        text(&harness),
        "I saw it on Monday and monday ",
        "once in its paragraph"
    );
    assert_eq!(marks(&mut harness), [corrected(0, 12..18, "monday", false)]);
    // Edited by hand, it's rejected.
    press(&mut harness, Key::ArrowLeft, 12);
    press(&mut harness, Key::Backspace, 1);
    type_text(&mut harness, "y");
    assert_eq!(text(&harness), "I saw it on Monday and monday ");
    press(&mut harness, Key::End, 1);
    key(&mut harness, Key::Enter, Modifiers::NONE);
    type_text(&mut harness, "monday ");
    assert_eq!(
        text(&harness),
        "I saw it on Monday and monday \nmonday ",
        "not made again"
    );
    assert!(
        marks(&mut harness).iter().all(|mark| mark.block == 0),
        "nor marked"
    );
}

#[test]
fn a_word_edited_inside_is_checked_again_and_marked_once_typing_pauses() {
    let mut harness = harness("", &Words::default());
    type_text(&mut harness, "I saw the cat ");
    press(&mut harness, Key::ArrowLeft, 6);
    type_text(&mut harness, "e");
    assert_eq!(text(&harness), "I saw thee cat ");
    assert_eq!(
        marks(&mut harness),
        [MarkedWord {
            block: 0,
            range: 6..10,
            corrected_from: None,
            dotted: false
        }],
        "marked, its dots waiting"
    );
    rest(&mut harness);
    harness.run_steps(30);
    assert_eq!(
        marks(&mut harness),
        [misspelled(0, 6..10)],
        "once typing pauses"
    );
    press(&mut harness, Key::Backspace, 1);
    assert_eq!(marks(&mut harness), [], "right again");
}

#[test]
fn a_snippets_last_words_are_left_until_another_key() {
    let mut harness = harness("", &Words::default());
    harness.state_mut().editor.insert_html("Thanks, tehx");
    harness.state_mut().editor.snippet_inserted();
    type_text(&mut harness, " ");
    assert!(
        marks(&mut harness).is_empty(),
        "Space after a snippet finishes nothing"
    );
    type_text(&mut harness, "I saw teh ");
    assert_eq!(text(&harness), "Thanks, tehx I saw the ");
}
