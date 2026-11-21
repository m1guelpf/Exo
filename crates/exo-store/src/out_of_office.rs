//! Out of Office Reply: Gmail's vacation responder, the copy of it the account's settings keep
//! (`outOfOfficeResponder`), and the calendar's out-of-office events Exo offers to set a reply for
//! (`outOfOfficeEventDetectionV2`).
//!
//! Exo reads and writes Gmail's `settings/vacation` ([`Store::vacation`],
//! [`Store::update_vacation`]). The copy is what the status bar and the nudge go by between reads.
//!
//! ```no_run
//! # async fn demo(store: &exo_store::Store) -> exo_store::Result<()> {
//! use exo_store::{OutOfOfficeResponder, Settings};
//!
//! let gmail = store.vacation().await?;
//! let mut settings: Settings = store.settings().await?;
//! let now = jiff::Timestamp::now();
//! if let Some(copy) = OutOfOfficeResponder::synced(&gmail, settings.out_of_office_responder.as_ref(), now) {
//!     settings.out_of_office_responder = Some(copy);
//!     store.save_settings(&settings).await?;
//! }
//! # Ok(()) }
//! ```

use std::collections::BTreeMap;

use gmail::VacationSettings;
use gmail::calendar::{Event, EventDateTime};
use jiff::civil::{Date, Weekday};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp, ToSpan, Zoned};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::quota::Lane;
use crate::store::Store;

/// How far ahead events are looked at: 35 days.
pub const OUT_OF_OFFICE_LOOKAHEAD: SignedDuration = SignedDuration::from_hours(35 * 24);
/// The words a summary marks an out-of-office event with.
const WORDS: [&str; 8] = [
    "ooo",
    "out of office",
    "out-of-office",
    "paid time off",
    "unpaid time off",
    "pto",
    "vacation",
    "annual leave",
];
/// An out-of-office event lasts at least this long: 8 hours.
const LONG_EVENT: SignedDuration = SignedDuration::from_hours(8);

/// `outOfOfficeResponder`: the account's copy of Gmail's vacation responder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OutOfOfficeResponder {
    /// `google`.
    pub google: GoogleResponder,
    /// `startTime`: when replies start, epoch milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<i64>,
    /// `endTime`: when they stop, epoch milliseconds; none for no last day.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<i64>,
}

/// `outOfOfficeResponder.google`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GoogleResponder {
    /// `enabledAutoReplyAt`: when the copy was last written with the reply on, an ISO timestamp,
    /// or empty while it's off.
    pub enabled_auto_reply_at: String,
    /// `responseSubject`.
    pub response_subject: String,
    /// `responseBodyHtml`.
    pub response_body_html: String,
    /// `restrictToContacts`.
    pub restrict_to_contacts: bool,
    /// `restrictToDomain`.
    pub restrict_to_domain: bool,
}

impl OutOfOfficeResponder {
    /// The copy of `settings` as written at `now`.
    pub fn of(settings: &VacationSettings, now: Timestamp) -> Self {
        let enabled = settings.enable_auto_reply == Some(true);
        Self {
            google: GoogleResponder {
                enabled_auto_reply_at: if enabled {
                    now.to_string()
                } else {
                    String::new()
                },
                response_subject: settings.response_subject.clone().unwrap_or_default(),
                response_body_html: settings.response_body_html.clone().unwrap_or_default(),
                restrict_to_contacts: settings.restrict_to_contacts == Some(true),
                restrict_to_domain: settings.restrict_to_domain == Some(true),
            },
            start_time: settings.start_time,
            end_time: settings.end_time,
        }
    }

