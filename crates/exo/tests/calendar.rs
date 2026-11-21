//! The calendar end to end: the right pane's day view (its keys, the palette's rows, following
//! the cursor and an invite, the footer's button), the calendars a new account switches on, the
//! week view, Jump to Date, and an event's details answering an invitation.

mod support;

use egui::{Event, Key, Modifiers, Pos2};
use jiff::civil::date;
use support::{Fixture, start};

/// The footer's calendar button at 1280×800.
const FOOTER_CALENDAR: Pos2 = Pos2::new(1217.0, 777.0);
/// The day view's next-day chevron at 1280×800 (1105.8,72 20×24).
const NEXT_DAY: Pos2 = Pos2::new(1115.0, 84.0);
/// The week's new event form's Repeat at 1280×800 (543,369 85×20), and the day view's Today
/// (1210,72 24×19) and `+` (1234,72 19×19).
const REPEAT: Pos2 = Pos2::new(570.0, 378.0);
const DAY_TODAY: Pos2 = Pos2::new(1222.0, 81.0);
const DAY_CREATE: Pos2 = Pos2::new(1244.0, 81.0);
/// In the week view at 1280×800: the rail's Calendar, Thursday's 2pm `Calloway: Atlas feedback
/// call`, and its details' `Yes`.
const RAIL_CALENDAR: Pos2 = Pos2::new(20.0, 112.0);
const CALLOWAY: Pos2 = Pos2::new(660.0, 520.0);
const YES: Pos2 = Pos2::new(271.0, 754.0);

const CMD: Modifiers = Modifiers {
    mac_cmd: true,
    command: true,
    ..Modifiers::NONE
};

fn palette(app: &mut Fixture, query: &str) {
    app.press(Key::K, CMD);
    app.harness
        .input_mut()
        .events
        .push(Event::Text(query.to_owned()));
    app.harness.run_steps(1);
    app.key(Key::Enter);
}

#[test]
fn keys_open_and_move_the_day_and_the_cursor_closes_it() {
    let mut app = start("calendar-keys");
    assert_eq!(app.mail().calendar_day(), None);
    app.key(Key::Num0);
    assert_eq!(
        app.mail().calendar_day(),
        Some(date(2026, 11, 17)),
        "today, the mock's now"
    );
    app.key(Key::Equals);
    app.key(Key::Equals);
    assert_eq!(app.mail().calendar_day(), Some(date(2026, 11, 19)));
    app.key(Key::Minus);
    assert_eq!(app.mail().calendar_day(), Some(date(2026, 11, 18)));
    // Moving to a conversation without an invite brings its contact back.
    app.key(Key::J);
    app.key(Key::J);
    app.settle();
    assert_eq!(app.mail().calendar_day(), None);
    app.click(FOOTER_CALENDAR);
    assert_eq!(app.mail().calendar_day(), Some(date(2026, 11, 17)));
    app.click(NEXT_DAY);
    assert_eq!(app.mail().calendar_day(), Some(date(2026, 11, 18)));
}

#[test]
fn a_day_opened_from_the_palette_stays_as_the_cursor_moves() {
    let mut app = start("calendar-palette");
    palette(&mut app, "open day");
    assert_eq!(app.mail().calendar_day(), Some(date(2026, 11, 17)));
    app.key(Key::J);
    app.key(Key::J);
    app.settle();
    assert_eq!(
        app.mail().calendar_day(),
        Some(date(2026, 11, 17)),
        "a command's day isn't contextual"
    );
    palette(&mut app, "close calendar");
    assert_eq!(app.mail().calendar_day(), None);
}

#[test]
fn jump_to_date_moves_the_day_and_the_week() {
    let mut app = start("calendar-jump");
    let jump = |app: &mut Fixture, query: &str| {
        palette(app, "jump to date");
        app.harness.run_steps(1);
        app.harness
            .input_mut()
            .events
            .push(Event::Text(query.to_owned()));
        app.harness.run_steps(1);
        app.key(Key::Enter);
    };
    app.key(Key::Num0);
    jump(&mut app, "nov 25");
    assert_eq!(app.mail().calendar_day(), Some(date(2026, 11, 25)));
    app.key(Key::Num2);
    jump(&mut app, "2 weeks");
    assert_eq!(
        app.mail().calendar_week(),
        Some(date(2026, 12, 1)),
        "the week view moves instead"
    );
}

#[test]
fn an_invite_shows_its_day() {
    let mut app = start("calendar-invite");
    // The second row is Grace's invitation for coffee on the 25th.
    app.key(Key::J);
    app.settle();
    assert!(
        app.cursor().starts_with("Invitation: Coffee with Grace"),
        "{}",
        app.cursor()
    );
    assert_eq!(
        app.mail().calendar_day(),
        Some(date(2026, 11, 25)),
        "the cursor's invite"
    );
    app.key(Key::Enter);
    app.settle();
    assert_eq!(
        app.mail().calendar_day(),
        Some(date(2026, 11, 25)),
        "and the open thread's"
    );
}

#[test]
fn a_new_account_switches_on_its_listed_calendars() {
    let mut app = start("calendar-defaults");
    app.settle();
    let account = app.mail().account();
    let calendars = &account.settings().calendar_accounts[account.email()].calendars;
    let colors: Vec<(&str, Option<&str>)> = calendars
        .values()
        .map(|calendar| (calendar.id.as_str(), calendar.color.as_deref()))
        .collect();
    assert_eq!(
        colors,
        [
            (
                "c_8f3b2a1d9e7c4b6a5f01@group.calendar.google.com",
                Some("green")
            ),
            ("en.usa#holiday@group.v.calendar.google.com", Some("forest")),
            ("maya@brightmoor.co", Some("blue")),
        ],
        "Birthdays is hidden"
    );
    palette(&mut app, "show declined events");
    assert!(
        app.mail()
            .account()
            .settings()
            .calendar
            .show_declined_events
    );
}

