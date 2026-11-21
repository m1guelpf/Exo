//! Renders the command system's overlays offscreen at 2×, for screenshot comparisons.
//!
//! usage: cargo run -p exo-commands --example shot -- <scene> <out.png> [backdrop.png]
//!
//! Scenes: `palette`, `palette-query` (`mark` typed), `palette-nomatch`, `label` (the Label
//! sub-palette), `sheet`, `sheet-mark`, `toast`, `toast-hover`, `hint`. The window is 1280×800
//! with the 41px tab bar inset; `backdrop` (a 2× screenshot of the same mailbox without the
//! overlay) is drawn underneath so shadows and the dimmed backdrop compare too.
//!
//! The rows are fixtures for the mock mailbox's first conversation, so every render lists the same
//! commands; they're not what Exo registers.

use anyhow::Context as _;
use egui::{
    Color32, ColorImage, Event, Key, Modifiers, Pos2, Rect, TextureHandle, TextureOptions, Vec2,
};
use exo_commands::{CONVERSATION, Command, Commands, SubPalette, Toast};

const SIZE: Vec2 = Vec2::new(1280.0, 800.0);
const TAB_BAR: f32 = 41.0;

#[derive(Clone, Debug)]
enum Action {
    Row,
}

struct Scene {
    commands: Commands<Action>,
    backdrop: Option<ColorImage>,
    /// The backdrop as textures (each at most 2048px wide) and where they go.
    tiles: Vec<(TextureHandle, Rect)>,
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: shot <scene> <out.png> [backdrop.png]";
    let scene = args.next().context(usage)?;
    let out = args.next().context(usage)?;
    let backdrop = args.next().map(|path| load(&path)).transpose()?;

    let mut harness = egui_kittest::Harness::builder()
        .with_size(SIZE)
        .with_pixels_per_point(2.0)
        .with_os(egui::os::OperatingSystem::Mac)
        // 60 frames a second, so 8s toasts and 10s hints are still up when we capture.
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_ui_state(
            |ui, scene: &mut Scene| frame(ui, scene),
            Scene {
                commands: Commands::new(),
                backdrop,
                tiles: Vec::new(),
            },
        );
    exo_ui::install(&harness.ctx)?;
    // Screenshots hide the caret: blink it with no "on" phase.
    harness.ctx.all_styles_mut(|style| {
        style.visuals.text_cursor.blink = true;
        style.visuals.text_cursor.on_duration = 0.0;
    });
    harness.run_steps(2);

    match scene.as_str() {
        "palette" => open_palette(&mut harness),
        "palette-query" => {
            open_palette(&mut harness);
            type_text(&mut harness, "mark");
        }
        "palette-nomatch" => {
            open_palette(&mut harness);
            type_text(&mut harness, "zqx");
        }
        "label" => {
            harness
                .state_mut()
                .commands
                .open_sub_palette(label_palette());
            harness.run_steps(2);
        }
        "sheet" => {
            harness.state_mut().commands.open_shortcuts();
            harness.run_steps(2);
        }
        "sheet-mark" => {
            harness.state_mut().commands.open_shortcuts();
            harness.run_steps(2);
            type_text(&mut harness, "mark");
        }
        "toast" | "toast-hover" => {
            harness
                .state_mut()
                .commands
                .toast(Toast::new("Marked as Done.").undo(Action::Row));
            harness.run_steps(2);
            if scene == "toast-hover" {
                // Over the Undo action, long enough for the tooltip's delay and fade.
                // The tooltip shows after 150ms and the shadow eases for 400ms.
                harness.hover_at(Pos2::new(238.0, 744.0));
                harness.run_steps(60);
            }
        }
        "hint" => {
            let hint = exo_commands::Hint::new()
                .text("Next time, hit")
                .keys(["Z"])
                .text("to undo instantly!");
            harness.state_mut().commands.hint(hint);
            // Past the 500ms fade in.
            harness.run_steps(40);
        }
        other => anyhow::bail!("unknown scene {other:?}; {usage}"),
    }
    // Let animations (the sheet's fade, highlights) settle.
    harness.run_steps(30);
    if scene != "toast-hover" {
        harness.remove_cursor();
        harness.run_steps(2);
    }
    harness.render().map_err(anyhow::Error::msg)?.save(&out)?;
    Ok(())
}

fn frame(ui: &mut egui::Ui, scene: &mut Scene) {
    let ctx = ui.ctx().clone();
    scene.commands.dispatch(&ctx);
    let screen = Rect::from_min_size(Pos2::ZERO, SIZE);
    match scene.backdrop.take() {
        Some(image) => scene.tiles = tiles(&ctx, &image),
        None if scene.tiles.is_empty() => {
            ui.painter()
                .rect_filled(screen, 0.0, exo_ui::palette(&ctx).background_base_default);
        }
        None => {}
    }
    for (texture, rect) in &scene.tiles {
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        ui.painter().image(texture.id(), *rect, uv, Color32::WHITE);
    }
    for command in fixture_rows() {
        scene.commands.add(command);
    }
    let area = Rect::from_min_max(Pos2::new(0.0, TAB_BAR), screen.max);
    scene.commands.show(&ctx, area);
}

