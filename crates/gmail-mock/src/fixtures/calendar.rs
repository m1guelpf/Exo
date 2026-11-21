//! Calendars and events, plus the invitation emails for meetings others organized, generated
//! from the same data so the inbox and the calendar agree.

use gmail::calendar::{
    CalendarListEntry, Event, EventAttendee, EventDateTime, EventPerson, EventReminder,
    EventReminders,
};
use gmail::mime::{Attachment, Mailbox};
use jiff::{ToSpan, Zoned};

use super::assets::Invite;
use super::cast;
use super::compose::{Post, State, Thread, fnv};
use crate::calendar::{Calendars, rfc3339_utc};

/// A meeting relative to "now": days from today (negative is past), local start, length.
struct Meeting {
    summary: &'static str,
    organizer: &'static str,
    attendees: &'static [&'static str],
    day: i64,
    start: (i8, i8),
    minutes: i64,
    location: &'static str,
    description: &'static str,
    recurrence: Option<&'static str>,
    /// Days before the meeting's creation that the invite arrived (organized by someone else).
    invited_days_ago: Option<i64>,
    /// The user's RSVP.
    response: &'static str,
    event_type: &'static str,
}

const MEETING: Meeting = Meeting {
    summary: "",
    organizer: "maya",
    attendees: &[],
    day: 0,
    start: (10, 0),
    minutes: 30,
    location: "",
    description: "",
    recurrence: None,
    invited_days_ago: None,
    response: "accepted",
    event_type: "default",
};

static MEETINGS: &[Meeting] = &[
    Meeting {
        summary: "Weekly product sync",
        attendees: &["priya", "tom", "lena"],
        day: -71,
        start: (10, 0),
        minutes: 45,
        location: "Hudson Room",
        recurrence: Some("RRULE:FREQ=WEEKLY;BYDAY=MO"),
        description: "Standing agenda: metrics, launches, open decisions.",
        ..MEETING
    },
    Meeting {
        summary: "Priya / Maya 1:1",
        organizer: "priya",
        attendees: &["maya"],
        day: -70,
        start: (11, 0),
        recurrence: Some("RRULE:FREQ=WEEKLY;BYDAY=TU"),
        ..MEETING
    },
    Meeting {
        summary: "Design crit",
        organizer: "tom",
        attendees: &["maya", "priya", "lena"],
        day: -63,
        start: (15, 0),
        minutes: 60,
        location: "Design corner",
        recurrence: Some("RRULE:FREQ=WEEKLY;BYDAY=TU"),
        ..MEETING
    },
    Meeting {
        summary: "Focus time",
        attendees: &[],
        day: -60,
        start: (9, 0),
        minutes: 120,
        recurrence: Some("RRULE:FREQ=WEEKLY;BYDAY=WE,FR"),
        event_type: "focusTime",
        ..MEETING
    },
    Meeting {
        summary: "Calloway QBR",
        organizer: "jordan",
        attendees: &["maya", "rachel", "dev", "daniel"],
        day: -9,
        start: (13, 0),
        minutes: 90,
        location: "Calloway HQ, Chicago / Meet",
        ..MEETING
    },
    Meeting {
        summary: "Incident review: Nov 12 routing delays",
        organizer: "hiro",
        attendees: &["maya", "priya", "oliver", "chloe"],
        day: -3,
        start: (11, 0),
        minutes: 45,
        ..MEETING
    },
    Meeting {
        summary: "Aiko: design exercise",
        organizer: "zara",
        attendees: &["maya", "tom"],
        day: -1,
        start: (14, 0),
        minutes: 60,
        location: "Hudson Room",
        ..MEETING
    },
    Meeting {
        summary: "Calloway: Atlas feedback call",
        organizer: "jordan",
        attendees: &["maya", "priya", "rachel", "dev"],
        day: 2,
        start: (14, 0),
        minutes: 60,
        location: "Big conference room / Meet",
        description: "Walk through the dock appointment issue and cross-dock transfer questions.",
        invited_days_ago: Some(1),
        response: "needsAction",
        ..MEETING
    },
    Meeting {
        summary: "Dinner with Jess 🍕",
        attendees: &[],
        day: 2,
        start: (19, 30),
        minutes: 120,
        location: "Atlantic Ave",
        ..MEETING
    },
    Meeting {
        summary: "Board prep",
        organizer: "daniel",
        attendees: &["maya", "marcus"],
        day: 6,
        start: (13, 0),
        minutes: 120,
        location: "Daniel's office",
        description: "Final pass on the November board deck.",
        invited_days_ago: Some(4),
        ..MEETING
    },
    Meeting {
        summary: "Coffee with Grace",
        organizer: "grace",
        attendees: &["maya"],
        day: 8,
        start: (9, 0),
        minutes: 45,
        location: "Harborlight Coffee, 5th Ave",
        invited_days_ago: Some(0),
        response: "needsAction",
        ..MEETING
    },
    Meeting {
        summary: "Out of office",
        attendees: &[],
        day: 9,
        start: (0, 0),
        minutes: 2 * 24 * 60,
        event_type: "outOfOffice",
        ..MEETING
    },
    Meeting {
        summary: "Atlas go/no-go",
        attendees: &["priya", "lena", "tom", "hiro", "daniel", "chloe", "oliver"],
        day: 14,
        start: (16, 0),
        minutes: 45,
        location: "Hudson Room / Meet",
        ..MEETING
    },
    Meeting {
        summary: "Team offsite: Hudson",
        organizer: "sam",
        attendees: &[
            "maya", "daniel", "priya", "tom", "lena", "jordan", "hiro", "chloe", "marcus", "ines",
            "oliver", "zara", "ruth",
        ],
        day: 22,
        start: (9, 0),
        minutes: 56 * 60,
        location: "Millbrook Inn, 28 Warren St, Hudson NY",
        invited_days_ago: Some(5),
        ..MEETING
    },
];

