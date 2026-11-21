//! Google Calendar v3: calendar list, events and free/busy (`discovery/calendar.v3.json`).
//!
//! Times stay strings on the wire (`dateTime` keeps its UTC offset, `date` is all-day), with
//! [`EventDateTime::timestamp`] for arithmetic. Enum-like fields (`status`, `responseStatus`,
//! `eventType`, ...) are strings, since Calendar adds values over time.

use std::collections::BTreeMap;

use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

use crate::client::{Call, Client};
use crate::endpoints::join;

/// `calendarList.list`.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarList {
    /// `calendar#calendarList`.
    pub kind: Option<String>,
    /// ETag of the collection.
    pub etag: Option<String>,
    /// Token for the next page.
    pub next_page_token: Option<String>,
    /// Token for incremental sync, on the last page.
    pub next_sync_token: Option<String>,
    /// The calendars.
    #[serde(default)]
    pub items: Vec<CalendarListEntry>,
}

/// A calendar on the user's list.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarListEntry {
    /// `calendar#calendarListEntry`.
    pub kind: Option<String>,
    /// ETag.
    pub etag: Option<String>,
    /// Calendar ID (an email address for primary calendars).
    pub id: Option<String>,
    /// Title.
    pub summary: Option<String>,
    /// Description.
    pub description: Option<String>,
    /// Location.
    pub location: Option<String>,
    /// IANA time zone.
    pub time_zone: Option<String>,
    /// The user's own title for the calendar.
    pub summary_override: Option<String>,
    /// Color ID.
    pub color_id: Option<String>,
    /// Background color, `#rrggbb`.
    pub background_color: Option<String>,
    /// Foreground color, `#rrggbb`.
    pub foreground_color: Option<String>,
    /// Hidden from the list.
    pub hidden: Option<bool>,
    /// Shown in the UI.
    pub selected: Option<bool>,
    /// `freeBusyReader`, `reader`, `writer` or `owner`.
    pub access_role: Option<String>,
    /// Default reminders for events on this calendar.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_reminders: Vec<EventReminder>,
    /// Notification settings.
    pub notification_settings: Option<serde_json::Value>,
    /// The user's primary calendar.
    pub primary: Option<bool>,
    /// Deleted from the list.
    pub deleted: Option<bool>,
    /// Supported conference solutions.
    pub conference_properties: Option<serde_json::Value>,
    /// Whether invitations are accepted automatically.
    pub auto_accept_invitations: Option<bool>,
    /// Owner of a secondary calendar.
    pub data_owner: Option<String>,
}

/// `events.list` and `events.instances`.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Events {
    /// `calendar#events`.
    pub kind: Option<String>,
    /// ETag of the collection.
    pub etag: Option<String>,
    /// Calendar title.
    pub summary: Option<String>,
    /// Calendar description.
    pub description: Option<String>,
    /// Last modification of the calendar, RFC 3339.
    pub updated: Option<String>,
    /// Calendar time zone.
    pub time_zone: Option<String>,
    /// The user's access role.
    pub access_role: Option<String>,
    /// Default reminders.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_reminders: Vec<EventReminder>,
    /// Token for the next page.
    pub next_page_token: Option<String>,
    /// Token for incremental sync, on the last page.
    pub next_sync_token: Option<String>,
    /// The events.
    #[serde(default)]
    pub items: Vec<Event>,
}

