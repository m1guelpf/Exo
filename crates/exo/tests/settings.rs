//! Settings end to end: the gear, the palette's commands, Escape, the Theme dialog, and the theme
//! surviving a relaunch.

mod support;

use egui::{Event, Key, Modifiers, Pos2};
use exo_store::Theme;
use support::{Fixture, start};

/// The footer's gear at 1280×800 (its 16×15 icon at 1237,770).
const GEAR: Pos2 = Pos2::new(1245.0, 777.0);

/// ⌘ on a Mac, as egui-winit reports it.
const CMD: Modifiers = Modifiers {
    mac_cmd: true,
    command: true,
    ..Modifiers::NONE
};

/// Opens the palette and runs the row `query` ranks first.
fn palette(app: &mut Fixture, query: &str) {
    app.press(Key::K, CMD);
    app.harness
        .input_mut()
        .events
        .push(Event::Text(query.to_owned()));
    app.harness.run_steps(1);
    app.key(Key::Enter);
}

fn theme(app: &Fixture) -> exo_ui::Theme {
    exo_ui::theme(&app.harness.ctx)
}

#[test]
fn the_gear_toggles_settings_and_clears_its_dot() {
    let mut app = start("settings-gear");
    assert!(!app.mail().settings_visible());
    assert_eq!(
        app.mail()
            .account()
            .device_settings()
            .new_setting_notification,
        None,
        "the dot shows"
    );
    app.click(GEAR);
    assert!(app.mail().settings_visible());
    assert_eq!(
        app.mail()
            .account()
            .device_settings()
            .new_setting_notification
            .as_deref(),
        Some("2023-06-13-salespackUI"),
        "the dot is gone for good"
    );
    app.click(GEAR);
    assert!(!app.mail().settings_visible());
}

#[test]
fn the_settings_command_opens_settings_and_escape_hides_them() {
    let mut app = start("settings-escape");
    palette(&mut app, "settings");
    assert!(app.mail().settings_visible());
    app.key(Key::Backspace);
    assert!(
        app.mail().settings_visible(),
        "only Escape hides them, not Backspace"
    );
    app.key(Key::J);
    assert!(
        !app.mail().settings_visible(),
        "moving to another conversation hides them"
    );
    palette(&mut app, "settings");
    app.key(Key::Enter);
    assert!(app.mail().thread().is_some());
    assert!(
        app.mail().settings_visible(),
        "opening the conversation doesn't"
    );
    app.key(Key::Escape);
    assert!(!app.mail().settings_visible());
    assert!(
        app.mail().thread().is_none(),
        "Escape still leaves the thread"
    );
}

#[test]
fn the_theme_dialog_sets_the_theme_and_escape_closes_it() {
    let mut app = start("settings-dialog");
    palette(&mut app, "theme settings");
    assert!(app.mail().settings_dialog_open());
    // Dark (Carbon), the second radio (its line 397,414 at 1280×800).
    app.click(Pos2::new(430.0, 424.0));
    assert_eq!(app.mail().account().settings().theme, Theme::Carbon);
    assert_eq!(theme(&app), exo_ui::Theme::Carbon);
    app.key(Key::J);
    assert_eq!(
        app.mail()
            .inbox()
            .cursor()
            .map(|thread| thread.subject.as_str()),
        Some("Design crit moved to 3pm"),
        "the list doesn't take keys under a dialog"
    );
    app.key(Key::Escape);
    assert!(!app.mail().settings_dialog_open());
}

#[test]
fn a_theme_set_from_the_palette_is_applied_at_the_next_launch() {
    let mut app = start("settings-theme");
    assert_eq!(
        app.mail().account().settings().theme,
        Theme::Auto,
        "a new account matches the system"
    );
    assert_eq!(
        theme(&app),
        exo_ui::Theme::Snow,
        "the test harness reports no system appearance"
    );
    app.harness.input_mut().system_theme = Some(egui::Theme::Dark);
    app.settle();
    assert_eq!(theme(&app), exo_ui::Theme::Carbon, "a dark system");
    app.harness.input_mut().system_theme = None;
    palette(&mut app, "set theme: carbon");
    assert_eq!(app.mail().account().settings().theme, Theme::Carbon);
    assert_eq!(theme(&app), exo_ui::Theme::Carbon);

    app.relaunch();
    assert_eq!(
        app.mail().account().settings().theme,
        Theme::Carbon,
        "saved"
    );
    assert_eq!(theme(&app), exo_ui::Theme::Carbon, "and applied at launch");

    palette(&mut app, "set theme: match");
    assert_eq!(app.mail().account().settings().theme, Theme::Auto);
    assert_eq!(
        theme(&app),
        exo_ui::Theme::Snow,
        "the test harness reports no system appearance"
    );
}

#[test]
fn an_accounts_theme_applies_as_its_mail_opens_before_any_frame() {
    // What "Almost there…" and the sign-in screen after it are drawn in when Google refuses the
    // grant before the mail's first frame.
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let server = runtime
        .block_on(gmail_mock::MockServer::start(gmail_mock::Config::default()))
        .expect("mock starts");
    let dir = std::env::temp_dir().join(format!("exo-settings-launch-{}", std::process::id()));
    let email = "maya@brightmoor.co";
    let token = || gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into());
    runtime.block_on(async {
        let store = exo_store::Store::open(&dir, email, server.endpoints(), token())
            .await
            .expect("store opens");
        let mut settings: exo_store::Settings = store.settings().await.expect("reads");
        settings.theme = Theme::Carbon;
        store.save_settings(&settings).await.expect("saves");
    });

    let ctx = egui::Context::default();
    let credentials = exo::account::Credentials {
        email: email.into(),
        endpoints: server.endpoints(),
        tokens: std::sync::Arc::new(token()),
    };
    let account =
        exo::account::Account::open(&dir, credentials, ctx.clone()).expect("account opens");
    assert_eq!(
        account.settings().theme,
        Theme::Carbon,
        "read before the first frame"
    );
    let _mail = exo::Mail::new(
        &ctx,
        &exo::assets_dir(),
        account,
        exo::Clock::fixed(gmail_mock::Config::default().now),
    );
    assert_eq!(exo_ui::theme(&ctx), exo_ui::Theme::Carbon);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn warn_before_quitting_toggles_and_the_alerts_do_not_ask_again_turns_it_off() {
    let mut app = start("settings-warn-quit");
    let warning = exo::desktop::QuitWarning::load(&app.folder("state"));
    app.harness.state_mut().set_quit_warning(warning.clone());
    app.click(GEAR);
    // The menu scrolled to its end, `Advanced` its last section, this its only row in a bare
    // binary (no `Open at Login`): its slider at the row's right.
    let row = Pos2::new(1243.0, 662.0);
    app.hover(row);
    app.harness.input_mut().events.push(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -2000.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    app.settle();
    assert!(warning.is_on(), "on to start with");
    app.click(row);
    assert!(!warning.is_on());
    assert!(
        !exo::desktop::QuitWarning::load(&app.folder("state")).is_on(),
        "kept"
    );
    app.click(row);
    assert!(warning.is_on(), "on again: the alert is back");
    // The alert's `Do not ask again`, ticked with `Quit`, turns it off; with `Cancel`, it stays.
    assert!(!warning.answered(false, true));
    assert!(warning.is_on());
    assert!(warning.answered(true, true));
    assert!(!warning.is_on());
    app.click(row);
    assert!(warning.is_on(), "and the row turns it back on");
}