/// The account zone's IANA name.
fn zone_name(now: &Zoned) -> String {
    now.time_zone()
        .iana_name()
        .unwrap_or("America/New_York")
        .to_owned()
}

/// A calendar on the list: its colour as Google's `(colorId, background, foreground)`.
fn entry(
    id: &str,
    summary: &str,
    color: (&str, &str, &str),
    role: &str,
    tz_name: &str,
) -> CalendarListEntry {
    CalendarListEntry {
        kind: Some("calendar#calendarListEntry".into()),
        etag: Some(format!(
            "\"{}\"",
            1_700_000_000_000u64 + fnv(id) % 100_000_000
        )),
        id: Some(id.into()),
        summary: Some(summary.into()),
        time_zone: Some(tz_name.to_owned()),
        color_id: Some(color.0.into()),
        background_color: Some(color.1.into()),
        foreground_color: Some(color.2.into()),
        selected: Some(true),
        access_role: Some(role.into()),
        conference_properties: Some(
            serde_json::json!({"allowedConferenceSolutionTypes": ["hangoutsMeet"]}),
        ),
        ..CalendarListEntry::default()
    }
}

/// `email`'s primary calendar on its own list, in Google's colour `color`.
fn primary(email: &str, color: (&str, &str, &str), tz_name: &str) -> CalendarListEntry {
    CalendarListEntry {
        primary: Some(true),
        default_reminders: vec![EventReminder {
            method: Some("popup".into()),
            minutes: Some(10),
        }],
        notification_settings: Some(serde_json::json!({"notifications": [
            {"type": "eventCreation", "method": "email"},
            {"type": "eventChange", "method": "email"},
            {"type": "eventCancellation", "method": "email"},
            {"type": "eventResponse", "method": "email"}
        ]})),
        ..entry(email, email, color, "owner", tz_name)
    }
}

const HOLIDAYS: &str = "en.usa#holiday@group.v.calendar.google.com";

/// Google's US holidays calendar, which anyone can add to their list.
fn holidays_entry(tz_name: &str) -> CalendarListEntry {
    CalendarListEntry {
        conference_properties: None,
        ..entry(
            HOLIDAYS,
            "Holidays in United States",
            ("8", "#16a765", "#000000"),
            "reader",
            tz_name,
        )
    }
}

/// The holidays calendar's events around "now".
fn holiday_events(now: &Zoned) -> Vec<Event> {
    [
        ("Veterans Day", 11, 11),
        ("Thanksgiving Day", 11, 26),
        ("Christmas Day", 12, 25),
        ("Indigenous Peoples' Day", 10, 12),
    ]
    .into_iter()
    .map(|(summary, month, day)| {
        let date = jiff::civil::date(now.year(), month, day);
        let offset = (date - now.date()).get_days();
        let mut event = all_day(
            &format!("holiday-{summary}"),
            summary,
            now,
            i64::from(offset),
            1,
        );
        event.transparency = Some("transparent".into());
        event.visibility = Some("public".into());
        event
    })
    .collect()
}

