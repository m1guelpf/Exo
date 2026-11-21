//! Out of Office Reply end to end on gmail-mock: the dialog reads Gmail's vacation responder and
//! saves to it, the status bar while replies go out, the alerts, and the nudge for
//! out-of-office days on the calendar.

mod support;

use egui::{Event, Key, Modifiers, Pos2};
use gmail::VacationSettings;
use jiff::Timestamp;
use support::{Fixture, start};

/// ⌘ on a Mac, as egui-winit reports it.
const CMD: Modifiers = Modifiers {
    mac_cmd: true,
    command: true,
    ..Modifiers::NONE
};

/// The dialog at 1280×800: its body starts at (397, 161).
const SWITCH: Pos2 = Pos2::new(568.0, 193.0);
const SAVE: Pos2 = Pos2::new(867.0, 684.0);
const FIRST_DAY: Pos2 = Pos2::new(470.0, 250.0);

fn at(text: &str) -> Timestamp {
    text.parse().expect("a timestamp")
}

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

/// The mock's responder, as Gmail web sees it.
fn vacation(app: &Fixture) -> VacationSettings {
    let gmail = app.gmail();
    app.block_on(async { gmail.gmail().vacation().await })
        .expect("the mock answers")
}

/// Sets the mock's responder, as Gmail web does.
fn set_vacation(app: &Fixture, settings: VacationSettings) {
    let gmail = app.gmail();
    app.block_on(async { gmail.gmail().update_vacation(&settings).await })
        .expect("the mock takes it");
}

#[test]
fn the_dialog_shows_gmails_responder_and_saves_to_gmail() {
    let mut app = start("ooo-save");
    palette(&mut app, "out of office");
    let form = app
        .mail()
        .out_of_office_form()
        .expect("the dialog read Gmail");
    // The mock's Thanksgiving responder: off, its days still to come, so they show.
    assert!(!form.enabled);
    assert_eq!(form.subject, "Out of office");
    assert_eq!(form.start, at("2026-11-26T05:00:00Z"));
    assert_eq!(
        form.end,
        Some(at("2026-11-30T04:59:59Z")),
        "the last day's last second"
    );
    assert!(form.body.contains("Thanksgiving"));

    app.click(SWITCH);
    assert!(app.mail().out_of_office_form().expect("still open").enabled);
    app.click(SAVE);
    assert_eq!(
        app.mail().out_of_office_form(),
        None,
        "saved, the dialog closes"
    );
    assert_eq!(
        app.mail().toast_text(),
        Some("Out of Office Reply settings updated.")
    );
    let saved = vacation(&app);
    assert_eq!(saved.enable_auto_reply, Some(true));
    assert_eq!(saved.response_subject.as_deref(), Some("Out of office"));
    assert_eq!(
        saved.start_time,
        Some(at("2026-11-26T05:00:00Z").as_millisecond())
    );
    assert_eq!(
        saved.end_time,
        Some(at("2026-11-30T05:00:00Z").as_millisecond()),
        "a second past the last day"
    );
    assert_eq!(saved.restrict_to_contacts, Some(false));
    assert!(
        saved
            .response_body_html
            .is_some_and(|html| html.contains("Thanksgiving"))
    );
    let copy = app
        .mail()
        .account()
        .settings()
        .out_of_office_responder
        .clone()
        .expect("the copy");
    assert!(copy.is_enabled());
    // Thanksgiving is next week: no bar yet.
    assert_eq!(app.mail().status_bar(), None);
}

#[test]
fn a_responder_set_in_gmail_web_shows_when_the_dialog_opens() {
    let mut app = start("ooo-gmail-web");
    set_vacation(
        &app,
        VacationSettings {
            enable_auto_reply: Some(true),
            response_subject: Some("Gone fishing".into()),
            response_body_html: Some("<div>Back soon.</div>".into()),
            restrict_to_contacts: Some(true),
            start_time: Some(at("2026-12-01T05:00:00Z").as_millisecond()),
            ..VacationSettings::default()
        },
    );
    palette(&mut app, "vacation mode");
    let form = app.mail().out_of_office_form().expect("the alias opens it");
    assert_eq!(
        (form.enabled, form.subject.as_str(), form.contacts, form.end),
        (true, "Gone fishing", true, None)
    );
    assert_eq!(form.start, at("2026-12-01T05:00:00Z"));
    // Unchanged, Escape just closes it.
    app.key(Key::Escape);
    assert_eq!(app.mail().out_of_office_form(), None);
    assert_eq!(app.mail().out_of_office_alert_text(), None);
}

