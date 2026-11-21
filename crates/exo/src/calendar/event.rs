//! An event as the calendar shows it: its title, times, colour, and my answer, resolved from
//! Calendar's event and the account's calendar settings.

use std::collections::BTreeMap;

use exo_store::{Settings, StoredEvent};
use exo_ui::CalendarColor;
use gmail::calendar::{CalendarListEntry, Event as ApiEvent, EventDateTime};
use jiff::{Timestamp, Zoned, civil::Date, tz::TimeZone};

use crate::thread::invite::{Invite, Response};

/// An attendee's answer (`responseStatus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rsvp {
    NeedsAction,
    Accepted,
    Tentative,
    Declined,
}

impl From<Rsvp> for Response {
    fn from(answer: Rsvp) -> Self {
        match answer {
            Rsvp::NeedsAction => Response::NeedsAction,
            Rsvp::Accepted => Response::Accepted,
            Rsvp::Tentative => Response::Tentative,
            Rsvp::Declined => Response::Declined,
        }
    }
}

impl Rsvp {
    fn parse(status: Option<&str>) -> Option<Self> {
        Some(match status? {
            "accepted" => Self::Accepted,
            "tentative" => Self::Tentative,
            "declined" => Self::Declined,
            "needsAction" => Self::NeedsAction,
            _ => return None,
        })
    }
}

/// How a block is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Accepted, or mine: a coloured fill.
    Filled,
    /// Declined or not answered: an outline.
    Bordered,
    /// Maybe: the fill, in a dashed outline.
    Dashed,
}

/// An event on the calendar.
#[derive(Clone, Debug)]
pub struct Event {
    /// Unique per instance.
    pub key: String,
    /// What the block says: the summary, `Busy` when its details are hidden, else `(No title)`.
    pub title: String,
    pub start: Zoned,
    pub end: Zoned,
    /// In the all-day row: Calendar says it's all-day, or it lasts a day or more.
    pub all_day: bool,
    pub color: CalendarColor,
    /// My answer.
    pub rsvp: Option<Rsvp>,
    /// The answer the block shows.
    pub busy: Rsvp,
    /// Everyone but me declined.
    pub everyone_else_declined: bool,
    /// iCalendar UID, shared with the invite in a thread.
    pub ical_uid: Option<String>,
    /// The calendar it's on (empty for an invite the calendar doesn't have).
    pub calendar_id: String,
    /// Calendar's id for it (none for such an invite).
    pub id: Option<String>,
    /// I may change it: I organize it on a calendar I can write to.
    pub editable: bool,
    /// What its block says for the time instead of its own range: a draft typed in another zone.
    pub time_text: Option<String>,
    /// A slot of availability being shared, and whether it can be shared: as long as the
    /// meeting and not over.
    pub share: Option<bool>,
    /// What its block says while hovered: why a slot can't be shared.
    pub tooltip: Option<&'static str>,
}

