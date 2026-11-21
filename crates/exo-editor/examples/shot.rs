//! Renders the editor offscreen at 2× where the inline reply's editor sits, for comparing with
//! screenshots.
//!
//! usage: cargo run -p exo-editor --example shot -- <out.png> <body.html> [--select A B] [--placeholder TEXT] [--caret] [--dark]
//!
//! `body.html` is the body to show, a draft's HTML. The window is 1280×800 and the editor sits at
//! x 248.375, y 269, 640px wide: the inline reply's editor in the first thread. `--select A B`
//! selects from character `A` to `B` of the body's text (blocks joined by newlines); without
//! `--caret` the caret is hidden.

use anyhow::Context as _;
use egui::{Color32, Pos2, Rect, UiBuilder, Vec2, pos2, vec2};
use exo_editor::{Editor, EditorStyle, MemoryClipboard, Pos, Selection};

const SIZE: Vec2 = Vec2::new(1280.0, 800.0);
const ORIGIN: Pos2 = pos2(248.375, 269.0);
const WIDTH: f32 = 640.0;

struct Scene {
    editor: Editor,
    style: EditorStyle,
    focus: bool,
    /// Fonts apply from the frame after they're installed, so the first frame installs them and
    /// draws nothing.
    ready: bool,
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let usage =
        "usage: shot <out.png> <body.html> [--select A B] [--placeholder TEXT] [--caret] [--dark]";
    let out = args.next().context(usage)?;
    let body = std::fs::read_to_string(args.next().context(usage)?)?;
    let (mut select, mut caret, mut dark) = (None, false, false);
    let mut style = EditorStyle {
        placeholder: None,
        min_height: 32.0,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--select" => {
                select = Some((
                    args.next().context(usage)?.parse::<usize>()?,
                    args.next().context(usage)?.parse::<usize>()?,
                ))
            }
            "--placeholder" => style.placeholder = Some(args.next().context(usage)?),
            "--caret" => caret = true,
            "--dark" => dark = true,
            other => anyhow::bail!("unknown argument {other}\n{usage}"),
        }
    }
    let mut editor = Editor::from_html(&body);
    editor.set_clipboard(Box::new(MemoryClipboard::default()));
    let end = editor.document().end();
    editor.set_selection(Selection::caret(end));
    if let Some((from, to)) = select {
        let (anchor, focus) = (position(&editor, from), position(&editor, to));
        editor.set_selection(Selection::new(anchor, focus));
    }

    let mut harness = egui_kittest::Harness::builder()
        .with_size(SIZE)
        .with_pixels_per_point(2.0)
        .with_os(egui::os::OperatingSystem::Mac)
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_ui_state(
            |ui, scene: &mut Scene| {
                if !std::mem::replace(&mut scene.ready, true) {
                    exo_ui::install(ui.ctx()).expect("the system fonts load");
                    if dark {
                        exo_ui::set_theme(ui.ctx(), exo_ui::Theme::Carbon);
                    }
                    return;
                }
                let background = if exo_ui::theme(ui.ctx()) == exo_ui::Theme::Carbon {
                    exo_ui::palette(ui.ctx()).background_elevated_default
                } else {
                    Color32::WHITE
                };
                ui.painter().rect_filled(ui.max_rect(), 0.0, background);
                let rect = Rect::from_min_size(ORIGIN, vec2(WIDTH, SIZE.y - ORIGIN.y));
                ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                    let response = scene.editor.ui(ui, &scene.style);
                    if std::mem::take(&mut scene.focus) {
                        response.response.request_focus();
                    }
                });
            },
            Scene {
                editor,
                style,
                focus: true,
                ready: false,
            },
        );
    if !caret {
        harness.ctx.all_styles_mut(|style| {
            style.visuals.text_cursor.blink = true;
            style.visuals.text_cursor.on_duration = 0.0;
        });
    }
    harness.run_steps(4);
    harness.render().map_err(anyhow::Error::msg)?.save(&out)?;
    Ok(())
}

/// The position `index` characters into the body's text, counting a newline between blocks.
fn position(editor: &Editor, mut index: usize) -> Pos {
    for (block, contents) in editor.document().blocks.iter().enumerate() {
        let len = contents.len();
        if index <= len {
            return Pos::new(block, index);
        }
        index -= len + 1;
    }
    editor.document().end()
}
