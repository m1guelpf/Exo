//! Inline images: what the editor asks the app to load, putting images in, their line boxes and
//! carets, focusing one and its resizer, undo, and what goes out.

use egui::{
    Color32, ColorImage, Event, Key, Modifiers, PointerButton, Pos2, Rect, UiBuilder, pos2, vec2,
};
use egui_kittest::Harness;
use exo_editor::{
    Editor, EditorStyle, Fit, ImageLoad, ImageSource, Picture, Pos, Selection, outgoing,
};

/// What the app answers for an image `width`×`height` at `retina` pixels a CSS pixel.
fn loaded(width: usize, height: usize, retina: u32) -> ImageLoad {
    ImageLoad::Loaded(Picture {
        pixels: ColorImage::filled([4, 4], Color32::RED),
        natural: [width, height],
        retina,
        file: Default::default(),
    })
}

fn caret(block: usize, offset: usize) -> Selection {
    Selection::caret(Pos::new(block, offset))
}

/// `Here it is` with a Retina screenshot (800×500 at 2×) put in after it and loaded.
fn with_screenshot() -> Editor {
    let mut editor = Editor::from_html("<div>Here it is</div>");
    editor.set_selection(caret(0, 10));
    editor.insert_image("c1");
    editor.image_requests();
    editor.set_image("cid:c1", loaded(800, 500, 2));
    editor
}

#[test]
fn an_inserted_image_takes_best_fit_once_it_loads_with_the_insert_as_its_undo_step() {
    let mut editor = Editor::from_html("<div>Here it is</div>");
    editor.set_selection(caret(0, 10));
    editor.insert_image("c1");
    assert_eq!(
        editor.html(),
        r#"<div>Here it is<img class="Exo-inline-image" src-cid="c1" style="max-width: 100%; height: auto;" src=""><br></div>"#
    );
    assert_eq!(editor.selection(), caret(0, 11), "the caret goes after it");
    let requests: Vec<ImageSource> = editor
        .image_requests()
        .into_iter()
        .map(|(source, _)| source)
        .collect();
    assert_eq!(requests, [ImageSource::Cid("c1".into())]);
    assert!(editor.image_requests().is_empty(), "each is asked for once");
    editor.set_image("cid:c1", loaded(800, 500, 2));
    let fitted = r#"<div>Here it is<img class="Exo-inline-image" src-cid="c1" style="max-width: 100%; height: auto; width: 400px;" src="" data-retina="2" width="400"><br></div>"#;
    assert_eq!(
        editor.html(),
        fitted,
        "Best Fit's width once the image loads"
    );
    editor.undo();
    assert_eq!(editor.html(), "<div>Here it is<br></div>");
    editor.redo();
    assert_eq!(editor.html(), fitted);
}

#[test]
fn images_are_asked_for_by_source_and_remote_ones_again_when_reloaded() {
    let mut editor = Editor::from_html(
        r#"<div><img src="cid:ii_x"><img src-cid="a@b" src="cid:a@b"><img src="https://x.io/logo.png"><img src="data:image/gif;base64,R0lG"><img src=""></div>"#,
    );
    let keys: Vec<String> = editor
        .image_requests()
        .into_iter()
        .map(|(source, _)| source.key())
        .collect();
    assert_eq!(
        keys,
        [
            "cid:ii_x",
            "cid:a@b",
            "https://x.io/logo.png",
            "data:image/gif;base64,R0lG"
        ]
    );
    editor.set_image("https://x.io/logo.png", ImageLoad::Blocked);
    editor.reload_remote_images();
    let again: Vec<String> = editor
        .image_requests()
        .into_iter()
        .map(|(source, _)| source.key())
        .collect();
    assert_eq!(again, ["https://x.io/logo.png"]);
}

