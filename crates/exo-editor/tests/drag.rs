//! Dragging the selection within the body, as Chromium does on a Mac, and across the editor's edge
//! with the Mac's drag (`exo_ui::text_drag`, its bridge played here).

use egui::{Event, Id, Key, Modifiers, PointerButton, Pos2, Rect, UiBuilder, pos2, vec2};
use egui_kittest::Harness;
use exo_editor::{Editor, EditorStyle, Pos, Selection};
use exo_ui::text_drag::{self, Dropped, Over, Report};

const ORIGIN: Pos2 = pos2(20.0, 20.0);
/// The middle of `alpha` in `alpha beta gamma`, and past the line's end.
const ALPHA: Pos2 = pos2(ORIGIN.x + 15.0, 30.0);
const END: Pos2 = pos2(ORIGIN.x + 250.0, 30.0);

struct State {
    editor: Editor,
    ready: bool,
    /// The bridge: whether there's a Mac drag to hand a selection to, and what it says next frame.
    native: bool,
    over: Option<Over>,
    dropped: Option<Dropped>,
    ended: Option<(Id, bool)>,
    /// What the editor left for it, the last frame anything was.
    report: Report,
}

fn harness(body: &str) -> Harness<'static, State> {
    let mut harness = Harness::builder()
        .with_size(vec2(400.0, 300.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            move |ui, state: &mut State| {
                if !std::mem::replace(&mut state.ready, true) {
                    exo_ui::install(ui.ctx()).expect("the system fonts load");
                    return;
                }
                text_drag::begin(
                    ui.ctx(),
                    state.native,
                    state.over,
                    state.dropped.take(),
                    state.ended.take(),
                );
                ui.scope_builder(
                    UiBuilder::new().max_rect(Rect::from_min_size(ORIGIN, vec2(300.0, 200.0))),
                    |ui| {
                        state.editor.ui(ui, &EditorStyle::default());
                    },
                );
                let report = text_drag::end(ui.ctx());
                if report.outgoing.is_some() || report.taken.is_some() {
                    state.report = report;
                }
            },
            State {
                editor: Editor::from_html(body),
                ready: false,
                native: false,
                over: None,
                dropped: None,
                ended: None,
                report: Report::default(),
            },
        );
    harness.run_steps(2);
    harness
}

fn button(harness: &mut Harness<'_, State>, at: Pos2, pressed: bool, modifiers: Modifiers) {
    harness.input_mut().events.push(Event::PointerMoved(at));
    harness.input_mut().events.push(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers,
    });
    harness.run_steps(1);
}

fn double_click(harness: &mut Harness<'_, State>, at: Pos2) {
    for _ in 0..2 {
        button(harness, at, true, Modifiers::NONE);
        button(harness, at, false, Modifiers::NONE);
    }
    // Long enough that the next press isn't a third click.
    harness.run_steps(40);
}

/// Presses at `from`, moves to `to` in steps, and lets go there with `modifiers`.
fn drag(harness: &mut Harness<'_, State>, from: Pos2, to: Pos2, modifiers: Modifiers) {
    button(harness, from, true, Modifiers::NONE);
    harness
        .input_mut()
        .events
        .push(Event::ModifiersChanged(modifiers));
    for step in 1..=5 {
        harness
            .input_mut()
            .events
            .push(Event::PointerMoved(from + (to - from) * step as f32 / 5.0));
        harness.run_steps(1);
    }
    button(harness, to, false, modifiers);
}

fn text(harness: &Harness<'_, State>) -> String {
    harness.state().editor.document().blocks[0].text()
}

#[test]
fn a_word_dragged_moves_with_smart_spaces_and_stays_selected() {
    let mut harness = harness("<div>alpha beta gamma</div>");
    double_click(&mut harness, ALPHA);
    assert_eq!(harness.state().editor.selected_text(), "alpha");
    drag(&mut harness, ALPHA, END, Modifiers::NONE);
    assert_eq!(
        text(&harness),
        "beta gamma alpha",
        "its space goes with it, one goes before it"
    );
    assert_eq!(harness.state().editor.selected_text(), "alpha");
    harness.state_mut().editor.undo();
    assert_eq!(text(&harness), "alpha beta gamma", "one undo step");
}

