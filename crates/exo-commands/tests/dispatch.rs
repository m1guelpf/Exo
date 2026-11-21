//! Key dispatch and the overlays, driven through egui like the app drives them.

use egui::{Event, Key, Modifiers, Pos2, Rect};
use egui_kittest::{Harness, kittest::Queryable};
use exo_commands::{CONVERSATION, Command, Commands, Region, Toast};

#[derive(Clone, Debug, PartialEq)]
enum Action {
    MarkDone,
    Undo,
    Down,
    ScanDown,
    ScanStop,
    Snippet,
    Emoji,
}

struct App {
    commands: Commands<Action>,
    ran: Vec<Action>,
}

fn harness() -> Harness<'static, App> {
    let harness = Harness::builder()
        .with_size([1280.0, 800.0])
        .with_os(egui::os::OperatingSystem::Mac)
        .build_ui_state(
            |ui, app: &mut App| {
                let ran = app.commands.dispatch(ui.ctx());
                app.ran.extend(ran);
                app.commands.add(
                    Command::new("MARK_DONE", "Mark Done", Action::MarkDone)
                        .key("e")
                        .default_score(11.0)
                        .regions(CONVERSATION),
                );
                app.commands.add(
                    Command::shortcut("NEXT", Action::Down)
                        .key("j")
                        .regions(CONVERSATION)
                        .repeat(Action::ScanDown)
                        .on_release(Action::ScanStop),
                );
                let area = Rect::from_min_max(Pos2::new(0.0, 41.0), Pos2::new(1280.0, 800.0));
                let ran = app.commands.show(ui.ctx(), area);
                app.ran.extend(ran);
            },
            App {
                commands: Commands::new(),
                ran: Vec::new(),
            },
        );
    exo_ui::install(&harness.ctx).expect("the system fonts load");
    harness
}

const CMD: Modifiers = Modifiers {
    mac_cmd: true,
    command: true,
    ..Modifiers::NONE
};

#[test]
fn keys_run_commands_in_their_regions() {
    let mut harness = harness();
    harness.key_press(Key::E);
    harness.run_steps(2);
    assert_eq!(harness.state().ran, [Action::MarkDone]);

    harness.state_mut().commands.set_region(Region::ComposeForm);
    harness.key_press(Key::E);
    harness.run_steps(2);
    assert_eq!(harness.state().ran.len(), 1, "`e` types in compose");
}

#[test]
fn command_k_opens_the_palette_and_enter_runs_the_first_row() {
    let mut harness = harness();
    harness.key_press_modifiers(CMD, Key::K);
    harness.run_steps(2);
    assert_eq!(harness.state().commands.region(), Region::Command);
    harness.key_press(Key::E);
    harness.run_steps(2);
    assert!(
        harness.state().ran.is_empty(),
        "letters type in the palette"
    );
    harness.key_press(Key::Enter);
    harness.run_steps(2);
    assert_eq!(harness.state().ran, [Action::MarkDone]);
    assert_eq!(harness.state().commands.region(), Region::ThreadList);
}

#[test]
fn question_mark_toggles_the_shortcuts_sheet() {
    let mut harness = harness();
    harness.key_press_modifiers(Modifiers::SHIFT, Key::Questionmark);
    harness.run_steps(2);
    assert_eq!(harness.state().commands.region(), Region::ShortcutsHelp);
    harness.key_press(Key::Escape);
    harness.run_steps(2);
    assert_eq!(harness.state().commands.region(), Region::ThreadList);
}

#[test]
fn z_runs_the_toasts_undo_while_it_shows() {
    let mut harness = harness();
    harness.key_press(Key::Z);
    harness.run_steps(2);
    assert!(harness.state().ran.is_empty(), "no toast, nothing to undo");
    harness
        .state_mut()
        .commands
        .toast(Toast::new("Marked as Done.").undo(Action::Undo));
    harness.run_steps(2);
    harness.key_press_modifiers(CMD, Key::Z);
    harness.run_steps(2);
    assert_eq!(harness.state().ran, [Action::Undo]);
    harness.key_press(Key::Z);
    harness.run_steps(2);
    assert_eq!(
        harness.state().ran,
        [Action::Undo],
        "undo is per toast, and running it hides the toast"
    );
}