/// What the palette lists for the mock mailbox's first conversation, in its order.
fn fixture_rows() -> Vec<Command<Action>> {
    let row = |id: &'static str, title: &'static str, icon: &'static str, score: f64| {
        Command::new(id, title, Action::Row)
            .icon(icon)
            .default_score(score)
            .regions(CONVERSATION)
            .display_only()
    };
    vec![
        row("MARK_DONE", "Mark Done", "data/done", 11.0)
            .key("e")
            .aliases(["Archive"]),
        row("SNOOZE", "Remind Me", "data/snooze", 10.0)
            .key("h")
            .aliases(["Snooze", "Boomerang", "Add Reminder"])
            .scale(1.01),
        row("STAR", "Star", "data/star", 9.0)
            .key("s")
            .aliases(["Flag"]),
        row("MOVE", "Move", "data/move", 8.0)
            .key("v")
            .aliases(["Folder", "File"])
            .operative_word("move"),
        row("LABEL", "Label", "data/label", 7.0).key("l").aliases([
            "Add Label",
            "Remove Label",
            "Create Label",
            "New Label",
            "Tag",
            "Create Tag",
            "Remove Tag",
        ]),
        row("REMOVE_FROM_LABEL", "Remove from Label", "data/label", 7.0)
            .key("y")
            .aliases(["Remove from List"]),
        row("REMOVE_ALL_LABELS", "Remove all Labels", "data/label", 6.9)
            .key_code(["shift", "Y"])
            .aliases([
                "Remove all Labels and Mark Done",
                "Remove all Labels and Archive",
            ]),
        row("TRASH", "Trash", "data/discard", 6.0)
            .key("#")
            .aliases(["Delete"]),
        row("MARK_SPAM", "Mark Spam", "data/spam", 5.0)
            .key("!")
            .aliases(["Mark Junk"]),
        row("OPEN", "Open Link", "data/open-links", 4.4)
            .key_code(["⌘", "O"])
            .aliases(["Open Attachment", "Open File"]),
        row("MARK_UNREAD", "Mark Read", "data/mark-unread", 4.0)
            .key("u")
            .aliases(["Mark as Read", "Toggle Read"]),
        row(
            "SHOW_MARK_DONE_ACHIEVEMENTS",
            "Show Mark Done Achievements",
            "data/trophy",
            0.0,
        ),
        row(
            "TOGGLE_SEND_AND_MARK_DONE",
            "Enable Send + Mark Done",
            "data/settings",
            0.0,
        ),
        row(
            "TOGGLE_RSVP_AND_MARK_DONE",
            "Enable RSVP + Mark Done",
            "data/settings",
            0.0,
        ),
    ]
}

/// The Label sub-palette with the mock mailbox's labels, in alphabetical order.
fn label_palette() -> SubPalette<Action> {
    let labels = [
        "Clients",
        "Clients/Calloway",
        "Clients/Harborlight",
        "Family",
        "Hiring",
        "Newsletters",
        "Projects",
        "Projects/Atlas Launch",
        "Projects/Exo",
        "Read later",
    ];
    let rows = labels
        .iter()
        .map(|label| Command::new(*label, *label, Action::Row))
        .collect();
    SubPalette::new("LABEL", "Add label", "data/label", rows)
}

fn open_palette(harness: &mut egui_kittest::Harness<'_, Scene>) {
    // What egui-winit reports for ⌘ on a Mac.
    harness.key_press_modifiers(Modifiers::MAC_CMD | Modifiers::COMMAND, Key::K);
    harness.run_steps(3);
}

fn type_text(harness: &mut egui_kittest::Harness<'_, Scene>, text: &str) {
    for c in text.chars() {
        harness.event(Event::Text(c.to_string()));
        harness.run_steps(1);
    }
    harness.run_steps(2);
}

/// Splits a 2× screenshot into textures egui accepts (at most 2048px a side).
fn tiles(ctx: &egui::Context, image: &ColorImage) -> Vec<(TextureHandle, Rect)> {
    const TILE: usize = 1024;
    let [width, height] = image.size;
    let mut tiles = Vec::new();
    for y in (0..height).step_by(TILE) {
        for x in (0..width).step_by(TILE) {
            let region = Rect::from_min_max(
                Pos2::new(x as f32, y as f32),
                Pos2::new((x + TILE).min(width) as f32, (y + TILE).min(height) as f32),
            );
            let tile = image.region(&region, None);
            let texture =
                ctx.load_texture(format!("backdrop-{x}-{y}"), tile, TextureOptions::NEAREST);
            tiles.push((
                texture,
                Rect::from_min_max(region.min / 2.0, region.max / 2.0),
            ));
        }
    }
    tiles
}

fn load(path: &str) -> anyhow::Result<ColorImage> {
    let image = image::open(path)
        .with_context(|| format!("reading {path}"))?
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Ok(ColorImage::from_rgba_unmultiplied(
        size,
        image.as_flat_samples().as_slice(),
    ))
}