/// An event. All fields are optional so the same type serves as a `patch` body.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    /// `calendar#event`.
    pub kind: Option<String>,
    /// ETag.
    pub etag: Option<String>,
    /// Event ID; instances of recurring events are `{id}_{start}`.
    pub id: Option<String>,
    /// `confirmed`, `tentative` or `cancelled`.
    pub status: Option<String>,
    /// Link to the event in Google Calendar.
    pub html_link: Option<String>,
    /// Creation time, RFC 3339.
    pub created: Option<String>,
    /// Last modification time, RFC 3339.
    pub updated: Option<String>,
    /// Title.
    pub summary: Option<String>,
    /// Description (may contain HTML).
    pub description: Option<String>,
    /// Location.
    pub location: Option<String>,
    /// Color ID.
    pub color_id: Option<String>,
    /// Who created it.
    pub creator: Option<EventPerson>,
    /// Who organizes it.
    pub organizer: Option<EventPerson>,
    /// Start (inclusive).
    pub start: Option<EventDateTime>,
    /// End (exclusive).
    pub end: Option<EventDateTime>,
    /// Whether the end time is unspecified.
    pub end_time_unspecified: Option<bool>,
    /// RRULE/EXRULE/RDATE/EXDATE lines, for recurring events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recurrence: Vec<String>,
    /// For instances: the recurring event's ID.
    pub recurring_event_id: Option<String>,
    /// For instances: the start the recurrence rule produced.
    pub original_start_time: Option<EventDateTime>,
    /// `opaque` (busy) or `transparent` (free).
    pub transparency: Option<String>,
    /// `default`, `public`, `private` or `confidential`.
    pub visibility: Option<String>,
    /// iCalendar UID, shared by all instances.
    #[serde(rename = "iCalUID")]
    pub ical_uid: Option<String>,
    /// iCalendar sequence number.
    pub sequence: Option<u32>,
    /// Attendees.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attendees: Vec<EventAttendee>,
    /// Whether attendees were left out of this representation.
    pub attendees_omitted: Option<bool>,
    /// Private and shared extended properties.
    pub extended_properties: Option<serde_json::Value>,
    /// Hangouts link.
    pub hangout_link: Option<String>,
    /// Conference (Meet) details.
    pub conference_data: Option<serde_json::Value>,
    /// Deprecated gadget.
    pub gadget: Option<serde_json::Value>,
    /// Anyone can add themselves.
    pub anyone_can_add_self: Option<bool>,
    /// Guests can invite others.
    pub guests_can_invite_others: Option<bool>,
    /// Guests can modify the event.
    pub guests_can_modify: Option<bool>,
    /// Guests can see other guests.
    pub guests_can_see_other_guests: Option<bool>,
    /// A private copy that doesn't propagate.
    pub private_copy: Option<bool>,
    /// Locked from changes to main fields.
    pub locked: Option<bool>,
    /// Reminders.
    pub reminders: Option<EventReminders>,
    /// Where the event was created from.
    pub source: Option<serde_json::Value>,
    /// Working location details.
    pub working_location_properties: Option<serde_json::Value>,
    /// Out-of-office details.
    pub out_of_office_properties: Option<serde_json::Value>,
    /// Focus time details.
    pub focus_time_properties: Option<serde_json::Value>,
    /// Birthday details.
    pub birthday_properties: Option<serde_json::Value>,
    /// File attachments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<EventAttachment>,
    /// `default`, `outOfOffice`, `focusTime`, `workingLocation`, `fromGmail`, `birthday`.
    pub event_type: Option<String>,
}

/// `start`/`end`: `date` for all-day events, `dateTime` otherwise.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDateTime {
    /// All-day date, `YYYY-MM-DD`.
    pub date: Option<String>,
    /// RFC 3339 time with offset.
    pub date_time: Option<String>,
    /// IANA time zone the time is expressed in.
    pub time_zone: Option<String>,
}

impl EventDateTime {
    /// The instant this denotes; all-day dates are midnight in `tz`.
    #[must_use]
    pub fn timestamp(&self, tz: &jiff::tz::TimeZone) -> Option<jiff::Timestamp> {
        if let Some(date_time) = &self.date_time {
            return date_time.parse().ok();
        }
        let date: jiff::civil::Date = self.date.as_deref()?.parse().ok()?;
        Some(date.to_zoned(tz.clone()).ok()?.timestamp())
    }
}

/// Creator or organizer.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventPerson {
    /// Profile ID.
    pub id: Option<String>,
    /// Email address.
    pub email: Option<String>,
    /// Name.
    pub display_name: Option<String>,
    /// Whether this is the authenticated user.
    #[serde(rename = "self")]
    pub is_self: Option<bool>,
}

/// An attendee.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventAttendee {
    /// Profile ID.
    pub id: Option<String>,
    /// Email address.
    pub email: Option<String>,
    /// Name.
    pub display_name: Option<String>,
    /// The organizer.
    pub organizer: Option<bool>,
    /// The authenticated user.
    #[serde(rename = "self")]
    pub is_self: Option<bool>,
    /// A resource (room).
    pub resource: Option<bool>,
    /// Optional attendance.
    pub optional: Option<bool>,
    /// `needsAction`, `declined`, `tentative` or `accepted`.
    pub response_status: Option<String>,
    /// RSVP comment.
    pub comment: Option<String>,
    /// Extra guests.
    pub additional_guests: Option<u32>,
}

