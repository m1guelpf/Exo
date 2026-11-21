//! Calendar notifications: an event's reminder comes
//! `notifications/calendarEvents/leadTimeInSeconds` before it starts (two minutes unless set) when
//! it keeps its calendar's default reminders, else at each of its own pop-up reminders. Every 30 s
//! the due ones show, never more than five minutes into the event; events are fetched again every
//! 30 minutes. Events without a title, declined, cancelled, all day, or on a calendar whose
//! notifications are off never notify.
//!
//! A notification has the event's title, its call's provider (or else its location), and when it
//! is (`in 2 mins · 10:00 – 10:30AM`); with a call, a `Join meeting` button. A click opens the
//! week on its day with its details; the button opens the call too.

use std::{
    collections::{BTreeSet, HashMap},
    time::Duration,
};

use exo_store::{Settings, Store, StoredEvent};
use gmail::calendar::Event as ApiEvent;
use jiff::{SignedDuration, Timestamp, ToSpan, tz::TimeZone};
use tokio::sync::watch;

use super::{Click, Notice, Notifier, Open};
use crate::{
    calendar::{Rsvp, event_key, my_attendance},
    format::dates::{calendar_range_compact, relative_compact},
};

/// How often due notifications are looked for.
const POLL: Duration = Duration::from_secs(30);
/// How often the events are fetched again.
const REFETCH: SignedDuration = SignedDuration::from_mins(30);
/// How late a notification may still show: five minutes past the start.
const LATE: SignedDuration = SignedDuration::from_mins(5);
/// The nudge a notification within the minute gets, so the 30 s poll doesn't make it read a
/// minute short.
const NUDGE: SignedDuration = SignedDuration::from_secs(30);

/// One time an event notifies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reminder<'a> {
    pub event: &'a StoredEvent,
    /// When it shows.
    pub at: Timestamp,
    pub start: Timestamp,
    pub end: Timestamp,
}

/// The calendars whose events notify on this device: those of the signed-in calendar accounts
/// that sync with notifications on.
pub fn calendars(settings: &Settings) -> Vec<String> {
    settings.calendars_where(|calendar| calendar.sync && calendar.notifies())
}

/// When `stored` notifies, with `lead` the setting's lead time and `me` the account: none for an
/// event without an id, a time or a title, one I declined, a cancelled one, or one all day.
pub fn reminders<'a>(stored: &'a StoredEvent, lead: SignedDuration, me: &str) -> Vec<Reminder<'a>> {
    let event = &stored.event;
    let (Some(start), Some(end)) = (event.start.as_ref(), event.end.as_ref()) else {
        return Vec::new();
    };
    let timed = |time: &gmail::calendar::EventDateTime| {
        time.date
            .is_none()
            .then(|| time.date_time.as_deref()?.parse::<Timestamp>().ok())
            .flatten()
    };
    let (Some(start), Some(end)) = (timed(start), timed(end)) else {
        return Vec::new();
    };
    let titled = event
        .summary
        .as_deref()
        .is_some_and(|summary| !summary.is_empty());
    if event.id.is_none()
        || !titled
        || event.status.as_deref() == Some("cancelled")
        || my_attendance(event, me) == Some(Rsvp::Declined)
    {
        return Vec::new();
    }
    let reminder = |at| Reminder {
        event: stored,
        at,
        start,
        end,
    };
    match &event.reminders {
        Some(reminders) if reminders.use_default != Some(true) => reminders
            .overrides
            .iter()
            .filter(|reminder| reminder.method.as_deref() == Some("popup"))
            .filter_map(|popup| {
                Some(reminder(
                    start - SignedDuration::from_mins(i64::from(popup.minutes?)),
                ))
            })
            .collect(),
        _ => vec![reminder(start - lead)],
    }
}

/// Whether `reminder` shows at `now`: it's due, and the event started at most five minutes ago.
pub fn due(reminder: &Reminder<'_>, now: Timestamp) -> bool {
    reminder.at < now && now <= reminder.start + LATE
}