#[test]
fn the_bar_shows_while_replies_go_out_and_the_offline_bar_comes_first() {
    let mut app = start("ooo-bar");
    assert_eq!(app.mail().status_bar(), None);
    set_vacation(
        &app,
        VacationSettings {
            enable_auto_reply: Some(true),
            response_subject: Some("Out".into()),
            response_body_html: Some("<div>Out.</div>".into()),
            start_time: Some(at("2026-11-16T05:00:00Z").as_millisecond()),
            end_time: Some(at("2026-11-21T05:00:00Z").as_millisecond()),
            ..VacationSettings::default()
        },
    );
    // Exo reads Gmail when it opens (and hourly).
    app.relaunch();
    let bar = app.mail().status_bar().expect("the bar shows");
    assert_eq!(
        (bar.title, bar.color),
        ("Out of Office Reply is On", exo_ui::MessageBarColor::Green)
    );
    assert_eq!(
        bar.text(),
        "To turn off automatic replies, hit⌘K → Out of Office Reply."
    );
    app.disconnect();
    app.mail().account().network_changed();
    app.settle();
    assert_eq!(
        app.mail().status_bar().map(|bar| bar.title),
        Some("Offline"),
        "offline wins"
    );
}

#[test]
fn offline_it_says_so_instead() {
    let mut app = start("ooo-offline");
    app.disconnect();
    app.mail().account().network_changed();
    app.settle();
    palette(&mut app, "out of office reply settings");
    let (title, text) = app.mail().out_of_office_alert_text().expect("an alert");
    assert_eq!(title, "You are offline");
    assert_eq!(
        text,
        "You cannot change your Out of Office Reply settings when offline. Please check your internet connection and try again."
    );
    app.key(Key::Escape);
    assert_eq!(app.mail().out_of_office_alert_text(), None);
    assert_eq!(app.mail().out_of_office_form(), None, "no dialog");
}

#[test]
fn closing_a_changed_form_asks_before_discarding() {
    let mut app = start("ooo-discard");
    palette(&mut app, "out of office");
    app.click(SWITCH);
    app.key(Key::Escape);
    let (title, text) = app.mail().out_of_office_alert_text().expect("it asks");
    assert_eq!(
        (title, text.as_str()),
        ("Discard changes?", "Your settings will not be saved.")
    );
    // Cancel keeps the dialog; Discard (selected) closes it.
    app.key(Key::ArrowRight);
    app.key(Key::Enter);
    assert!(
        app.mail()
            .out_of_office_form()
            .is_some_and(|form| form.enabled)
    );
    app.key(Key::Escape);
    app.key(Key::Enter);
    assert_eq!(app.mail().out_of_office_form(), None);
    assert_eq!(
        vacation(&app).enable_auto_reply,
        Some(false),
        "nothing saved"
    );
}

#[test]
fn a_save_gmail_refuses_keeps_the_dialog_open() {
    let mut app = start("ooo-save-fails");
    palette(&mut app, "out of office");
    app.click(SWITCH);
    app.fault(Some(serde_json::json!({ "method": "PUT", "path": "/gmail/v1/users/me/settings/vacation", "status": 400, "message": "Invalid end time" })));
    app.click(SAVE);
    let (title, _) = app.mail().out_of_office_alert_text().expect("an alert");
    assert_eq!(title, "Changes not saved");
    app.key(Key::Enter);
    assert!(
        app.mail().out_of_office_form().is_some(),
        "the form stays, to try again"
    );
    assert!(
        !app.mail()
            .account()
            .settings()
            .out_of_office_responder
            .as_ref()
            .is_some_and(exo_store::OutOfOfficeResponder::is_enabled),
        "the copy kept Gmail's"
    );
}

#[test]
fn a_typed_first_day_moves_the_last_day_along() {
    let mut app = start("ooo-typed");
    palette(&mut app, "out of office");
    app.click(SWITCH);
    app.click(FIRST_DAY);
    app.press(Key::A, CMD);
    app.type_text("12/3");
    app.key(Key::Enter);
    let form = app.mail().out_of_office_form().expect("open");
    assert_eq!(form.start, at("2026-12-03T05:00:00Z"));
    // Past the last day, which moves along to keep four days away.
    assert_eq!(form.end, Some(at("2026-12-07T04:59:59Z")));
}

