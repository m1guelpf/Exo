//! Coaching hints after mouse use, Quick Tips, the palette's nudge, `Version Info`, `Check for
//! Updates` and deep links, end to end at 1280×800.

mod support;

use egui::{Event, Key, Modifiers, Pos2};
use exo::{
    links,
    notice::Notice,
    quick_tips::Context,
    update::{Alert, Dialog, Releases, Shell, Updates},
};
use support::{Fixture, start, start_with_settings};

/// ⌘ on a Mac, as egui-winit reports it.
const CMD: Modifiers = Modifiers {
    mac_cmd: true,
    command: true,
    ..Modifiers::NONE
};
/// The list header's compose and search icons, and the first row's Done and ⌘K buttons.
const COMPOSE: Pos2 = Pos2::new(895.0, 83.0);
const SEARCH: Pos2 = Pos2::new(921.0, 83.0);
const ROW_DONE: Pos2 = Pos2::new(865.0, 136.0);
const ROW_COMMAND: Pos2 = Pos2::new(920.0, 136.0);
/// The rail's Mail and Calendar.
const RAIL_MAIL: Pos2 = Pos2::new(20.0, 83.0);
const RAIL_CALENDAR: Pos2 = Pos2::new(20.0, 112.0);
/// The Quick Tips bar's cross: `2rem` (13.5px) in from the right, the bar's 32px at the bottom.
const QUICK_TIPS_CLOSE: Pos2 = Pos2::new(1246.0, 784.0);

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

fn hint(app: &Fixture) -> Option<String> {
    app.mail().hint_text()
}

#[test]
fn the_list_headers_icons_and_a_rows_buttons_coach_their_keys() {
    let mut app = start("hints-list-buttons");
    app.click(SEARCH);
    assert_eq!(
        hint(&app).as_deref(),
        Some("Next time, hit / to search instantly!")
    );
    app.key(Key::Escape);
    app.key(Key::Escape);
    app.click(ROW_COMMAND);
    assert_eq!(
        hint(&app).as_deref(),
        Some("Next time, hit ⌘ K to instantly open Exo Command!")
    );
    app.key(Key::Escape);
    app.click(ROW_DONE);
    assert_eq!(
        hint(&app).as_deref(),
        Some("Next time, hit E to Mark Done instantly!")
    );
    app.click(COMPOSE);
    assert_eq!(
        hint(&app).as_deref(),
        Some("Next time, hit C to compose instantly!")
    );
}

#[test]
fn the_rail_coaches_1_and_2_only_when_it_switches() {
    let mut app = start("hints-rail");
    app.click(RAIL_MAIL);
    assert_eq!(hint(&app), None, "already in Mail");
    app.click(RAIL_CALENDAR);
    assert!(app.mail().calendar_week().is_some());
    assert_eq!(
        hint(&app).as_deref(),
        Some("Next time, hit 2 to go to Calendar instantly!")
    );
    app.click(RAIL_MAIL);
    assert_eq!(app.mail().calendar_week(), None);
    assert_eq!(
        hint(&app).as_deref(),
        Some("Next time, hit 1 to go to Mail instantly!")
    );
}

#[test]
fn quick_tips_follow_the_keyboard_and_close_with_their_hint() {
    let mut app = start_with_settings("hints-quick-tips", &|settings| {
        settings.quick_tips.show = true
    });
    assert_eq!(app.mail().quick_tips(), Some(Context::Inbox));
    app.key(Key::Enter);
    assert_eq!(app.mail().quick_tips(), Some(Context::Thread));
    app.press(Key::K, CMD);
    assert_eq!(app.mail().quick_tips(), Some(Context::Palette));
    app.key(Key::Escape);
    app.key(Key::Escape);
    assert_eq!(app.mail().quick_tips(), Some(Context::Inbox));
    app.click(QUICK_TIPS_CLOSE);
    assert!(!app.mail().account().settings().quick_tips.show);
    assert_eq!(app.mail().quick_tips(), None);
    assert_eq!(
        hint(&app).as_deref(),
        Some("Want to reenable Quick Tips? Hit ⌘ K → Quick Tips!")
    );
}

#[test]
fn the_palette_turns_quick_tips_on_and_off() {
    let mut app = start("hints-quick-tips-palette");
    assert_eq!(app.mail().quick_tips(), None, "off for a new account");
    palette(&mut app, "enable quick tips");
    assert!(app.mail().account().settings().quick_tips.show);
    assert_eq!(hint(&app).as_deref(), Some("Enabled Quick Tips"));
    palette(&mut app, "disable quick tips");
    assert!(!app.mail().account().settings().quick_tips.show);
    assert_eq!(
        hint(&app).as_deref(),
        Some("Want to reenable Quick Tips? Hit ⌘ K → Quick Tips!")
    );
}

#[test]
fn version_info_copies_and_closes() {
    let mut app = start("hints-version-info");
    palette(&mut app, "version info");
    assert_eq!(app.mail().notice(), Some(Notice::VersionInfo));
    assert!(Notice::VersionInfo.text().starts_with("- Version: "));
    app.press(Key::C, CMD);
    assert_eq!(hint(&app).as_deref(), Some("Alert copied to clipboard."));
    app.key(Key::J);
    assert_eq!(
        app.mail().notice(),
        Some(Notice::VersionInfo),
        "other keys don't get through"
    );
    app.key(Key::Escape);
    assert_eq!(app.mail().notice(), None);
}