/// The notification for `reminder` at `now` in `account`, its times in `tz`.
pub fn notice(reminder: &Reminder<'_>, account: &str, now: Timestamp, tz: &TimeZone) -> Notice {
    let event = &reminder.event.event;
    // A start minutes or seconds away reads 30 s later, unless the notification is late.
    let relative = relative_compact(reminder.start, now);
    let soon = relative.contains("min") || relative == "now";
    let start = if now <= reminder.start && now - NUDGE <= reminder.at && soon {
        reminder.start + NUDGE
    } else {
        reminder.start
    };
    let (provider, link) = conference(event, account);
    let key = event_key(reminder.event, event, &reminder.start.to_zoned(tz.clone()));
    let (day, end_hour) = (
        reminder.start.to_zoned(tz.clone()).date(),
        reminder.end.to_zoned(tz.clone()).hour(),
    );
    let open = |url| Click {
        account: account.to_owned(),
        open: Open::Event {
            key: key.clone(),
            day,
            end_hour,
            url,
        },
    };
    Notice {
        key: format!("CALENDAR-{}", event.id.as_deref().unwrap_or_default()),
        title: event.summary.clone().unwrap_or_default(),
        subtitle: provider.map(str::to_owned).or_else(|| {
            event
                .location
                .clone()
                .filter(|location| !location.is_empty())
        }),
        body: format!(
            "{} · {}",
            relative_compact(start, now),
            calendar_range_compact(start, reminder.end, tz)
        ),
        button: provider.map(|_| open(link)),
        click: open(None),
    }
}

/// The call's provider as the notification names it, and its link: Calendar's preferred entry
/// point (video, else phone, else SIP, else the first), a Meet link as this account.
fn conference(event: &ApiEvent, me: &str) -> (Option<&'static str>, Option<String>) {
    let Some(data) = event.conference_data.as_ref() else {
        return (None, None);
    };
    let entries = data
        .get("entryPoints")
        .and_then(|entries| entries.as_array())
        .map(Vec::as_slice)
        .unwrap_or_default();
    let kind = |entry: &&serde_json::Value| {
        entry
            .get("entryPointType")
            .and_then(|kind| kind.as_str())
            .map(str::to_owned)
    };
    let Some(entry) = ["video", "phone", "sip"]
        .iter()
        .find_map(|wanted| {
            entries
                .iter()
                .find(|entry| kind(entry).as_deref() == Some(wanted))
        })
        .or_else(|| entries.first())
    else {
        return (None, None);
    };
    let link = entry
        .get("uri")
        .and_then(|uri| uri.as_str())
        .map(str::to_owned);
    let solution = data.get("conferenceSolution");
    let name = solution
        .and_then(|solution| solution.get("name"))
        .and_then(|name| name.as_str())
        .unwrap_or_default();
    let meet = solution
        .and_then(|solution| solution.get("key"))
        .and_then(|key| key.get("type"))
        .and_then(|kind| kind.as_str())
        == Some("hangoutsMeet")
        || matches!(name, "Google Meet" | "Hangouts Meet");
    match kind(&entry).as_deref() {
        Some("video") if name == "Microsoft Teams" => (Some("Microsoft Teams"), link),
        Some("video") if name == "Zoom Meeting" => (Some("Zoom"), link),
        Some("video") if meet => {
            let link = link
                .and_then(|link| url::Url::parse(&link).ok())
                .map(|mut link| {
                    link.query_pairs_mut().append_pair("authuser", me);
                    link.to_string()
                });
            (Some("Google Meet"), link)
        }
        Some("video") => (Some("Video"), link),
        Some("phone" | "sip") => (Some("Phone"), link),
        _ => (None, link),
    }
}