/// An all-day event on the mock's primary calendar.
fn add_event(app: &Fixture, summary: &str, start: &str, end: &str) {
    let gmail = app.gmail();
    let event = gmail::calendar::Event {
        summary: Some(summary.into()),
        start: Some(gmail::calendar::EventDateTime {
            date: Some(start.into()),
            ..Default::default()
        }),
        end: Some(gmail::calendar::EventDateTime {
            date: Some(end.into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    app.block_on(async { gmail.calendar().insert_event("primary", &event, None).await })
        .expect("the mock takes it");
}

#[test]
fn the_nudge_offers_a_reply_for_days_off_this_week() {
    let mut app = start("ooo-nudge");
    add_event(&app, "Vacation", "2026-11-19", "2026-11-20");
    add_event(&app, "PTO", "2026-11-20", "2026-11-21");
    app.relaunch();
    app.wait(2.5);
    let (title, text) = app.mail().out_of_office_alert_text().expect("the nudge");
    assert_eq!(title, "Set Out of Office Reply?");
    assert_eq!(
        text,
        "It looks like you may be out of office from Thursday, November 19 to Friday, November 20.\nWould you like to send automatic out of office replies while you are away?"
    );
    // `Set reply`: the dialog, with those days and the reply on.
    app.key(Key::Enter);
    let form = app.mail().out_of_office_form().expect("the dialog");
    assert!(form.enabled);
    assert_eq!(
        (form.start, form.end),
        (
            at("2026-11-19T05:00:00Z"),
            Some(at("2026-11-21T04:59:59.999Z"))
        )
    );
    let detection = &app
        .mail()
        .account()
        .settings()
        .out_of_office_event_detection;
    let mut offered: Vec<&str> = detection
        .events
        .values()
        .filter(|event| event.nudged_at.is_some())
        .filter_map(|event| event.start_time.date.as_deref())
        .collect();
    offered.sort_unstable();
    assert_eq!(
        offered,
        ["2026-11-19", "2026-11-20"],
        "both offered, not next week's (the mock's Thanksgiving)"
    );
    // Offered once: not again.
    app.relaunch();
    app.wait(2.5);
    assert_eq!(app.mail().out_of_office_alert_text(), None);
}

#[test]
fn dont_show_this_again_opts_out() {
    let mut app = start("ooo-opt-out");
    add_event(&app, "Out of office", "2026-11-20", "2026-11-21");
    app.relaunch();
    app.wait(2.5);
    assert!(
        app.mail()
            .out_of_office_alert_text()
            .is_some_and(|(_, text)| text
                .starts_with("It looks like you may be out of office on Friday, November 20."))
    );
    // The box, 16px in and 151px down the 340×241 alert centred below the tab strip.
    app.click(Pos2::new(493.0, 459.0));
    app.key(Key::ArrowRight);
    app.key(Key::Enter);
    assert_eq!(app.mail().out_of_office_alert_text(), None);
    assert_eq!(app.mail().out_of_office_form(), None, "No thanks");
    assert!(
        app.mail()
            .account()
            .settings()
            .out_of_office_event_detection
            .opted_out_at
            .is_some()
    );
}

#[test]
fn gmail_failing_to_answer_closes_the_dialog_after_saying_so() {
    let mut app = start("ooo-sync-fails");
    app.fault(Some(serde_json::json!({ "method": "GET", "path": "/gmail/v1/users/me/settings/vacation", "status": 400, "message": "Bad Request" })));
    palette(&mut app, "ooo settings");
    let (title, text) = app.mail().out_of_office_alert_text().expect("an alert");
    assert_eq!(
        (title, text.as_str()),
        (
            "Sorry!",
            "We were unable to sync your Out of Office Reply settings with your email provider. Please check your connection and try again."
        )
    );
    app.key(Key::Enter);
    assert_eq!(app.mail().out_of_office_alert_text(), None);
    app.wait(0.5);
    assert_eq!(
        app.mail().out_of_office_form(),
        None,
        "`ok` closes the dialog too"
    );
}

#[test]
fn a_day_off_made_in_the_calendar_is_offered_a_reply_at_once() {
    let mut app = start("ooo-created");
    app.key(Key::Num2);
    app.settle();
    app.key(Key::B);
    app.type_text("Vacation");
    app.key(Key::Tab);
    app.key(Key::Tab);
    app.press(Key::A, CMD);
    app.type_text("tomorrow 8am to 6pm");
    app.key(Key::Enter);
    app.press(Key::Enter, CMD);
    let (title, text) = app.mail().out_of_office_alert_text().expect("the nudge");
    assert_eq!(
        (title, text.as_str()),
        (
            "Set Out of Office Reply?",
            "Would you like to send automatic out of office replies while you are away?"
        )
    );
    app.key(Key::Enter);
    let form = app.mail().out_of_office_form().expect("the dialog");
    assert_eq!(
        (form.enabled, form.start, form.end),
        (
            true,
            at("2026-11-18T13:00:00Z"),
            Some(at("2026-11-18T23:00:00Z"))
        )
    );
}

#[test]
fn tab_reaches_the_subject_and_save_needs_one() {
    let mut app = start("ooo-tab");
    palette(&mut app, "out of office");
    app.click(SWITCH);
    // From the switch, Tab goes on to the first day; from the form, to the subject (its
    // `tabIndex` of 4 comes before the switch's 0).
    app.key(Key::Tab);
    assert_eq!(
        app.mail().out_of_office_focus(),
        Some(exo::out_of_office::Control::Start)
    );
    app.click(Pos2::new(700.0, 360.0));
    assert_eq!(app.mail().out_of_office_focus(), None);
    app.key(Key::Tab);
    assert_eq!(
        app.mail().out_of_office_focus(),
        Some(exo::out_of_office::Control::Subject)
    );
    app.press(Key::A, CMD);
    app.key(Key::Backspace);
    assert_eq!(
        app.mail()
            .out_of_office_form()
            .map(|form| form.subject.as_str()),
        Some("")
    );
    app.click(SAVE);
    assert!(
        app.mail().out_of_office_form().is_some(),
        "Save is off without a subject"
    );
    assert_eq!(vacation(&app).enable_auto_reply, Some(false));
    app.click(Pos2::new(700.0, 360.0));
    app.key(Key::Tab);
    app.type_text("Away");
    // Subject, Message, Contacts, Domain, then Save.
    for _ in 0..4 {
        app.key(Key::Tab);
    }
    assert_eq!(
        app.mail().out_of_office_focus(),
        Some(exo::out_of_office::Control::Save)
    );
    app.key(Key::Enter);
    assert_eq!(app.mail().out_of_office_form(), None);
    assert_eq!(vacation(&app).response_subject.as_deref(), Some("Away"));
}

#[test]
fn a_date_in_the_message_shows_its_day_and_moves_with_its_keys() {
    let mut app = start("ooo-dates");
    set_vacation(
        &app,
        VacationSettings {
            enable_auto_reply: Some(true),
            response_subject: Some("Away".into()),
            response_body_html: Some("<div>Back on Monday</div>".into()),
            ..VacationSettings::default()
        },
    );
    palette(&mut app, "out of office");
    assert_eq!(app.mail().calendar_day(), None);
    // The subject's line from (397, 312); Tab goes on to the message, End past its date.
    app.click(Pos2::new(800.0, 322.0));
    app.key(Key::Tab);
    app.key(Key::End);
    assert_eq!(
        app.mail().calendar_day(),
        Some(jiff::civil::date(2026, 11, 23)),
        "the caret on `on Monday` shows that Monday"
    );
    // Here the date keys move the date the tooltip says they do.
    app.press(Key::Equals, CMD.plus(Modifiers::SHIFT));
    assert!(
        app.mail()
            .out_of_office_form()
            .is_some_and(|form| form.body.contains("Back next Tuesday")),
        "{:?}",
        app.mail().out_of_office_form().map(|form| &form.body)
    );
    assert_eq!(
        app.mail().calendar_day(),
        Some(jiff::civil::date(2026, 11, 24)),
        "the date's new text shows its day"
    );
    app.press(Key::Minus, CMD.plus(Modifiers::SHIFT));
    app.press(Key::Minus, CMD.plus(Modifiers::SHIFT));
    assert!(
        app.mail()
            .out_of_office_form()
            .is_some_and(|form| form.body.contains("Back Sunday")),
        "{:?}",
        app.mail().out_of_office_form().map(|form| &form.body)
    );
}
