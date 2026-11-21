//! Remind me end to end on gmail-mock: `H` snoozes the cursor's conversation into Reminders, `Z`
//! takes it back, the footer removes it, and the conversation comes back to the inbox once its
//! reminder is due, by the app's clock. Compose's ⌘⇧H gives a reply a reminder its thread gets
//! from Send on, the follow-up hint does for a message just sent, Move takes a thread out of
//! Reminders, the Reminders dialog keeps the defaults, and Automatic Reminders put them on
//! drafts to someone outside.

mod support;

use egui::{Key, Modifiers};
use exo_store::{DraftReminder, ReminderStatus, TriggerCondition};
use support::{Fixture, start, start_with_settings};

const COMMAND: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);

/// Thursday at 8:00 in New York: a reminder's `in 2 days` at the mock's now.
fn thursday_morning() -> jiff::Timestamp {
    "2026-11-19T08:00[America/New_York]"
        .parse::<jiff::Zoned>()
        .expect("a time")
        .timestamp()
}

/// Opens the first conversation and starts a reply.
fn reply(app: &mut Fixture) {
    app.key(Key::Enter);
    app.key(Key::R);
    app.type_text("Sounds good");
}

/// The first conversation in the mock's inbox.
const FIRST: &str = "Design crit moved to 3pm";

/// `g` then `h`: the Reminders folder.
fn go_to_reminders(app: &mut Fixture) {
    app.key(Key::G);
    app.key(Key::H);
}

/// The reminder of the row titled `subject` on screen.
fn reminder(app: &Fixture, subject: &str) -> Option<ReminderStatus> {
    app.mail()
        .inbox()
        .threads()
        .iter()
        .find(|thread| thread.subject == subject)
        .and_then(|thread| thread.reminder.clone())
}

/// Every label on thread `id`'s messages in Gmail, Exo's own by name.
fn gmail_labels(app: &Fixture, id: &str) -> Vec<String> {
    let gmail = app.gmail();
    app.block_on(async {
        let names: Vec<(String, String)> = gmail
            .gmail()
            .labels()
            .await
            .expect("labels.list")
            .labels
            .into_iter()
            .filter_map(|label| Some((label.id?, label.name?)))
            .collect();
        let thread = gmail
            .gmail()
            .thread(id, gmail::Format::Minimal, &[])
            .await
            .expect("threads.get");
        let mut labels: Vec<String> = thread
            .messages
            .into_iter()
            .flat_map(|message| message.label_ids)
            .map(|id| {
                names
                    .iter()
                    .find(|(label, _)| *label == id)
                    .map_or(id.clone(), |(_, name)| name.clone())
            })
            .collect();
        labels.sort();
        labels.dedup();
        labels
    })
}

/// The cursor's thread id.
fn cursor_id(app: &Fixture) -> String {
    app.mail()
        .inbox()
        .cursor()
        .map(|thread| thread.id.clone())
        .expect("a cursor")
}

#[test]
fn h_snoozes_until_tomorrow_morning_and_z_takes_it_back() {
    let mut app = start("remind-snooze");
    assert_eq!(app.cursor(), FIRST);
    let id = cursor_id(&app);
    app.key(Key::H);
    app.key(Key::Enter);
    app.settle();
    assert!(
        !app.subjects().contains(&FIRST.to_owned()),
        "out of the inbox"
    );
    let labels = gmail_labels(&app, &id);
    assert!(
        labels.contains(&"[Exo]/Is Snoozed".to_owned()) && !labels.contains(&"INBOX".to_owned()),
        "Gmail sees it leave: {labels:?}"
    );

    go_to_reminders(&mut app);
    app.settle();
    let tomorrow: jiff::Zoned = "2026-11-18T08:00[America/New_York]"
        .parse()
        .expect("a time");
    assert_eq!(
        reminder(&app, FIRST),
        Some(ReminderStatus::Set {
            at: tomorrow.timestamp(),
            keep_on_reply: false
        })
    );

    app.key(Key::Z);
    app.settle();
    assert!(
        app.subjects().contains(&FIRST.to_owned()),
        "undo goes back to the inbox with it"
    );
    assert_eq!(reminder(&app, FIRST), None);
}

