//! Spelling in compose and the body's right-click menu, end to end on gmail-mock: a misspelled
//! word's corrections, `Add to Dictionary`, the selection's `Search with Google`, and `Paste` of a
//! screenshot.

mod support;

use std::{
    cell::RefCell,
    collections::HashSet,
    ops::Range,
    rc::Rc,
    sync::{Arc, Mutex},
};

use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use exo::desktop::context_menu::MenuItem;
use support::{Fixture, start};

/// Knows a few words; anything else made of letters is misspelled.
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
                    if !["I", "saw", "the", "cat", "Hi", "Priya"].contains(&word.as_str())
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
        ["the", "tea", "ten", "tech"].map(String::from).to_vec()
    }

    fn learn(&mut self, word: &str) {
        self.learned.borrow_mut().insert(word.to_owned());
    }

    fn ignore(&mut self, _word: &str) {}
}

/// The menus shown, by their items' titles.
type Menus = Arc<Mutex<Vec<Vec<String>>>>;

/// A reply to the first conversation with `text` typed in its body, spelling checked by `words`,
/// its menus choosing the item titled `choose`.
fn reply_with(name: &str, text: &str, words: &Words, choose: &'static str) -> (Fixture, Menus) {
    let mut app = start(name);
    let words = words.clone();
    app.mail_mut().set_spelling(move || Box::new(words.clone()));
    let shown = Menus::default();
    let seen = shown.clone();
    app.mail_mut().set_context_menu(move |items, _| {
        let titles = items
            .iter()
            .map(|item| {
                if let MenuItem::Item { title, .. } = item {
                    title.clone()
                } else {
                    "—".to_owned()
                }
            })
            .collect();
        seen.lock().expect("not poisoned").push(titles);
        items
            .iter()
            .position(|item| matches!(item, MenuItem::Item { title, .. } if title == choose))
    });
    app.key(Key::Enter);
    app.key(Key::R);
    app.type_text(text);
    (app, shown)
}

/// The caret's centre on screen.
fn caret(app: &Fixture) -> Pos2 {
    app.mail()
        .compose_caret()
        .expect("a draft has the keyboard")
        .center()
}

fn last_menu(menus: &Menus) -> Vec<String> {
    menus
        .lock()
        .expect("not poisoned")
        .last()
        .cloned()
        .expect("a menu showed")
}

#[test]
fn a_misspelled_words_menu_corrects_it() {
    let (mut app, menus) = reply_with("spelling-correct", "I saw teh", &Words::default(), "the");
    // On the "h" just before the caret.
    app.right_click(caret(&app) - vec2(4.0, 0.0));
    app.settle();
    assert_eq!(
        last_menu(&menus),
        [
            "the",
            "tea",
            "ten",
            "—",
            "Add to Dictionary",
            "Ignore Spelling",
            "Look Up \"teh\"",
            "Search with Google",
            "—",
            "Cut",
            "Copy",
            "Paste"
        ],
        "the editable text's menu, the word selected"
    );
    assert_eq!(
        app.mail().compose_body().expect("a draft"),
        "<div class=\"\">I saw the</div>"
    );
}

#[test]
fn add_to_dictionary_learns_the_word() {
    let words = Words::default();
    let (mut app, menus) = reply_with("spelling-learn", "Hi qwzx", &words, "Add to Dictionary");
    app.right_click(caret(&app) - vec2(4.0, 0.0));
    app.settle();
    assert_eq!(last_menu(&menus)[4], "Add to Dictionary");
    assert!(words.learned.borrow().contains("qwzx"));
    // Right-clicked again, it's no longer misspelled.
    app.right_click(caret(&app) - vec2(4.0, 0.0));
    app.settle();
    assert_eq!(
        last_menu(&menus),
        [
            "Look Up \"qwzx\"",
            "Search with Google",
            "—",
            "Cut",
            "Copy",
            "Paste"
        ]
    );
}

