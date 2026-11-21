//! Calendar v3 state: the calendar list and events, with recurring-event expansion and a
//! revision counter behind sync tokens.

mod recurrence;
pub mod routes;

use gmail::calendar::{CalendarListEntry, Event, EventDateTime};
use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::error::{Failure, Result};

/// The user's calendars and their events.
#[derive(Debug, Clone)]
pub struct Calendars {
    /// The primary calendar's ID (the account address).
    pub primary: String,
    pub tz: TimeZone,
    pub entries: Vec<CalendarListEntry>,
    events: Vec<StoredEvent>,
    revision: u64,
    sequence: u64,
}

/// An event plus bookkeeping. Exceptions to a recurring series are stored under the
/// instance ID with `recurringEventId` set.
#[derive(Debug, Clone)]
pub struct StoredEvent {
    pub calendar_id: String,
    pub event: Event,
    /// Revision of the last change, compared against sync tokens.
    pub revision: u64,
}

impl Calendars {
    pub fn new(primary: &str, tz: TimeZone, entries: Vec<CalendarListEntry>) -> Self {
        Self {
            primary: primary.to_owned(),
            tz,
            entries,
            events: Vec::new(),
            revision: 1,
            sequence: 0,
        }
    }

    /// Resolves `primary` and checks the calendar exists.
    pub fn calendar(&self, id: &str) -> Result<&CalendarListEntry> {
        let id = if id == "primary" {
            self.primary.as_str()
        } else {
            id
        };
        self.entries
            .iter()
            .find(|entry| entry.id.as_deref() == Some(id))
            .ok_or_else(|| Failure::new(404, "Not Found", "notFound").legacy())
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Stored events (masters, single events and exceptions) of a calendar.
    pub fn stored(&self, calendar_id: &str) -> impl Iterator<Item = &StoredEvent> {
        self.events
            .iter()
            .filter(move |stored| stored.calendar_id == calendar_id)
    }

    /// Every concrete occurrence in `[min, max)`: single events, generated instances of
    /// recurring events, and exceptions. Cancelled ones are included; callers filter.
    pub fn occurrences(&self, calendar_id: &str, min: Timestamp, max: Timestamp) -> Vec<Event> {
        let mut out = Vec::new();
        for stored in self.stored(calendar_id) {
            let event = &stored.event;
            if event.recurrence.is_empty() {
                if self.overlaps(event, min, max) {
                    out.push(event.clone());
                }
                continue;
            }
            for instance in recurrence::expand(event, &self.tz, max) {
                let replaced = self
                    .stored(calendar_id)
                    .any(|other| other.event.id == instance.id);
                if !replaced && self.overlaps(&instance, min, max) {
                    out.push(instance);
                }
            }
        }
        out.sort_by_key(|event| self.start(event));
        out
    }

    /// An event by ID, including generated instances (`{id}_{yyyymmddThhmmssZ}`).
    pub fn event(&self, calendar_id: &str, id: &str) -> Result<Event> {
        if let Some(stored) = self
            .stored(calendar_id)
            .find(|stored| stored.event.id.as_deref() == Some(id))
        {
            return Ok(stored.event.clone());
        }
        let (master, _) = id.rsplit_once('_').ok_or_else(not_found)?;
        let series = self
            .stored(calendar_id)
            .find(|stored| stored.event.id.as_deref() == Some(master))
            .ok_or_else(not_found)?;
        recurrence::expand(&series.event, &self.tz, Timestamp::MAX)
            .find(|instance| instance.id.as_deref() == Some(id))
            .ok_or_else(not_found)
    }

    /// Stores a new or replacement event and bumps the revision.
    pub fn put(&mut self, calendar_id: &str, event: Event) -> Event {
        self.revision += 1;
        let revision = self.revision;
        match self
            .events
            .iter_mut()
            .find(|stored| stored.calendar_id == calendar_id && stored.event.id == event.id)
        {
            Some(stored) => {
                stored.event = event.clone();
                stored.revision = revision;
            }
            None => self.events.push(StoredEvent {
                calendar_id: calendar_id.to_owned(),
                event: event.clone(),
                revision,
            }),
        }
        event
    }

    /// Events changed after `revision`, for sync tokens.
    pub fn changed_since(&self, calendar_id: &str, revision: u64) -> Vec<Event> {
        self.stored(calendar_id)
            .filter(|stored| stored.revision > revision)
            .map(|stored| stored.event.clone())
            .collect()
    }

    /// A new 26-character event ID in Google's base32hex alphabet.
    pub fn next_id(&mut self) -> String {
        const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuv";
        self.sequence += 1;
        let halves = [
            self.sequence.wrapping_mul(0x9E37_79B9_7F4A_7C15),
            self.sequence.wrapping_mul(0xC2B2_AE3D_27D4_EB4F),
        ];
        halves
            .into_iter()
            .flat_map(|mut n| {
                (0..13).map(move |_| {
                    let ch = char::from(ALPHABET[(n % 32) as usize]);
                    n /= 32;
                    ch
                })
            })
            .collect()
    }

    pub fn start(&self, event: &Event) -> Timestamp {
        time(event.start.as_ref(), &self.tz)
    }

    pub fn end(&self, event: &Event) -> Timestamp {
        time(event.end.as_ref(), &self.tz)
    }

    fn overlaps(&self, event: &Event, min: Timestamp, max: Timestamp) -> bool {
        self.end(event) > min && self.start(event) < max
    }
}

fn time(value: Option<&EventDateTime>, tz: &TimeZone) -> Timestamp {
    value
        .and_then(|value| value.timestamp(tz))
        .unwrap_or(Timestamp::MIN)
}

fn not_found() -> Failure {
    Failure::new(404, "Not Found", "notFound").legacy()
}

/// Calendar's RFC 3339 rendering: UTC with milliseconds, as in `created`/`updated`.
pub fn rfc3339_utc(at: Timestamp) -> String {
    at.strftime("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}
