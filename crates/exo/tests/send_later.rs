//! Send later end to end on gmail-mock: ⌘⇧L's picker gives a draft its time, Send schedules
//! it, a Gmail draft meanwhile that its thread shows as scheduled and Scheduled lists, and Undo
//! or the time passing take it back.

mod support;

use egui::{Key, Modifiers};
use gmail::{DraftQuery, Format, ListQuery};
use jiff::{Timestamp, ToSpan as _, Zoned, tz::TimeZone};
use support::{Fixture, start_with_settings};

const COMMAND: Modifiers = Modifiers::MAC_CMD.plus(Modifiers::COMMAND);
const REPLY_SUBJECT: &str = "Re: Design crit moved to 3pm";

/// Now in the mock's New York. The store sends by the system's clock, so the app's has to agree
/// for "tomorrow morning" to be ahead for both. Before 4 am the picker offers this morning
/// instead, so the app's clock starts at 4 am then: still ahead of the store's, and tomorrow
/// morning is the first row.
fn now() -> Zoned {
    let now = Zoned::now().with_time_zone(TimeZone::get("America/New_York").expect("a zone"));
    if now.hour() < 4 {
        now.date()
            .at(4, 0, 0, 0)
            .to_zoned(now.time_zone().clone())
            .expect("a time")
    } else {
        now
    }
}

/// The app on the system's clock. Within a week of the mock calendar's out-of-office days the
/// nudge would open over the thread, so these start opted out of it.
fn start(name: &str) -> Fixture {
    start_with_settings(name, &|settings| {
        settings.out_of_office_event_detection.opted_out_at = Some("2026-11-17T00:00:00Z".into())
    })
}

/// Tomorrow at 8:00 in New York.
fn tomorrow_morning(now: &Zoned) -> Timestamp {
    now.date()
        .checked_add(1.day())
        .expect("a day")
        .at(8, 0, 0, 0)
        .to_zoned(now.time_zone().clone())
        .expect("a time")
        .timestamp()
}

fn subjects(app: &Fixture, sent: bool) -> Vec<String> {
    let gmail = app.gmail();
    app.block_on(async {
        let ids: Vec<String> = if sent {
            let query = ListQuery {
                label_ids: vec!["SENT".into()],
                max_results: Some(500),
                ..ListQuery::default()
            };
            gmail
                .gmail()
                .messages(&query)
                .await
                .expect("messages.list")
                .messages
                .into_iter()
                .filter_map(|message| message.id)
                .collect()
        } else {
            let mut ids = Vec::new();
            for draft in gmail
                .gmail()
                .drafts(&DraftQuery::default())
                .await
                .expect("drafts.list")
                .drafts
            {
                ids.extend(
                    gmail
                        .gmail()
                        .draft(&draft.id.expect("an id"), Format::Minimal)
                        .await
                        .expect("drafts.get")
                        .message
                        .and_then(|message| message.id),
                );
            }
            ids
        };
        let mut out = Vec::new();
        for id in ids {
            out.push(
                gmail
                    .gmail()
                    .message(&id, Format::Metadata, &["Subject"])
                    .await
                    .expect("messages.get")
                    .header("Subject")
                    .unwrap_or_default()
                    .to_owned(),
            );
        }
        out
    })
}

/// Opens the first thread, replies, and picks the picker's first row (`tomorrow morning`).
fn reply_for_tomorrow(app: &mut Fixture) {
    app.key(Key::Enter);
    app.key(Key::R);
    app.type_text("Sounds good");
    app.press(Key::L, COMMAND.plus(Modifiers::SHIFT));
    app.key(Key::Enter);
}

#[test]
fn a_reply_scheduled_for_tomorrow_waits_in_scheduled_as_a_draft() {
    let mut app = start("send-later-reply");
    let now = now();
    app.set_now(now.clone());
    reply_for_tomorrow(&mut app);
    assert_eq!(
        app.mail().compose_send_later(),
        Some(tomorrow_morning(&now)),
        "the picker's first row"
    );

    app.press(Key::Enter, COMMAND);
    // `Scheduling…`, until Gmail has it (here, within the frames the key took).
    assert!(
        matches!(
            app.mail().toast_text(),
            Some("Scheduling…" | "Message scheduled.")
        ),
        "{:?}",
        app.mail().toast_text()
    );
    app.wait(1.5);
    assert_eq!(
        app.mail().toast_text(),
        Some("Message scheduled."),
        "once Gmail has it"
    );
    assert!(
        subjects(&app, false).contains(&REPLY_SUBJECT.to_owned()),
        "a Gmail draft meanwhile"
    );
    assert!(
        !subjects(&app, true).contains(&REPLY_SUBJECT.to_owned()),
        "not sent"
    );
    let scheduled = app.mail().thread().and_then(|thread| {
        thread
            .messages()
            .iter()
            .find_map(|message| message.scheduled)
    });
    assert_eq!(
        scheduled,
        Some(tomorrow_morning(&now)),
        "its thread shows it to go then"
    );

    // Scheduled lists it (`go to Scheduled`).
    app.press(Key::K, COMMAND);
    app.type_text("go to scheduled");
    app.key(Key::Enter);
    app.settle();
    let rows = app.mail().inbox().threads().to_vec();
    assert_eq!(
        rows.iter()
            .map(|row| row.subject.as_str())
            .collect::<Vec<_>>(),
        ["Design crit moved to 3pm"],
        "{rows:?}"
    );
    assert_eq!(rows[0].scheduled, Some((tomorrow_morning(&now), true)));
}