    /// The copy to keep after reading Gmail's `settings` at `now`, if `copy` no longer matches
    /// them: a plain-text body (one set in Gmail's plain-text mode) stands in for the HTML one.
    /// `None` when nothing changed.
    pub fn synced(
        settings: &VacationSettings,
        copy: Option<&Self>,
        now: Timestamp,
    ) -> Option<Self> {
        if !differs(settings, copy) {
            return None;
        }
        let mut settings = settings.clone();
        if let Some(plain) = settings
            .response_body_plain_text
            .take()
            .filter(|plain| !plain.is_empty())
            && copy.is_none_or(|copy| plain != copy.google.response_body_html)
        {
            settings.response_body_html = Some(plain);
        }
        Some(Self::of(&settings, now))
    }

    /// The reply is on (`enabledAutoReplyAt` is set).
    pub fn is_enabled(&self) -> bool {
        !self.google.enabled_auto_reply_at.is_empty()
    }

    /// Replies are going out at `now`: on, started, and not ended.
    pub fn is_out_of_office(&self, now: Timestamp) -> bool {
        let now = now.as_millisecond();
        self.is_enabled()
            && self.start_time.is_some_and(|start| start < now)
            && self.end_time.is_none_or(|end| end > now)
    }
}

/// Whether Gmail's `settings` differ from `copy`: nothing differs from nothing.
fn differs(settings: &VacationSettings, copy: Option<&OutOfOfficeResponder>) -> bool {
    let empty = *settings == VacationSettings::default();
    if empty && copy.is_none_or(|copy| *copy == OutOfOfficeResponder::default()) {
        return false;
    }
    let google = copy.map(|copy| &copy.google);
    let plain = settings
        .response_body_plain_text
        .as_deref()
        .filter(|plain| !plain.is_empty());
    copy.and_then(|copy| copy.start_time) != settings.start_time
        || copy.and_then(|copy| copy.end_time) != settings.end_time
        || copy.is_some_and(OutOfOfficeResponder::is_enabled)
            != (settings.enable_auto_reply == Some(true))
        || (plain.is_none()
            && google.map(|google| google.response_body_html.as_str())
                != settings.response_body_html.as_deref())
        || google.map(|google| google.response_subject.as_str())
            != settings.response_subject.as_deref()
        || google.is_some_and(|google| google.restrict_to_contacts)
            != (settings.restrict_to_contacts == Some(true))
        || google.is_some_and(|google| google.restrict_to_domain)
            != (settings.restrict_to_domain == Some(true))
        || plain.is_some_and(|plain| google.is_none_or(|google| plain != google.response_body_html))
}

/// `outOfOfficeEventDetectionV2`: the user's out-of-office events coming up, and whether they
/// asked not to be offered a reply for them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OutOfOfficeEventDetection {
    /// `events`, by event id.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub events: BTreeMap<String, DetectedEvent>,
    /// `optedOutAt`: when `Don't show this again` was checked, an ISO timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opted_out_at: Option<String>,
}

/// `outOfOfficeEventDetectionV2.events[id]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DetectedEvent {
    /// `id`: the event's.
    pub id: String,
    /// `startTime`.
    pub start_time: EventTime,
    /// `endTime`.
    pub end_time: EventTime,
    /// `nudgedAt`: when the nudge was shown for it, an ISO timestamp.
    pub nudged_at: Option<String>,
}

/// An event's start or end: `date` for an all-day one (`2026-11-26`), `datetime` (an ISO
/// timestamp) for the rest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventTime {
    /// `date`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// `datetime`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub datetime: Option<String>,
}

impl EventTime {
    fn of(time: &EventDateTime) -> Option<Self> {
        if let Some(date) = &time.date {
            return Some(Self {
                date: Some(date.clone()),
                datetime: None,
            });
        }
        let at: Timestamp = time.date_time.as_deref()?.parse().ok()?;
        Some(Self {
            date: None,
            datetime: Some(at.to_string()),
        })
    }

    /// The instant it starts at in `tz`: a date's local midnight.
    fn start(&self, tz: &TimeZone) -> Option<Timestamp> {
        match (&self.datetime, &self.date) {
            (Some(at), _) => at.parse().ok(),
            (None, Some(date)) => date
                .parse::<Date>()
                .ok()?
                .to_zoned(tz.clone())
                .ok()
                .map(|zoned| zoned.timestamp()),
            (None, None) => None,
        }
    }