/// Posts `account`'s calendar notifications from `store` while `notifier` has somewhere to post
/// them, until the store goes.
pub(crate) async fn run(
    store: Store,
    account: String,
    notifier: watch::Receiver<Option<Notifier>>,
) {
    // Each reminder shows once (by the event's key and its time), remembered until its event is
    // too far gone to show it.
    let mut shown: HashMap<(String, Timestamp), Timestamp> = HashMap::new();
    let mut fetched: Option<(Timestamp, Vec<String>)> = None;
    let mut poll = tokio::time::interval(POLL);
    loop {
        poll.tick().await;
        let Some(notifier) = notifier.borrow().clone() else {
            continue;
        };
        let settings: Settings = match store.settings().await {
            Ok(settings) => settings,
            Err(error) => return log::warn!("calendar notifications stop: {error}"),
        };
        let calendars = calendars(&settings);
        let (now, tz) = (Timestamp::now(), TimeZone::system());
        let today = now.to_zoned(tz.clone()).date();
        // Today and tomorrow, for what's on just after midnight.
        let days = today..today.checked_add(2.days()).unwrap_or(today);
        let stale = fetched
            .as_ref()
            .is_none_or(|(at, ids)| now.duration_since(*at) >= REFETCH || *ids != calendars);
        if stale {
            for calendar in &calendars {
                if let Err(error) = store.sync_events(calendar, days.clone(), &tz).await {
                    log::info!("fetching events for notifications: {error}");
                }
            }
            fetched = Some((now, calendars.clone()));
        }
        let events = match store.events(&calendars, days, &tz).await {
            Ok(events) => events,
            Err(error) => return log::warn!("calendar notifications stop: {error}"),
        };
        let lead = SignedDuration::from_secs(settings.notifications.calendar_events.lead_time());
        shown.retain(|_, start| now <= *start + LATE);
        let mut due_now = Vec::new();
        for stored in &events {
            for reminder in reminders(stored, lead, &account)
                .into_iter()
                .filter(|reminder| due(reminder, now))
            {
                let id = (stored.event.id.clone().unwrap_or_default(), reminder.at);
                if shown.insert(id, reminder.start).is_none() {
                    due_now.push(notice(&reminder, &account, now, &tz));
                }
            }
        }
        // An event due twice in one poll (two reminders) shows once.
        let mut keys = BTreeSet::new();
        due_now.retain(|notice| keys.insert(notice.key.clone()));
        notifier.post(due_now);
    }
}

#[cfg(test)]
mod tests {
    use exo_store::{CalendarAccount, CalendarNotifications, CalendarSetting};
    use gmail::calendar::{EventAttendee, EventDateTime, EventReminder, EventReminders};
    use serde_json::json;

    use super::*;

    const ME: &str = "maya@brightmoor.co";

    fn tz() -> TimeZone {
        TimeZone::get("America/New_York").expect("zone")
    }

    fn at(text: &str) -> Timestamp {
        format!("{text}[America/New_York]")
            .parse::<jiff::Zoned>()
            .expect("time")
            .timestamp()
    }

    fn stored(start: &str, end: &str) -> StoredEvent {
        let time = |text: &str| {
            Some(EventDateTime {
                date_time: Some(at(text).to_string()),
                ..EventDateTime::default()
            })
        };
        StoredEvent {
            calendar_id: ME.into(),
            event: ApiEvent {
                id: Some("evt1".into()),
                summary: Some("Design review".into()),
                start: time(start),
                end: time(end),
                ..ApiEvent::default()
            },
            unsynced: false,
        }
    }

    fn lead() -> SignedDuration {
        SignedDuration::from_secs(120)
    }