#[test]
fn a_selection_made_by_keys_moves_without_spaces_put_in() {
    let mut harness = harness("<div>alpha beta gamma</div>");
    harness
        .state_mut()
        .editor
        .set_selection(Selection::new(Pos::new(0, 0), Pos::new(0, 5)));
    drag(&mut harness, ALPHA, END, Modifiers::NONE);
    assert_eq!(text(&harness), "beta gammaalpha");
}

#[test]
fn option_copies_and_a_drop_on_the_selection_does_nothing() {
    let mut harness = harness("<div>alpha beta gamma</div>");
    double_click(&mut harness, ALPHA);
    drag(&mut harness, ALPHA, END, Modifiers::ALT);
    assert_eq!(text(&harness), "alpha beta gamma alpha");
    harness.run_steps(40);
    let mut harness = self::harness("<div>alpha beta gamma</div>");
    harness
        .state_mut()
        .editor
        .set_selection(Selection::new(Pos::new(0, 0), Pos::new(0, 10)));
    drag(
        &mut harness,
        ALPHA,
        pos2(ALPHA.x + 30.0, ALPHA.y),
        Modifiers::NONE,
    );
    assert_eq!(text(&harness), "alpha beta gamma");
    assert_eq!(harness.state().editor.selected_text(), "alpha beta");
}