    /// The last instant it covers as an end in `tz`: a timed end on a local midnight is the
    /// second before; an all-day end (exclusive) the day before's last millisecond.
    fn inclusive_end(&self, tz: &TimeZone) -> Option<Timestamp> {
        match (&self.datetime, &self.date) {
            (Some(at), _) => {
                let at: Timestamp = at.parse().ok()?;
                let local = at.to_zoned(tz.clone());
                Some(if local.hour() == 0 && local.minute() == 0 {
                    at - SignedDuration::from_secs(1)
                } else {
                    at
                })
            }
            (None, Some(date)) => Some(
                date.parse::<Date>()
                    .ok()?
                    .to_zoned(tz.clone())
                    .ok()?
                    .timestamp()
                    - SignedDuration::from_millis(1),
            ),
            (None, None) => None,
        }
    }

    /// The instant pruning compares with now: `datetime`, else `date`'s UTC midnight.
    fn as_written(&self) -> Option<Timestamp> {
        self.start(&TimeZone::UTC)
    }
}

/// Out-of-office days to offer a reply for: consecutive events merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutOfOfficeDays {
    /// When the first starts.
    pub start: Timestamp,
    /// The last instant the last covers.
    pub end: Timestamp,
    /// The events.
    pub ids: Vec<String>,
}

impl OutOfOfficeEventDetection {
    /// The events to keep after Calendar listed `events` (the user's primary calendar, from now
    /// to [`OUT_OF_OFFICE_LOOKAHEAD`] ahead) for `me` at `now`: the user's out-of-office events,
    /// with when each was offered, plus those offered that Calendar no longer lists; less those
    /// offered over a week ago or already started. `None` when that's what's kept already, or
    /// after opting out.
    pub fn detected(
        &self,
        events: &[Event],
        me: &str,
        now: Timestamp,
    ) -> Option<BTreeMap<String, DetectedEvent>> {
        if self.opted_out_at.is_some() {
            return None;
        }
        let mut found = BTreeMap::new();
        for event in events {
            let Some(id) = event.id.clone() else { continue };
            let stored = self.events.get(&id);
            // An event kept that isn't the user's out-of-office event any more is dropped.
            if !is_owners_out_of_office(event, me) || event.status.as_deref() == Some("cancelled") {
                continue;
            }
            let (Some(start), Some(end)) = (
                event.start.as_ref().and_then(EventTime::of),
                event.end.as_ref().and_then(EventTime::of),
            ) else {
                continue;
            };
            let nudged_at = stored.and_then(|stored| stored.nudged_at.clone());
            found.insert(
                id.clone(),
                DetectedEvent {
                    id,
                    start_time: start,
                    end_time: end,
                    nudged_at,
                },
            );
        }
        for (id, stored) in &self.events {
            if stored.nudged_at.is_some() && !found.contains_key(id) {
                found.insert(id.clone(), stored.clone());
            }
        }
        if found == self.events {
            return None;
        }
        let week_ago = now - SignedDuration::from_hours(7 * 24);
        found.retain(|_, event| match &event.nudged_at {
            Some(at) => at.parse::<Timestamp>().is_ok_and(|at| at >= week_ago),
            None => event
                .start_time
                .as_written()
                .is_some_and(|start| start >= now),
        });
        Some(found)
    }

