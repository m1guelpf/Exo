//! Dates detected in the body.

use exo_editor::{ActiveDate, Editor, Pos, Selection, Settings};

fn caret_at(body: &str, block: usize, offset: usize) -> Editor {
    let mut editor = Editor::from_html(body);
    editor.set_selection(Selection::caret(Pos::new(block, offset)));
    editor
}

fn active(body: &str, offset: usize) -> Option<String> {
    caret_at(body, 0, offset)
        .active_date()
        .map(|date| date.text)
}

#[test]
fn the_caret_is_on_a_date_in_it_or_just_after_it() {
    let body = "<div>see you tomorrow at noon</div>";
    // `see you |tomorrow`: the caret between two text nodes is the first's (Chromium's upstream
    // position), so not on the date.
    assert_eq!(active(body, 8), None);
    assert_eq!(active(body, 9).as_deref(), Some("tomorrow"));
    assert_eq!(active(body, 16).as_deref(), Some("tomorrow"), "at its end");
    assert_eq!(
        active(body, 19).as_deref(),
        Some("tomorrow"),
        "three past it"
    );
    assert_eq!(active(body, 20), None, "four past it");
    assert_eq!(
        active("<div>tomorrow</div>", 0).as_deref(),
        Some("tomorrow"),
        "a line's first text is the date's"
    );
}

#[test]
fn a_colon_or_a_trigger_after_the_date_takes_the_caret_off_it() {
    assert_eq!(active("<div>tomorrow :sm</div>", 11), None);
    assert_eq!(active("<div>tomorrow ;a</div>", 10), None);
    assert_eq!(active("<div>tomorrow @a</div>", 10), None);
    assert_eq!(
        active("<div>tomorrow +a</div>", 11).as_deref(),
        Some("tomorrow"),
        "`+` isn't excluded"
    );
}

#[test]
fn dates_are_found_within_one_text_node_and_not_in_links() {
    // `<b>next</b> Friday`: `next  Friday` spans two text nodes, so nothing is wrapped.
    assert_eq!(active("<div><b>next</b> Friday</div>", 10), None);
    assert_eq!(active("<div>next <b>Friday</b></div>", 8), None);
    assert_eq!(
        active("<div>on <b>Friday</b></div>", 6),
        None,
        "`on  Friday` too"
    );
    assert_eq!(
        active("<div>see <a href=\"https://x.co\">Friday</a></div>", 6),
        None
    );
    assert_eq!(
        active("<div>see <b>Friday</b></div>", 6).as_deref(),
        Some("Friday")
    );
    // The text after a date counts only in its own text node.
    assert_eq!(active("<div>Friday<b>!!</b></div>", 7), None);
}

#[test]
fn a_selection_or_a_snippet_has_no_active_date() {
    let mut editor = Editor::from_html("<div>see you tomorrow</div>");
    editor.set_selection(Selection::new(Pos::new(0, 8), Pos::new(0, 12)));
    assert_eq!(editor.active_date(), None);
    editor.set_selection(Selection::caret(Pos::new(0, 12)));
    assert!(editor.active_date().is_some());
    editor.set_settings(Settings {
        skip_dates: true,
        ..Settings::default()
    });
    assert_eq!(editor.active_date(), None, "`skipDateDetection`");
}

#[test]
fn shifting_rewrites_the_date_in_its_marks_and_leaves_the_caret_after_it() {
    let mut editor = caret_at("<div>see you <b>tomorrow</b> then</div>", 0, 12);
    let date = editor.active_date().expect("on the date");
    assert_eq!(
        date,
        ActiveDate {
            block: 0,
            range: 8..16,
            text: "tomorrow".to_owned()
        }
    );
    editor.shift_date(&date, "Thursday");
    assert_eq!(editor.html(), "<div>see you <b>Thursday</b> then<br></div>");
    assert_eq!(editor.selection(), Selection::caret(Pos::new(0, 16)));
    assert_eq!(
        editor.active_date().map(|date| date.text).as_deref(),
        Some("Thursday")
    );
    // As Squire's, the change isn't a step of its own: undo goes back to before the last one.
    editor.type_text("x");
    editor.undo();
    assert_eq!(editor.document().blocks[0].text(), "see you tomorrow then");
}

/// Where the date tooltip's left edge lands for `tomorrow` at the start of a body flush with the
/// window's left (centred under it, it would start left of the window), and for one at the end of
/// a line flush with the window's right, with `settings`.
fn tooltip_sides(settings: Settings) -> (f32, f32) {
    use egui::{Rect, UiBuilder, pos2, vec2};
    let side = |body: &str, offset: usize| {
        let mut editor = caret_at(body, 0, offset);
        editor.set_settings(settings);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(vec2(400.0, 200.0))
            .with_pixels_per_point(2.0)
            .build_ui_state(
                |ui, state: &mut (Editor, bool, f32)| {
                    if !std::mem::replace(&mut state.1, true) {
                        exo_ui::install(ui.ctx()).expect("the system fonts load");
                        return;
                    }
                    ui.scope_builder(
                        UiBuilder::new()
                            .max_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 100.0))),
                        |ui| {
                            let response = state.0.ui(ui, &exo_editor::EditorStyle::default());
                            state.2 = response.response.rect.min.x;
                            response.response.request_focus();
                        },
                    );
                },
                (editor, false, 0.0),
            );
        harness.run_steps(4);
        let id = exo_ui::id(
            &harness.ctx,
            ("exo_editor_date", harness.state().2.to_bits()),
        );
        harness
            .ctx
            .memory(|memory| {
                memory
                    .layer_ids()
                    .find(|layer| layer.id == id)
                    .and_then(|layer| memory.area_rect(layer.id))
            })
            .expect("the tooltip shows")
    };
    let left = side("<div>tomorrow</div>", 8).left();
    let right = side(&format!("<div>{}tomorrow</div>", "xx ".repeat(15)), 53).right();
    (left, 400.0 - right)
}

#[test]
fn the_date_tooltip_keeps_clear_of_the_windows_sides_closer_popped_out() {
    assert_eq!(tooltip_sides(Settings::default()), (22.5, 22.5));
    assert_eq!(
        tooltip_sides(Settings {
            popped_out: true,
            ..Settings::default()
        }),
        (7.5, 22.5),
        "a popped-out draft's, from the left"
    );
    assert_eq!(
        tooltip_sides(Settings {
            another_popped_out: true,
            ..Settings::default()
        }),
        (22.5, 7.5),
        "an inline draft's while another is popped out, from the right"
    );
}