#[test]
fn typed_times_and_regardless_and_the_footer_removes_it() {
    let mut app = start("remind-typed");
    app.key(Key::H);
    app.type_text("3 days");
    // ⌘⇧H cycles the dropdown to `regardless`.
    app.press(
        Key::H,
        Modifiers::MAC_CMD
            .plus(Modifiers::COMMAND)
            .plus(Modifiers::SHIFT),
    );
    app.key(Key::Enter);
    app.settle();
    go_to_reminders(&mut app);
    app.settle();
    let friday: jiff::Zoned = "2026-11-20T08:00[America/New_York]"
        .parse()
        .expect("a time");
    assert_eq!(
        reminder(&app, FIRST),
        Some(ReminderStatus::Set {
            at: friday.timestamp(),
            keep_on_reply: true
        })
    );
    assert_eq!(
        app.mail()
            .account()
            .settings()
            .previous_reminder
            .as_ref()
            .map(|previous| previous.suggestion.title.as_str()),
        Some("in 3 days")
    );

    // In Reminders the thread isn't in the inbox: the footer moves it back too.
    app.key(Key::H);
    app.press(Key::ArrowUp, Modifiers::MAC_CMD.plus(Modifiers::COMMAND));
    app.key(Key::ArrowUp);
    app.key(Key::Enter);
    app.settle();
    assert!(
        !app.subjects().contains(&FIRST.to_owned()),
        "out of Reminders"
    );
    app.key(Key::G);
    app.key(Key::I);
    app.settle();
    assert!(
        app.subjects().contains(&FIRST.to_owned()),
        "and back in the inbox"
    );
}

#[test]
fn a_due_reminder_brings_the_thread_back_marked() {
    let mut app = start("remind-due");
    let id = cursor_id(&app);
    app.key(Key::H);
    app.type_text("10am");
    app.key(Key::Enter);
    app.settle();
    assert!(!app.subjects().contains(&FIRST.to_owned()));

    app.set_now(
        "2026-11-17T10:01[America/New_York]"
            .parse()
            .expect("a time"),
    );
    app.settle();
    app.sync();
    app.settle();
    assert!(
        app.subjects().contains(&FIRST.to_owned()),
        "back once it's due"
    );
    assert!(
        matches!(
            reminder(&app, FIRST),
            Some(ReminderStatus::Returned { unread: true, .. })
        ),
        "with its pink dot: {:?}",
        reminder(&app, FIRST)
    );
    let labels = gmail_labels(&app, &id);
    assert!(
        labels.contains(&"[Exo]/ru".to_owned())
            && labels.contains(&"INBOX".to_owned())
            && !labels.contains(&"[Exo]/Is Snoozed".to_owned()),
        "{labels:?}"
    );
}

#[test]
fn a_reply_with_a_reminder_gives_its_thread_one_from_send() {
    let mut app = start("remind-compose");
    let id = cursor_id(&app);
    reply(&mut app);
    // ⌘⇧H: Draft mode's rows, `in 2 days` first.
    app.press(Key::H, COMMAND.plus(Modifiers::SHIFT));
    app.key(Key::Enter);
    let picked = DraftReminder {
        at: thursday_morning(),
        keep_on_reply: false,
        automatic: false,
    };
    assert_eq!(app.mail().compose_reminder(), Some(picked));
    assert_eq!(
        app.mail().toast_text(),
        None,
        "a draft's reminder says nothing until it's sent"
    );

    // Sent, held for undo: the thread has it at once.
    let thursday = Some(ReminderStatus::Set {
        at: thursday_morning(),
        keep_on_reply: false,
    });
    app.press(Key::Enter, COMMAND);
    app.settle();
    assert_eq!(reminder(&app, FIRST), thursday, "from Send on");
    // Undo send takes it back with the send; the draft keeps it.
    app.key(Key::Z);
    app.settle();
    assert_eq!(reminder(&app, FIRST), None);
    assert_eq!(app.mail().compose_reminder(), Some(picked));

    app.press(Key::Enter, COMMAND);
    // Instant Send (⌘⇧Z on the toast).
    app.press(Key::Z, COMMAND.plus(Modifiers::SHIFT));
    app.wait(1.5);
    app.settle();
    assert!(
        app.subjects().contains(&FIRST.to_owned()) || app.mail().thread().is_some(),
        "not marked done"
    );
    let labels = gmail_labels(&app, &id);
    assert!(
        labels.contains(&"[Exo]/Is Snoozed".to_owned()) && labels.contains(&"INBOX".to_owned()),
        "a reminder, still in the inbox: {labels:?}"
    );
    app.key(Key::Escape);
    go_to_reminders(&mut app);
    app.settle();
    assert_eq!(
        reminder(&app, FIRST),
        Some(ReminderStatus::Set {
            at: thursday_morning(),
            keep_on_reply: false
        })
    );
}

#[test]
fn the_follow_up_hint_reminds_about_the_message_just_sent() {
    let mut app = start("remind-follow-up");
    app.mail_mut()
        .account_mut()
        .update_device_settings(|device| device.local_sends_count = 6);
    reply(&mut app);
    app.press(Key::Enter, COMMAND);
    assert_eq!(
        app.mail().compose_alert(),
        Some(
            "Hit H on any email you've sent, and choose a time. If you don't hear back by then, we'll remind you to follow up."
        )
    );
    // `h` is Set reminder; the picker has a reminder's rows.
    app.key(Key::H);
    assert_eq!(app.mail().compose_alert(), None);
    app.key(Key::Enter);
    assert_eq!(
        app.mail().toast_text(),
        Some("Reminder set: Thursday, November 19th if no reply.")
    );
    assert_eq!(
        app.mail().hint_text().as_deref(),
        Some("Nice! Next time hit ⌘ shift H before sending, or H on any sent email.")
    );
    app.settle();
    let held = app
        .mail()
        .account()
        .snapshot()
        .drafts
        .iter()
        .find_map(|draft| draft.record.reminder);
    assert_eq!(
        held,
        Some(DraftReminder {
            at: thursday_morning(),
            keep_on_reply: false,
            automatic: false
        }),
        "the held send carries it to its thread"
    );
    assert!(
        app.mail()
            .account()
            .settings()
            .has_shown_follow_up_reminder_hint
    );
    assert_eq!(app.mail().account().device_settings().local_sends_count, 7);
}