/// `reminders`.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventReminders {
    /// Use the calendar's defaults.
    pub use_default: Option<bool>,
    /// Event-specific reminders.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<EventReminder>,
}

/// A reminder.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventReminder {
    /// `email` or `popup`.
    pub method: Option<String>,
    /// Minutes before the start.
    pub minutes: Option<u32>,
}

/// A file attached to an event.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventAttachment {
    /// URL of the file.
    pub file_url: Option<String>,
    /// Title.
    pub title: Option<String>,
    /// MIME type.
    pub mime_type: Option<String>,
    /// Icon URL.
    pub icon_link: Option<String>,
    /// Drive file ID.
    pub file_id: Option<String>,
}

/// `freebusy.query` body.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeBusyRequest {
    /// Range start, RFC 3339.
    pub time_min: Option<String>,
    /// Range end, RFC 3339.
    pub time_max: Option<String>,
    /// Time zone of the response.
    pub time_zone: Option<String>,
    /// Max calendars per group.
    pub group_expansion_max: Option<u32>,
    /// Max calendars overall.
    pub calendar_expansion_max: Option<u32>,
    /// Calendars to query.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<FreeBusyRequestItem>,
}

/// A calendar to query.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreeBusyRequestItem {
    /// Calendar or group ID.
    pub id: String,
}

/// `freebusy.query` response.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeBusyResponse {
    /// `calendar#freeBusy`.
    pub kind: Option<String>,
    /// Range start.
    pub time_min: Option<String>,
    /// Range end.
    pub time_max: Option<String>,
    /// Groups queried.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub groups: BTreeMap<String, FreeBusyGroup>,
    /// Busy times per calendar.
    #[serde(default)]
    pub calendars: BTreeMap<String, FreeBusyCalendar>,
}

/// Busy times of one calendar.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreeBusyCalendar {
    /// Why the calendar couldn't be queried, e.g. `notFound`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<CalendarError>,
    /// Busy periods.
    #[serde(default)]
    pub busy: Vec<TimePeriod>,
}

/// A group's expansion.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreeBusyGroup {
    /// Errors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<CalendarError>,
    /// Member calendars.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calendars: Vec<String>,
}

/// An error entry in a free/busy response.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarError {
    /// Usually `global`.
    pub domain: String,
    /// e.g. `notFound`.
    pub reason: String,
}

/// A busy interval.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimePeriod {
    /// Start, RFC 3339.
    pub start: String,
    /// End, RFC 3339.
    pub end: String,
}

/// `calendarList.list` parameters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CalendarListQuery {
    /// Page size (max 250).
    pub max_results: Option<u32>,
    /// Page token.
    pub page_token: Option<String>,
    /// Include hidden calendars.
    pub show_hidden: bool,
}

/// `events.list` parameters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventsQuery {
    /// Lower bound (exclusive) on event end, RFC 3339.
    pub time_min: Option<String>,
    /// Upper bound (exclusive) on event start, RFC 3339.
    pub time_max: Option<String>,
    /// Expand recurring events into instances.
    pub single_events: bool,
    /// `startTime` (requires `single_events`) or `updated`.
    pub order_by: Option<String>,
    /// Page size (default 250, max 2500).
    pub max_results: Option<u32>,
    /// Page token.
    pub page_token: Option<String>,
    /// Incremental sync token; a 410 means start over.
    pub sync_token: Option<String>,
    /// Include cancelled events.
    pub show_deleted: bool,
    /// Free-text search.
    pub q: Option<String>,
    /// Only events with this iCalendar UID.
    pub ical_uid: Option<String>,
    /// Only these event types.
    pub event_types: Vec<String>,
}

/// Calendar calls. Get one with [`Client::calendar`].
#[derive(Debug, Clone, Copy)]
pub struct Calendar<'a> {
    client: &'a Client,
}

impl Client {
    /// Calendar calls for the authenticated user.
    #[must_use]
    pub fn calendar(&self) -> Calendar<'_> {
        Calendar { client: self }
    }
}