/// The calendars of `email`, another Google account signed in on the mock (a `login_hint` that
/// isn't the mailbox's owner): its primary calendar with a few events of its own this week, and
/// the holidays calendar the mailbox's owner has too.
pub fn personal(email: &str, now: &Zoned) -> Calendars {
    let tz_name = zone_name(now);
    let entries = vec![
        primary(email, ("16", "#4986e7", "#000000"), &tz_name),
        holidays_entry(&tz_name),
    ];
    let mut calendars = Calendars::new(email, now.time_zone().clone(), entries);
    let mine = EventPerson {
        email: Some(email.into()),
        is_self: Some(true),
        ..EventPerson::default()
    };
    for (summary, day, start, minutes) in [
        ("Dentist", 1, (8, 0), 60),
        ("Yoga", 2, (18, 0), 60),
        ("Pick up dry cleaning", 3, (12, 30), 30),
    ] {
        let mut event = timed(
            &format!("{email}-{summary}"),
            summary,
            now,
            day,
            start,
            minutes,
            &tz_name,
        );
        event.organizer = Some(mine.clone());
        event.creator = Some(mine.clone());
        calendars.put(email, event);
    }
    for event in holiday_events(now) {
        calendars.put(HOLIDAYS, event);
    }
    calendars
}

pub fn calendars(now: &Zoned) -> Calendars {
    let me = cast::me();
    let tz_name = zone_name(now);
    let entry = |id: &str, summary: &str, color: (&str, &str, &str), role: &str| {
        entry(id, summary, color, role, &tz_name)
    };
    let team = "c_8f3b2a1d9e7c4b6a5f01@group.calendar.google.com";
    let holidays = HOLIDAYS;
    let entries = vec![
        primary(me.email, ("14", "#9fe1e7", "#000000"), &tz_name),
        entry(
            team,
            "Brightmoor Team",
            ("7", "#42d692", "#000000"),
            "writer",
        ),
        holidays_entry(&tz_name),
        CalendarListEntry {
            hidden: Some(true),
            selected: Some(false),
            conference_properties: None,
            ..entry(
                "addressbook#contacts@group.v.calendar.google.com",
                "Birthdays",
                ("13", "#92e1c0", "#000000"),
                "reader",
            )
        },
    ];
    let mut calendars = Calendars::new(me.email, now.time_zone().clone(), entries);

    for meeting in MEETINGS {
        let event = event_for(meeting, now, &tz_name);
        calendars.put(me.email, event);
    }
    for (summary, day, days) in [
        ("Atlas launch 🚀", 15, 1),
        ("Offsite: Hudson", 22, 3),
        ("Company all-hands", 3, 0),
    ] {
        let event = if days == 0 {
            timed(
                &format!("team-{summary}"),
                summary,
                now,
                day,
                (16, 0),
                60,
                &tz_name,
            )
        } else {
            all_day(&format!("team-{summary}"), summary, now, day, days)
        };
        calendars.put(team, event);
    }
    for event in holiday_events(now) {
        calendars.put(holidays, event);
    }
    calendars
}

fn local(now: &Zoned, day: i64, (hour, minute): (i8, i8)) -> Zoned {
    (now.date() + day.days())
        .at(hour, minute, 0, 0)
        .to_zoned(now.time_zone().clone())
        .expect("fixture times exist in the account zone")
}

fn base(key: &str, summary: &str, now: &Zoned) -> Event {
    let id = format!("{:016x}{:010x}", fnv(key), fnv(summary) & 0xff_ffff_ffff);
    // Every fixture event was created a month before "now".
    let created = rfc3339_utc(now.timestamp() - 720.hours());
    Event {
        kind: Some("calendar#event".into()),
        etag: Some(format!("\"{}\"", fnv(&id) % 10_000_000_000_000_000)),
        html_link: Some(format!("https://www.google.com/calendar/event?eid={id}")),
        ical_uid: Some(format!("{id}@google.com")),
        status: Some("confirmed".into()),
        summary: Some(summary.into()),
        created: Some(created.clone()),
        updated: Some(created),
        sequence: Some(0),
        event_type: Some("default".into()),
        reminders: Some(EventReminders {
            use_default: Some(true),
            overrides: Vec::new(),
        }),
        id: Some(id),
        ..Event::default()
    }
}

