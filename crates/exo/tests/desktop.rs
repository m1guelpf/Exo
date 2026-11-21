//! Exo as a Mac app end to end: closing hides the window, the Dock and other launches bring it
//! back with what they ask for, a launch at login opens it in the Dock, and the Dock badge counts
//! the list on screen.

mod support;

use egui::{Key, Modifiers, ViewportCommand, ViewportEvent, ViewportId};
use exo::desktop::{Desktop, Launcher, Opening, Placement};
use gmail::{DraftQuery, Format};
use support::{Fixture, start};

const COMMAND: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);

/// The fixture running as a Mac app, its window placed from a folder of its own; and the
/// launcher that stands in for the Dock and other launches.
fn start_desktop(name: &str) -> (Fixture, Launcher) {
    open_desktop(name, Opening::Shown, None)
}

/// [`start_desktop`], the window opening as `opening` has it, from `saved` (`window.json`) if
/// given.
fn open_desktop(name: &str, opening: Opening, saved: Option<&str>) -> (Fixture, Launcher) {
    let mut app = start(name);
    let launcher = Launcher::default();
    let folder = app.folder("window");
    if let Some(saved) = saved {
        std::fs::create_dir_all(&folder).expect("a folder");
        std::fs::write(folder.join("window.json"), saved).expect("written");
    }
    let desktop = Desktop::new(
        &app.harness.ctx,
        launcher.clone(),
        Placement::load(&folder),
        opening,
    );
    app.harness.state_mut().set_desktop(desktop);
    (app, launcher)
}

/// Runs a frame in which the window is asked to close (the red button, or the Dock's Close).
fn close(app: &mut Fixture) {
    viewport(app).events.push(ViewportEvent::Close);
    app.harness.run_steps(1);
}

fn viewport(app: &mut Fixture) -> &mut egui::ViewportInfo {
    app.harness
        .input_mut()
        .viewports
        .get_mut(&ViewportId::ROOT)
        .expect("the window")
}

/// What the last frame asked of the window.
fn commands(app: &Fixture) -> Vec<ViewportCommand> {
    app.harness
        .output()
        .viewport_output
        .get(&ViewportId::ROOT)
        .map(|output| output.commands.clone())
        .unwrap_or_default()
}

#[test]
fn closing_hides_the_window_and_the_dock_brings_it_back() {
    let (mut app, dock) = start_desktop("close");
    close(&mut app);
    let asked = commands(&app);
    assert!(
        asked.contains(&ViewportCommand::CancelClose),
        "Exo runs on: {asked:?}"
    );
    assert!(
        asked.contains(&ViewportCommand::Visible(false)),
        "hidden: {asked:?}"
    );

    dock.show();
    app.harness.run_steps(1);
    let asked = commands(&app);
    assert!(
        asked.contains(&ViewportCommand::Visible(true)) && asked.contains(&ViewportCommand::Focus),
        "back in front: {asked:?}"
    );
}

#[test]
fn a_launch_at_login_opens_the_window_in_the_dock_and_it_comes_back_in_full_screen() {
    let saved =
        r#"{"size": [1100, 700], "position": [300, 120], "maximized": false, "full_screen": true}"#;
    let (mut app, _dock) = open_desktop("login", Opening::Minimized, Some(saved));
    app.harness.run_steps(1);
    let asked = commands(&app);
    assert!(
        asked.contains(&ViewportCommand::Visible(false))
            && asked.contains(&ViewportCommand::Minimized(true)),
        "in the Dock: {asked:?}"
    );
    // As eframe has it once the window is in the Dock, built out of full screen.
    viewport(&mut app).minimized = Some(true);
    viewport(&mut app).fullscreen = Some(false);
    viewport(&mut app).occluded = Some(true);
    app.harness.run_steps(2);
    assert!(
        !commands(&app).contains(&ViewportCommand::Minimized(false)),
        "it stays there"
    );

    // Its window in the Dock clicked.
    viewport(&mut app).occluded = Some(false);
    app.harness.run_steps(1);
    let asked = commands(&app);
    assert!(
        asked.contains(&ViewportCommand::Minimized(false))
            && asked.contains(&ViewportCommand::Fullscreen(true)),
        "back as it was left: {asked:?}"
    );
    let window =
        std::fs::read_to_string(app.folder("window").join("window.json")).expect("still there");
    assert_eq!(
        window, saved,
        "the Dock didn't change where it goes back to"
    );
}

#[test]
fn a_launch_by_hand_shows_the_window() {
    let (mut app, _dock) = start_desktop("by-hand");
    app.harness.run_steps(1);
    let asked = commands(&app);
    assert!(
        !asked.contains(&ViewportCommand::Visible(false))
            && !asked.contains(&ViewportCommand::Minimized(true)),
        "{asked:?}"
    );
}

#[test]
fn command_w_closes_the_window() {
    let (mut app, _dock) = start_desktop("command-w");
    let press = |app: &mut Fixture, modifiers| {
        app.harness.input_mut().events.push(egui::Event::Key {
            key: Key::W,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        });
        app.harness.run_steps(1);
    };
    // With two tabs, ⌘W closes one, and the window stays.
    app.press(Key::T, COMMAND);
    press(&mut app, COMMAND);
    assert!(!commands(&app).contains(&ViewportCommand::Visible(false)));
    assert_eq!(app.harness.state().tab_count(), 1);
    press(&mut app, COMMAND);
    assert!(
        commands(&app).contains(&ViewportCommand::Visible(false)),
        "Close Tab closes the one tab's window"
    );
    // ⇧⌘W closes the window whatever its tabs, which it keeps.
    let (mut app, _dock) = start_desktop("command-shift-w");
    app.press(Key::T, COMMAND);
    press(&mut app, COMMAND | Modifiers::SHIFT);
    assert!(
        commands(&app).contains(&ViewportCommand::Visible(false)),
        "Close Window"
    );
    assert_eq!(app.harness.state().tab_count(), 2);
}