impl Event {
    /// `event` from `calendar`, for `me`, coloured by `colors` (each calendar's colour from the
    /// settings), its times in `tz`. `None` for an event without usable times.
    pub fn new(
        stored: &StoredEvent,
        calendar: Option<&CalendarListEntry>,
        colors: &BTreeMap<String, CalendarColor>,
        me: &str,
        tz: &TimeZone,
    ) -> Option<Self> {
        let event = &stored.event;
        let (start, all_day_start) = time(event.start.as_ref()?, tz)?;
        let (end, all_day_end) = time(event.end.as_ref()?, tz)?;
        let all_day = (all_day_start && all_day_end)
            || end.timestamp().duration_since(start.timestamp()).as_hours() >= 24;
        let rsvp = my_attendance(event, me);
        let access = calendar.and_then(|calendar| calendar.access_role.as_deref());
        let private = matches!(
            event.visibility.as_deref(),
            Some("private" | "confidential")
        );
        let hidden = access == Some("freeBusyReader")
            || (private && matches!(access, Some("reader" | "writerWithoutPrivateAccess")));
        let title = match event
            .summary
            .as_deref()
            .filter(|summary| !summary.is_empty())
        {
            Some(summary) => summary.to_owned(),
            None if hidden => "Busy".to_owned(),
            None => "(No title)".to_owned(),
        };
        let color = event
            .color_id
            .as_deref()
            .and_then(google_color)
            .or_else(|| colors.get(&stored.calendar_id).copied())
            .unwrap_or(CalendarColor::BLUE);
        let organizer = event.organizer.as_ref().is_some_and(|organizer| {
            organizer.is_self == Some(true)
                || organizer
                    .email
                    .as_deref()
                    .is_some_and(|email| email.eq_ignore_ascii_case(me))
        });
        let writable = matches!(access, Some("owner" | "writer"))
            || (access == Some("writerWithoutPrivateAccess") && !private);
        // A new event Calendar hasn't got yet is editable too: its edit waits behind it in its
        // queue.
        let editable = organizer && writable && event.event_type.as_deref() != Some("fromGmail");
        let creator = event
            .creator
            .as_ref()
            .and_then(|creator| creator.email.as_deref())
            .is_some_and(|email| email.eq_ignore_ascii_case(me));
        let invitee = event
            .attendees
            .iter()
            .any(|attendee| attendee.email.as_deref() == Some(me));
        let busy = rsvp.unwrap_or(if creator || !invitee {
            Rsvp::Accepted
        } else {
            Rsvp::NeedsAction
        });
        let others: Vec<_> = event
            .attendees
            .iter()
            .filter(|attendee| {
                attendee.resource != Some(true)
                    && attendee.is_self != Some(true)
                    && !attendee
                        .email
                        .as_deref()
                        .is_some_and(|email| email.eq_ignore_ascii_case(me))
            })
            .collect();
        let everyone_else_declined = !others.is_empty()
            && others
                .iter()
                .all(|attendee| attendee.response_status.as_deref() == Some("declined"));
        Some(Self {
            key: unique_key(stored, event, &start),
            title,
            start,
            end,
            all_day,
            color,
            rsvp,
            busy,
            everyone_else_declined,
            ical_uid: event.ical_uid.clone(),
            calendar_id: stored.calendar_id.clone(),
            id: event.id.clone(),
            editable,
            time_text: None,
            share: None,
            tooltip: None,
        })
    }

    /// The invite being read as an event, for a day that doesn't have it on the calendar, in the
    /// primary calendar's colour.
    pub fn from_invite(invite: &Invite, color: CalendarColor, me: &str, tz: &TimeZone) -> Self {
        let rsvp = invite
            .attendees
            .iter()
            .find(|(_, email, _)| email.eq_ignore_ascii_case(me))
            .map(|(_, _, response)| match response {
                Response::Accepted => Rsvp::Accepted,
                Response::Tentative => Rsvp::Tentative,
                Response::Declined => Rsvp::Declined,
                Response::NeedsAction => Rsvp::NeedsAction,
            });
        let (start, end) = (
            invite.start.to_zoned(tz.clone()),
            invite.end.to_zoned(tz.clone()),
        );
        Self {
            key: format!("calendar-invite_{}", invite.uid),
            title: if invite.summary.is_empty() {
                "(No title)".to_owned()
            } else {
                invite.summary.clone()
            },
            all_day: end.timestamp().duration_since(start.timestamp()).as_hours() >= 24,
            start,
            end,
            color,
            rsvp,
            busy: rsvp.unwrap_or(Rsvp::NeedsAction),
            everyone_else_declined: false,
            ical_uid: Some(invite.uid.clone()),
            calendar_id: String::new(),
            id: None,
            editable: false,
            time_text: None,
            share: None,
            tooltip: None,
        }
    }