#[test]
fn search_with_google_searches_the_selection() {
    let (mut app, _menus) = reply_with(
        "spelling-search",
        "Hi Priya",
        &Words::default(),
        "Search with Google",
    );
    let at = caret(&app) - vec2(4.0, 0.0);
    app.harness.input_mut().events.push(Event::PointerMoved(at));
    for pressed in [true, false] {
        app.harness.input_mut().events.push(Event::PointerButton {
            pos: at,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        app.harness.run_steps(1);
    }
    // The menu shows the frame after the click, which opens the search.
    app.harness.run_steps(1);
    let opened: Vec<String> = app
        .harness
        .output()
        .platform_output
        .commands
        .iter()
        .filter_map(|command| {
            if let egui::OutputCommand::OpenUrl(url) = command {
                Some(url.url.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(opened, ["https://www.google.com/search?q=Priya"]);
}

#[test]
fn paste_in_the_menu_pastes_a_screenshot() {
    let (mut app, _menus) = reply_with("spelling-paste", "Here ", &Words::default(), "Paste");
    let image = image::RgbaImage::from_pixel(40, 20, image::Rgba([40, 90, 200, 255]));
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("encoded");
    exo::clipboard::capture_image(&app.harness.ctx, png);
    // Past the end of the line: no word, the caret there.
    app.right_click(caret(&app) + vec2(40.0, 0.0));
    app.settle();
    assert!(
        app.mail()
            .compose_body()
            .expect("a draft")
            .contains("<img class=\"Exo-inline-image\""),
        "{:?}",
        app.mail().compose_body()
    );
}

/// The caret of the text field with the keyboard, on screen.
fn field_caret(app: &Fixture) -> Pos2 {
    app.harness
        .output()
        .platform_output
        .ime
        .as_ref()
        .expect("a text field has the keyboard")
        .cursor_rect
        .center()
}

#[test]
fn a_misspelled_subject_is_marked_once_typing_stops_and_its_menu_corrects_it() {
    let mut app = start("spelling-subject");
    let words = Words::default();
    app.mail_mut().set_spelling(move || Box::new(words.clone()));
    let shown = Menus::default();
    let seen = shown.clone();
    app.mail_mut().set_context_menu(move |items, _| {
        seen.lock().expect("not poisoned").push(
            items
                .iter()
                .map(|item| {
                    if let MenuItem::Item { title, .. } = item {
                        title.clone()
                    } else {
                        "—".to_owned()
                    }
                })
                .collect(),
        );
        items
            .iter()
            .position(|item| matches!(item, MenuItem::Item { title, .. } if title == "the"))
    });
    app.key(Key::C);
    app.type_text("priya@brightmoor.co");
    app.key(Key::Enter);
    app.key(Key::Tab);
    app.type_text("I saw teh");
    // Chromium's cold mode: the word typed is checked a second after typing stops.
    app.wait(1.2);
    app.right_click(field_caret(&app) - vec2(4.0, 0.0));
    app.settle();
    assert_eq!(
        last_menu(&shown)[..5],
        ["the", "tea", "ten", "—", "Add to Dictionary"],
        "{:?}",
        last_menu(&shown)
    );
    assert_eq!(
        app.mail()
            .compose_focus()
            .map(|(_, subject)| subject)
            .as_deref(),
        Some("I saw the")
    );
}

#[test]
fn a_field_without_spelling_has_the_editable_menu_and_cut_works() {
    let mut app = start("field-menu");
    let shown = Menus::default();
    let seen = shown.clone();
    app.mail_mut().set_context_menu(move |items, _| {
        seen.lock().expect("not poisoned").push(
            items
                .iter()
                .map(|item| {
                    if let MenuItem::Item { title, .. } = item {
                        title.clone()
                    } else {
                        "—".to_owned()
                    }
                })
                .collect(),
        );
        items
            .iter()
            .position(|item| matches!(item, MenuItem::Item { title, .. } if title == "Cut"))
    });
    app.key(Key::Slash);
    app.settle();
    app.type_text("hello world");
    // On the "d" just before the caret: it selects "world".
    app.right_click(field_caret(&app) - vec2(4.0, 0.0));
    app.settle();
    assert_eq!(
        last_menu(&shown),
        [
            "Look Up \"world\"",
            "Search with Google",
            "—",
            "Cut",
            "Copy",
            "Paste"
        ]
    );
    assert_eq!(
        app.mail().search_text(),
        Some("hello "),
        "Cut took the word"
    );
}

#[test]
fn the_out_of_office_subject_is_checked_and_corrected() {
    let mut app = start("spelling-ooo-subject");
    let gmail = app.gmail();
    let responder = gmail::VacationSettings {
        enable_auto_reply: Some(true),
        response_subject: Some("I saw teh".into()),
        response_body_html: Some("<div>Back soon</div>".into()),
        ..gmail::VacationSettings::default()
    };
    app.block_on(async { gmail.gmail().update_vacation(&responder).await })
        .expect("the mock takes it");
    let words = Words::default();
    app.mail_mut().set_spelling(move || Box::new(words.clone()));
    app.mail_mut().set_context_menu(move |items, _| {
        items
            .iter()
            .position(|item| matches!(item, MenuItem::Item { title, .. } if title == "the"))
    });
    app.press(Key::K, Modifiers::MAC_CMD.plus(Modifiers::COMMAND));
    app.harness
        .input_mut()
        .events
        .push(Event::Text("out of office".to_owned()));
    app.harness.run_steps(1);
    app.key(Key::Enter);
    assert_eq!(
        app.mail()
            .out_of_office_form()
            .map(|form| form.subject.as_str()),
        Some("I saw teh")
    );
    // The dialog at 1280×800: the subject's line from (397, 312), past its end.
    app.click(Pos2::new(800.0, 322.0));
    app.right_click(field_caret(&app) - vec2(4.0, 0.0));
    app.settle();
    assert_eq!(
        app.mail()
            .out_of_office_form()
            .map(|form| form.subject.as_str()),
        Some("I saw the")
    );
}
