//! Notifications end to end: the Notifications dialog and its palette rows, and what a click on
//! a notification opens.

mod support;

use egui::{Event, Key, Modifiers, Pos2};
use exo::notifications::Open;
use support::{Fixture, start};

/// ⌘ on a Mac, as egui-winit reports it.
const CMD: Modifiers = Modifiers {
    mac_cmd: true,
    command: true,
    ..Modifiers::NONE
};

/// Where the dialog's controls are at 1280×800 with the mock's three calendars: the email
/// checkbox, each calendar's, `Notify me:`'s dropdown, and its list's `5 mins before events`.
const EMAIL: Pos2 = Pos2::new(875.0, 334.0);
const CALENDARS: [Pos2; 3] = [
    Pos2::new(875.0, 427.0),
    Pos2::new(875.0, 459.0),
    Pos2::new(875.0, 491.0),
];
const LEAD: Pos2 = Pos2::new(520.0, 536.0);
const FIVE_MINUTES: Pos2 = Pos2::new(500.0, 654.0);
/// The settings menu's `Notifications` row.
const MENU_ROW: Pos2 = Pos2::new(1010.0, 586.0);

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

fn email_on(app: &Fixture) -> bool {
    let account = app.mail().account();
    account
        .settings()
        .email_notifications(account.device_settings())
}

fn calendar_notifies(app: &Fixture, id: &str) -> bool {
    let settings = app.mail().account().settings();
    settings.calendar_accounts["maya@brightmoor.co"]
        .calendars
        .get(id)
        .and_then(|calendar| calendar.notifications.as_ref())
        .is_some_and(|notifications| notifications.enabled_desktop)
}

#[test]
fn the_dialog_sets_what_notifies() {
    let mut app = start("notifications-dialog");
    palette(&mut app, "notification settings");
    assert!(app.mail().settings_dialog_open());

    assert!(email_on(&app), "on to start with");
    app.click(EMAIL);
    assert!(!email_on(&app));
    assert!(
        app.mail().account().device_settings().disable_notifications,
        "off on this device"
    );
    app.click(EMAIL);
    assert!(email_on(&app));

    let team = "c_8f3b2a1d9e7c4b6a5f01@group.calendar.google.com";
    assert!(
        calendar_notifies(&app, "maya@brightmoor.co"),
        "the primary calendar's are on"
    );
    assert!(!calendar_notifies(&app, team));
    app.click(CALENDARS[0]);
    app.click(CALENDARS[1]);
    assert!(!calendar_notifies(&app, "maya@brightmoor.co"));
    assert!(calendar_notifies(&app, team));

    assert_eq!(
        app.mail()
            .account()
            .settings()
            .notifications
            .calendar_events
            .lead_time(),
        120
    );
    app.click(LEAD);
    app.click(FIVE_MINUTES);
    assert_eq!(
        app.mail()
            .account()
            .settings()
            .notifications
            .calendar_events
            .lead_time_in_seconds,
        Some(300)
    );
    assert!(
        app.mail().settings_dialog_open(),
        "a choice closes the list, not the dialog"
    );
    app.click(LEAD);
    app.key(Key::Escape);
    assert!(
        app.mail().settings_dialog_open(),
        "Escape closes the list first"
    );
    app.key(Key::Escape);
    assert!(!app.mail().settings_dialog_open());
}

#[test]
fn the_menu_row_opens_the_dialog_and_the_palette_toggles_email() {
    let mut app = start("notifications-menu");
    palette(&mut app, "settings");
    app.click(MENU_ROW);
    assert!(app.mail().settings_dialog_open());
    app.key(Key::Escape);

    palette(&mut app, "disable email notifications");
    assert!(!email_on(&app));
    palette(&mut app, "enable email notifications");
    assert!(email_on(&app));
}

#[test]
fn a_mail_notification_opens_its_conversation_from_home() {
    let mut app = start("notifications-thread");
    let threads = app.mail().inbox().threads().to_vec();
    let target = threads
        .iter()
        .find(|thread| thread.unread)
        .expect("an unread conversation")
        .clone();
    app.key(Key::G);
    app.key(Key::T);
    assert_eq!(app.mail().list(), &exo_store::ListId::Sent);
    palette(&mut app, "notification settings");

    let gmail = app.gmail();
    let thread = app
        .block_on(async {
            gmail
                .gmail()
                .thread(&target.id, gmail::Format::Minimal, &[])
                .await
        })
        .expect("the thread");
    let message = thread
        .messages
        .last()
        .and_then(|message| message.id.clone())
        .expect("a message");
    app.mail_mut().open_notification(Open::Thread {
        thread: target.id.clone(),
        message: message.clone(),
    });
    app.settle();
    assert!(!app.mail().settings_dialog_open(), "dialogs go");
    assert_eq!(app.mail().list(), &exo_store::ListId::Inbox, "home");
    let open = app.mail().thread().expect("the conversation is open");
    assert_eq!(
        (open.id(), open.focused()),
        (target.id.as_str(), Some(message.as_str()))
    );
    assert!(
        !app.mail()
            .inbox()
            .threads()
            .iter()
            .any(|thread| thread.id == target.id && thread.unread),
        "read"
    );
}

#[test]
fn a_calendar_notification_opens_its_day_with_the_event() {
    let mut app = start("notifications-event");
    let query = gmail::calendar::EventsQuery {
        time_min: Some("2026-11-17T00:00:00Z".into()),
        time_max: Some("2026-11-21T00:00:00Z".into()),
        single_events: true,
        ..gmail::calendar::EventsQuery::default()
    };
    let gmail = app.gmail();
    let events = app
        .block_on(async { gmail.calendar().events("maya@brightmoor.co", &query).await })
        .expect("events")
        .items;
    let event = events
        .iter()
        .find(|event| {
            event.recurring_event_id.is_none()
                && event
                    .start
                    .as_ref()
                    .is_some_and(|start| start.date_time.is_some())
        })
        .expect("a timed event");
    let start: jiff::Timestamp = event
        .start
        .as_ref()
        .and_then(|start| start.date_time.as_deref())
        .expect("a time")
        .parse()
        .expect("RFC 3339");
    let tz = jiff::tz::TimeZone::get("America/New_York").expect("zone");
    let day = start.to_zoned(tz).date();
    let key = format!("maya@brightmoor.co_{}", event.id.as_deref().expect("an id"));

    app.mail_mut().open_notification(Open::Event {
        key,
        day,
        end_hour: 18,
        url: None,
    });
    app.settle();
    assert_eq!(app.mail().calendar_week(), Some(day), "the week on its day");
    assert!(app.mail().calendar_details().is_some(), "with its details");
}