#[test]
fn a_click_on_the_selection_puts_the_caret_there_and_escape_cancels_a_drag() {
    let mut harness = harness("<div>alpha beta gamma</div>");
    harness
        .state_mut()
        .editor
        .set_selection(Selection::new(Pos::new(0, 0), Pos::new(0, 10)));
    button(&mut harness, ALPHA, true, Modifiers::NONE);
    assert_eq!(
        harness.state().editor.selected_text(),
        "alpha beta",
        "the press keeps it"
    );
    button(&mut harness, ALPHA, false, Modifiers::NONE);
    assert!(harness.state().editor.selection().is_collapsed());
    harness.run_steps(40);
    harness
        .state_mut()
        .editor
        .set_selection(Selection::new(Pos::new(0, 0), Pos::new(0, 5)));
    button(&mut harness, ALPHA, true, Modifiers::NONE);
    harness.input_mut().events.push(Event::PointerMoved(END));
    harness.run_steps(1);
    harness.input_mut().events.push(Event::Key {
        key: Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(1);
    button(&mut harness, END, false, Modifiers::NONE);
    assert_eq!(text(&harness), "alpha beta gamma");
}

#[test]
fn a_word_dropped_before_itself_and_into_another_line() {
    let mut harness = harness("<div>alpha beta gamma</div><div>delta</div>");
    let gamma = pos2(ORIGIN.x + 95.0, 30.0);
    double_click(&mut harness, gamma);
    assert_eq!(harness.state().editor.selected_text(), "gamma");
    drag(
        &mut harness,
        gamma,
        pos2(ORIGIN.x + 1.0, 30.0),
        Modifiers::NONE,
    );
    assert_eq!(text(&harness), "gamma alpha beta");
    harness.run_steps(40);
    // The second line's end: the selection the drop left is by characters, so nothing is put
    // between.
    drag(
        &mut harness,
        pos2(ORIGIN.x + 15.0, 30.0),
        pos2(ORIGIN.x + 250.0, 54.0),
        Modifiers::NONE,
    );
    let blocks: Vec<String> = harness
        .state()
        .editor
        .document()
        .blocks
        .iter()
        .map(|block| block.text())
        .collect();
    assert_eq!(blocks, ["alpha beta", "deltagamma"]);
}

/// Off the editor, right of it.
const OUTSIDE: Pos2 = pos2(ORIGIN.x + 340.0, 30.0);

/// Double-clicks `alpha` and drags it out of the editor, the button still down: the editor's id,
/// from what it handed the bridge.
fn drag_alpha_out(harness: &mut Harness<'_, State>) -> Id {
    harness.state_mut().native = true;
    double_click(harness, ALPHA);
    button(harness, ALPHA, true, Modifiers::NONE);
    for step in 1..=5 {
        harness.input_mut().events.push(Event::PointerMoved(
            ALPHA + (OUTSIDE - ALPHA) * step as f32 / 5.0,
        ));
        harness.run_steps(1);
    }
    let outgoing = harness
        .state()
        .report
        .outgoing
        .clone()
        .expect("handed to the Mac's drag once out of the editor");
    assert_eq!(outgoing.text, "alpha");
    assert!(outgoing.html.contains("alpha"), "{}", outgoing.html);
    let (image, at) = outgoing.image.expect("a picture of the text");
    assert!(
        image.size[0] > 0 && image.pixels.iter().any(|pixel| pixel.a() > 0),
        "the text is drawn in it"
    );
    assert!(
        at.min.x > ALPHA.x - 20.0,
        "where the text is now, moved with the pointer: {at:?}"
    );
    outgoing.source
}

#[test]
fn a_selection_moved_out_to_another_place_in_the_window_goes_from_here_in_one_undo_step() {
    let mut harness = harness("<div>alpha beta gamma</div>");
    let source = drag_alpha_out(&mut harness);
    harness.state_mut().ended = Some((source, true));
    harness.run_steps(1);
    assert_eq!(
        text(&harness),
        "beta gamma",
        "a smart delete, its space with it"
    );
    harness.state_mut().editor.undo();
    assert_eq!(text(&harness), "alpha beta gamma");
}

#[test]
fn a_selection_copied_out_or_dropped_nowhere_stays() {
    let mut harness = harness("<div>alpha beta gamma</div>");
    let source = drag_alpha_out(&mut harness);
    harness.state_mut().ended = Some((source, false));
    harness.run_steps(1);
    button(&mut harness, OUTSIDE, false, Modifiers::NONE);
    assert_eq!(text(&harness), "alpha beta gamma");
    assert_eq!(harness.state().editor.selected_text(), "alpha");
    // Over, a press puts the caret down again.
    button(&mut harness, END, true, Modifiers::NONE);
    button(&mut harness, END, false, Modifiers::NONE);
    assert!(harness.state().editor.selection().is_collapsed());
}

#[test]
fn a_selection_dragged_out_and_back_moves_as_inside() {
    let mut harness = harness("<div>alpha beta gamma</div>");
    let source = drag_alpha_out(&mut harness);
    harness.state_mut().over = Some(Over {
        at: END,
        source: Some(source),
    });
    harness.run_steps(1);
    harness.state_mut().over = None;
    harness.state_mut().dropped = Some(Dropped {
        at: END,
        html: Some("<b>alpha</b>".into()),
        text: "alpha".into(),
        source: Some(source),
        copy: false,
    });
    harness.run_steps(1);
    assert_eq!(harness.state().report.taken, Some(source));
    harness.state_mut().ended = Some((source, false));
    harness.run_steps(1);
    assert_eq!(
        text(&harness),
        "beta gamma alpha",
        "moved, with smart spaces, its own runs and not the drag's HTML"
    );
    assert_eq!(harness.state().editor.selected_text(), "alpha");
}

#[test]
fn text_from_outside_goes_in_where_it_is_let_go_selected_in_one_undo_step() {
    let mut harness = harness("<div>alpha beta gamma</div>");
    harness.state_mut().native = true;
    harness.state_mut().dropped = Some(Dropped {
        at: END,
        html: None,
        text: " delta\n".into(),
        source: None,
        copy: true,
    });
    harness.run_steps(1);
    assert_eq!(
        text(&harness),
        "alpha beta gamma delta",
        "as plain text pastes, less its last newline"
    );
    assert_eq!(harness.state().editor.selected_text(), " delta");
    harness.state_mut().editor.undo();
    assert_eq!(text(&harness), "alpha beta gamma");
    // HTML is cleaned as a paste's: its bold kept, its font not.
    harness.state_mut().dropped = Some(Dropped {
        at: ALPHA,
        html: Some("<span style=\"font-family: Times\"><b>bold</b></span>".into()),
        text: "bold".into(),
        source: None,
        copy: true,
    });
    harness.run_steps(1);
    assert_eq!(text(&harness), "alboldpha beta gamma");
    let html = harness.state().editor.html();
    assert!(
        html.contains("<b>bold</b>") && !html.contains("Times"),
        "{html}"
    );
    // Let go off the editor, nothing goes in.
    harness.state_mut().dropped = Some(Dropped {
        at: OUTSIDE,
        html: None,
        text: "x".into(),
        source: None,
        copy: true,
    });
    harness.run_steps(1);
    assert_eq!(text(&harness), "alboldpha beta gamma");
}