fn timed(
    key: &str,
    summary: &str,
    now: &Zoned,
    day: i64,
    start: (i8, i8),
    minutes: i64,
    tz: &str,
) -> Event {
    let begin = local(now, day, start);
    let end = begin.checked_add(minutes.minutes()).expect("in range");
    let at = |time: &Zoned| EventDateTime {
        date: None,
        date_time: Some(time.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string()),
        time_zone: Some(tz.to_owned()),
    };
    Event {
        start: Some(at(&begin)),
        end: Some(at(&end)),
        ..base(key, summary, now)
    }
}

fn all_day(key: &str, summary: &str, now: &Zoned, day: i64, days: i64) -> Event {
    let start = now.date() + day.days();
    let date = |date: jiff::civil::Date| EventDateTime {
        date: Some(date.to_string()),
        ..EventDateTime::default()
    };
    Event {
        start: Some(date(start)),
        end: Some(date(start + days.days())),
        transparency: Some("transparent".into()),
        ..base(key, summary, now)
    }
}

fn event_for(meeting: &Meeting, now: &Zoned, tz: &str) -> Event {
    let organizer = cast::get(meeting.organizer);
    let person = |who: &cast::Person| EventPerson {
        email: Some(who.email.into()),
        display_name: (!who.is_me()).then(|| who.name()),
        is_self: who.is_me().then_some(true),
        ..EventPerson::default()
    };
    let mut event = if meeting.minutes >= 24 * 60 && meeting.start == (0, 0) {
        all_day(
            meeting.summary,
            meeting.summary,
            now,
            meeting.day,
            meeting.minutes / (24 * 60),
        )
    } else {
        timed(
            meeting.summary,
            meeting.summary,
            now,
            meeting.day,
            meeting.start,
            meeting.minutes,
            tz,
        )
    };
    event.organizer = Some(person(organizer));
    event.creator = Some(person(organizer));
    event.location = (!meeting.location.is_empty()).then(|| meeting.location.into());
    event.description = (!meeting.description.is_empty()).then(|| meeting.description.into());
    event.recurrence = meeting
        .recurrence
        .map(|rule| vec![rule.to_owned()])
        .unwrap_or_default();
    event.event_type = Some(meeting.event_type.into());
    if meeting.event_type == "outOfOffice" {
        event.transparency = None;
        event.out_of_office_properties = Some(
            serde_json::json!({"autoDeclineMode": "declineAllConflictingInvitations", "declineMessage": "Out for Thanksgiving, back Monday."}),
        );
    }
    if meeting.event_type == "focusTime" {
        event.focus_time_properties = Some(
            serde_json::json!({"autoDeclineMode": "declineOnlyNewConflictingInvitations", "chatStatus": "doNotDisturb"}),
        );
    }
    if !meeting.attendees.is_empty() {
        let mut everyone: Vec<&cast::Person> = vec![organizer];
        everyone.extend(
            meeting
                .attendees
                .iter()
                .map(|key| cast::get(key))
                .filter(|p| p.key != organizer.key),
        );
        event.attendees = everyone
            .into_iter()
            .map(|who| EventAttendee {
                email: Some(who.email.into()),
                display_name: (!who.is_me()).then(|| who.name()),
                organizer: (who.key == organizer.key).then_some(true),
                is_self: who.is_me().then_some(true),
                response_status: Some(
                    if who.is_me() {
                        meeting.response
                    } else {
                        "accepted"
                    }
                    .into(),
                ),
                ..EventAttendee::default()
            })
            .collect();
        event.hangout_link = meeting
            .location
            .contains("Meet")
            .then(|| format!("https://meet.google.com/{}", meet_code(meeting.summary)));
    }
    event
}

fn meet_code(seed: &str) -> String {
    let hash = fnv(seed);
    let letters: Vec<char> = (0..10)
        .map(|i| char::from(b'a' + u8::try_from((hash >> (i * 5)) % 26).unwrap_or(0)))
        .collect();
    format!(
        "{}-{}-{}",
        letters[..3].iter().collect::<String>(),
        letters[3..7].iter().collect::<String>(),
        letters[7..].iter().collect::<String>()
    )
}