    /// The days to offer a reply for at `now` in `tz`, if any: the earliest event yet to start
    /// and not offered, starting by the end of the seventh day from today, merged with each
    /// following one that starts at most a day after it ends, or on the Monday after a Friday it
    /// ends.
    pub fn upcoming(&self, now: &Zoned) -> Option<OutOfOfficeDays> {
        let tz = now.time_zone();
        let mut candidates: Vec<(Timestamp, Timestamp, &str)> = self
            .events
            .values()
            .filter(|event| event.nudged_at.is_none())
            .filter_map(|event| {
                Some((
                    event.start_time.start(tz)?,
                    event.end_time.inclusive_end(tz)?,
                    event.id.as_str(),
                ))
            })
            .filter(|(start, ..)| *start > now.timestamp())
            .collect();
        candidates.sort_by_key(|(start, ..)| *start);
        let window = now
            .date()
            .checked_add(8.days())
            .ok()?
            .to_zoned(tz.clone())
            .ok()?
            .timestamp()
            - SignedDuration::from_millis(1);
        let mut days: Option<OutOfOfficeDays> = None;
        for (start, end, id) in candidates {
            match &mut days {
                None if start > window => break,
                None => {
                    days = Some(OutOfOfficeDays {
                        start,
                        end,
                        ids: vec![id.to_owned()],
                    })
                }
                Some(days) => {
                    let (last, next) = (
                        days.end.to_zoned(tz.clone()).date(),
                        start.to_zoned(tz.clone()).date(),
                    );
                    let adjacent = (next - last).get_days() <= 1
                        || (last.weekday() == Weekday::Friday
                            && next.weekday() == Weekday::Monday
                            && start < days.end + SignedDuration::from_hours(4 * 24));
                    if !adjacent {
                        break;
                    }
                    days.end = days.end.max(end);
                    days.ids.push(id.to_owned());
                }
            }
        }
        days
    }

    /// Whether the nudge may show at `now` for `days`: not opted out, and no nudge in the last
    /// day nor more than one in the last week. When it may, marks `days`' events offered.
    pub fn nudge(&mut self, days: &OutOfOfficeDays, now: Timestamp) -> bool {
        if !self.may_nudge(days, now) {
            return false;
        }
        for id in &days.ids {
            if let Some(event) = self.events.get_mut(id) {
                event.nudged_at = Some(now.to_string());
            }
        }
        true
    }

    /// [`nudge`](Self::nudge) for an out-of-office event the user just made in Exo, which the
    /// calendar hasn't listed yet: it's kept, offered at `now`.
    pub fn nudge_created(&mut self, days: &OutOfOfficeDays, now: Timestamp) -> bool {
        if !self.may_nudge(days, now) {
            return false;
        }
        let at = |instant: Timestamp| EventTime {
            date: None,
            datetime: Some(instant.to_string()),
        };
        for id in &days.ids {
            self.events.insert(
                id.clone(),
                DetectedEvent {
                    id: id.clone(),
                    start_time: at(days.start),
                    end_time: at(days.end),
                    nudged_at: Some(now.to_string()),
                },
            );
        }
        true
    }

    fn may_nudge(&self, days: &OutOfOfficeDays, now: Timestamp) -> bool {
        if self.opted_out_at.is_some() || days.ids.is_empty() {
            return false;
        }
        let nudged: Vec<Timestamp> = self
            .events
            .values()
            .filter_map(|event| event.nudged_at.as_deref()?.parse().ok())
            .collect();
        let since = |hours: i64| {
            nudged
                .iter()
                .filter(|at| **at > now - SignedDuration::from_hours(hours))
                .count()
        };
        since(24) == 0 && since(7 * 24) <= 1
    }
}

impl OutOfOfficeDays {
    /// The days `event`, which `me` just made, is out of office for, if it starts within a week
    /// of `now`: its start, and its end, a second earlier when that's a local midnight.
    pub fn created(event: &Event, me: &str, now: &Zoned) -> Option<Self> {
        if !is_owners_out_of_office(event, me) {
            return None;
        }
        let tz = now.time_zone();
        let start = EventTime::of(event.start.as_ref()?)?.start(tz)?;
        let end = EventTime::of(event.end.as_ref()?)?;
        let end = match end.datetime {
            Some(_) => end.inclusive_end(tz)?,
            // An all-day event ends at the local midnight after its last day.
            None => end.start(tz)? - SignedDuration::from_secs(1),
        };
        let id = event.id.clone()?;
        (start < now.timestamp() + SignedDuration::from_hours(7 * 24)).then(|| Self {
            start,
            end,
            ids: vec![id],
        })
    }
}