#[test]
fn a_revealed_signature_and_quote_show_their_images() {
    let mut editor = Editor::new();
    let signature = outgoing::signature(
        Some(r#"<div>Maya<br><img src="data:image/png;base64,iVBO" width="88" height="31"></div>"#),
        true,
        Some("d1"),
    );
    let quote = r#"<div class="gmail_quote">On Tue, Tom wrote:<blockquote><img src="cid:chart@x" src-cid="chart@x" class="Exo-inline-image"></blockquote></div>"#;
    editor.reveal_quoted(Some(&signature), Some(quote));
    let keys: Vec<String> = editor
        .image_requests()
        .into_iter()
        .map(|(source, _)| source.key())
        .collect();
    assert_eq!(
        keys,
        ["data:image/png;base64,iVBO", "cid:chart@x"],
        "the signature's logo and the quote's inline image"
    );
}

#[test]
fn arrows_focus_an_image_and_step_past_it() {
    let mut editor = with_screenshot();
    assert_eq!(editor.focused_image(), None);
    editor.press(Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(
        editor.focused_image(),
        Some(Pos::new(0, 10)),
        "← beside an image focuses it"
    );
    assert_eq!(
        editor.selection(),
        Selection::new(Pos::new(0, 10), Pos::new(0, 11)),
        "selected whole"
    );
    editor.press(Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(
        (editor.focused_image(), editor.selection()),
        (None, caret(0, 10)),
        "← again: before it"
    );
    editor.press(Key::ArrowRight, Modifiers::NONE);
    assert_eq!(editor.focused_image(), Some(Pos::new(0, 10)));
    editor.press(Key::End, Modifiers::NONE);
    assert_eq!(
        (editor.focused_image(), editor.selection()),
        (None, caret(0, 11)),
        "End: after it"
    );
    editor.press(Key::ArrowLeft, Modifiers::NONE);
    editor.press(Key::ArrowLeft, Modifiers::NONE);
    editor.press(Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(
        editor.selection(),
        caret(0, 9),
        "past it, the caret moves on through the text"
    );
}

#[test]
fn a_focused_image_is_deleted_or_replaced_by_what_is_typed() {
    let mut editor = with_screenshot();
    editor.press(Key::ArrowLeft, Modifiers::NONE);
    editor.press(Key::Backspace, Modifiers::NONE);
    assert_eq!(editor.html(), "<div>Here it is<br></div>");
    editor.undo();
    assert!(
        editor.html().contains("<img"),
        "one undo step brings it back"
    );
    editor.set_selection(caret(0, 11));
    editor.press(Key::ArrowLeft, Modifiers::NONE);
    editor.type_text("!");
    assert_eq!(
        editor.html(),
        "<div>Here it is!<br></div>",
        "a letter replaces it"
    );
}

#[test]
fn small_and_best_fit_resize_with_an_undo_step_each() {
    let mut editor = with_screenshot();
    editor.press(Key::ArrowLeft, Modifiers::NONE);
    editor.fit_image(Fit::Small);
    let html = editor.html();
    assert!(
        html.contains(r#"style="max-width: 100%; height: 125px; width: 200px;""#)
            && html.contains(r#"width="200" height="125" data-resized="true""#),
        "{html}"
    );
    assert_eq!(
        editor.focused_image(),
        Some(Pos::new(0, 10)),
        "it stays focused"
    );
    editor.fit_image(Fit::BestFit);
    assert!(editor.html().contains("height: 250px; width: 400px;"));
    editor.undo();
    assert!(
        editor.html().contains("height: 125px; width: 200px;"),
        "undo goes back to Small"
    );
}

#[test]
fn a_blocked_image_cant_be_focused() {
    let mut editor =
        Editor::from_html(r#"<div>a<img src="https://x.io/a.png" width="40" height="40"></div>"#);
    editor.image_requests();
    editor.set_image("https://x.io/a.png", ImageLoad::Blocked);
    editor.set_selection(caret(0, 2));
    editor.press(Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(
        (editor.focused_image(), editor.selection()),
        (None, caret(0, 1)),
        "the caret moves over it"
    );
}

#[test]
fn it_goes_out_by_content_id_keeping_src_cid() {
    let editor = with_screenshot();
    let sent = outgoing::html(
        editor.document(),
        &outgoing::Parts {
            inlined: true,
            ..outgoing::Parts::default()
        },
    );
    assert!(sent.contains(r#"<img class="" src-cid="c1" style="max-width: 100%; height: auto; width: 400px;" src="cid:c1" data-retina="2" width="400">"#), "{sent}");
    let quote = r#"<div class="gmail_quote"><img src="cid:logo@x" src-cid="logo@x" class="Exo-inline-image"></div>"#;
    let reply = outgoing::html(
        editor.document(),
        &outgoing::Parts {
            quote: Some(quote),
            ..outgoing::Parts::default()
        },
    );
    assert!(
        reply.contains(r#"<img src="cid:logo@x" src-cid="logo@x" class="">"#),
        "the quote's images too: {reply}"
    );
}

// The widget.

const ORIGIN: Pos2 = pos2(20.0, 20.0);

struct State {
    editor: Editor,
    ready: bool,
    caret: Rect,
    height: f32,
}

fn harness(editor: Editor) -> Harness<'static, State> {
    harness_sized(editor, 400.0)
}

/// [`harness`] in a window `height` tall.
fn harness_sized(editor: Editor, height: f32) -> Harness<'static, State> {
    let mut harness = Harness::builder()
        .with_size(vec2(720.0, height))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            move |ui, state: &mut State| {
                if !std::mem::replace(&mut state.ready, true) {
                    exo_ui::install(ui.ctx()).expect("the system fonts load");
                    return;
                }
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(Rect::from_min_size(ORIGIN, vec2(640.0, height - 40.0))),
                    |ui| {
                        let response = state.editor.ui(ui, &EditorStyle::default());
                        state.caret = response.caret;
                        state.height = response.response.rect.height();
                        response.response.request_focus();
                    },
                );
            },
            State {
                editor,
                ready: false,
                caret: Rect::NOTHING,
                height: 0.0,
            },
        );
    harness.run_steps(3);
    harness
}

fn press(harness: &mut Harness<'_, State>, at: Pos2) {
    harness.input_mut().events.push(Event::PointerMoved(at));
    harness.input_mut().events.push(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(1);
}

fn release(harness: &mut Harness<'_, State>, at: Pos2) {
    harness.input_mut().events.push(Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(1);
}

#[test]
fn an_image_grows_its_line_box_and_sits_on_the_baseline() {
    // `Here it is` and a 400×250 image make a 259px line (the image above the baseline, the text's
    // 9px of descent and leading below), the text's caret on that baseline, and the caret after the
    // image (no text beside it) the line's whole height.
    let mut harness = harness(with_screenshot());
    assert_eq!(harness.state().height, 259.0);
    assert_eq!(
        harness.state().caret,
        Rect::from_min_size(pos2(ORIGIN.x + 456.0, ORIGIN.y), vec2(1.0, 259.0)),
        "{:?}",
        harness.state().caret
    );
    harness.state_mut().editor.set_selection(caret(0, 4));
    harness.run_steps(1);
    let text = harness.state().caret;
    assert_eq!((text.top(), text.height()), (ORIGIN.y + 238.0, 17.0));
}

#[test]
fn clicking_an_image_focuses_it_and_its_handle_drags_the_width() {
    let mut harness = harness(with_screenshot());
    // The image spans x 55.76–455.76 (after `Here it is`) and y 0–250 of the editor.
    let center = ORIGIN + vec2(256.0, 125.0);
    press(&mut harness, center);
    release(&mut harness, center);
    assert_eq!(
        harness.state().editor.focused_image(),
        Some(Pos::new(0, 10))
    );
    let corner = ORIGIN + vec2(456.0, 250.0);
    press(&mut harness, corner);
    for step in 1..=4 {
        harness
            .input_mut()
            .events
            .push(Event::PointerMoved(corner - vec2(25.0 * step as f32, 0.0)));
        harness.run_steps(1);
    }
    release(&mut harness, corner - vec2(100.0, 0.0));
    let html = harness.state().editor.html();
    // The width is the pointer's distance from the image's left edge.
    assert!(
        html.contains(r#"width="300" height="187" data-resized="true""#),
        "dragged 100px narrower: {html}"
    );
    assert_eq!(
        harness.state().editor.focused_image(),
        Some(Pos::new(0, 10))
    );
    harness.state_mut().editor.undo();
    assert!(
        harness
            .state()
            .editor
            .html()
            .contains("height: auto; width: 400px;"),
        "the drag is one undo step"
    );
}

#[test]
fn a_block_image_alone_is_as_tall_as_it_is() {
    // `display: block` makes a block box: no strut's descent under it, as an inline image has.
    let height = |style: &str| {
        let mut editor = Editor::from_html(&format!(
            r#"<div><img src="cid:logo" width="100" height="50"{style}></div>"#
        ));
        editor.image_requests();
        editor.set_image("cid:logo", loaded(100, 50, 1));
        harness(editor).state().height
    };
    assert_eq!(height(r#" style="display:block""#), 50.0);
    assert_eq!(
        height(""),
        59.0,
        "an inline one keeps the text's 9px below the baseline"
    );
}

// Images in the tables the editor keeps whole, drawn by Blitz.

/// A `width`×`height` PNG filled with `rgb`.
fn png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut out);
    image::ImageEncoder::write_image(
        encoder,
        &[rgb[0], rgb[1], rgb[2], 255].repeat((width * height) as usize),
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )
    .expect("encoded");
    out
}

/// Steps `harness` until `done` holds of it (a table's images decode on a thread of their own).
fn wait_for(harness: &mut Harness<'_, State>, done: impl Fn(&mut Harness<'_, State>) -> bool) {
    for _ in 0..200 {
        if done(harness) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        harness.run_steps(1);
    }
    panic!("timed out");
}

#[test]
fn a_kept_tables_images_load_as_the_bodys_and_draw_in_it() {
    // A table-built signature: its logo by Content-ID (as Exo puts images in), a remote icon.
    let signature = r#"<div>Hi</div><table><tbody><tr><td><img src="" src-cid="logo" width="40" height="20"></td><td>Maya Raman<br>CEO</td></tr><tr><td><img src="https://x.io/in.png" width="16" height="16"></td></tr></tbody></table><div><br></div>"#;
    let mut harness = harness(Editor::from_html(signature));
    let requests: Vec<String> = harness
        .state_mut()
        .editor
        .image_requests()
        .into_iter()
        .map(|(source, element)| {
            format!(
                "{} {}",
                source.key(),
                element.attr("width").unwrap_or_default()
            )
        })
        .collect();
    assert_eq!(
        requests,
        ["cid:logo 40", "https://x.io/in.png 16"],
        "asked for as the body's images are, each with its <img>"
    );
    let before = harness.state().height;
    assert!(
        before > 24.0 + 24.0 + 40.0,
        "the table laid out as a table, not a line of its text: {before}"
    );
    let file = png(40, 20, [255, 0, 0]);
    let picture = Picture {
        pixels: ColorImage::filled([40, 20], Color32::RED),
        natural: [40, 20],
        retina: 1,
        file: file.into(),
    };
    harness
        .state_mut()
        .editor
        .set_image("cid:logo", ImageLoad::Loaded(picture));
    harness
        .state_mut()
        .editor
        .set_image("https://x.io/in.png", ImageLoad::Broken);
    let red_at = |harness: &mut Harness<'_, State>| {
        let image = harness.render().expect("renders");
        image
            .save(std::env::temp_dir().join("exo-kept-table.png"))
            .ok();
        let scale = image.width() as f32 / 720.0;
        // The logo, in the table's first cell (after the first line), centred down its row.
        let pixel = image.get_pixel(
            ((ORIGIN.x + 20.0) * scale) as u32,
            ((ORIGIN.y + 24.0 + 23.0) * scale) as u32,
        );
        pixel[0] > 200 && pixel[1] < 60 && pixel[2] < 60
    };
    wait_for(&mut harness, red_at);
    assert_eq!(
        harness.state().height,
        before,
        "its box came from its attributes"
    );
}

#[test]
fn a_kept_tables_remote_images_show_blocked_under_the_images_setting() {
    let mut editor = Editor::from_html(
        r#"<table><tbody><tr><td><img src="https://x.io/logo.png" width="40" height="40"></td></tr></tbody></table>"#,
    );
    let mut harness = harness(editor);
    harness.state_mut().editor.image_requests();
    harness
        .state_mut()
        .editor
        .set_image("https://x.io/logo.png", ImageLoad::Blocked);
    harness.run_steps(2);
    let image = harness.render().expect("renders");
    let scale = image.width() as f32 / 720.0;
    // The blocked image's box, from the cell's top left (2px of spacing, 1px of padding): its 1px
    // border `rgba(0, 0, 0, .2)` inside it, darker than its middle.
    let pixel = |x: f32, y: f32| {
        image
            .get_pixel(
                ((ORIGIN.x + x) * scale) as u32,
                ((ORIGIN.y + y) * scale) as u32,
            )
            .0
    };
    assert!(
        pixel(3.5, 20.0)[0] < pixel(20.0, 20.0)[0],
        "a border: {:?} {:?}",
        pixel(3.5, 20.0),
        pixel(20.0, 20.0)
    );
    editor = std::mem::take(&mut harness.state_mut().editor);
    editor.reload_remote_images();
    let again: Vec<String> = editor
        .image_requests()
        .into_iter()
        .map(|(source, _)| source.key())
        .collect();
    assert_eq!(
        again,
        ["https://x.io/logo.png"],
        "Show Images asks for it again"
    );
}

#[test]
fn a_kept_tables_css_images_are_asked_for_by_url() {
    let mut harness = harness(Editor::from_html(
        r#"<table><tbody><tr><td style="background-image:url(https://x.io/bg.png);width:40px;height:40px">x</td></tr></tbody></table>"#,
    ));
    harness.run_steps(2);
    let keys: Vec<String> = harness
        .state_mut()
        .editor
        .image_requests()
        .into_iter()
        .map(|(source, _)| source.key())
        .collect();
    assert_eq!(
        keys,
        ["https://x.io/bg.png"],
        "what the table's CSS loads, once its document asks"
    );
}

#[test]
fn a_tall_kept_table_is_painted_in_bands_where_it_shows() {
    // Taller than a band (1024px) in a window that shows 1400px of it: rows past the first
    // band draw too.
    let rows: String = (0..60)
        .map(|row| format!("<tr><td>row {row}</td></tr>"))
        .collect();
    let mut harness = harness_sized(
        Editor::from_html(&format!("<table><tbody>{rows}</tbody></table>")),
        1440.0,
    );
    assert!(
        harness.state().height > 1400.0,
        "{}",
        harness.state().height
    );
    let image = harness.render().expect("renders");
    let scale = image.width() as f32 / 720.0;
    let ink = |y: f32| {
        (0..60).any(|x| {
            image
                .get_pixel(((ORIGIN.x + x as f32) * scale) as u32, (y * scale) as u32)
                .0[0]
                < 100
        })
    };
    assert!(
        (0..24).any(|dy| ink(ORIGIN.y + 4.0 + dy as f32)),
        "its first row"
    );
    assert!(
        (0..24).any(|dy| ink(ORIGIN.y + 1100.0 + dy as f32)),
        "a row in its second band"
    );
}