    /// The event a form makes or changes: mine, accepted, editable.
    pub fn draft(
        key: String,
        title: String,
        start: Zoned,
        end: Zoned,
        all_day: bool,
        color: CalendarColor,
    ) -> Self {
        Self {
            key,
            title,
            start,
            end,
            all_day,
            color,
            rsvp: None,
            busy: Rsvp::Accepted,
            everyone_else_declined: false,
            ical_uid: None,
            calendar_id: String::new(),
            id: None,
            editable: true,
            time_text: None,
            share: None,
            tooltip: None,
        }
    }

    /// The block is drawn: declined events are left off unless the settings show them.
    pub fn shown(&self, show_declined: bool) -> bool {
        show_declined || self.rsvp != Some(Rsvp::Declined)
    }

    /// The block's style.
    pub fn style(&self) -> Style {
        match self.busy {
            Rsvp::Declined | Rsvp::NeedsAction => Style::Bordered,
            Rsvp::Tentative => Style::Dashed,
            Rsvp::Accepted => Style::Filled,
        }
    }

    /// On `day`: starts before its end and ends after its start.
    pub fn on(&self, day: Date) -> bool {
        let tz = self.start.time_zone().clone();
        let (Ok(from), Ok(to)) = (
            day.to_zoned(tz.clone()),
            day.tomorrow().and_then(|next| next.to_zoned(tz)),
        ) else {
            return false;
        };
        self.start < to && self.end > from
    }
}

/// A start or end in `tz`, and whether it's an all-day date (which is local midnight).
fn time(value: &EventDateTime, tz: &TimeZone) -> Option<(Zoned, bool)> {
    if let Some(date) = &value.date {
        let date: Date = date.parse().ok()?;
        return Some((date.to_zoned(tz.clone()).ok()?, true));
    }
    let at: Timestamp = value.date_time.as_deref()?.parse().ok()?;
    Some((at.to_zoned(tz.clone()), false))
}

/// My answer to `event`, from my attendee entry.
pub(crate) fn my_attendance(event: &ApiEvent, me: &str) -> Option<Rsvp> {
    let mine = event
        .attendees
        .iter()
        .find(|attendee| attendee.is_self == Some(true))
        .or_else(|| {
            event.attendees.iter().find(|attendee| {
                attendee
                    .email
                    .as_deref()
                    .is_some_and(|email| email.eq_ignore_ascii_case(me))
            })
        })?;
    Rsvp::parse(mine.response_status.as_deref())
}

/// An event's own colour, by Calendar's colour id.
fn google_color(id: &str) -> Option<CalendarColor> {
    let name = match id {
        "1" => "cobalt",
        "2" => "green",
        "3" => "magenta",
        "4" => "red",
        "5" => "yellow",
        "6" => "orange",
        "7" => "blue",
        "8" => "pewter",
        "9" => "navy",
        "10" => "lime",
        "11" => "pink",
        _ => return None,
    };
    CalendarColor::from_name(name)
}

/// An event's key: the calendar and the event, an instance by its series and original start.
pub(crate) fn unique_key(stored: &StoredEvent, event: &ApiEvent, start: &Zoned) -> String {
    let id = match &event.recurring_event_id {
        Some(series) => {
            let original = event
                .original_start_time
                .as_ref()
                .and_then(|original| original.timestamp(start.time_zone()))
                .unwrap_or(start.timestamp());
            format!("{series}_{}", original.as_millisecond())
        }
        None => event.id.clone().unwrap_or_default(),
    };
    format!("{}_{id}", stored.calendar_id)
}

/// Each calendar's colour, over every signed-in calendar account: the settings' colour, else
/// blue.
pub fn colors(settings: &Settings) -> BTreeMap<String, CalendarColor> {
    let mut colors = BTreeMap::new();
    for account in settings
        .calendar_accounts
        .values()
        .filter(|account| account.authed)
    {
        for (id, setting) in &account.calendars {
            colors.entry(id.clone()).or_insert_with(|| {
                setting
                    .color
                    .as_deref()
                    .and_then(CalendarColor::from_name)
                    .unwrap_or(CalendarColor::BLUE)
            });
        }
    }
    colors
}

