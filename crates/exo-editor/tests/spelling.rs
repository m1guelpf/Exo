//! Spelling in the widget: marks as you type, the right-click's word and corrections, learning and
//! ignoring, and Chromium's dots on screen.

use std::{cell::RefCell, collections::HashSet, ops::Range, rc::Rc};

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, UiBuilder, pos2, vec2};
use egui_kittest::Harness;
use exo_editor::{ContextMenu, Editor, EditorStyle, Spelling};

const ORIGIN: Pos2 = pos2(20.0, 20.0);

/// Knows a few words; anything else made of letters is misspelled.
#[derive(Clone, Default)]
struct Words {
    learned: Rc<RefCell<HashSet<String>>>,
    ignored: Rc<RefCell<HashSet<String>>>,
    checks: Rc<RefCell<usize>>,
}

const KNOWN: &[&str] = &["i", "saw", "the", "cat", "a", "dog", "and"];

impl Spelling for Words {
    fn check(&mut self, text: &str) -> Vec<Range<usize>> {
        *self.checks.borrow_mut() += 1;
        let chars: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        let mut start = None;
        for at in 0..=chars.len() {
            match (chars.get(at).is_some_and(|ch| ch.is_alphabetic()), start) {
                (true, None) => start = Some(at),
                (false, Some(from)) => {
                    let word: String = chars[from..at].iter().collect::<String>().to_lowercase();
                    if !KNOWN.contains(&word.as_str())
                        && !self.learned.borrow().contains(&word)
                        && !self.ignored.borrow().contains(&word)
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
        let word: String = text.chars().skip(word.start).take(word.len()).collect();
        if word == "teh" {
            vec!["the".into(), "tea".into(), "ten".into(), "tech".into()]
        } else {
            Vec::new()
        }
    }

    fn learn(&mut self, word: &str) {
        self.learned.borrow_mut().insert(word.to_lowercase());
    }

    fn ignore(&mut self, word: &str) {
        self.ignored.borrow_mut().insert(word.to_lowercase());
    }
}

struct State {
    editor: Editor,
    ready: bool,
    menus: Vec<ContextMenu>,
}

fn harness(body: &str, words: &Words, focus: bool) -> Harness<'static, State> {
    let mut editor = Editor::from_html(body);
    editor.set_spelling(Box::new(words.clone()));
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
                        state.menus.extend(out.context_menu);
                    },
                );
            },
            State {
                editor,
                ready: false,
                menus: Vec::new(),
            },
        );
    harness.run_steps(2);
    if focus {
        click(&mut harness, pos2(290.0, 30.0), PointerButton::Primary);
    }
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

fn type_text(harness: &mut Harness<'_, State>, text: &str) {
    for ch in text.chars() {
        harness.input_mut().events.push(Event::Text(ch.to_string()));
        harness.run_steps(1);
    }
}

fn marks(harness: &Harness<'_, State>) -> Vec<(usize, Range<usize>)> {
    harness.state().editor.marked_misspellings()
}