#[test]
fn moving_a_thread_out_of_reminders_removes_its_reminder() {
    let mut app = start("remind-move");
    app.key(Key::H);
    app.key(Key::Enter);
    app.settle();
    go_to_reminders(&mut app);
    app.settle();
    assert!(reminder(&app, FIRST).is_some());
    app.key(Key::V);
    app.type_text("inbox");
    app.key(Key::Enter);
    assert_eq!(
        app.mail().toast_text(),
        Some("Reminder removed and moved to Inbox.")
    );
    app.settle();
    assert!(
        !app.subjects().contains(&FIRST.to_owned()),
        "out of Reminders"
    );
    app.key(Key::G);
    app.key(Key::I);
    app.settle();
    assert!(app.subjects().contains(&FIRST.to_owned()), "in the inbox");
    assert_eq!(reminder(&app, FIRST), None);
}

#[test]
fn regardless_picked_from_the_menu_hints_at_the_key() {
    let mut app = start("remind-regardless-hint");
    app.key(Key::H);
    // Tab opens the dropdown's menu; ↓ and Enter pick `regardless`.
    app.key(Key::Tab);
    app.key(Key::ArrowDown);
    app.key(Key::Enter);
    assert_eq!(
        app.mail().hint_text().as_deref(),
        Some("Hit ⌘ shift H to toggle this reminder setting!")
    );
    app.key(Key::Enter);
    app.settle();
    go_to_reminders(&mut app);
    app.settle();
    assert!(matches!(
        reminder(&app, FIRST),
        Some(ReminderStatus::Set {
            keep_on_reply: true,
            ..
        })
    ));
}

#[test]
fn the_reminders_dialog_keeps_the_defaults() {
    let mut app = start("remind-settings");
    app.press(Key::K, COMMAND);
    app.type_text("reminder settings");
    app.key(Key::Enter);
    app.settle();
    assert!(app.mail().settings_dialog_open());
    // The days are selected: typing replaces them.
    app.type_text("3");
    app.key(Key::Tab);
    app.type_text("9:30 pm");
    app.key(Key::Tab);
    app.key(Key::Enter);
    app.key(Key::ArrowDown);
    app.key(Key::Enter);
    app.settle();
    let saved = app.mail().account().settings().reminders.clone();
    assert_eq!(
        (
            saved.days,
            saved.hour,
            saved.minute,
            saved.trigger_condition
        ),
        (3, 21, 30, TriggerCondition::Regardless)
    );
    app.key(Key::Escape);
    app.settle();
    assert!(!app.mail().settings_dialog_open());

    // Remind me takes its time, and starts on `regardless`.
    app.key(Key::H);
    app.key(Key::Enter);
    app.settle();
    go_to_reminders(&mut app);
    app.settle();
    let tomorrow: jiff::Zoned = "2026-11-18T21:30[America/New_York]"
        .parse()
        .expect("a time");
    assert_eq!(
        reminder(&app, FIRST),
        Some(ReminderStatus::Set {
            at: tomorrow.timestamp(),
            keep_on_reply: true
        })
    );
}

#[test]
fn automatic_reminders_go_on_a_draft_to_someone_outside() {
    let mut app = start_with_settings("remind-automatic", &|settings| {
        settings.reminders.default_reminders_trigger = "EXTERNAL".into()
    });
    // Tom is a colleague: a reply to him gets none.
    reply(&mut app);
    app.settle();
    assert_eq!(
        app.mail().compose_reminder(),
        None,
        "Tom is at brightmoor.co, as Maya is"
    );

    // Grace isn't.
    app.key(Key::Escape);
    app.key(Key::Escape);
    let at = app
        .subjects()
        .iter()
        .position(|subject| subject == "Coffee next week?")
        .expect("Grace's thread");
    for _ in 0..at {
        app.key(Key::J);
    }
    reply(&mut app);
    app.settle();
    let automatic = DraftReminder {
        at: thursday_morning(),
        keep_on_reply: false,
        automatic: true,
    };
    assert_eq!(
        app.mail().compose_reminder(),
        Some(automatic),
        "in 2 days at 8:00, weekdays unset"
    );

    // ⌘K's Remove Reminder takes it off until the recipients change.
    app.press(Key::K, COMMAND);
    app.type_text("remove reminder");
    app.key(Key::Enter);
    app.settle();
    assert_eq!(app.mail().compose_reminder(), None, "and it stays off");
}