/// Whether `event` is the user's own out-of-office event: theirs to organize, not recurring,
/// and out of office: an `outOfOffice` event or a summary saying so, at least 8 hours long.
/// (Whether an invitee accepted doesn't come into it: the organizer is the user.)
fn is_owners_out_of_office(event: &Event, me: &str) -> bool {
    let organizes = event.organizer.as_ref().is_none_or(|organizer| {
        organizer.is_self == Some(true)
            || organizer
                .email
                .as_deref()
                .is_some_and(|email| email.eq_ignore_ascii_case(me))
    });
    if !organizes || !event.recurrence.is_empty() || event.recurring_event_id.is_some() {
        return false;
    }
    let tz = TimeZone::UTC;
    let instant = |time: Option<&EventDateTime>| {
        time.and_then(EventTime::of)
            .and_then(|time| time.start(&tz))
    };
    let (Some(start), Some(end)) = (instant(event.start.as_ref()), instant(event.end.as_ref()))
    else {
        return false;
    };
    if end.duration_since(start) < LONG_EVENT {
        return false;
    }
    let summary = event.summary.as_deref().unwrap_or_default().to_lowercase();
    event.event_type.as_deref() == Some("outOfOffice")
        || WORDS.iter().any(|word| has_word(&summary, word))
}

/// Whether `text` has `word` between word boundaries (`\b{word}\b`, ASCII word characters).
fn has_word(text: &str, word: &str) -> bool {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        before.is_none_or(|c| !is_word(c)) && after.is_none_or(|c| !is_word(c))
    })
}

impl Store {
    /// Gmail's vacation responder (`users.settings.getVacation`, 1 unit).
    ///
    /// # Errors
    /// [`crate::Error::Offline`] without a network; [`crate::Error::Gmail`] when Gmail refuses.
    pub async fn vacation(&self) -> Result<VacationSettings> {
        Ok(self
            .inner
            .api
            .call(Lane::Act, 3, |gmail| gmail.vacation())
            .await?)
    }

    /// Sets Gmail's vacation responder to `settings` (`users.settings.updateVacation`, 5 units)
    /// and returns what Gmail kept.
    ///
    /// # Errors
    /// As [`Store::vacation`].
    pub async fn update_vacation(&self, settings: &VacationSettings) -> Result<VacationSettings> {
        Ok(self
            .inner
            .api
            .call(Lane::Act, 1, |gmail| gmail.update_vacation(settings))
            .await?)
    }
}

#[cfg(test)]
mod tests {
    use gmail::calendar::EventPerson;

    use super::*;

    const ME: &str = "maya@brightmoor.co";

    fn at(text: &str) -> Timestamp {
        text.parse().expect("a timestamp")
    }

    fn now() -> Zoned {
        "2026-11-17T09:00:00-05:00[America/New_York]"
            .parse()
            .expect("a time")
    }

    fn vacation(enabled: bool) -> VacationSettings {
        VacationSettings {
            enable_auto_reply: Some(enabled),
            response_subject: Some("Out of office".into()),
            response_body_html: Some("<div>Back Monday.</div>".into()),
            restrict_to_contacts: Some(false),
            restrict_to_domain: Some(false),
            start_time: Some(at("2026-11-17T05:00:00Z").as_millisecond()),
            end_time: Some(at("2026-11-21T05:00:00Z").as_millisecond()),
            response_body_plain_text: None,
        }
    }