#[test]
fn a_misspelling_is_marked_once_the_caret_leaves_it() {
    let mut harness = harness("", &Words::default(), true);
    type_text(&mut harness, "I saw teh");
    assert_eq!(marks(&harness), [], "not while it's being typed");
    type_text(&mut harness, " cat");
    assert_eq!(marks(&harness), [(0, 6..9)], "a space finishes it");

    type_text(&mut harness, " zzx");
    assert_eq!(marks(&harness), [(0, 6..9)]);
    harness.input_mut().events.push(Event::Key {
        key: Key::ArrowLeft,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(1);
    assert_eq!(
        marks(&harness),
        [(0, 6..9), (0, 14..17)],
        "moving the caret finishes the word it was typing"
    );
    harness.input_mut().events.push(Event::Key {
        key: Key::Backspace,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(1);
    assert_eq!(
        marks(&harness),
        [(0, 6..9)],
        "editing a word unmarks it until it's left again"
    );
}

#[test]
fn a_body_is_checked_once_it_has_the_keyboard_and_keeps_its_marks_after() {
    let words = Words::default();
    let mut harness = harness("<div>a teh</div><div>cat qwz</div>", &words, false);
    assert_eq!(marks(&harness), [], "not checked without the keyboard");
    click(&mut harness, pos2(290.0, 30.0), PointerButton::Primary);
    assert_eq!(marks(&harness), [(0, 2..5), (1, 4..7)]);
    let checks = *words.checks.borrow();
    type_text(&mut harness, " a");
    assert_eq!(
        *words.checks.borrow(),
        checks + 2,
        "a keystroke checks only its own line"
    );
    // Off the editor: the keyboard goes, the marks stay.
    click(&mut harness, pos2(390.0, 290.0), PointerButton::Primary);
    assert_eq!(marks(&harness), [(0, 2..5), (1, 4..7)]);
}

#[test]
fn a_right_click_selects_the_word_and_offers_its_corrections() {
    let words = Words::default();
    let mut harness = harness("<div>I saw teh cat</div>", &words, true);
    // The middle of "teh" (x ≈ 40–60 at 14px).
    click(
        &mut harness,
        pos2(ORIGIN.x + 50.0, 30.0),
        PointerButton::Secondary,
    );
    let menu = harness.state_mut().menus.pop().expect("a menu");
    assert_eq!(menu.selection, "teh", "the word is selected");
    let misspelling = menu.misspelling.expect("a misspelling");
    assert_eq!(
        (
            misspelling.block,
            misspelling.range.clone(),
            misspelling.word.as_str()
        ),
        (0, 6..9, "teh")
    );
    assert_eq!(misspelling.guesses[..3], ["the", "tea", "ten"]);

    harness.state_mut().editor.correct(&misspelling, "the");
    harness.run_steps(1);
    assert_eq!(
        harness.state().editor.html(),
        "<div>I saw the cat<br></div>"
    );
    assert_eq!(marks(&harness), []);
    harness.input_mut().events.push(Event::Key {
        key: Key::Z,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::MAC_CMD.plus(Modifiers::COMMAND),
    });
    harness.run_steps(1);
    assert_eq!(
        harness.state().editor.html(),
        "<div>I saw teh cat<br></div>",
        "one undo step"
    );

    // A word spelled right: no corrections; off a word: the caret, nothing selected.
    click(
        &mut harness,
        pos2(ORIGIN.x + 68.0, 30.0),
        PointerButton::Secondary,
    );
    let menu = harness.state_mut().menus.pop().expect("a menu");
    assert_eq!((menu.selection.as_str(), menu.misspelling), ("cat", None));
    click(
        &mut harness,
        pos2(ORIGIN.x + 250.0, 30.0),
        PointerButton::Secondary,
    );
    assert_eq!(
        harness.state_mut().menus.pop().expect("a menu").selection,
        ""
    );
}

#[test]
fn learning_or_ignoring_a_word_unmarks_it_everywhere() {
    let words = Words::default();
    let mut harness = harness("<div>teh cat qwz</div><div>teh</div>", &words, true);
    assert_eq!(marks(&harness), [(0, 0..3), (0, 8..11), (1, 0..3)]);
    harness.state_mut().editor.learn_spelling("teh");
    harness.run_steps(1);
    assert_eq!(marks(&harness), [(0, 8..11)]);
    assert!(words.learned.borrow().contains("teh"));
    harness.state_mut().editor.ignore_spelling("qwz");
    harness.run_steps(1);
    assert_eq!(marks(&harness), []);
}

#[test]
fn a_misspelling_shows_chromiums_red_dots_under_it() {
    let mut harness = harness("<div>teh</div>", &Words::default(), true);
    harness.run_steps(1);
    let image = harness.render().expect("renders");
    let scale = image.width() as f32 / 400.0;
    let red = |x: f32, y: f32| {
        let pixel = image.get_pixel((x * scale) as u32, (y * scale) as u32);
        pixel[0] > 150 && pixel[0] > pixel[1] + 80
    };
    // The 24px line box from y 20, its baseline 15px down: the dots' 2px rows 1px under it, the
    // first dot at the text's left, the next 4px on.
    assert!(
        red(ORIGIN.x + 1.0, ORIGIN.y + 16.5) && red(ORIGIN.x + 1.0, ORIGIN.y + 17.5),
        "the first dot"
    );
    assert!(
        !red(ORIGIN.x + 1.0, ORIGIN.y + 15.5) && !red(ORIGIN.x + 1.0, ORIGIN.y + 18.5),
        "2px tall"
    );
    assert!(!red(ORIGIN.x + 3.0, ORIGIN.y + 17.0), "a gap between dots");
    assert!(red(ORIGIN.x + 5.0, ORIGIN.y + 17.0), "the next dot, 4px on");
}
