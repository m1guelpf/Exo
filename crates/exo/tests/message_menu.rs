//! A message body's right-click menu, end to end on gmail-mock: a word
//! is selected and offers Look Up, Search with Google and Copy; a link offers Copy Link and Open
//! Link, an address link Copy Email Address.

mod support;

use std::sync::{Arc, Mutex};

use egui::{CursorIcon, Key, Pos2, pos2};
use exo::desktop::context_menu::MenuItem;
use support::{Fixture, start};

/// The menus shown, by their items' titles, and the title to choose in the next one.
#[derive(Clone, Default)]
struct Menus {
    shown: Arc<Mutex<Vec<Vec<String>>>>,
    choose: Arc<Mutex<String>>,
}

impl Menus {
    fn install(&self, app: &mut Fixture) {
        let menus = self.clone();
        app.mail_mut().set_context_menu(move |items, _| {
            let titles: Vec<String> = items
                .iter()
                .map(|item| {
                    if let MenuItem::Item { title, .. } = item {
                        title.clone()
                    } else {
                        "—".to_owned()
                    }
                })
                .collect();
            let choose = menus.choose.lock().expect("not poisoned").clone();
            let chosen = titles.iter().position(|title| *title == choose);
            menus.shown.lock().expect("not poisoned").push(titles);
            chosen
        });
    }

    /// Right-clicks `at`, choosing `choose`: the menu's titles, and the text it copied, if any.
    fn right_click(
        &self,
        app: &mut Fixture,
        at: Pos2,
        choose: &str,
    ) -> (Vec<String>, Option<String>) {
        *self.choose.lock().expect("not poisoned") = choose.to_owned();
        let count = self.shown.lock().expect("not poisoned").len();
        app.hover(at);
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        app.harness.input_mut().events.push(button(true));
        app.harness.run_steps(1);
        app.harness.input_mut().events.push(button(false));
        let mut copied = None;
        for _ in 0..200 {
            app.harness.run_steps(1);
            copied = copied.or_else(|| {
                app.harness
                    .output()
                    .platform_output
                    .commands
                    .iter()
                    .find_map(|command| match command {
                        egui::OutputCommand::CopyText(text) => Some(text.clone()),
                        _ => None,
                    })
            });
            if self.shown.lock().expect("not poisoned").len() > count
                && (copied.is_some() || choose.is_empty())
            {
                break;
            }
        }
        (
            self.shown
                .lock()
                .expect("not poisoned")
                .last()
                .cloned()
                .unwrap_or_default(),
            copied,
        )
    }
}

#[test]
fn a_bodys_word_and_links_have_the_shells_menu() {
    let mut app = start("message-menu");
    let html = "<div>Supercalifragilisticexpialidocious and more</div><div><a href=\"https://example.com/notes\">The notes</a></div><div><a href=\"mailto:ann%2Bwork@example.com?subject=Hi\">Write to Ann</a></div>";
    let delivery = serde_json::json!({ "from": "Ann Lee <ann@example.com>", "subject": "Words", "html": html });
    let deliver = app.mock_url().join("__mock/deliver").expect("a URL");
    app.block_on(async {
        reqwest::Client::new()
            .post(deliver)
            .json(&delivery)
            .send()
            .await?
            .error_for_status()
    })
    .expect("delivered");
    app.sync();
    while app.cursor() != "Words" {
        app.key(Key::K);
    }
    app.key(Key::Enter);
    let menus = Menus::default();
    menus.install(&mut app);
    // The body's lines, down the column from its first point of text.
    let x = 300.0;
    let first = (140..400)
        .step_by(4)
        .map(|y| pos2(x, y as f32))
        .find(|at| app.hover(*at) == CursorIcon::Text)
        .expect("the body's first line");
    let word = first + egui::vec2(0.0, 4.0);
    let links: Vec<Pos2> = (0..80)
        .map(|step| pos2(260.0, word.y + 12.0 + step as f32))
        .filter(|at| app.hover(*at) == CursorIcon::PointingHand)
        .collect();
    let (web, mail) = (
        links.first().copied().expect("a link"),
        links.last().copied().expect("another link"),
    );

    let (titles, copied) = menus.right_click(&mut app, word, "Copy");
    assert_eq!(
        titles,
        [
            "Look Up \"Supercalifragilisticexpialidocious\"",
            "Search with Google",
            "—",
            "Copy"
        ],
        "the word is selected"
    );
    assert_eq!(
        copied.as_deref(),
        Some("Supercalifragilisticexpialidocious")
    );

    let (titles, copied) = menus.right_click(&mut app, web, "Copy Link");
    assert_eq!(titles, ["Copy Link", "Open Link"]);
    assert_eq!(copied.as_deref(), Some("https://example.com/notes"));
    let (titles, copied) = menus.right_click(&mut app, mail, "Copy Email Address");
    assert_eq!(titles, ["Copy Email Address", "Open Link"]);
    assert_eq!(
        copied.as_deref(),
        Some("ann+work@example.com"),
        "decoded, without its query"
    );
}