    #[test]
    fn the_copy_is_out_of_office_between_its_days_while_on() {
        let copy = OutOfOfficeResponder::of(&vacation(true), now().timestamp());
        assert!(copy.is_enabled());
        assert!(copy.is_out_of_office(now().timestamp()));
        assert!(
            !copy.is_out_of_office(at("2026-11-17T04:00:00Z")),
            "before the first day"
        );
        assert!(
            !copy.is_out_of_office(at("2026-11-21T05:00:00Z")),
            "the end is exclusive"
        );
        let open = OutOfOfficeResponder {
            end_time: None,
            ..copy.clone()
        };
        assert!(
            open.is_out_of_office(at("2027-06-01T00:00:00Z")),
            "no last day"
        );
        assert!(
            !OutOfOfficeResponder::of(&vacation(false), now().timestamp())
                .is_out_of_office(now().timestamp())
        );
    }

    #[test]
    fn reading_gmail_rewrites_the_copy_only_when_it_changed() {
        let now = now().timestamp();
        let gmail = vacation(true);
        let copy = OutOfOfficeResponder::synced(&gmail, None, now).expect("nothing kept yet");
        assert_eq!(copy.google.enabled_auto_reply_at, now.to_string());
        assert_eq!(OutOfOfficeResponder::synced(&gmail, Some(&copy), now), None);
        let off = VacationSettings {
            enable_auto_reply: Some(false),
            ..gmail.clone()
        };
        assert_eq!(
            OutOfOfficeResponder::synced(&off, Some(&copy), now).map(|copy| copy.is_enabled()),
            Some(false)
        );
        let moved = VacationSettings {
            end_time: None,
            ..gmail.clone()
        };
        assert_eq!(
            OutOfOfficeResponder::synced(&moved, Some(&copy), now).and_then(|copy| copy.end_time),
            None
        );
        assert_eq!(
            OutOfOfficeResponder::synced(&VacationSettings::default(), None, now),
            None,
            "nothing, and nothing kept"
        );
    }

    #[test]
    fn a_plain_text_body_stands_in_for_the_html_one() {
        let now = now().timestamp();
        let plain = VacationSettings {
            response_body_html: None,
            response_body_plain_text: Some("Back Monday.\n\nMaya".into()),
            ..vacation(false)
        };
        let copy = OutOfOfficeResponder::synced(&plain, None, now).expect("new");
        assert_eq!(copy.google.response_body_html, "Back Monday.\n\nMaya");
        assert_eq!(
            OutOfOfficeResponder::synced(&plain, Some(&copy), now),
            None,
            "the same text again"
        );
    }

    fn event(
        id: &str,
        summary: &str,
        kind: &str,
        start: EventDateTime,
        end: EventDateTime,
    ) -> Event {
        Event {
            id: Some(id.into()),
            summary: Some(summary.into()),
            event_type: Some(kind.into()),
            start: Some(start),
            end: Some(end),
            ..Event::default()
        }
    }

    fn day(date: &str) -> EventDateTime {
        EventDateTime {
            date: Some(date.into()),
            ..EventDateTime::default()
        }
    }

    fn timed(at: &str) -> EventDateTime {
        EventDateTime {
            date_time: Some(at.into()),
            ..EventDateTime::default()
        }
    }