#[test]
fn the_week_opens_moves_and_closes() {
    let mut app = start("calendar-week");
    app.key(Key::Num2);
    assert_eq!(app.mail().calendar_week(), Some(date(2026, 11, 17)));
    app.key(Key::Equals);
    app.key(Key::J);
    assert_eq!(
        app.mail().calendar_week(),
        Some(date(2026, 12, 1)),
        "= and j a week on each"
    );
    app.key(Key::T);
    assert_eq!(app.mail().calendar_week(), Some(date(2026, 11, 17)));
    app.key(Key::Escape);
    assert_eq!(app.mail().calendar_week(), None);
    // From the day view, `0` opens the week on its day.
    app.key(Key::Num0);
    app.key(Key::Equals);
    app.key(Key::Num0);
    assert_eq!(
        (app.mail().calendar_week(), app.mail().calendar_day()),
        (Some(date(2026, 11, 18)), None)
    );
    app.key(Key::Num1);
    assert_eq!(app.mail().calendar_week(), None);
    app.click(RAIL_CALENDAR);
    assert_eq!(app.mail().calendar_week(), Some(date(2026, 11, 17)));
}

#[test]
fn calendar_settings_start_the_week_on_monday_and_hide_weekends() {
    let mut app = start("calendar-week-options");
    app.key(Key::Num2);
    app.settle();
    let week = |app: &Fixture| app.mail().calendar_week_days(date(2026, 11, 17));
    assert_eq!(
        week(&app).first(),
        Some(&date(2026, 11, 15)),
        "Sunday first"
    );
    // The gutter's time zone opens Calendar Settings.
    app.click(Pos2::new(90.0, 180.0));
    assert!(app.mail().calendar_settings_open());
    // `Start week on Monday`, the dialog's first option.
    app.click(Pos2::new(480.0, 303.0));
    assert_eq!(
        week(&app).first(),
        Some(&date(2026, 11, 16)),
        "Monday first"
    );
    app.key(Key::Escape);
    palette(&mut app, "Hide Weekends");
    assert_eq!(
        week(&app),
        (16..=20).map(|day| date(2026, 11, day)).collect::<Vec<_>>()
    );
    palette(&mut app, "Show Weekends");
    assert_eq!(week(&app).len(), 7);
}

#[test]
fn an_events_details_answer_it() {
    let mut app = start("calendar-details");
    app.key(Key::Num2);
    app.settle();
    app.click(CALLOWAY);
    assert_eq!(
        app.mail().calendar_details(),
        Some(false),
        "the details are up"
    );
    app.click(YES);
    app.settle();
    let gmail = app.gmail();
    let query = gmail::calendar::EventsQuery {
        q: Some("Calloway: Atlas".into()),
        single_events: true,
        ..Default::default()
    };
    let events = app
        .block_on(
            gmail
                .calendar()
                .events("maya@brightmoor.co", &query)
                .into_future(),
        )
        .expect("lists");
    let call = events
        .items
        .iter()
        .find(|event| event.summary.as_deref() == Some("Calloway: Atlas feedback call"))
        .expect("on the calendar");
    let mine = call
        .attendees
        .iter()
        .find(|attendee| attendee.email.as_deref() == Some("maya@brightmoor.co"))
        .and_then(|attendee| attendee.response_status.as_deref());
    assert_eq!(mine, Some("accepted"));
    // Escape closes the details, then the week.
    app.key(Key::Escape);
    assert_eq!(
        (
            app.mail().calendar_details(),
            app.mail().calendar_week().is_some()
        ),
        (None, true)
    );
    app.key(Key::Escape);
    assert_eq!(app.mail().calendar_week(), None);
}

/// My answer to Thursday's `Calloway: Atlas feedback call`, as Calendar has it.
fn calloway_answer(app: &Fixture) -> Option<String> {
    let gmail = app.gmail();
    let query = gmail::calendar::EventsQuery {
        q: Some("Calloway: Atlas".into()),
        single_events: true,
        ..Default::default()
    };
    let events = app
        .block_on(
            gmail
                .calendar()
                .events("maya@brightmoor.co", &query)
                .into_future(),
        )
        .expect("lists");
    let call = events
        .items
        .iter()
        .find(|event| event.summary.as_deref() == Some("Calloway: Atlas feedback call"))
        .expect("on the calendar");
    call.attendees
        .iter()
        .find(|attendee| attendee.email.as_deref() == Some("maya@brightmoor.co"))
        .and_then(|attendee| attendee.response_status.clone())
}