impl<'a> Calendar<'a> {
    fn call<T>(&self, method: Method, path: &[&str]) -> Call<'a, T> {
        let base = ["calendar", "v3"];
        let url = join(
            &self.client.endpoints.calendar,
            base.iter().chain(path).copied(),
        );
        Call::new(self.client, method, url)
    }

    /// `calendarList.list`.
    pub fn calendar_list(&self, query: &CalendarListQuery) -> Call<'a, CalendarList> {
        self.call(Method::GET, &["users", "me", "calendarList"])
            .query_opt("maxResults", query.max_results)
            .query_opt("pageToken", query.page_token.as_ref())
            .query_opt("showHidden", query.show_hidden.then_some(true))
    }

    /// `events.list`.
    pub fn events(&self, calendar_id: &str, query: &EventsQuery) -> Call<'a, Events> {
        self.call(Method::GET, &["calendars", calendar_id, "events"])
            .query_opt("timeMin", query.time_min.as_ref())
            .query_opt("timeMax", query.time_max.as_ref())
            .query_opt("singleEvents", query.single_events.then_some(true))
            .query_opt("orderBy", query.order_by.as_ref())
            .query_opt("maxResults", query.max_results)
            .query_opt("pageToken", query.page_token.as_ref())
            .query_opt("syncToken", query.sync_token.as_ref())
            .query_opt("showDeleted", query.show_deleted.then_some(true))
            .query_opt("q", query.q.as_ref())
            .query_opt("iCalUID", query.ical_uid.as_ref())
            .query_all("eventTypes", &query.event_types)
    }

    /// `events.get`.
    pub fn event(&self, calendar_id: &str, event_id: &str) -> Call<'a, Event> {
        self.call(Method::GET, &["calendars", calendar_id, "events", event_id])
    }

    /// `events.insert`. `send_updates` is `all`, `externalOnly` or `none`. An event with
    /// `conferenceData` is sent with `conferenceDataVersion=1`, which a Meet `createRequest`
    /// needs.
    pub fn insert_event(
        &self,
        calendar_id: &str,
        event: &Event,
        send_updates: Option<&str>,
    ) -> Call<'a, Event> {
        self.call(Method::POST, &["calendars", calendar_id, "events"])
            .query_opt("sendUpdates", send_updates)
            .query_opt(
                "conferenceDataVersion",
                event.conference_data.is_some().then_some(1),
            )
            .json(event)
    }

    /// `events.import`: adds a private copy of an event from an invite.
    pub fn import_event(&self, calendar_id: &str, event: &Event) -> Call<'a, Event> {
        self.call(
            Method::POST,
            &["calendars", calendar_id, "events", "import"],
        )
        .json(event)
    }

    /// `events.patch`: changes only the fields set in `event` (with `conferenceData`, as
    /// [`Calendar::insert_event`] sends it).
    pub fn patch_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        event: &Event,
        send_updates: Option<&str>,
    ) -> Call<'a, Event> {
        self.call(
            Method::PATCH,
            &["calendars", calendar_id, "events", event_id],
        )
        .query_opt("sendUpdates", send_updates)
        .query_opt(
            "conferenceDataVersion",
            event.conference_data.is_some().then_some(1),
        )
        .json(event)
    }

    /// `events.delete`.
    pub fn delete_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        send_updates: Option<&str>,
    ) -> Call<'a, ()> {
        self.call(
            Method::DELETE,
            &["calendars", calendar_id, "events", event_id],
        )
        .query_opt("sendUpdates", send_updates)
    }

    /// `events.instances` of a recurring event within `[time_min, time_max)`.
    pub fn instances(
        &self,
        calendar_id: &str,
        event_id: &str,
        time_min: Option<&str>,
        time_max: Option<&str>,
    ) -> Call<'a, Events> {
        self.call(
            Method::GET,
            &["calendars", calendar_id, "events", event_id, "instances"],
        )
        .query_opt("timeMin", time_min)
        .query_opt("timeMax", time_max)
    }

    /// `freebusy.query`.
    pub fn free_busy(&self, request: &FreeBusyRequest) -> Call<'a, FreeBusyResponse> {
        self.call(Method::POST, &["freeBusy"]).json(request)
    }
}