    #[test]
    fn the_owners_long_out_of_office_events_are_found() {
        let now = now().timestamp();
        let events = [
            event(
                "ooo",
                "Out of office",
                "outOfOffice",
                timed("2026-11-19T00:00:00-05:00"),
                timed("2026-11-21T00:00:00-05:00"),
            ),
            event(
                "pto",
                "PTO (Lisbon)",
                "default",
                day("2026-11-24"),
                day("2026-11-25"),
            ),
            event(
                "short",
                "Vacation planning",
                "default",
                timed("2026-11-18T10:00:00-05:00"),
                timed("2026-11-18T11:00:00-05:00"),
            ),
            event(
                "photo",
                "Photoshoot",
                "default",
                day("2026-11-24"),
                day("2026-11-25"),
            ),
            Event {
                recurring_event_id: Some("weekly".into()),
                ..event(
                    "recurring",
                    "OOO",
                    "default",
                    day("2026-11-26"),
                    day("2026-11-27"),
                )
            },
            Event {
                organizer: Some(EventPerson {
                    email: Some("sam@brightmoor.co".into()),
                    ..EventPerson::default()
                }),
                ..event(
                    "invited",
                    "Team offsite (out of office)",
                    "default",
                    day("2026-11-27"),
                    day("2026-11-28"),
                )
            },
        ];
        let detection = OutOfOfficeEventDetection::default();
        let found = detection.detected(&events, ME, now).expect("new events");
        assert_eq!(
            found.keys().map(String::as_str).collect::<Vec<_>>(),
            ["ooo", "pto"]
        );
        assert_eq!(
            found["ooo"].start_time.datetime.as_deref(),
            Some("2026-11-19T05:00:00Z")
        );
        assert_eq!(found["pto"].start_time.date.as_deref(), Some("2026-11-24"));
        let kept = OutOfOfficeEventDetection {
            events: found,
            opted_out_at: None,
        };
        assert_eq!(kept.detected(&events, ME, now), None, "nothing new");
        let opted_out = OutOfOfficeEventDetection {
            opted_out_at: Some(now.to_string()),
            ..OutOfOfficeEventDetection::default()
        };
        assert_eq!(opted_out.detected(&events, ME, now), None);
    }

    #[test]
    fn events_offered_stay_a_week_and_the_rest_until_they_start() {
        let now = now().timestamp();
        let offered = |id: &str, at: &str| DetectedEvent {
            id: id.into(),
            start_time: date("2026-11-10"),
            nudged_at: Some(at.into()),
            ..DetectedEvent::default()
        };
        let events = BTreeMap::from([
            (
                "recent".to_owned(),
                offered("recent", "2026-11-15T12:00:00Z"),
            ),
            ("old".to_owned(), offered("old", "2026-11-01T12:00:00Z")),
        ]);
        let detection = OutOfOfficeEventDetection {
            events,
            opted_out_at: None,
        };
        // What's kept is cleaned only when something else changed.
        assert_eq!(detection.detected(&[], ME, now), None);
        let new = [event(
            "pto",
            "PTO",
            "default",
            day("2026-11-24"),
            day("2026-11-25"),
        )];
        let kept = detection.detected(&new, ME, now).expect("a new event");
        assert_eq!(
            kept.keys().map(String::as_str).collect::<Vec<_>>(),
            ["pto", "recent"],
            "the old one goes"
        );
    }

    fn stored(id: &str, start: EventTime, end: EventTime) -> (String, DetectedEvent) {
        (
            id.to_owned(),
            DetectedEvent {
                id: id.into(),
                start_time: start,
                end_time: end,
                nudged_at: None,
            },
        )
    }

    fn date(text: &str) -> EventTime {
        EventTime {
            date: Some(text.into()),
            datetime: None,
        }
    }

    #[test]
    fn consecutive_days_and_a_weekend_merge_into_one_offer() {
        let now = now();
        let detection = OutOfOfficeEventDetection {
            events: [
                stored("thu", date("2026-11-19"), date("2026-11-20")),
                stored("fri", date("2026-11-20"), date("2026-11-21")),
                stored("mon", date("2026-11-23"), date("2026-11-24")),
                stored("later", date("2026-11-26"), date("2026-11-27")),
            ]
            .into(),
            opted_out_at: None,
        };
        let days = detection.upcoming(&now).expect("days this week");
        assert_eq!(days.ids, ["thu", "fri", "mon"]);
        assert_eq!(days.start, at("2026-11-19T05:00:00Z"));
        assert_eq!(
            days.end,
            at("2026-11-24T04:59:59.999Z"),
            "the Monday's last millisecond"
        );
    }