#[cfg(test)]
mod tests {
    use super::*;
    use gmail::calendar::{EventAttendee, EventPerson};

    fn tz() -> TimeZone {
        TimeZone::get("America/New_York").expect("zone")
    }

    fn timed(start: &str, end: &str) -> ApiEvent {
        let at = |text: &str| EventDateTime {
            date_time: Some(text.into()),
            ..EventDateTime::default()
        };
        ApiEvent {
            id: Some("e1".into()),
            start: Some(at(start)),
            end: Some(at(end)),
            ..ApiEvent::default()
        }
    }

    fn stored(event: ApiEvent) -> StoredEvent {
        StoredEvent {
            calendar_id: "maya@brightmoor.co".into(),
            event,
            unsynced: false,
        }
    }

    #[test]
    fn answers_and_styles() {
        let mut event = timed("2026-11-19T14:00:00-05:00", "2026-11-19T15:00:00-05:00");
        event.attendees = vec![
            EventAttendee {
                email: Some("jordan@calloway.com".into()),
                response_status: Some("accepted".into()),
                ..EventAttendee::default()
            },
            EventAttendee {
                email: Some("maya@brightmoor.co".into()),
                is_self: Some(true),
                response_status: Some("needsAction".into()),
                ..EventAttendee::default()
            },
        ];
        let invite = Event::new(
            &stored(event.clone()),
            None,
            &BTreeMap::new(),
            "maya@brightmoor.co",
            &tz(),
        )
        .expect("event");
        assert_eq!(
            (invite.rsvp, invite.style(), invite.title.as_str()),
            (Some(Rsvp::NeedsAction), Style::Bordered, "(No title)")
        );
        event.attendees[1].response_status = Some("declined".into());
        let declined = Event::new(
            &stored(event),
            None,
            &BTreeMap::new(),
            "maya@brightmoor.co",
            &tz(),
        )
        .expect("event");
        assert!(!declined.shown(false) && declined.shown(true));

        // Without attendees, my own event is accepted.
        let mut own = timed("2026-11-19T19:30:00-05:00", "2026-11-19T21:30:00-05:00");
        own.creator = Some(EventPerson {
            email: Some("maya@brightmoor.co".into()),
            ..EventPerson::default()
        });
        let own = Event::new(
            &stored(own),
            None,
            &BTreeMap::new(),
            "maya@brightmoor.co",
            &tz(),
        )
        .expect("event");
        assert_eq!(own.busy, Rsvp::Accepted);
    }

    #[test]
    fn a_day_long_timed_event_is_all_day() {
        let long = Event::new(
            &stored(timed(
                "2026-12-09T09:00:00-05:00",
                "2026-12-11T17:00:00-05:00",
            )),
            None,
            &BTreeMap::new(),
            "maya@brightmoor.co",
            &tz(),
        )
        .expect("event");
        assert!(long.all_day);
        assert!(long.on("2026-12-10".parse().expect("date")));
        assert!(!long.on("2026-12-12".parse().expect("date")));
    }

    #[test]
    fn colours_come_from_the_event_then_the_calendar() {
        let mut event = timed("2026-11-19T14:00:00-05:00", "2026-11-19T15:00:00-05:00");
        let colors = BTreeMap::from([(
            "maya@brightmoor.co".to_owned(),
            CalendarColor::from_name("green").expect("colour"),
        )]);
        assert_eq!(
            Event::new(&stored(event.clone()), None, &colors, "me", &tz())
                .expect("event")
                .color
                .name(),
            "green"
        );
        event.color_id = Some("11".into());
        assert_eq!(
            Event::new(&stored(event), None, &colors, "me", &tz())
                .expect("event")
                .color
                .name(),
            "pink"
        );
    }
}