    #[test]
    fn default_reminders_use_the_lead_time_and_pop_ups_their_own() {
        let event = stored("2026-11-17T10:00", "2026-11-17T10:30");
        let times = |event: &StoredEvent| {
            reminders(event, lead(), ME)
                .into_iter()
                .map(|reminder| reminder.at)
                .collect::<Vec<_>>()
        };
        assert_eq!(times(&event), [at("2026-11-17T09:58")]);
        let mut own = event.clone();
        own.event.reminders = Some(EventReminders {
            use_default: Some(false),
            overrides: vec![
                EventReminder {
                    method: Some("popup".into()),
                    minutes: Some(10),
                },
                EventReminder {
                    method: Some("email".into()),
                    minutes: Some(30),
                },
                EventReminder {
                    method: Some("popup".into()),
                    minutes: Some(1),
                },
            ],
        });
        assert_eq!(
            times(&own),
            [at("2026-11-17T09:50"), at("2026-11-17T09:59")],
            "pop-ups only"
        );
        own.event.reminders = Some(EventReminders {
            use_default: Some(false),
            overrides: Vec::new(),
        });
        assert!(times(&own).is_empty(), "no reminders, no notification");
        own.event.reminders = Some(EventReminders {
            use_default: Some(true),
            overrides: Vec::new(),
        });
        assert_eq!(times(&own), [at("2026-11-17T09:58")]);
    }

    #[test]
    fn some_events_never_notify() {
        let event = stored("2026-11-17T10:00", "2026-11-17T10:30");
        let quiet = |change: &dyn Fn(&mut ApiEvent)| {
            let mut event = event.clone();
            change(&mut event.event);
            reminders(&event, lead(), ME).is_empty()
        };
        assert!(!quiet(&|_| {}));
        assert!(
            quiet(&|event| event.summary = Some(String::new())),
            "untitled"
        );
        assert!(quiet(&|event| event.status = Some("cancelled".into())));
        assert!(
            quiet(&|event| event.attendees = vec![EventAttendee {
                email: Some(ME.into()),
                response_status: Some("declined".into()),
                ..EventAttendee::default()
            }]),
            "declined"
        );
        assert!(
            quiet(&|event| {
                event.start = Some(EventDateTime {
                    date: Some("2026-11-17".into()),
                    ..EventDateTime::default()
                });
                event.end = Some(EventDateTime {
                    date: Some("2026-11-18".into()),
                    ..EventDateTime::default()
                });
            }),
            "all day"
        );
    }

    #[test]
    fn due_until_five_minutes_into_the_event() {
        let event = stored("2026-11-17T10:00", "2026-11-17T10:30");
        let [reminder] = reminders(&event, lead(), ME).try_into().expect("one");
        assert!(
            !due(&reminder, at("2026-11-17T09:58")),
            "strictly after its time"
        );
        assert!(due(&reminder, at("2026-11-17T09:58") + 1.second()));
        assert!(due(&reminder, at("2026-11-17T10:05")), "five minutes in");
        assert!(!due(&reminder, at("2026-11-17T10:05") + 1.second()));
    }

    #[test]
    fn what_a_notification_says() {
        let event = stored("2026-11-17T10:00", "2026-11-17T10:30");
        let [reminder] = reminders(&event, lead(), ME).try_into().expect("one");
        // The poll sees it up to 30 s late; it still reads two minutes.
        for late in [1, 29] {
            let notice = notice(
                &reminder,
                ME,
                reminder.at + SignedDuration::from_secs(late),
                &tz(),
            );
            assert_eq!(notice.body, "in 2 mins · 10:00 – 10:30AM", "{late}s late");
        }
        let notice = super::notice(&reminder, ME, at("2026-11-17T10:03"), &tz());
        assert_eq!(notice.body, "3 mins ago · 10:00 – 10:30AM");
        assert_eq!(
            (
                notice.key.as_str(),
                notice.title.as_str(),
                notice.subtitle.as_deref(),
                notice.button.is_some()
            ),
            ("CALENDAR-evt1", "Design review", None, false)
        );
        assert_eq!(
            notice.click.open,
            Open::Event {
                key: format!("{ME}_evt1"),
                day: jiff::civil::date(2026, 11, 17),
                end_hour: 10,
                url: None
            }
        );

        let mut located = event.clone();
        located.event.location = Some("Room 4".into());
        let [reminder] = reminders(&located, lead(), ME).try_into().expect("one");
        assert_eq!(
            super::notice(&reminder, ME, reminder.at + 1.second(), &tz())
                .subtitle
                .as_deref(),
            Some("Room 4")
        );
    }

