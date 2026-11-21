//! The first-run flow after the welcome splash: the discovery questions, account setup's loading
//! screen and split step, and what ends it.

mod support;

use egui::{Key, pos2};
use exo_store::{Answer, DiscoveryQuestions};
use support::{Fixture, start_new_account};

/// The Calendar split's row on the split step, the first after Important; its switch; and
/// `Continue with this setup`.
const CALENDAR_ROW: egui::Pos2 = pos2(300.0, 345.0);
const CALENDAR_SWITCH: egui::Pos2 = pos2(523.0, 355.0);
const CONTINUE: egui::Pos2 = pos2(341.0, 755.0);
/// The first question's third option, its `Labels` tile, and its `Next` under the tiles.
const FORGET_TO_RESPOND: egui::Pos2 = pos2(500.0, 270.0);
const LABELS: egui::Pos2 = pos2(480.0, 455.0);
const FIRST_NEXT: egui::Pos2 = pos2(640.0, 626.0);

fn calendar_on(app: &Fixture) -> bool {
    app.mail()
        .account()
        .settings()
        .enabled_splits()
        .any(|split| split.kind.as_deref() == Some("calendar"))
}

/// Account setup on a new account, past its loading screen.
fn setup(name: &str) -> Fixture {
    let mut app = start_new_account(name);
    app.with_sample_split_library();
    app.mail_mut().open_account_setup();
    app.wait(1.0);
    assert!(app.mail().account_setup_open() && app.mail().first_run_covers());
    assert_eq!(
        app.mail().account_setup_splits(),
        None,
        "tailoring the inbox first"
    );
    // Four messages, two seconds each.
    app.wait(8.0);
    app
}

#[test]
fn account_setup_offers_the_splits_the_inbox_has_mail_for_then_gets_you_to_zero() {
    let mut app = setup("account-setup");
    // The mock's inbox has two invitations, and nothing from the sample library's other senders.
    assert_eq!(
        app.mail().account_setup_splits(),
        Some(vec![("Calendar".to_owned(), true)])
    );
    assert!(calendar_on(&app), "the split is on while it's offered");

    // Its row turns it off and on again.
    app.click(CALENDAR_ROW);
    assert_eq!(
        app.mail().account_setup_splits(),
        Some(vec![("Calendar".to_owned(), false)])
    );
    assert!(!calendar_on(&app));
    app.click(CALENDAR_ROW);
    assert!(calendar_on(&app));

    // The mail's keys don't get through: 2 doesn't open the week.
    app.key(Key::Num2);
    assert_eq!(app.mail().calendar_week(), None);

    // `Continue with this setup`: an inbox of more than ten ends in Get Me To Zero.
    app.click(CONTINUE);
    app.settle();
    let mail = app.mail();
    assert!(!mail.account_setup_open() && !mail.account().device_settings().account_setup_pending);
    assert!(calendar_on(&app), "the split stays on");
    assert!(
        !app.mail().account().settings().unified_inbox,
        "with Important and Other"
    );
    assert!(app.mail().get_me_to_zero_open() && !app.mail().welcome_alert_open());
}

#[test]
fn quitting_in_account_setup_opens_it_again() {
    let mut app = setup("account-setup-again");
    assert!(app.mail().account_setup_splits().is_some());
    app.relaunch();
    assert!(
        app.mail().account_setup_open(),
        "the next launch picks setup up again"
    );
    app.wait(9.0);
    assert_eq!(
        app.mail().account_setup_splits(),
        Some(vec![("Calendar".to_owned(), true)])
    );
    app.key(Key::Enter);
    app.relaunch();
    assert!(
        !app.mail().account_setup_open(),
        "once it's been through, never again"
    );
}

#[test]
fn account_setup_without_a_split_to_offer_ends_after_loading() {
    // No library: nothing to offer, so the split step is skipped (`r42`).
    let mut app = start_new_account("account-setup-nothing");
    app.mail_mut().open_account_setup();
    app.wait(9.0);
    assert!(!app.mail().account_setup_open());
    assert!(app.mail().get_me_to_zero_open());
}

/// A new account's first time, past the welcome splash: the discovery questions.
fn questions(name: &str) -> Fixture {
    let mut app = start_new_account(name);
    app.with_sample_split_library();
    app.mail_mut().welcome_first_time();
    // Five seconds once its photo is up.
    for _ in 0..40 {
        if !app.mail().welcome_covers() {
            break;
        }
        app.wait(0.5);
    }
    let mail = app.mail();
    assert!(mail.discovery_open() && mail.first_run_covers() && !mail.welcome_covers());
    assert!(
        !mail.account().settings().do_show_tutorial
            && mail.account().device_settings().ftue_in_progress
    );
    app
}