/// Invitation emails for meetings someone else organized, as Google Calendar sends them.
pub fn invites(now: &Zoned) -> Vec<Thread> {
    let tz = now.time_zone().clone();
    let tz_name = tz.iana_name().unwrap_or("America/New_York").to_owned();
    MEETINGS
        .iter()
        .filter_map(|meeting| {
            let days_ago = meeting.invited_days_ago?;
            let organizer = cast::get(meeting.organizer);
            let event = event_for(meeting, now, &tz_name);
            let start = event.start.as_ref()?.timestamp(&tz)?;
            let end = event.end.as_ref()?.timestamp(&tz)?;
            let sent = local(now, -days_ago, (8, 10)).min(now.checked_sub(5.minutes()).ok()?);
            let when = start.to_zoned(tz.clone());
            let range = format!(
                "{} - {}",
                when.strftime("%a %b %-d, %Y %-I:%M%P"),
                end.to_zoned(tz.clone()).strftime("%-I:%M%P (%Z)")
            );
            let attendees: Vec<(String, &str)> = event
                .attendees
                .iter()
                .filter_map(|a| Some((a.display_name.clone().unwrap_or_else(|| cast::me().name()), a.email.as_deref()?)))
                .collect();
            let attendee_refs: Vec<(&str, &str)> = attendees.iter().map(|(n, e)| (n.as_str(), *e)).collect();
            let ics = Invite {
                uid: event.ical_uid.as_deref()?,
                summary: meeting.summary,
                description: meeting.description,
                location: meeting.location,
                start,
                end,
                organizer: (&organizer.name(), organizer.email),
                attendees: &attendee_refs,
                stamp: sent.timestamp(),
            }
            .ics();
            let guests: String = attendees
                .iter()
                .map(|(name, email)| format!("<li>{name} &lt;{email}&gt;</li>"))
                .collect();
            let html = format!(
                "<div style=\"font-family:Roboto,Helvetica,Arial,sans-serif;color:#3c4043\">\
                 <h2 style=\"font-size:20px;font-weight:400;margin:0 0 12px\">{summary}</h2>\
                 <p style=\"margin:0 0 8px\"><b>When</b><br>{range}</p>\
                 {location}\
                 <p style=\"margin:0 0 8px\"><b>Organizer</b><br>{name} &lt;{email}&gt;</p>\
                 <p style=\"margin:0 0 8px\"><b>Guests</b></p><ul>{guests}</ul>\
                 <p><a href=\"{link}\">More details</a> · Reply for {me}: Yes · Maybe · No</p></div>",
                summary = super::compose::escape(meeting.summary),
                location = if meeting.location.is_empty() { String::new() } else { format!("<p style=\"margin:0 0 8px\"><b>Where</b><br>{}</p>", super::compose::escape(meeting.location)) },
                name = organizer.name(),
                email = organizer.email,
                link = event.html_link.clone().unwrap_or_default(),
                me = cast::me().email,
            );
            let text = format!(
                "{}\n\nWhen\n{range}\n\n{}Organizer\n{} <{}>\n\nGuests\n{}\n\nView all guest info and reply: {}",
                meeting.summary,
                if meeting.location.is_empty() { String::new() } else { format!("Where\n{}\n\n", meeting.location) },
                organizer.name(),
                organizer.email,
                attendees.iter().map(|(n, e)| format!("{n} <{e}>")).collect::<Vec<_>>().join("\n"),
                event.html_link.clone().unwrap_or_default(),
            );
            let to: Vec<Mailbox> = meeting
                .attendees
                .iter()
                .map(|key| cast::get(key).mailbox())
                .collect();
            Some(Thread {
                subject: format!("Invitation: {} @ {} ({})", meeting.summary, range, cast::me().email),
                posts: vec![Post {
                    from: organizer.mailbox(),
                    to,
                    cc: Vec::new(),
                    text,
                    html: Some(html),
                    calendar: Some(ics.clone()),
                    attachments: vec![Attachment {
                        filename: "invite.ics".into(),
                        content_type: "application/ics".into(),
                        data: ics.into_bytes(),
                        content_id: None,
                    }],
                    at: sent.timestamp(),
                }],
                state: State {
                    inbox: days_ago <= 2,
                    unread: usize::from(days_ago == 0),
                    important: true,
                    category: Some("CATEGORY_PERSONAL"),
                    ..State::default()
                },
                list: None,
                quote: false,
            })
        })
        .collect()
}
