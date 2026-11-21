//! Achievements end to end: marking done and reaching Inbox Zero count, the streak's popup and
//! `Week N`, the Achievements dialog from the palette, Inbox Zero's button and the settings
//! menu, and the photo's palette rows.

mod support;

use egui::{Event, Key, Modifiers, Pos2};
use exo::achievements::View;
use support::{Fixture, start};

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

/// Marks every conversation in the inbox done: ⌘A, then E.
fn clear_inbox(app: &mut Fixture) {
    app.press(Key::A, CMD);
    app.key(Key::E);
    app.settle();
    assert!(app.mail().inbox().at_inbox_zero());
}

#[test]
fn reaching_inbox_zero_counts_the_day_and_starts_a_streak() {
    let mut app = start("achievements-streak");
    clear_inbox(&mut app);
    let statistics = app.mail().account().settings().statistics.clone();
    assert_eq!(
        statistics.archived_all_time, 24,
        "the conversations marked done count"
    );
    assert_eq!(
        (
            statistics.inbox_zero_days_all_time,
            statistics.inbox_zero_streak_week_current,
            statistics.inbox_zero_streak_week_max
        ),
        (1, 1, 1)
    );
    assert_eq!(
        statistics.last_inbox_zero_week.as_deref(),
        Some("2026-11-16"),
        "weeks start on Monday"
    );
    assert_eq!(
        app.mail().achievement_popups(),
        ["You've started an Inbox Zero streak!"]
    );
    // `Week 1` hides the header for two seconds.
    assert!(app.mail().inbox().view().hide_chrome);
    app.wait(2.1);
    assert!(!app.mail().inbox().view().hide_chrome);
    // The popup goes by itself after 8 s, fading for 2.
    app.wait(10.5);
    assert!(app.mail().achievement_popups().is_empty());

    // Inbox Zero again the same day counts nothing more: Done, and back to the inbox.
    for key in [Key::G, Key::E, Key::G, Key::I] {
        app.key(key);
    }
    app.settle();
    assert!(app.mail().inbox().at_inbox_zero());
    assert_eq!(
        app.mail()
            .account()
            .settings()
            .statistics
            .inbox_zero_days_all_time,
        1
    );
    assert!(app.mail().achievement_popups().is_empty());
}

#[test]
fn the_dialog_opens_from_the_palette_and_its_cards_open_their_category() {
    let mut app = start("achievements-dialog");
    palette(&mut app, "Achievements");
    assert_eq!(app.mail().achievements_dialog(), Some(View::Main));
    // The first card, Inbox Zero, at 1280×800: under the 144px banner of a 476px dialog
    // centered in the page below the tab strip.
    let middle = 41.0 + (800.0 - 41.0) / 2.0;
    app.click(Pos2::new(640.0, middle - 476.0 / 2.0 + 144.0 + 40.0));
    assert_eq!(app.mail().achievements_dialog(), Some(View::InboxZero));
    // The back button, 20px into the 470px dialog's top left.
    app.click(Pos2::new(640.0 - 230.0 + 40.0, middle - 470.0 / 2.0 + 40.0));
    assert_eq!(app.mail().achievements_dialog(), Some(View::Main));
    app.key(Key::Escape);
    assert_eq!(app.mail().achievements_dialog(), None);
    assert!(
        !app.mail().inbox().is_selecting(),
        "Escape stayed in the dialog"
    );
}

#[test]
fn inbox_zero_downloads_its_photo_and_toggles_animated_images() {
    let mut app = start("achievements-button");
    clear_inbox(&mut app);
    app.wait(2.5);
    // `Download Inbox Zero Image` is there at Inbox Zero and saves the photo.
    let downloads = app.folder("Downloads");
    exo::downloads::choose(&app.harness.ctx, downloads.clone()).expect("chosen");
    palette(&mut app, "Download Inbox Zero Image");
    app.settle();
    let saved: Vec<_> = std::fs::read_dir(&downloads)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name())
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(saved.len(), 1, "{saved:?}");
    // Animated images: off, then on, with the hint saying so.
    palette(&mut app, "Disable Animated Images");
    assert!(app.mail().account().settings().reduce_motion);
    palette(&mut app, "Enable Animated Images");
    assert!(!app.mail().account().settings().reduce_motion);
}