#[test]
fn escape_clears_the_query_then_closes_the_palette() {
    let mut harness = harness();
    harness.key_press_modifiers(CMD, Key::K);
    harness.run_steps(2);
    harness.event(egui::Event::Text("mark".into()));
    harness.run_steps(2);
    harness.key_press(Key::Escape);
    harness.run_steps(2);
    assert_eq!(
        harness.state().commands.region(),
        Region::Command,
        "the first Escape clears the query"
    );
    harness.key_press(Key::Backtick);
    harness.run_steps(2);
    assert_eq!(
        harness.state().commands.region(),
        Region::ThreadList,
        "backtick is Escape too"
    );
}

#[test]
fn a_held_key_repeats_what_it_started_while_registrations_change() {
    let mut harness = harness();
    harness
        .state_mut()
        .commands
        .toast(Toast::new("Marked as Done.").undo(Action::Undo));
    harness.run_steps(1);
    harness.key_down(Key::J);
    harness.run_steps(1);
    // The toast's `z` and ⌘Z go away, and they're registered before the app's commands.
    harness.state_mut().commands.hide_toast();
    harness.run_steps(2);
    harness.key_up(Key::J);
    harness.run_steps(1);
    assert_eq!(
        harness.state().ran,
        [
            Action::Down,
            Action::ScanDown,
            Action::ScanDown,
            Action::ScanStop
        ]
    );
}

#[test]
fn a_toast_brought_back_by_hovering_settles() {
    let mut harness = harness();
    harness
        .state_mut()
        .commands
        .toast(Toast::new("Marked as Done.").undo(Action::Undo));
    // 8.5s in: fading out.
    harness.run_steps(34);
    harness.hover_at(Pos2::new(238.0, 744.0));
    harness.run_steps(4);
    assert!(
        harness.try_run().is_ok(),
        "nothing left to animate, so no more frames"
    );
}

#[test]
fn question_mark_closes_the_sheet_without_typing_into_it() {
    let mut harness = harness();
    let question = |pressed| Event::Key {
        key: Key::Questionmark,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::SHIFT,
    };
    for _ in 0..3 {
        // A key and the text it types arrive in the same frame.
        harness
            .input_mut()
            .events
            .extend([question(true), Event::Text("?".into())]);
        harness.run_steps(1);
        harness.input_mut().events.push(question(false));
        harness.run_steps(1);
    }
    assert_eq!(harness.state().commands.region(), Region::ShortcutsHelp);
    assert!(
        harness.query_by_value("?").is_none(),
        "the search field is empty"
    );
}

/// Compose binds `;` (snippets) and `:` (emoji) while its editor has the keyboard; the key runs
/// the command and the editor never types it.
#[test]
fn a_handled_key_never_reaches_a_focused_text_field() {
    struct Compose {
        commands: Commands<Action>,
        text: String,
        ran: Vec<Action>,
    }
    let mut harness = Harness::builder()
        .with_size([1280.0, 800.0])
        .with_os(egui::os::OperatingSystem::Mac)
        .build_ui_state(
            |ui, compose: &mut Compose| {
                let ran = compose.commands.dispatch(ui.ctx());
                compose.ran.extend(ran);
                ui.text_edit_singleline(&mut compose.text).request_focus();
                compose.commands.set_region(Region::ComposeForm);
                compose.commands.add(
                    Command::shortcut("SNIPPET", Action::Snippet)
                        .key(";")
                        .regions(&[Region::ComposeForm]),
                );
                compose.commands.add(
                    Command::shortcut("EMOJI", Action::Emoji)
                        .key(":")
                        .regions(&[Region::ComposeForm]),
                );
                let area = Rect::from_min_max(Pos2::new(0.0, 41.0), Pos2::new(1280.0, 800.0));
                let ran = compose.commands.show(ui.ctx(), area);
                compose.ran.extend(ran);
            },
            Compose {
                commands: Commands::new(),
                text: String::new(),
                ran: Vec::new(),
            },
        );
    harness.run_steps(2);
    for (key, modifiers, text) in [
        (Key::A, Modifiers::NONE, "a"),
        (Key::Semicolon, Modifiers::NONE, ";"),
        (Key::Colon, Modifiers::SHIFT, ":"),
        (Key::B, Modifiers::NONE, "b"),
    ] {
        // A key and the text it types arrive in the same frame.
        let press = |pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        harness
            .input_mut()
            .events
            .extend([press(true), Event::Text(text.into())]);
        harness.run_steps(1);
        harness.input_mut().events.push(press(false));
        harness.run_steps(1);
    }
    assert_eq!(harness.state().ran, [Action::Snippet, Action::Emoji]);
    assert_eq!(harness.state().text, "ab", "`;` and `:` don't type");
}