#[test]
fn undo_takes_a_scheduled_send_back_with_its_time() {
    let mut app = start("send-later-undo");
    let now = now();
    app.set_now(now.clone());
    reply_for_tomorrow(&mut app);
    app.press(Key::Enter, COMMAND);
    app.wait(1.0);
    app.key(Key::Z);
    app.wait(0.5);
    assert_eq!(app.mail().toast_text(), Some("Scheduling undone."));
    assert_eq!(
        app.mail().compose_send_later(),
        Some(tomorrow_morning(&now)),
        "back in compose, its time still set"
    );

    // The picker's footer clears it (`Unschedule`, under the rows: ↑ from the first).
    app.press(Key::L, COMMAND.plus(Modifiers::SHIFT));
    app.key(Key::ArrowUp);
    app.key(Key::Enter);
    assert_eq!(app.mail().compose_send_later(), None, "unscheduled");
    assert!(app.mail().compose_focus().is_some(), "still being written");
}

#[test]
fn a_time_that_passes_while_writing_is_cleared() {
    let mut app = start("send-later-passed");
    let now = now();
    app.set_now(now.clone());
    reply_for_tomorrow(&mut app);
    assert!(app.mail().compose_send_later().is_some());
    app.set_now(
        tomorrow_morning(&now)
            .checked_add(jiff::SignedDuration::from_mins(1))
            .expect("a time")
            .to_zoned(now.time_zone().clone()),
    );
    app.settle();
    assert_eq!(app.mail().compose_send_later(), None);
    assert_eq!(
        app.mail().toast_text(),
        Some("Selected time has passed. Please pick a new time to send.")
    );
}

#[test]
fn a_reply_calls_off_an_if_no_reply_send() {
    let mut app = start("send-later-reply-aborts");
    let now = now();
    app.set_now(now.clone());
    // `if no reply` is a reply's default.
    reply_for_tomorrow(&mut app);
    app.press(Key::Enter, COMMAND);
    app.wait(1.5);
    let thread = app
        .mail()
        .thread()
        .map(|thread| thread.id().to_owned())
        .expect("the thread is open");

    // Tom answers before it goes.
    let url = app.mock_url().join("/__mock/deliver").expect("a URL");
    let body = serde_json::json!({ "from": "Tom Becker <tom@brightmoor.co>", "threadId": thread, "text": "Actually, let's push it to 4." });
    app.block_on(reqwest::Client::new().post(url).json(&body).send())
        .expect("delivered");
    app.sync();
    app.wait(0.5);
    let scheduled = app.mail().thread().and_then(|thread| {
        thread
            .messages()
            .iter()
            .find_map(|message| message.scheduled)
    });
    let count = app.mail().thread().map(|thread| thread.messages().len());
    assert_eq!(
        (scheduled, count),
        (None, Some(2)),
        "no longer scheduled, Tom's reply in"
    );
    assert_eq!(app.mail().thread_drafts(), 1, "a draft in its thread again");
    assert_eq!(
        app.mail().compose_unscheduled_by(),
        ["Tom"],
        "`Unscheduled: Tom replied`"
    );
    assert!(
        !subjects(&app, true).contains(&REPLY_SUBJECT.to_owned()),
        "not sent"
    );
}

#[test]
fn a_send_waiting_for_the_network_is_listed_in_scheduled() {
    let mut app = start("send-later-offline");
    app.disconnect();
    app.mail().account().network_changed();
    app.settle();
    app.key(Key::Enter);
    app.key(Key::R);
    app.type_text("On my way");
    app.press(Key::Enter, COMMAND);
    app.key(Key::Escape);
    app.press(Key::K, COMMAND);
    app.type_text("go to scheduled");
    app.key(Key::Enter);
    app.settle();
    let rows = app.mail().inbox().threads().to_vec();
    assert_eq!(
        rows.iter()
            .map(|row| row.subject.as_str())
            .collect::<Vec<_>>(),
        ["Design crit moved to 3pm"],
        "`Send: when online` there too"
    );
    assert!(rows[0].sending && rows[0].scheduled.is_none());
    assert_eq!(rows[0].snippet, "On my way", "the waiting reply's own text");
}