#[test]
fn closing_in_full_screen_leaves_it_before_hiding() {
    let (mut app, _dock) = start_desktop("full-screen");
    viewport(&mut app).fullscreen = Some(true);
    close(&mut app);
    let asked = commands(&app);
    assert!(
        asked.contains(&ViewportCommand::Fullscreen(false))
            && !asked.contains(&ViewportCommand::Visible(false)),
        "{asked:?}"
    );
    app.harness.run_steps(1);
    assert!(
        !commands(&app).contains(&ViewportCommand::Visible(false)),
        "not while still in full screen"
    );
    viewport(&mut app).fullscreen = Some(false);
    app.harness.run_steps(1);
    assert!(
        commands(&app).contains(&ViewportCommand::Visible(false)),
        "hidden once out"
    );
}

#[test]
fn a_mailto_link_opens_a_new_message_to_it() {
    let (mut app, launcher) = start_desktop("mailto");
    close(&mut app);
    launcher.open_url("mailto:priya@brightmoor.co?subject=Quarterly%20numbers");
    app.harness.run_steps(1);
    let asked = commands(&app);
    assert!(
        asked.contains(&ViewportCommand::Visible(true)) && asked.contains(&ViewportCommand::Focus),
        "back in front: {asked:?}"
    );
    app.settle();
    assert_eq!(
        app.mail().compose_focus(),
        Some((
            vec!["priya@brightmoor.co".to_owned()],
            "Quarterly numbers".to_owned()
        ))
    );
    assert_eq!(
        (
            app.harness.state().tab_count(),
            app.harness.state().active_tab()
        ),
        (2, 1),
        "in a new tab (`/~compose/{{link}}`)"
    );
}

#[test]
fn closing_saves_what_was_typed_in_the_last_second() {
    let (mut app, _dock) = start_desktop("close-typing");
    app.key(Key::C);
    app.type_text("priya@brightmoor.co");
    app.key(Key::Enter);
    app.key(Key::Tab);
    app.type_text("Before closing");
    app.key(Key::Tab);
    app.type_text("Hi there");
    app.wait(1.5);
    // A key, then closing before the once-a-second save comes round; a hidden window has no
    // more frames to save it on.
    app.harness
        .input_mut()
        .events
        .push(egui::Event::Text("!".into()));
    app.harness.run_steps(1);
    close(&mut app);
    let gmail = app.gmail();
    let saved = || {
        app.block_on(async {
            for draft in gmail
                .gmail()
                .drafts(&DraftQuery::default())
                .await
                .expect("drafts.list")
                .drafts
            {
                let full = gmail
                    .gmail()
                    .draft(&draft.id.expect("an id"), Format::Raw)
                    .await
                    .expect("drafts.get");
                let raw = String::from_utf8_lossy(
                    &full
                        .message
                        .and_then(|message| message.raw)
                        .unwrap_or_default(),
                )
                .into_owned();
                if raw.contains("Subject: Before closing") && raw.contains("Hi there!") {
                    return true;
                }
            }
            false
        })
    };
    let started = std::time::Instant::now();
    while !saved() {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(15),
            "the last key reaches Gmail without another frame"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[test]
fn the_badge_counts_the_inbox_on_screen() {
    let (mut app, _dock) = start_desktop("badge");
    let rows = app.subjects().len();
    assert_eq!(
        app.mail().badge(),
        Some(rows.to_string()),
        "every conversation, the store having them all"
    );
    // Sent isn't an inbox view, and the badge counts only those.
    app.key(Key::G);
    app.key(Key::T);
    assert_eq!(app.mail().badge(), None);
}

/// Hands the next frame what menu items and macOS gave whatever has the keyboard, as eframe's
/// `raw_input_hook` does before each frame, and runs it.
fn hand_over(app: &mut Fixture) {
    let ctx = app.harness.ctx.clone();
    let mut raw = std::mem::take(app.harness.input_mut());
    eframe::App::raw_input_hook(app.harness.state_mut(), &ctx, &mut raw);
    *app.harness.input_mut() = raw;
    app.settle();
}

#[test]
fn a_clicked_menu_item_sends_its_keystroke_to_what_has_the_keyboard() {
    let (mut app, launcher) = start_desktop("menu-select-all");
    // Edit ▸ Select All, in the list: every conversation, as ⌘A.
    launcher.keystroke(exo::desktop::menu::Keystroke::SelectAll);
    hand_over(&mut app);
    assert!(app.mail().inbox().is_selecting(), "selected");
    // Its ⌘ is held for that frame only: the next frame's hook lets go of it.
    assert!(app.harness.ctx.input(|input| input.modifiers.command));
    hand_over(&mut app);
    assert!(!app.harness.ctx.input(|input| input.modifiers.command));
}

#[test]
fn text_macos_inserts_with_no_key_goes_into_the_draft() {
    let (mut app, launcher) = start_desktop("emoji-palette");
    app.key(Key::C);
    app.type_text("priya@brightmoor.co");
    app.key(Key::Enter);
    app.key(Key::Tab);
    app.type_text("Lunch");
    app.key(Key::Tab);
    app.type_text("Pizza ");
    // Emoji & Symbols' pick.
    launcher.text("🍕".to_owned());
    hand_over(&mut app);
    assert!(
        app.mail()
            .compose_body()
            .is_some_and(|body| body.contains("Pizza 🍕")),
        "{:?}",
        app.mail().compose_body()
    );
}