    #[test]
    fn only_days_starting_within_a_week_are_offered() {
        let now = now();
        let next_week = OutOfOfficeEventDetection {
            events: [stored("far", date("2026-11-25"), date("2026-11-26"))].into(),
            opted_out_at: None,
        };
        assert_eq!(
            next_week.upcoming(&now),
            None,
            "the 25th is past the 24th's end"
        );
        let in_a_week = OutOfOfficeEventDetection {
            events: [stored("near", date("2026-11-24"), date("2026-11-25"))].into(),
            opted_out_at: None,
        };
        assert!(in_a_week.upcoming(&now).is_some());
        let timed = OutOfOfficeEventDetection {
            events: [stored(
                "ooo",
                EventTime {
                    date: None,
                    datetime: Some("2026-11-19T05:00:00Z".into()),
                },
                EventTime {
                    date: None,
                    datetime: Some("2026-11-21T05:00:00Z".into()),
                },
            )]
            .into(),
            opted_out_at: None,
        };
        assert_eq!(
            timed.upcoming(&now).map(|days| days.end),
            Some(at("2026-11-21T04:59:59Z")),
            "a midnight end is the second before"
        );
    }

    #[test]
    fn the_nudge_shows_at_most_once_a_day_and_twice_a_week() {
        let now = now().timestamp();
        let mut detection = OutOfOfficeEventDetection {
            events: [stored("thu", date("2026-11-19"), date("2026-11-20"))].into(),
            opted_out_at: None,
        };
        let days = OutOfOfficeDays {
            start: now,
            end: now,
            ids: vec!["thu".into()],
        };
        assert!(detection.nudge(&days, now));
        assert_eq!(
            detection.events["thu"].nudged_at.as_deref(),
            Some(now.to_string().as_str())
        );
        detection
            .events
            .extend([stored("fri", date("2026-11-20"), date("2026-11-21"))]);
        let friday = OutOfOfficeDays {
            ids: vec!["fri".into()],
            ..days.clone()
        };
        assert!(
            !detection.nudge(&friday, now + SignedDuration::from_hours(12)),
            "within a day"
        );
        assert!(detection.nudge(&friday, now + SignedDuration::from_hours(25)));
        detection
            .events
            .extend([stored("sat", date("2026-11-21"), date("2026-11-22"))]);
        let saturday = OutOfOfficeDays {
            ids: vec!["sat".into()],
            ..days
        };
        assert!(
            !detection.nudge(&saturday, now + SignedDuration::from_hours(50)),
            "two this week"
        );
    }

    #[test]
    fn an_event_made_in_exo_is_offered_when_it_starts_this_week() {
        let now = now();
        let mut vacation = event(
            "new",
            "Vacation",
            "default",
            day("2026-11-20"),
            day("2026-11-22"),
        );
        let days = OutOfOfficeDays::created(&vacation, ME, &now).expect("this week");
        assert_eq!(
            (days.start, days.end),
            (at("2026-11-20T05:00:00Z"), at("2026-11-22T04:59:59Z"))
        );
        let mut detection = OutOfOfficeEventDetection::default();
        assert!(detection.nudge_created(&days, now.timestamp()));
        assert_eq!(
            detection.events["new"].nudged_at.as_deref(),
            Some(now.timestamp().to_string().as_str())
        );
        vacation.start = Some(day("2026-11-30"));
        vacation.end = Some(day("2026-12-01"));
        assert_eq!(
            OutOfOfficeDays::created(&vacation, ME, &now),
            None,
            "next week"
        );
        assert_eq!(
            OutOfOfficeDays::created(
                &event(
                    "lunch",
                    "Lunch",
                    "default",
                    day("2026-11-20"),
                    day("2026-11-21")
                ),
                ME,
                &now
            ),
            None
        );
    }

    #[test]
    fn summary_words_match_whole_words() {
        assert!(has_word("pto (lisbon)", "pto"));
        assert!(has_word("out-of-office", "out-of-office"));
        assert!(!has_word("photo shoot", "pto"));
        assert!(!has_word("ooops", "ooo"));
    }
}