#[test]
fn an_answer_given_offline_goes_once_online() {
    let mut app = start("calendar-answer-offline");
    app.key(Key::Num2);
    app.settle();
    app.click(CALLOWAY);
    assert_eq!(
        app.mail().calendar_details(),
        Some(false),
        "the details are up"
    );
    app.disconnect();
    app.mail().account().network_changed();
    // Not settled: offline, the details' guests keep the app busy for longer than a toast shows.
    let started = std::time::Instant::now();
    while app.mail().status_bar().is_none() && started.elapsed() < std::time::Duration::from_secs(5)
    {
        app.harness.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    app.harness.run_steps(2);
    // The offline bar takes the page's bottom, and the details move up with it.
    let yes = YES - egui::vec2(0.0, exo_ui::MESSAGE_BAR_HEIGHT);
    app.harness
        .input_mut()
        .events
        .push(Event::PointerMoved(yes));
    app.harness.run_steps(1);
    for pressed in [true, false] {
        app.harness.input_mut().events.push(Event::PointerButton {
            pos: yes,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        app.harness.run_steps(1);
    }
    assert_eq!(
        app.mail().toast_text(),
        Some("RSVP will be sent when online.")
    );
    app.reconnect();
    assert_eq!(
        calloway_answer(&app).as_deref(),
        Some("needsAction"),
        "not sent yet"
    );
    // Opened again online, it goes.
    app.relaunch();
    app.wait(1.0);
    assert_eq!(calloway_answer(&app).as_deref(), Some("accepted"));
}

#[test]
fn an_answer_calendar_refuses_says_so() {
    let mut app = start("calendar-answer-refused");
    app.key(Key::Num2);
    app.settle();
    app.fault(Some(serde_json::json!({ "method": "PATCH", "path": "/calendar/v3/calendars/", "status": 403, "message": "Forbidden", "reason": "forbidden" })));
    app.click(CALLOWAY);
    app.click(YES);
    app.wait(0.5);
    assert_eq!(
        app.mail().toast_text(),
        Some("Sorry! Failed to save your RSVP.")
    );
    assert_eq!(calloway_answer(&app).as_deref(), Some("needsAction"));
}

/// The week view at 1280×800, the morning's scroll: a time's y on the grid, and the day
/// columns' centres (Sunday first).
fn at(day: usize, hour: f32) -> Pos2 {
    Pos2::new(
        104.0 + 127.0 * (day as f32 + 0.5),
        194.0 + hour * 44.0 - 312.0,
    )
}

/// A double click at `at`, two clicks a frame apart.
fn double_click(app: &mut Fixture, at: Pos2) {
    app.harness.input_mut().events.push(Event::PointerMoved(at));
    app.harness.run_steps(1);
    for _ in 0..2 {
        for pressed in [true, false] {
            app.harness.input_mut().events.push(Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            app.harness.run_steps(1);
        }
    }
    app.settle();
}

/// A drag from `from` to `to` in steps, as a hand moves.
fn drag(app: &mut Fixture, from: Pos2, to: Pos2) {
    app.harness
        .input_mut()
        .events
        .push(Event::PointerMoved(from));
    app.harness.run_steps(1);
    app.harness.input_mut().events.push(Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    app.harness.run_steps(1);
    for step in 1..=10 {
        app.harness.input_mut().events.push(Event::PointerMoved(
            from + (to - from) * (step as f32 / 10.0),
        ));
        app.harness.run_steps(1);
    }
    app.harness.input_mut().events.push(Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    app.harness.run_steps(1);
    app.settle();
}

fn shown_on(app: &Fixture, day: jiff::civil::Date) -> Vec<String> {
    app.mail()
        .calendar_events_on(day)
        .into_iter()
        .map(|(title, start)| format!("{title} {start}"))
        .collect()
}

#[test]
fn a_new_event_is_held_for_undo_then_sent() {
    let mut app = start("calendar-create");
    app.key(Key::Num2);
    app.settle();
    // `b`: the next half hour on the week's day, its title focused.
    app.key(Key::B);
    assert_eq!(app.mail().calendar_form(), Some(true));
    app.press(Key::Enter, CMD);
    assert_eq!(
        app.mail().calendar_prompt(),
        Some("Almost there…"),
        "a title is needed"
    );
    app.key(Key::Enter);
    app.type_text("Lunch with Lena");
    app.press(Key::Enter, CMD);
    assert_eq!(app.mail().calendar_form(), None);
    let tuesday = date(2026, 11, 17);
    assert!(
        shown_on(&app, tuesday).contains(&"Lunch with Lena 9:00 AM".to_owned()),
        "shown at once: {:?}",
        shown_on(&app, tuesday)
    );
    // Undo takes it back and opens the form again.
    app.key(Key::Z);
    assert!(
        !shown_on(&app, tuesday)
            .iter()
            .any(|event| event.starts_with("Lunch"))
    );
    assert_eq!(app.mail().calendar_form(), Some(true));
    app.press(Key::Enter, CMD);
    // Past the hold, Calendar has it.
    std::thread::sleep(std::time::Duration::from_millis(10_500));
    app.wait(1.0);
    let gmail = app.gmail();
    let query = gmail::calendar::EventsQuery {
        q: Some("Lunch with Lena".into()),
        ..Default::default()
    };
    let found = app
        .block_on(
            gmail
                .calendar()
                .events("maya@brightmoor.co", &query)
                .into_future(),
        )
        .expect("lists");
    assert_eq!(found.items.len(), 1, "sent once");
    assert!(shown_on(&app, tuesday).contains(&"Lunch with Lena 9:00 AM".to_owned()));
    // The toast outlives the hold (hovered, it stays): an Undo then can't take the event back,
    // so it doesn't reopen the form to make it a second time.
    app.key(Key::Z);
    assert_eq!(app.mail().calendar_form(), None, "nothing was undone");
    assert!(shown_on(&app, tuesday).contains(&"Lunch with Lena 9:00 AM".to_owned()));
}

#[test]
fn the_calendar_field_filters_the_calendars_as_typed_and_enter_takes_one_then_saves() {
    const TEAM: &str = "c_8f3b2a1d9e7c4b6a5f01@group.calendar.google.com";
    let mut app = start("calendar-field");
    app.key(Key::Num2);
    app.settle();
    app.key(Key::B);
    app.type_text("Standup");
    // The writable calendars, the primary first: ↓ takes the highlight to the team's, and round
    // to the primary again.
    palette(&mut app, "change calendar");
    app.key(Key::ArrowDown);
    app.key(Key::Enter);
    assert_eq!(app.mail().calendar_form_calendar(), Some(TEAM));
    palette(&mut app, "change calendar");
    app.key(Key::ArrowDown);
    app.key(Key::ArrowDown);
    app.key(Key::Enter);
    assert_eq!(
        app.mail().calendar_form_calendar(),
        Some("maya@brightmoor.co")
    );
    // What's typed that matches nothing leaves the field as it is; Escape empties it and the
    // field keeps the keyboard.
    palette(&mut app, "change calendar");
    app.type_text("zzz");
    app.key(Key::Enter);
    app.key(Key::Escape);
    assert_eq!(
        app.mail().calendar_form(),
        Some(true),
        "Escape takes the text first"
    );
    app.type_text("team");
    app.key(Key::Enter);
    assert_eq!(app.mail().calendar_form_calendar(), Some(TEAM));
    // Taking one moves on to Save, which Enter presses.
    app.key(Key::Enter);
    assert_eq!(app.mail().calendar_form(), None);
    assert!(
        shown_on(&app, date(2026, 11, 17)).contains(&"Standup 9:00 AM".to_owned()),
        "{:?}",
        shown_on(&app, date(2026, 11, 17))
    );
}

/// Presses `key` and runs the frames it takes, without settling: offline, sync's retries keep
/// the app busy for longer than a toast shows.
fn press_unsettled(app: &mut Fixture, key: Key, modifiers: Modifiers) {
    for pressed in [true, false] {
        app.harness.input_mut().events.push(Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        });
        app.harness.run_steps(1);
    }
}

/// Runs frames until `done` holds, for a few seconds at most, without settling.
fn frames_until(app: &mut Fixture, done: impl Fn(&Fixture) -> bool) {
    let started = std::time::Instant::now();
    while !done(app) && started.elapsed() < std::time::Duration::from_secs(5) {
        app.harness.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn an_event_made_offline_waits_through_a_quit_and_goes_once_online() {
    let mut app = start("calendar-offline");
    app.key(Key::Num2);
    app.settle();
    app.disconnect();
    app.mail().account().network_changed();
    app.settle();
    let tuesday = date(2026, 11, 17);
    for title in ["Lunch with Lena", "Coffee with Sam"] {
        app.key(Key::B);
        app.type_text(title);
        press_unsettled(&mut app, Key::Enter, CMD);
        assert_eq!(
            app.mail().toast_text(),
            Some("Event will be scheduled when online.")
        );
        let shown = format!("{title} 9:00 AM");
        frames_until(&mut app, |app| shown_on(app, tuesday).contains(&shown));
        assert!(
            shown_on(&app, tuesday).contains(&shown),
            "shown at once: {:?}",
            shown_on(&app, tuesday)
        );
    }
    // Past the hold, still offline, Undo on the toast (it stays while hovered) takes the last
    // back: it hasn't gone.
    std::thread::sleep(std::time::Duration::from_millis(10_500));
    app.harness.run_steps(2);
    press_unsettled(&mut app, Key::Z, Modifiers::NONE);
    assert_eq!(app.mail().toast_text(), Some("Scheduling undone."));
    app.settle();
    app.key(Key::Escape);
    app.key(Key::Enter);
    assert_eq!(app.mail().calendar_form(), None);
    let shown = shown_on(&app, tuesday);
    assert!(
        shown.contains(&"Lunch with Lena 9:00 AM".to_owned())
            && !shown.iter().any(|event| event.starts_with("Coffee")),
        "{shown:?}"
    );

    // Quit offline, and open again online: the event Calendar didn't get goes then.
    app.reconnect();
    app.relaunch();
    app.key(Key::Num2);
    app.settle();
    assert!(
        shown_on(&app, tuesday).contains(&"Lunch with Lena 9:00 AM".to_owned()),
        "kept across the quit: {:?}",
        shown_on(&app, tuesday)
    );
    app.wait(1.0);
    let gmail = app.gmail();
    let made = |title: &str| {
        let query = gmail::calendar::EventsQuery {
            q: Some(title.into()),
            ..Default::default()
        };
        app.block_on(
            gmail
                .calendar()
                .events("maya@brightmoor.co", &query)
                .into_future(),
        )
        .expect("lists")
        .items
        .len()
    };
    assert_eq!((made("Lunch with Lena"), made("Coffee with Sam")), (1, 0));
}

#[test]
fn a_new_events_time_is_typed() {
    let mut app = start("calendar-when");
    app.key(Key::Num2);
    app.settle();
    app.key(Key::B);
    app.type_text("Lunch with Lena");
    // Tab to Who, then to When, whose text is the draft's time.
    app.key(Key::Tab);
    app.key(Key::Tab);
    app.press(Key::A, CMD);
    app.type_text("blah");
    app.key(Key::Enter);
    assert_eq!(app.mail().calendar_form(), Some(true), "nothing to take");
    app.press(Key::A, CMD);
    app.type_text("tomorrow 2pm");
    app.key(Key::Enter);
    app.press(Key::Enter, CMD);
    assert_eq!(app.mail().calendar_form(), None);
    let wednesday = date(2026, 11, 18);
    assert!(
        shown_on(&app, wednesday).contains(&"Lunch with Lena 2:00 PM".to_owned()),
        "{:?}",
        shown_on(&app, wednesday)
    );
}

#[test]
fn an_event_is_edited_moved_and_deleted() {
    let mut app = start("calendar-edit");
    app.key(Key::Num2);
    app.settle();
    let thursday = date(2026, 11, 19);
    // A double click opens my dinner's form, the title focused.
    double_click(&mut app, at(4, 19.8));
    assert_eq!(app.mail().calendar_form(), Some(false));
    app.press(Key::End, Modifiers::NONE);
    app.type_text(" at Lilia");
    app.press(Key::Enter, CMD);
    assert_eq!(app.mail().calendar_prompt(), None, "no guests, no dialog");
    assert!(
        shown_on(&app, thursday).contains(&"Dinner with Jess 🍕 at Lilia 7:30 PM".to_owned()),
        "{:?}",
        shown_on(&app, thursday)
    );

    // Dragging a recurring event of mine asks which events; Enter takes this one.
    let wednesday = date(2026, 11, 18);
    drag(&mut app, at(3, 9.5), at(3, 10.5));
    assert_eq!(app.mail().calendar_prompt(), Some("Edit recurring event"));
    app.key(Key::Enter);
    assert_eq!(app.mail().calendar_prompt(), None);
    assert!(
        shown_on(&app, wednesday).contains(&"Focus time 10:00 AM".to_owned()),
        "{:?}",
        shown_on(&app, wednesday)
    );

    // Deleting the weekly sync: which events, then whether to tell its guests.
    let monday = date(2026, 11, 16);
    app.click(at(1, 10.2));
    assert_eq!(app.mail().calendar_details(), Some(false));
    app.click(Pos2::new(810.0, 327.0));
    app.click(Pos2::new(820.0, 407.0));
    assert_eq!(app.mail().calendar_prompt(), Some("Delete recurring event"));
    app.key(Key::ArrowDown);
    app.key(Key::ArrowDown);
    app.key(Key::Enter);
    assert_eq!(
        app.mail().calendar_prompt(),
        Some("Send a cancellation email?")
    );
    app.key(Key::Escape);
    assert_eq!(
        app.mail().calendar_prompt(),
        Some("Delete recurring event"),
        "Go back"
    );
    app.key(Key::Enter);
    app.key(Key::Enter);
    assert!(
        !shown_on(&app, monday)
            .iter()
            .any(|event| event.starts_with("Weekly product sync"))
    );
    assert!(
        !shown_on(&app, monday + jiff::ToSpan::days(7))
            .iter()
            .any(|event| event.starts_with("Weekly product sync")),
        "all of them"
    );
}

#[test]
fn an_edited_events_date_times_and_zone_are_typed() {
    let mut app = start("calendar-when-edit");
    app.key(Key::Num2);
    app.settle();
    double_click(&mut app, at(4, 19.8));
    assert_eq!(app.mail().calendar_form(), Some(false));
    // `Edit Date & Time` takes the keyboard to the start's date; Enter takes a typed date and
    // moves on to the start's time, then its end.
    palette(&mut app, "edit date");
    app.type_text("nov 20");
    app.key(Key::Enter);
    app.type_text("8p");
    app.key(Key::Enter);
    app.type_text("11p");
    app.key(Key::Enter);
    app.press(Key::Enter, CMD);
    assert_eq!(app.mail().calendar_form(), None);
    let friday = date(2026, 11, 20);
    assert!(
        shown_on(&app, friday).contains(&"Dinner with Jess 🍕 8:00 PM".to_owned()),
        "{:?}",
        shown_on(&app, friday)
    );
    assert!(
        !shown_on(&app, date(2026, 11, 19))
            .iter()
            .any(|event| event.starts_with("Dinner"))
    );

    // Another zone keeps the clock times: 8 pm in London is 3 pm here.
    app.key(Key::Num2);
    app.settle();
    double_click(&mut app, at(5, 20.3));
    assert_eq!(app.mail().calendar_form(), Some(false));
    palette(&mut app, "edit date");
    for _ in 0..4 {
        app.key(Key::Tab);
    }
    app.type_text("london");
    app.key(Key::Enter);
    app.press(Key::Enter, CMD);
    assert!(
        shown_on(&app, friday).contains(&"Dinner with Jess 🍕 3:00 PM".to_owned()),
        "{:?}",
        shown_on(&app, friday)
    );
}

#[test]
fn who_first_offers_everybody_in_the_conversation() {
    let mut app = start("calendar-everybody");
    // The first conversation open, a new event's Who empty: its people, once each, me left out.
    app.key(Key::Enter);
    app.settle();
    app.key(Key::B);
    app.key(Key::Tab);
    app.key(Key::Enter);
    let (_, guests) = app.mail().calendar_form_values().expect("the form");
    assert_eq!(guests, ["tom@brightmoor.co", "priya@brightmoor.co"]);
}

#[test]
fn about_is_rich_text() {
    let mut app = start("calendar-about");
    app.key(Key::Num2);
    app.settle();
    app.key(Key::B);
    palette(&mut app, "edit description");
    app.type_text("Agenda ");
    app.press(Key::B, CMD);
    app.type_text("first");
    let (about, _) = app.mail().calendar_form_about().expect("the form");
    assert!(
        about.contains("Agenda") && about.contains("<b>first</b>"),
        "{about}"
    );
}

#[test]
fn where_takes_a_place_apple_maps_suggests() {
    exo::places::answer_with(vec![
        exo::places::Place {
            name: "Lilia".into(),
            address: "567 Union Ave, Brooklyn, NY".into(),
        },
        exo::places::Place {
            name: "Lilac Wine".into(),
            address: "Lilac Wine, Berlin".into(),
        },
    ]);
    let mut app = start("calendar-where");
    app.key(Key::Num2);
    app.settle();
    app.key(Key::B);
    palette(&mut app, "edit location");
    app.type_text("lil");
    app.key(Key::ArrowDown);
    app.key(Key::ArrowDown);
    app.key(Key::Enter);
    let (_, location) = app.mail().calendar_form_about().expect("the form");
    assert_eq!(
        location, "Lilac Wine, Berlin",
        "the address alone when it starts with the name"
    );
    // What's typed and not picked stays as typed.
    app.press(Key::A, CMD);
    app.type_text("Rooftop");
    app.key(Key::Enter);
    assert_eq!(
        app.mail()
            .calendar_form_about()
            .map(|(_, location)| location),
        Some("Rooftop".to_owned())
    );
}

#[test]
fn tab_answers_an_invite_and_the_card_reads_the_answer_back() {
    let mut app = start("calendar-invite-keys");
    // Grace's invitation for coffee on the 25th, open.
    app.key(Key::J);
    app.key(Key::Enter);
    app.settle();
    assert_eq!(app.mail().invite_answer(), Some("needsAction"));
    // Tab takes the card's first button, → the next; Enter answers with it.
    app.key(Key::Tab);
    app.key(Key::ArrowRight);
    app.key(Key::Enter);
    app.settle();
    assert_eq!(app.mail().invite_answer(), Some("tentative"));
    let gmail = app.gmail();
    let query = gmail::calendar::EventsQuery {
        q: Some("Coffee with Grace".into()),
        single_events: true,
        ..Default::default()
    };
    let found = app
        .block_on(
            gmail
                .calendar()
                .events("maya@brightmoor.co", &query)
                .into_future(),
        )
        .expect("lists");
    let mine = found
        .items
        .iter()
        .flat_map(|event| &event.attendees)
        .find(|attendee| attendee.email.as_deref() == Some("maya@brightmoor.co"))
        .and_then(|attendee| attendee.response_status.clone());
    assert_eq!(mine.as_deref(), Some("tentative"));
    // Opened again, the card has the calendar's answer, not the invitation's.
    app.key(Key::Escape);
    app.key(Key::Escape);
    app.settle();
    app.key(Key::Enter);
    app.settle();
    app.wait(1.0);
    assert_eq!(app.mail().invite_answer(), Some("tentative"));
}

#[test]
fn an_invite_answer_calendar_refuses_offers_gmail() {
    let mut app = start("calendar-invite-refused");
    app.key(Key::J);
    app.key(Key::Enter);
    app.settle();
    assert_eq!(app.mail().invite_answer(), Some("needsAction"));
    app.fault(Some(serde_json::json!({ "method": "PATCH", "path": "/calendar/v3/calendars/", "status": 403, "message": "Forbidden", "reason": "forbidden" })));
    app.key(Key::Tab);
    app.key(Key::Enter);
    app.wait(0.5);
    let alert = app
        .mail()
        .alert()
        .map(|(title, text)| (title.to_owned(), text.to_owned()));
    assert_eq!(alert, Some(("Sorry!".to_owned(), "We couldn't RSVP due to an error in Gmail. Please try opening this message in Gmail to save your RSVP.".to_owned())));
    assert_eq!(
        app.mail().invite_answer(),
        Some("needsAction"),
        "the card shows the calendar's answer again"
    );
    // Escape closes it, without going anywhere.
    app.key(Key::Escape);
    assert_eq!(app.mail().alert(), None);
}

#[test]
fn find_time_offers_free_slots_and_a_slot_makes_the_meeting() {
    let mut app = start("calendar-find-time");
    app.key(Key::Num2);
    app.settle();
    // `m`: Meet's input; Lena from the suggestions, then Enter finds time.
    app.key(Key::M);
    app.type_text("lena");
    app.key(Key::Enter);
    app.key(Key::Enter);
    app.wait(0.5);
    assert!(app.mail().calendar_free_slots() > 0, "free slots this week");
    // Tuesday 9:00, the first: a meeting with Lena, named for the two of us.
    app.click(at(2, 9.25));
    let (title, guests) = app.mail().calendar_form_values().expect("the form");
    assert_eq!(
        (title.as_str(), guests),
        ("Maya / Lena", vec!["lena@brightmoor.co".to_owned()])
    );
    // Escape asks before the meeting goes (it has a title and a guest); then Escape ends Find Time.
    app.key(Key::Escape);
    assert_eq!(app.mail().calendar_prompt(), Some("Discard this event?"));
    app.key(Key::Enter);
    app.key(Key::Escape);
    assert_eq!(app.mail().calendar_free_slots(), 0);
    assert!(app.mail().calendar_week().is_some(), "the week stays");
}

#[test]
fn the_right_pane_s_form_gives_the_keyboard_to_the_mail_and_cmd_d_takes_it_back() {
    let mut app = start("calendar-form-away");
    app.key(Key::B);
    app.type_text("Lunch");
    // A click in the list: the mail has the keyboard, the form says how to come back.
    app.click(Pos2::new(300.0, 250.0));
    assert_eq!(app.mail().calendar_form_away(), Some(true));
    app.type_text("x");
    assert_eq!(
        app.mail()
            .calendar_form_values()
            .map(|(title, _)| title)
            .as_deref(),
        Some("Lunch"),
        "the title doesn't take the mail's keys"
    );
    // ⌘D: back to the title, where typing goes on.
    app.press(Key::D, CMD);
    assert_eq!(app.mail().calendar_form_away(), Some(false));
    app.type_text("!");
    assert!(
        app.mail()
            .calendar_form_values()
            .is_some_and(|(title, _)| title.contains('!')),
        "{:?}",
        app.mail().calendar_form_values()
    );
    // ⌘D again: the mail's.
    app.press(Key::D, CMD);
    assert_eq!(app.mail().calendar_form_away(), Some(true));
}

#[test]
fn a_guest_s_event_opens_its_details() {
    let mut app = start("calendar-guest-details");
    app.key(Key::Num2);
    app.settle();
    app.key(Key::M);
    app.type_text("priya");
    app.wait(0.3);
    app.key(Key::Enter);
    app.wait(1.0);
    app.settle();
    // Tuesday's Design crit, 3:00–4:00, in Priya's column (the day's right half).
    let mine = at(2, 15.5);
    app.click(Pos2::new(mine.x + 127.0 / 4.0, mine.y));
    assert_eq!(
        app.mail().calendar_details_event(),
        Some(("Design crit".to_owned(), "priya@brightmoor.co".to_owned()))
    );
}

#[test]
fn today_is_the_clocks_day_not_the_day_exo_opened() {
    let mut app = start("calendar-today-button");
    let later = gmail_mock::Config::default()
        .now
        .checked_add(jiff::SignedDuration::from_hours(2 * 24))
        .expect("in range");
    app.set_now(later);
    app.key(Key::Num0);
    assert_eq!(
        app.mail().calendar_day(),
        Some(date(2026, 11, 19)),
        "two days on"
    );
}

#[test]
fn a_new_event_repeats_and_shows_its_instances_at_once() {
    let mut app = start("calendar-repeat");
    app.key(Key::Num2);
    app.settle();
    app.key(Key::B);
    app.type_text("Standup");
    assert_eq!(
        app.mail().calendar_form_repeat(),
        Some(("Does not repeat".to_owned(), false))
    );
    // The presets, for the draft's Tuesday: Weekly on Tuesday is the third.
    app.click(REPEAT);
    app.key(Key::ArrowDown);
    app.key(Key::ArrowDown);
    app.key(Key::Enter);
    assert_eq!(
        app.mail().calendar_form_repeat(),
        Some(("Weekly on Tuesday".to_owned(), false))
    );
    app.press(Key::Enter, CMD);
    assert_eq!(app.mail().calendar_form(), None);
    // Its instances show before Calendar has the series.
    let (tuesday, next) = (date(2026, 11, 17), date(2026, 11, 24));
    assert!(
        shown_on(&app, tuesday).contains(&"Standup 9:00 AM".to_owned()),
        "{:?}",
        shown_on(&app, tuesday)
    );
    app.key(Key::Equals);
    app.settle();
    assert!(
        shown_on(&app, next).contains(&"Standup 9:00 AM".to_owned()),
        "next week's: {:?}",
        shown_on(&app, next)
    );
    // Past the hold, Calendar has the series and its rule.
    std::thread::sleep(std::time::Duration::from_millis(10_500));
    app.wait(1.0);
    let gmail = app.gmail();
    let query = gmail::calendar::EventsQuery {
        q: Some("Standup".into()),
        ..Default::default()
    };
    let found = app
        .block_on(
            gmail
                .calendar()
                .events("maya@brightmoor.co", &query)
                .into_future(),
        )
        .expect("lists");
    let rules: Vec<Vec<String>> = found
        .items
        .iter()
        .map(|event| event.recurrence.clone())
        .collect();
    assert_eq!(
        rules,
        [vec!["RRULE:FREQ=WEEKLY;BYDAY=TU;WKST=SU".to_owned()]]
    );
    assert!(
        shown_on(&app, next).contains(&"Standup 9:00 AM".to_owned()),
        "Calendar's instances"
    );
}

#[test]
fn repeat_s_custom_dialog_makes_a_rule_and_escape_closes_it() {
    let mut app = start("calendar-repeat-custom");
    app.key(Key::Num2);
    app.settle();
    app.key(Key::B);
    app.click(REPEAT);
    for _ in 0..6 {
        app.key(Key::ArrowDown);
    }
    app.key(Key::Enter);
    assert_eq!(
        app.mail().calendar_form_repeat(),
        Some(("Does not repeat".to_owned(), true)),
        "Custom opens its dialog"
    );
    app.key(Key::Escape);
    assert_eq!(
        app.mail().calendar_form_repeat(),
        Some(("Does not repeat".to_owned(), false)),
        "Escape closes it, not the form"
    );
    assert_eq!(app.mail().calendar_form(), Some(true));
    app.click(REPEAT);
    for _ in 0..6 {
        app.key(Key::ArrowDown);
    }
    app.key(Key::Enter);
    // ⌘Enter is Done: weekly on the draft's day, said as a custom rule is.
    app.press(Key::Enter, CMD);
    assert_eq!(
        app.mail().calendar_form_repeat(),
        Some(("Weekly on Tue".to_owned(), false))
    );
    assert_eq!(app.mail().calendar_form(), Some(true), "the form stays");
}

#[test]
fn the_day_view_s_today_and_plus_and_t() {
    let mut app = start("calendar-today-button");
    app.key(Key::Num0);
    app.key(Key::Equals);
    assert_eq!(app.mail().calendar_day(), Some(date(2026, 11, 18)));
    app.click(DAY_TODAY);
    assert_eq!(app.mail().calendar_day(), Some(date(2026, 11, 17)));
    app.key(Key::Equals);
    app.key(Key::T);
    assert_eq!(
        app.mail().calendar_day(),
        Some(date(2026, 11, 17)),
        "t opens today"
    );
    app.click(DAY_CREATE);
    assert_eq!(app.mail().calendar_form(), Some(true));
}

#[test]
fn this_and_following_events_take_an_edit_as_a_new_series() {
    let mut app = start("calendar-following");
    app.key(Key::Num2);
    app.settle();
    let (wednesday, friday) = (date(2026, 11, 18), date(2026, 11, 20));
    drag(&mut app, at(3, 9.5), at(3, 10.5));
    assert_eq!(app.mail().calendar_prompt(), Some("Edit recurring event"));
    app.key(Key::ArrowDown);
    app.key(Key::Enter);
    assert_eq!(app.mail().calendar_prompt(), None, "no guests to tell");
    // Shown at once: this one and those after it, an hour later.
    assert!(
        shown_on(&app, wednesday).contains(&"Focus time 10:00 AM".to_owned()),
        "{:?}",
        shown_on(&app, wednesday)
    );
    assert!(
        shown_on(&app, friday).contains(&"Focus time 10:00 AM".to_owned()),
        "{:?}",
        shown_on(&app, friday)
    );
    // Past the hold: the series ends before Wednesday's, and a new one takes over from it.
    std::thread::sleep(std::time::Duration::from_millis(10_500));
    app.wait(1.0);
    let gmail = app.gmail();
    let query = gmail::calendar::EventsQuery {
        q: Some("Focus time".into()),
        ..Default::default()
    };
    let found = app
        .block_on(
            gmail
                .calendar()
                .events("maya@brightmoor.co", &query)
                .into_future(),
        )
        .expect("lists");
    let mut series: Vec<(Option<String>, Vec<String>)> = found
        .items
        .iter()
        .map(|event| {
            (
                event
                    .start
                    .as_ref()
                    .and_then(|start| start.date_time.clone()),
                event.recurrence.clone(),
            )
        })
        .collect();
    series.sort();
    assert_eq!(series.len(), 2, "{series:?}");
    assert_eq!(
        series[0].1,
        ["RRULE:FREQ=WEEKLY;BYDAY=WE,FR;UNTIL=20261118T135959Z"]
    );
    assert_eq!(
        series[1],
        (
            Some("2026-11-18T10:00:00-05:00".to_owned()),
            vec!["RRULE:FREQ=WEEKLY;BYDAY=WE,FR;WKST=SU".to_owned()]
        )
    );
    assert!(
        shown_on(&app, friday).contains(&"Focus time 10:00 AM".to_owned()),
        "Calendar's instances: {:?}",
        shown_on(&app, friday)
    );
}

#[test]
fn a_series_details_say_its_rule() {
    let mut app = start("calendar-rule");
    app.key(Key::Num2);
    app.settle();
    app.click(at(1, 10.2));
    assert_eq!(app.mail().calendar_details(), Some(false));
    app.wait(0.5);
    assert_eq!(
        app.mail().calendar_details_rule().as_deref(),
        Some("Weekly on Monday"),
        "Calendar's rule for the series"
    );
}

#[test]
fn the_details_date_opens_the_form_on_the_start_s_date() {
    let mut app = start("calendar-details-date");
    app.key(Key::Num2);
    app.settle();
    // The dinner's details, then their date (`Thu, Nov 19`, 183,679 64×16).
    app.click(at(4, 19.8));
    assert_eq!(app.mail().calendar_details(), Some(false));
    app.click(Pos2::new(210.0, 687.0));
    assert_eq!(app.mail().calendar_form(), Some(false));
    app.type_text("nov 20");
    app.key(Key::Enter);
    app.press(Key::Enter, CMD);
    assert_eq!(app.mail().calendar_form(), None);
    assert!(
        shown_on(&app, date(2026, 11, 20)).contains(&"Dinner with Jess 🍕 7:30 PM".to_owned()),
        "{:?}",
        shown_on(&app, date(2026, 11, 20))
    );
}

#[test]
fn a_guest_s_calendar_shows_beside_mine_and_their_chip_says_when_they_re_busy() {
    let mut app = start("calendar-guest-columns");
    app.key(Key::Num2);
    app.settle();
    // A new event (Tuesday 9:00 to 9:30) with Priya, a colleague, and Grace, who isn't.
    app.key(Key::B);
    app.key(Key::Tab);
    for guest in ["priya", "grace"] {
        app.type_text(guest);
        app.wait(0.3);
        app.key(Key::Enter);
    }
    app.wait(1.0);
    app.settle();
    let tuesday = date(2026, 11, 17);
    let columns = app.mail_mut().calendar_guest_columns(tuesday);
    assert_eq!(
        columns,
        [(
            "priya@brightmoor.co".to_owned(),
            vec!["Priya / Maya 1:1".to_owned(), "Design crit".to_owned()]
        )],
        "Grace's calendar isn't shared"
    );
    let chips = app.mail().calendar_form_chips();
    assert_eq!(
        chips,
        [
            (
                "grace@juniperpartners.vc".to_owned(),
                "empty".to_owned(),
                None
            ),
            ("priya@brightmoor.co".to_owned(), "pink".to_owned(), None)
        ],
        "Priya's colour, after my blue; she's free at 9"
    );
}

#[test]
fn a_calendar_s_colour_picker_takes_the_keyboard() {
    let mut app = start("calendar-recolor-keys");
    app.key(Key::Num2);
    app.settle();
    app.hover(egui::pos2(1100.0, 501.0));
    app.click(egui::pos2(1256.0, 501.0));
    let color = |app: &Fixture| {
        let account = app.mail().account();
        account.settings().calendar_accounts[account.email()].calendars["maya@brightmoor.co"]
            .color
            .clone()
    };
    let opened = color(&app);
    // → applies the next colour at once, ↓ the one a row on, and Escape puts the first back and
    // closes the picker.
    app.key(Key::ArrowRight);
    let next = color(&app);
    assert_ne!(next, opened, "→ applies the next colour at once");
    app.key(Key::ArrowDown);
    assert_ne!(color(&app), next);
    app.key(Key::Escape);
    assert_eq!(color(&app), opened, "Escape puts the colour back");
    assert!(
        app.mail().calendar_week().is_some(),
        "Escape closed the picker, not the week"
    );
    // Enter keeps a new one and closes the picker; ← then is the week's again.
    app.click(egui::pos2(1256.0, 501.0));
    app.key(Key::ArrowLeft);
    let picked = color(&app);
    app.key(Key::Enter);
    assert_eq!(color(&app), picked);
    app.key(Key::ArrowLeft);
    assert_eq!(color(&app), picked, "the picker closed");
}

#[test]
fn a_calendar_s_menu_picks_its_colour() {
    let mut app = start("calendar-recolor");
    app.key(Key::Num2);
    app.settle();
    // `maya@brightmoor.co`'s row: its `…` shows on hover, and opens the 25 colours.
    app.hover(egui::pos2(1100.0, 501.0));
    app.click(egui::pos2(1256.0, 501.0));
    // Red, the first, at the picker's top left (its box 16px past the button, 8px under it).
    app.click(egui::pos2(1092.0 + 7.0 + 16.0, 516.0 + 6.0 + 16.0));
    let account = app.mail().account();
    let color =
        account.settings().calendar_accounts[account.email()].calendars["maya@brightmoor.co"]
            .color
            .clone();
    assert_eq!(color.as_deref(), Some("red"));
}
