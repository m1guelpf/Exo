//! The widget driven through egui input: typing, IME, the pointer, the clipboard.

use std::{cell::RefCell, rc::Rc};

use egui::{Event, ImeEvent, Key, Modifiers, PointerButton, Pos2, Rect, UiBuilder, pos2, vec2};
use egui_kittest::Harness;
use exo_editor::{Clipboard, Editor, EditorStyle, Pos, Selection};

const ORIGIN: Pos2 = pos2(20.0, 20.0);

/// A clipboard the test can look into.
#[derive(Clone, Default)]
struct Shared(Rc<RefCell<(Option<String>, Option<String>)>>);

impl Clipboard for Shared {
    fn write(&mut self, html: &str, text: &str) {
        *self.0.borrow_mut() = (Some(html.to_owned()), Some(text.to_owned()));
    }

    fn read_html(&mut self) -> Option<String> {
        self.0.borrow().0.clone()
    }
}

struct State {
    editor: Editor,
    ready: bool,
    /// Links the link popover opened, and the toolbar's clicks.
    opened: Vec<String>,
    toolbar: Vec<exo_editor::ToolbarAction>,
}

fn harness(body: &str, clipboard: Shared) -> Harness<'static, State> {
    let mut editor = Editor::from_html(body);
    editor.set_clipboard(Box::new(clipboard));
    // 60 frames a second, so two clicks a few frames apart are a double click.
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
                        state.opened.extend(out.open_link);
                        state.toolbar.extend(out.toolbar);
                    },
                );
            },
            State {
                editor,
                ready: false,
                opened: Vec::new(),
                toolbar: Vec::new(),
            },
        );
    harness.run_steps(2);
    // Focus it with a click at the end of the first line.
    click(&mut harness, pos2(290.0, 30.0), 1);
    harness
}