fn keys(app: &mut Fixture, keys: &[Key]) {
    for key in keys {
        app.key(*key);
    }
}

#[test]
fn the_discovery_questions_keep_their_answers_and_lead_to_account_setup() {
    let mut app = questions("discovery");
    // The mail's keys don't get through: 2 doesn't open the week.
    app.key(Key::Num2);
    assert_eq!(app.mail().calendar_week(), None);

    // The first question by pointer: a dislike, then a tile, then Next.
    app.click(FORGET_TO_RESPOND);
    app.click(LABELS);
    app.click(FIRST_NEXT);
    app.wait(1.0);
    let answered = app.mail().account().settings().discovery_questions.clone();
    assert_eq!(answered.pain_point.answer, "followup");
    assert_eq!(answered.workflow.answer, "labels");

    // The second by keyboard: its first dropdown has the keys; ↓ opens it and highlights, Enter
    // chooses and (let go) moves to the next field, which the choice brought up.
    keys(
        &mut app,
        &[Key::ArrowDown, Key::ArrowDown, Key::ArrowDown, Key::Enter],
    );
    keys(&mut app, &[Key::ArrowDown, Key::ArrowDown, Key::Enter]);
    keys(
        &mut app,
        &[Key::ArrowDown, Key::ArrowDown, Key::ArrowDown, Key::Enter],
    );
    // The last Enter moved to `Next`, enabled now; Enter clicks it.
    app.key(Key::Enter);
    app.wait(1.0);
    assert!(app.mail().discovery_open(), "the last question");

    // The last: its dropdown has the keys from the start; `Friend or colleague` focuses `Next`.
    keys(
        &mut app,
        &[Key::ArrowDown, Key::ArrowDown, Key::Enter, Key::Enter],
    );
    app.settle();
    let mail = app.mail();
    assert!(
        !mail.discovery_open() && mail.account_setup_open(),
        "account setup is next"
    );
    assert!(mail.account().device_settings().account_setup_pending);
    let answer = |id: &str, other: Option<&str>| Answer {
        answer: id.to_owned(),
        other_input: other.map(str::to_owned),
    };
    assert_eq!(
        mail.account().settings().discovery_questions,
        DiscoveryQuestions {
            pain_point: answer("followup", None),
            workflow: answer("labels", None),
            your_role: answer("sales", Some("")),
            title: answer("executive", None),
            company_size: answer("2-50", None),
            how_did_you_hear: answer("friend_or_colleague", Some("")),
        }
    );
}

#[test]
fn quitting_in_the_questions_gets_you_to_zero_at_the_next_launch() {
    let mut app = questions("discovery-interrupted");
    app.relaunch();
    let mail = app.mail();
    assert!(
        !mail.discovery_open() && !mail.account_setup_open(),
        "the questions don't come back"
    );
    // Get Me To Zero comes up as the questions were left: the mock's inbox has more than ten
    // conversations.
    assert!(mail.get_me_to_zero_open());
    assert!(
        mail.account().device_settings().gmtz_auto_shown
            && !mail.account().device_settings().ftue_in_progress
    );
    app.relaunch();
    assert!(!app.mail().get_me_to_zero_open(), "only once");
}

#[test]
fn get_me_to_zero_run_before_isnt_offered_after_an_interrupted_first_run() {
    let mut app = questions("discovery-interrupted-zero");
    app.mail_mut()
        .account_mut()
        .update_device_settings(|device| device.gmtz_started = true);
    app.relaunch();
    assert!(!app.mail().get_me_to_zero_open());
}

#[test]
fn account_setup_keys_follow_what_was_clicked() {
    let mut app = setup("account-setup-focus");
    // A click on a row toggles it and focuses the content, where Enter does nothing.
    app.click(CALENDAR_ROW);
    assert!(!calendar_on(&app));
    app.key(Key::Enter);
    assert!(app.mail().account_setup_open());
    // The switch takes the keyboard itself: Space toggles it again.
    app.click(CALENDAR_SWITCH);
    assert!(calendar_on(&app));
    app.key(Key::Space);
    assert!(!calendar_on(&app));
    // Tab goes to the button's region, where the keys stop.
    app.key(Key::Tab);
    app.key(Key::Enter);
    app.key(Key::Tab);
    app.key(Key::Space);
    assert!(app.mail().account_setup_open() && !calendar_on(&app));
    app.click(CONTINUE);
    app.settle();
    assert!(!app.mail().account_setup_open());
}