    #[test]
    fn a_call_names_its_provider_and_joins() {
        let mut event = stored("2026-11-17T10:00", "2026-11-17T10:30");
        event.event.location = Some("Room 4".into());
        event.event.conference_data = Some(json!({
            "entryPoints": [{ "entryPointType": "phone", "uri": "tel:+1-555-0100" }, { "entryPointType": "video", "uri": "https://meet.google.com/abc-defg-hij" }],
            "conferenceSolution": { "key": { "type": "hangoutsMeet" }, "name": "Google Meet" },
        }));
        let [reminder] = reminders(&event, lead(), ME).try_into().expect("one");
        let notice = notice(&reminder, ME, reminder.at + 1.second(), &tz());
        assert_eq!(
            notice.subtitle.as_deref(),
            Some("Google Meet"),
            "over the location"
        );
        let Some(Click {
            open: Open::Event { url, .. },
            ..
        }) = notice.button
        else {
            panic!("Join meeting")
        };
        assert_eq!(
            url.as_deref(),
            Some("https://meet.google.com/abc-defg-hij?authuser=maya%40brightmoor.co")
        );

        let provider = |solution: serde_json::Value, kind: &str| {
            let event = ApiEvent {
                conference_data: Some(
                    json!({ "entryPoints": [{ "entryPointType": kind, "uri": "https://example.com/call" }], "conferenceSolution": solution }),
                ),
                ..ApiEvent::default()
            };
            conference(&event, ME).0
        };
        assert_eq!(
            provider(json!({ "name": "Zoom Meeting" }), "video"),
            Some("Zoom")
        );
        assert_eq!(
            provider(json!({ "name": "Microsoft Teams" }), "video"),
            Some("Microsoft Teams")
        );
        assert_eq!(
            provider(json!({ "name": "Whereby" }), "video"),
            Some("Video")
        );
        assert_eq!(provider(json!({}), "sip"), Some("Phone"));
        assert_eq!(provider(json!({}), "more"), None);
    }

    #[test]
    fn only_calendars_that_sync_with_notifications_on() {
        let setting = |sync: bool, desktop: bool| CalendarSetting {
            sync,
            show: true,
            notifications: Some(CalendarNotifications {
                enabled: true,
                enabled_desktop: desktop,
            }),
            ..CalendarSetting::default()
        };
        let mut settings = Settings::default();
        let calendars_of = [
            ("a", setting(true, true)),
            ("b", setting(true, false)),
            ("c", setting(false, true)),
            (
                "d",
                CalendarSetting {
                    sync: true,
                    ..CalendarSetting::default()
                },
            ),
        ];
        settings.calendar_accounts.insert(
            ME.into(),
            CalendarAccount {
                email: ME.into(),
                calendars: calendars_of
                    .into_iter()
                    .map(|(id, setting)| (id.to_owned(), setting))
                    .collect(),
                ..CalendarAccount::default()
            },
        );
        assert_eq!(calendars(&settings), ["a"]);
        // A calendar account's notify too, unless it's signed out.
        let other = |authed: bool| CalendarAccount {
            email: "maya.lin@example.com".into(),
            authed,
            calendars: [("e".to_owned(), setting(true, true))].into(),
            ..CalendarAccount::default()
        };
        settings
            .calendar_accounts
            .insert("maya.lin@example.com".into(), other(true));
        assert_eq!(calendars(&settings), ["a", "e"]);
        settings
            .calendar_accounts
            .insert("maya.lin@example.com".into(), other(false));
        assert_eq!(calendars(&settings), ["a"]);
    }
}