fn click(harness: &mut Harness<'_, State>, at: Pos2, count: usize) {
    for _ in 0..count {
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

#[test]
fn typing_reaches_the_document() {
    let mut harness = harness("<div>hi</div>", Shared::default());
    harness
        .input_mut()
        .events
        .push(Event::Text(" there".into()));
    harness.run_steps(1);
    assert_eq!(harness.state().editor.html(), "<div>hi there<br></div>");
}

#[test]
fn ime_composition_inserts_once_on_commit() {
    let mut harness = harness("<div>a</div>", Shared::default());
    harness
        .input_mut()
        .events
        .push(Event::Ime(ImeEvent::Preedit {
            text: "に".into(),
            active_range_chars: None,
        }));
    harness.run_steps(1);
    assert_eq!(
        harness.state().editor.document().blocks[0].text(),
        "a",
        "composition isn't in the document"
    );
    harness
        .input_mut()
        .events
        .push(Event::Ime(ImeEvent::Commit("日".into())));
    harness.run_steps(1);
    assert_eq!(harness.state().editor.document().blocks[0].text(), "a日");
}

#[test]
fn double_click_selects_a_word_and_triple_click_the_line() {
    let mut harness = harness("<div>hello brave world</div>", Shared::default());
    // "hello brave world": the middle of "brave".
    click(&mut harness, pos2(ORIGIN.x + 52.0, 30.0), 2);
    assert_eq!(harness.state().editor.selected_text(), "brave");
    click(&mut harness, pos2(ORIGIN.x + 52.0, 30.0), 1);
    assert_eq!(harness.state().editor.selected_text(), "hello brave world");
}

#[test]
fn copy_carries_html_and_paste_prefers_it() {
    let clipboard = Shared::default();
    let mut harness = harness(
        "<div>plain <b>bold</b></div><div>next</div>",
        clipboard.clone(),
    );
    harness
        .state_mut()
        .editor
        .set_selection(Selection::new(Pos::new(0, 6), Pos::new(0, 10)));
    harness.input_mut().events.push(Event::Copy);
    harness.run_steps(1);
    assert_eq!(
        clipboard.0.borrow().clone(),
        (Some("<b>bold</b>".to_owned()), Some("bold".to_owned()))
    );
    harness
        .state_mut()
        .editor
        .set_selection(Selection::caret(Pos::new(1, 4)));
    harness.input_mut().events.push(Event::Paste("bold".into()));
    harness.run_steps(1);
    assert_eq!(
        harness.state().editor.html(),
        "<div>plain <b>bold</b><br></div><div>next<b>bold</b><br></div>"
    );
}

#[test]
fn arrows_move_by_line_and_shift_extends() {
    let mut harness = harness(
        "<div>first line</div><div>first more</div>",
        Shared::default(),
    );
    harness
        .state_mut()
        .editor
        .set_selection(Selection::caret(Pos::new(0, 3)));
    key(&mut harness, Key::ArrowDown, Modifiers::NONE);
    assert_eq!(
        harness.state().editor.selection(),
        Selection::caret(Pos::new(1, 3)),
        "\"fir|\" is over \"fir|\""
    );
    key(&mut harness, Key::ArrowRight, Modifiers::SHIFT);
    key(
        &mut harness,
        Key::ArrowRight,
        Modifiers {
            alt: true,
            shift: true,
            ..Modifiers::NONE
        },
    );
    assert_eq!(harness.state().editor.selected_text(), "st");
    key(
        &mut harness,
        Key::ArrowUp,
        Modifiers {
            mac_cmd: true,
            command: true,
            ..Modifiers::NONE
        },
    );
    assert_eq!(
        harness.state().editor.selection(),
        Selection::caret(Pos::new(0, 0))
    );
}

#[test]
fn dragging_selects_across_blocks() {
    let mut harness = harness(
        "<div>first line</div><div>second line</div>",
        Shared::default(),
    );
    let (from, to) = (pos2(ORIGIN.x + 1.0, 30.0), pos2(ORIGIN.x + 30.0, 54.0));
    harness.input_mut().events.push(Event::PointerMoved(from));
    harness.input_mut().events.push(Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(1);
    harness.input_mut().events.push(Event::PointerMoved(to));
    harness.run_steps(2);
    harness.input_mut().events.push(Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(1);
    let selection = harness.state().editor.selection();
    assert_eq!(
        (selection.anchor, selection.focus.block),
        (Pos::new(0, 0), 1)
    );
}

#[test]
fn a_pasted_colour_that_isnt_ascii_draws() {
    // Six bytes after `#`, but not six hex digits: slicing them by byte would split the `é`.
    let mut harness = harness("<div>plain</div>", Shared::default());
    harness
        .state_mut()
        .editor
        .paste_html("<span style=\"color:#a\u{e9}\u{e9}a\">odd</span>");
    harness.run_steps(1);
    assert_eq!(
        harness.state().editor.document().blocks[0].text(),
        "plainodd"
    );
}

#[test]
fn undo_brings_back_what_was_cut() {
    let clipboard = Shared::default();
    let mut harness = harness("<div>abc def</div>", clipboard.clone());
    harness
        .state_mut()
        .editor
        .set_selection(Selection::new(Pos::new(0, 4), Pos::new(0, 7)));
    harness.input_mut().events.push(Event::Cut);
    harness.run_steps(1);
    assert_eq!(clipboard.0.borrow().1.as_deref(), Some("def"));
    // A format at the caret records an undo step; it mustn't replace the one before the cut.
    harness
        .state_mut()
        .editor
        .toggle_format(exo_editor::Format::Bold);
    harness.input_mut().events.push(Event::Text("x".into()));
    harness.run_steps(1);
    assert_eq!(harness.state().editor.html(), "<div>abc <b>x</b><br></div>");
    harness.state_mut().editor.undo();
    harness.state_mut().editor.undo();
    assert_eq!(harness.state().editor.html(), "<div>abc def<br></div>");
}

/// Random input through the widget: layout, painting, hit testing, the toolbar, IME and the
/// clipboard must not panic, whatever the document holds and however input batches in a frame.
#[test]
fn random_input_through_the_widget() {
    const BODIES: [&str; 3] = [
        "<div>plain <b>bold</b> <i>it</i> <u>u</u> <s>s</s> <a href=\"http://x.com\">link</a></div><ol><li>one</li><ol><li>two</li></ol></ol><blockquote style=\"x\"><div>q</div></blockquote>",
        "<div>🇺🇸e\u{301}👨\u{200d}👩\u{200d}👧 <span style=\"color:#a\u{e9}\u{e9}a\">odd</span> <span style=\"color:rgb(1,2)\">x</span></div><div><hr></div><table><tr><td>t</td></tr></table><div>a long line that wraps around the editor's width at least once, maybe twice, with more words</div>",
        "",
    ];
    let texts = ["a", " ", "日本", "e\u{301}", "👍🏽", ":", "* ", "\n", "->"];
    let keys = [
        Key::Enter,
        Key::Backspace,
        Key::Delete,
        Key::ArrowLeft,
        Key::ArrowRight,
        Key::ArrowUp,
        Key::ArrowDown,
        Key::Home,
        Key::End,
        Key::Escape,
        Key::Tab,
        Key::A,
        Key::Z,
    ];
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = |n: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % n as u64) as usize
    };
    for body in BODIES {
        let clipboard = Shared::default();
        let mut harness = harness(body, clipboard);
        for step in 0..300 {
            let point = pos2(
                ORIGIN.x + next(320) as f32 - 10.0,
                ORIGIN.y + next(220) as f32 - 10.0,
            );
            let modifiers = Modifiers {
                shift: next(3) == 0,
                alt: next(5) == 0,
                mac_cmd: next(6) == 0,
                command: false,
                ctrl: false,
            };
            let modifiers = Modifiers {
                command: modifiers.mac_cmd,
                ..modifiers
            };
            match next(9) {
                0 | 1 => harness
                    .input_mut()
                    .events
                    .push(Event::Text(texts[next(texts.len())].into())),
                2 | 3 => harness.input_mut().events.push(Event::Key {
                    key: keys[next(keys.len())],
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }),
                4 => harness
                    .input_mut()
                    .events
                    .push(Event::Ime(ImeEvent::Preedit {
                        text: "にほ".into(),
                        active_range_chars: None,
                    })),
                5 => harness
                    .input_mut()
                    .events
                    .push(Event::Ime(ImeEvent::Commit("日本".into()))),
                6 => harness
                    .input_mut()
                    .events
                    .push([Event::Copy, Event::Cut, Event::Paste("p\nq".into())][next(3)].clone()),
                7 => click(&mut harness, point, 1 + next(3)),
                _ => {
                    harness.input_mut().events.push(Event::PointerMoved(point));
                    harness.input_mut().events.push(Event::PointerButton {
                        pos: point,
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers: Modifiers::NONE,
                    });
                    harness.run_steps(1);
                    let to = pos2(point.x + 60.0, point.y + 30.0);
                    harness.input_mut().events.push(Event::PointerMoved(to));
                    harness.run_steps(1);
                    harness.input_mut().events.push(Event::PointerButton {
                        pos: to,
                        button: PointerButton::Primary,
                        pressed: false,
                        modifiers: Modifiers::NONE,
                    });
                }
            }
            // Every other step queues the next input in the same frame, so it meets a layout
            // the first one made stale.
            if step % 2 == 1 {
                continue;
            }
            harness.run_steps(1);
            let editor = &harness.state().editor;
            let selection = editor.selection();
            assert_eq!(
                editor.document().clamp(selection.focus),
                selection.focus,
                "the caret left the document"
            );
        }
    }
}

#[test]
fn a_composition_ends_when_the_editor_loses_focus() {
    // The IME's commit or cancel then goes elsewhere; typing afterwards must still reach the body.
    let mut harness = harness("<div>a</div>", Shared::default());
    harness
        .input_mut()
        .events
        .push(Event::Ime(ImeEvent::Preedit {
            text: "に".into(),
            active_range_chars: None,
        }));
    harness.run_steps(1);
    click(&mut harness, pos2(390.0, 290.0), 1);
    click(&mut harness, pos2(270.0, 30.0), 1);
    harness.input_mut().events.push(Event::Text("x".into()));
    harness.run_steps(1);
    assert_eq!(harness.state().editor.document().blocks[0].text(), "ax");
}

#[test]
fn copying_up_to_a_table_leaves_the_table_out() {
    let clipboard = Shared::default();
    let mut harness = harness(
        "<div>ab</div><table><tr><td>t</td></tr></table><div>c</div>",
        clipboard.clone(),
    );
    harness
        .state_mut()
        .editor
        .set_selection(Selection::new(Pos::new(0, 1), Pos::new(1, 0)));
    harness.input_mut().events.push(Event::Copy);
    harness.run_steps(1);
    assert_eq!(clipboard.0.borrow().0.as_deref(), Some("b"));
}

#[test]
fn shift_down_after_down_keeps_the_column() {
    // egui reports Shift itself as a key: pressing it before ↓ mustn't forget where the caret
    // came from (Chromium's `x_pos_for_vertical_arrow_navigation`).
    let mut harness = harness(
        "<div>first line here</div><div>short</div><div>third long line here</div>",
        Shared::default(),
    );
    harness
        .state_mut()
        .editor
        .set_selection(Selection::caret(Pos::new(0, 12)));
    key(&mut harness, Key::ArrowDown, Modifiers::NONE);
    assert_eq!(
        harness.state().editor.selection(),
        Selection::caret(Pos::new(1, 5))
    );
    key(&mut harness, Key::ShiftLeft, Modifiers::SHIFT);
    key(&mut harness, Key::ArrowDown, Modifiers::SHIFT);
    assert_eq!(
        harness.state().editor.selection(),
        Selection::new(Pos::new(1, 5), Pos::new(2, 11)),
        "under `first line h|`"
    );
}

#[test]
fn a_caret_in_a_link_shows_its_popover() {
    use egui_kittest::kittest::Queryable as _;
    let mut harness = harness(
        r#"<div>See <a href="https://www.example.com/docs/">example</a> now</div>"#,
        Shared::default(),
    );
    assert!(
        !harness.state().editor.wants_escape(),
        "the caret at the end isn't in the link"
    );
    harness
        .state_mut()
        .editor
        .set_selection(Selection::caret(Pos::new(0, 6)));
    harness.run_steps(2);
    assert!(
        harness.state().editor.wants_escape(),
        "Escape closes the popover first"
    );
    harness.get_by_label("example.com/docs").click();
    harness.run_steps(2);
    assert_eq!(harness.state().opened, ["https://www.example.com/docs/"]);
    key(&mut harness, Key::Escape, Modifiers::NONE);
    harness.run_steps(1);
    assert!(
        harness.query_by_label("Remove link").is_none(),
        "Escape closed it"
    );
    // (egui drops focus on an Escape the editor lets through; compose gives it back.)
    click(&mut harness, pos2(290.0, 30.0), 1);
    harness
        .state_mut()
        .editor
        .set_selection(Selection::caret(Pos::new(0, 7)));
    harness.run_steps(1);
    harness.get_by_label("Remove link").click();
    harness.run_steps(2);
    assert_eq!(
        harness.state().editor.html(),
        "<div>See example now<br></div>"
    );
    assert_eq!(
        harness.state().toolbar,
        [exo_editor::ToolbarAction::LinkRemoved],
        "for its hint"
    );
    assert!(harness.query_by_label("Remove link").is_none());
}