#[test]
fn deep_links_open_settings_or_ask_for_an_update() {
    let mut app = start("hints-deep-links");
    app.key(Key::Enter);
    let link = links::parse("exo://deeplink/emoji-settings").expect("a deep link");
    app.mail_mut().open_link(&link);
    app.settle();
    assert!(app.mail().thread().is_none(), "the home list first");
    assert!(app.mail().settings_dialog_open());
    app.key(Key::Escape);
    let link = links::parse("exo://deeplink/auto-labels-settings").expect("a deep link");
    app.mail_mut().open_link(&link);
    app.settle();
    assert!(
        !app.mail().settings_dialog_open() && app.mail().notice().is_none(),
        "an AI feature's: nothing"
    );
    let link = links::parse("exo://deeplink/teleport").expect("a deep link");
    app.mail_mut().open_link(&link);
    app.settle();
    assert_eq!(app.mail().notice(), Some(Notice::UpdateRequired));
    app.key(Key::Enter);
    assert_eq!(app.mail().notice(), None);
}

/// Releases with nothing newer than this Exo.
struct UpToDate;

impl Releases for UpToDate {
    fn check(&self) -> anyhow::Result<Option<fastframe_update::Release>> {
        Ok(None)
    }

    fn download(
        &self,
        _: &fastframe_update::Release,
        _: &mut dyn FnMut(u64, u64),
    ) -> anyhow::Result<fastframe_update::Prepared> {
        anyhow::bail!("nothing to download")
    }

    fn install(&self, _: fastframe_update::Prepared, _: Vec<String>) -> anyhow::Result<()> {
        anyhow::bail!("nothing to install")
    }
}

/// An app around the updater that shows nothing of its own.
struct Quiet;

impl Shell for Quiet {
    fn ready(&self, _: &str) {}

    fn ask(&self, _: &Dialog) -> Option<usize> {
        None
    }

    fn quit(&self) {}

    fn changed(&self) {}
}

/// Runs frames until the alert on screen is `alert`, which the updater's thread puts up.
fn wait_for_alert(app: &mut Fixture, alert: Alert) {
    let start = std::time::Instant::now();
    while app.mail().notice() != Some(Notice::Update(alert)) {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "{:?} for {alert:?}",
            app.mail().notice()
        );
        app.harness.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn check_for_updates_says_when_exo_is_up_to_date() {
    let mut app = start("hints-check-for-updates");
    app.mail_mut()
        .set_updates(Some(Updates::with(UpToDate, Quiet)));
    palette(&mut app, "update exo");
    wait_for_alert(&mut app, Alert::Latest);
    assert_eq!(
        Notice::Update(Alert::Latest).text(),
        "You are on the latest version of Exo!"
    );
    app.key(Key::J);
    assert_eq!(
        app.mail().notice(),
        Some(Notice::Update(Alert::Latest)),
        "other keys don't get through"
    );
    app.key(Key::Escape);
    assert_eq!(app.mail().notice(), None);

    // The `upgrade` deep link checks the same way, from the home list.
    let link = links::parse("exo://deeplink/upgrade").expect("a deep link");
    app.mail_mut().open_link(&link);
    wait_for_alert(&mut app, Alert::Latest);
    app.key(Key::Enter);
    assert_eq!(app.mail().notice(), None);
}

/// Gives the account a join date, which the nudge goes by.
fn joined(app: &mut Fixture, at: &str) {
    let joined = Some(at.parse().expect("a timestamp"));
    let accounts = app
        .mail()
        .accounts()
        .iter()
        .map(|account| exo_auth::Account {
            joined,
            ..account.clone()
        })
        .collect();
    app.mail_mut().set_accounts(accounts);
    app.settle();
}

#[test]
fn the_palettes_nudge_waits_three_seconds_and_ends_once_closed() {
    let mut app = start("hints-nudge");
    joined(&mut app, "2026-11-16T12:00:00Z");
    app.wait(3.5);
    assert_eq!(
        app.mail().command_nudge(),
        Some("Try: label, folder, trash, or unread")
    );
    app.press(Key::K, CMD);
    app.key(Key::Escape);
    assert_eq!(app.mail().command_nudge(), None, "seen");
    assert!(app.mail().account().settings().has_shown_command_nudge);
}

#[test]
fn leaving_a_conversation_opens_the_palette_on_its_reminder_until_a_triage_key() {
    let mut app = start("hints-nudge-leave");
    joined(&mut app, "2026-11-16T12:00:00Z");
    app.key(Key::Enter);
    app.key(Key::Escape);
    assert_eq!(
        app.mail().command_nudge(),
        Some("Remember: use ⌘K to perform any action in seconds!")
    );
    app.key(Key::Escape);
    assert!(
        app.mail().account().settings().has_shown_command_nudge,
        "closing it ends it"
    );

    let mut app = start("hints-nudge-triage");
    joined(&mut app, "2026-11-16T12:00:00Z");
    app.key(Key::S);
    assert!(
        app.mail().account().settings().has_shown_command_nudge,
        "a triage key ends it"
    );
    app.key(Key::Enter);
    app.key(Key::Escape);
    assert_eq!(app.mail().command_nudge(), None);

    let mut app = start("hints-nudge-old");
    joined(&mut app, "2024-06-01T12:00:00Z");
    app.wait(3.5);
    assert_eq!(app.mail().command_nudge(), None, "joined before 2025-01-13");
}
