//! The calendar cache: the user's calendar list and the events of the days the calendar has
//! shown, fetched from Google Calendar a window at a time.
//!
//! Exo asks Calendar for the instances of a window (`singleEvents`) and replaces what it has for
//! that window with the answer, so exceptions and cancellations come out as Calendar has them.
//! Changes to events wait in their own queue until Calendar has them, and every read shows them
//! ([`outbox`]).
//!
//! Other Google accounts can be signed in for their calendars alone (`calendarAccounts`,
//! [`Store::add_calendar_account`]). Their lists are kept beside the mail account's, and each
//! calendar is read, and changed, through one account: the one whose primary it is, else the one
//! with the most access to it ([`Store::calendars`]). Events are kept by calendar, whichever
//! account read them.
//!
//! ```no_run
//! # async fn demo(store: &exo_store::Store) -> exo_store::Result<()> {
//! use jiff::{civil::date, tz::TimeZone};
//!
//! let tz = TimeZone::get("America/New_York").expect("zone");
//! let calendars = store.sync_calendars("maya@brightmoor.co").await?;
//! let primary = calendars.iter().find(|calendar| calendar.primary == Some(true));
//! if let Some(id) = primary.and_then(|calendar| calendar.id.clone()) {
//!     let days = date(2026, 11, 15)..date(2026, 11, 22);
//!     store.sync_events(&id, days.clone(), &tz).await?;
//!     let week = store.events(&[id], days, &tz).await?;
//! }
//! # Ok(()) }
//! ```

use std::collections::{BTreeMap, HashSet};
use std::ops::Range;

use gmail::calendar::{
    CalendarListEntry, CalendarListQuery, Event, EventAttendee, EventDateTime, EventsQuery,
};
use gmail::{Client, TokenProvider};
use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use toasty::{Executor, Json};

use crate::api::Failure;
use crate::db::{CalendarRow, EventRow};
use crate::error::Result;
use crate::store::Store;

pub(crate) mod outbox;
pub(crate) mod recurrence;

/// The event types the calendar shows: not working locations or birthdays.
const EVENT_TYPES: [&str; 4] = ["default", "focusTime", "outOfOffice", "fromGmail"];
/// Calendar's largest page.
const PAGE: u32 = 2500;
/// Sync's retries for one call.
const ATTEMPTS: u32 = 3;
/// The `maxResults` for someone else's calendar: one page of it.
const ATTENDEE_PAGE: u32 = 500;

/// An event in the cache, and the calendar it's on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    /// The calendar's id.
    pub calendar_id: String,
    /// The event (an instance, for a recurring one) as Calendar listed it, with the changes
    /// queued for it.
    pub event: Event,
    /// Calendar doesn't have it yet: it's a new event still queued.
    pub unsynced: bool,
}

/// A calendar on a calendar account's list, and that account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountCalendar {
    /// The calendar account it's read through: the mail account, or another one signed in for
    /// its calendars.
    pub account: String,
    pub entry: CalendarListEntry,
}

impl Store {
    /// The calendars on the lists of the account and of its calendar accounts signed in, as last
    /// synced. Each shows once, from the account it's read through: the one whose primary
    /// calendar it is, else the one with the most access to it (owner, writer, reader), else the
    /// mail account, else the first by address. The mail account's come first, then the others'
    /// by address, each in Calendar's order. Hidden calendars aren't kept, except a hidden
    /// primary one.
    ///
    /// # Errors
    /// On database failures.
    pub async fn calendars(&self) -> Result<Vec<AccountCalendar>> {
        let mut db = self.inner.db.clone();
        let rows = CalendarRow::all().exec(&mut db).await?;
        Ok(held(rows, &self.inner.account, |account| {
            self.reads(account)
        }))
    }

    /// Lists `account`'s calendars from Google (the mail account's, or a calendar account's
    /// signed in) and stores them in place of its last list.
    ///
    /// # Errors
    /// When Calendar can't be reached or refuses, or on database failures.
    pub async fn sync_calendars(&self, account: &str) -> Result<Vec<CalendarListEntry>> {
        let client = self.client(account);
        let mut entries = Vec::new();
        let mut query = CalendarListQuery {
            max_results: Some(1000),
            show_hidden: true,
            ..CalendarListQuery::default()
        };
        loop {
            let page = self
                .inner
                .api
                .request_with(&client, ATTEMPTS, |client| {
                    client.calendar().calendar_list(&query)
                })
                .await?;
            entries.extend(page.items.into_iter().filter(|entry| {
                entry.id.is_some()
                    && entry.summary.is_some()
                    && (entry.hidden != Some(true) || entry.primary == Some(true))
            }));
            match page.next_page_token {
                Some(token) => query.page_token = Some(token),
                None => break,
            }
        }
        let mut db = self.inner.db.clone();
        let mut write = self.inner.begin(&mut db).await?;
        CalendarRow::filter(CalendarRow::fields().account().eq(account))
            .delete()
            .exec(write.exec())
            .await?;
        if !entries.is_empty() {
            let mut rows = CalendarRow::create_many();
            for (position, entry) in entries.iter().enumerate() {
                let id = entry.id.clone().unwrap_or_default();
                rows = rows.with_item(|row| {
                    row.account(account)
                        .id(id)
                        .position(u32::try_from(position).unwrap_or(u32::MAX))
                        .entry(Json(entry.clone()))
                });
            }
            rows.exec(write.exec()).await?;
        }
        write.publish().await?;
        Ok(entries)
    }

    /// Reads calendar account `email`'s calendars with `tokens` from now on (it signed in, or the
    /// account opened): its list joins [`Store::calendars`], and the calls about the calendars
    /// it's read through use it.
    pub fn add_calendar_account(&self, email: &str, tokens: impl TokenProvider + 'static) {
        let client = Client::new(self.inner.api.client().endpoints().clone(), tokens);
        self.inner
            .calendar_clients
            .write()
            .expect("calendar clients are never poisoned")
            .insert(email.to_owned(), client);
    }

    /// Stops reading calendar account `email` (Google signed it out): its calendars leave
    /// [`Store::calendars`] until it's added again. With `forget` (it was removed), its list
    /// goes, and the events of the calendars no other account lists.
    ///
    /// # Errors
    /// On database failures.
    pub async fn remove_calendar_account(&self, email: &str, forget: bool) -> Result<()> {
        self.inner
            .calendar_clients
            .write()
            .expect("calendar clients are never poisoned")
            .remove(email);
        if !forget {
            return Ok(());
        }
        let mut db = self.inner.db.clone();
        let rows = CalendarRow::all().exec(&mut db).await?;
        let (theirs, others): (Vec<CalendarRow>, Vec<CalendarRow>) =
            rows.into_iter().partition(|row| row.account == email);
        let orphans: Vec<String> = theirs
            .into_iter()
            .map(|row| row.id)
            .filter(|id| !others.iter().any(|row| &row.id == id))
            .collect();
        let mut write = self.inner.begin(&mut db).await?;
        CalendarRow::filter(CalendarRow::fields().account().eq(email))
            .delete()
            .exec(write.exec())
            .await?;
        for ids in orphans.chunks(500) {
            EventRow::filter(EventRow::fields().calendar_id().in_list(ids))
                .delete()
                .exec(write.exec())
                .await?;
        }
        write.publish().await
    }

    /// Whether `account`'s calendars are read now: it's the mail account, or a calendar account
    /// signed in.
    fn reads(&self, account: &str) -> bool {
        account == self.inner.account
            || self
                .inner
                .calendar_clients
                .read()
                .expect("calendar clients are never poisoned")
                .contains_key(account)
    }

    /// `account`'s client: the mail account's, or a calendar account's signed in (the mail
    /// account's for one that isn't, whose calls Calendar then refuses as it would anyone's).
    fn client(&self, account: &str) -> Client {
        let clients = self
            .inner
            .calendar_clients
            .read()
            .expect("calendar clients are never poisoned");
        clients
            .get(account)
            .cloned()
            .unwrap_or_else(|| self.inner.api.client().clone())
    }

    /// The client `calendar` is read through ([`Store::calendars`]' rule): the mail account's
    /// for a calendar on no list (someone's address).
    ///
    /// # Errors
    /// [`crate::Error::Unauthorized`] for a calendar only signed-out accounts list (or ones not
    /// added yet this session, at launch), whose changes wait for them.
    async fn client_for(&self, calendar: &str) -> Result<Client> {
        let mut db = self.inner.db.clone();
        let rows = CalendarRow::filter(CalendarRow::fields().id().eq(calendar))
            .exec(&mut db)
            .await?;
        let listed = !rows.is_empty();
        match held(rows, &self.inner.account, |account| self.reads(account))
            .into_iter()
            .next()
        {
            Some(calendar) => Ok(self.client(&calendar.account)),
            None if listed => Err(crate::Error::Unauthorized(gmail::Error::Token(
                "the calendar's account is signed out".into(),
            ))),
            None => Ok(self.inner.api.client().clone()),
        }
    }

    /// The accounts whose calendars are read now: the mail account, then its calendar accounts
    /// signed in, by address.
    #[must_use]
    pub fn calendar_accounts(&self) -> Vec<String> {
        let clients = self
            .inner
            .calendar_clients
            .read()
            .expect("calendar clients are never poisoned");
        let mut others: Vec<String> = clients.keys().cloned().collect();
        others.sort();
        std::iter::once(self.inner.account.clone())
            .chain(others)
            .collect()
    }

    /// Fetches `calendar`'s events on `days` (in `tz`) and stores them in place of what the
    /// cache had for those days.
    ///
    /// # Errors
    /// When Calendar can't be reached or refuses, or on database failures.
    pub async fn sync_events(
        &self,
        calendar: &str,
        days: Range<Date>,
        tz: &TimeZone,
    ) -> Result<()> {
        let events = self
            .fetch_events(calendar, midnight(days.start, tz), midnight(days.end, tz))
            .await?;
        let rows: Vec<(String, (i64, i64, bool), Event)> = events
            .into_iter()
            .filter(|event| event.status.as_deref() != Some("cancelled"))
            .filter_map(|event| Some((event.id.clone()?, span(&event)?, event)))
            .collect();

        let mut db = self.inner.db.clone();
        let mut write = self.inner.begin(&mut db).await?;
        let mut stale: HashSet<String> =
            overlapping(write.exec(), &[calendar.to_owned()], &days, tz)
                .await?
                .into_iter()
                .map(|row| row.id)
                .collect();
        stale.extend(rows.iter().map(|(id, ..)| id.clone()));
        let stale: Vec<String> = stale.into_iter().collect();
        for ids in stale.chunks(500) {
            EventRow::filter(
                EventRow::fields()
                    .calendar_id()
                    .eq(calendar)
                    .and(EventRow::fields().id().in_list(ids)),
            )
            .delete()
            .exec(write.exec())
            .await?;
        }
        if !rows.is_empty() {
            let mut create = EventRow::create_many();
            for (id, (start, end, all_day), event) in rows {
                create = create.with_item(|row| {
                    row.calendar_id(calendar)
                        .id(id)
                        .start(start)
                        .end(end)
                        .all_day(all_day)
                        .event(Json(event))
                });
            }
            create.exec(write.exec()).await?;
        }
        write.publish().await
    }

    /// `calendar`'s events (instances of recurring ones) overlapping `from` to `to`, as Calendar
    /// lists them now, without touching the cache: what Out of Office Reply looks through for
    /// out-of-office events coming up.
    ///
    /// # Errors
    /// When Calendar can't be reached or refuses.
    pub async fn upcoming_events(
        &self,
        calendar: &str,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<Event>> {
        self.fetch_events(calendar, from, to).await
    }

    /// Lists `calendar`'s events overlapping `from` to `to`, every page.
    async fn fetch_events(
        &self,
        calendar: &str,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<Event>> {
        let mut events = Vec::new();
        let mut query = EventsQuery {
            time_min: Some(from.to_string()),
            time_max: Some(to.to_string()),
            single_events: true,
            max_results: Some(PAGE),
            event_types: EVENT_TYPES.map(str::to_owned).to_vec(),
            ..EventsQuery::default()
        };
        let client = self.client_for(calendar).await?;
        loop {
            let page = self
                .inner
                .api
                .request_with(&client, ATTEMPTS, |client| {
                    client.calendar().events(calendar, &query)
                })
                .await?;
            events.extend(page.items);
            match page.next_page_token {
                Some(token) => query.page_token = Some(token),
                None => break,
            }
        }
        Ok(events)
    }

    /// Writes `event` (which has an id) over the cache's copy on `calendar`: what Calendar
    /// answered.
    async fn put_event(&self, calendar: &str, event: &Event) -> Result<()> {
        let mut db = self.inner.db.clone();
        let mut write = self.inner.begin(&mut db).await?;
        put(write.exec(), calendar, event).await?;
        write.publish().await
    }

    /// Sets my answer to event `id` on `calendar` (whose id is my address) and caches what
    /// Calendar answers: a patch of my attendee entry alone, and updates sent.
    async fn respond(&self, calendar: &str, id: &str, status: &str) -> Result<Event> {
        let patch = Event {
            attendees_omitted: Some(true),
            attendees: vec![EventAttendee {
                email: Some(calendar.to_owned()),
                response_status: Some(status.to_owned()),
                ..EventAttendee::default()
            }],
            ..Event::default()
        };
        let client = self.client_for(calendar).await?;
        let event = self
            .inner
            .api
            .request_with(&client, ATTEMPTS, |client| {
                client
                    .calendar()
                    .patch_event(calendar, id, &patch, Some("all"))
            })
            .await?;
        if event.recurrence.is_empty() {
            self.put_event(calendar, &event).await?;
        }
        Ok(event)
    }

    /// The event on `calendar` with iCalendar UID `uid`, if Calendar has one (cancelled ones
    /// too), which is how an invite is found.
    async fn event_by_uid(&self, calendar: &str, uid: &str) -> Result<Option<Event>> {
        let query = EventsQuery {
            ical_uid: Some(uid.to_owned()),
            show_deleted: true,
            ..EventsQuery::default()
        };
        let client = self.client_for(calendar).await?;
        let page = self
            .inner
            .api
            .request_with(&client, ATTEMPTS, |client| {
                client.calendar().events(calendar, &query)
            })
            .await?;
        Ok(page.items.into_iter().next())
    }

    /// Adds a private copy of an invite's event to `calendar` (`events.import`).
    async fn import_event(&self, calendar: &str, event: &Event) -> Result<Event> {
        let client = self.client_for(calendar).await?;
        Ok(self
            .inner
            .api
            .request_with(&client, ATTEMPTS, |client| {
                client.calendar().import_event(calendar, event)
            })
            .await?)
    }

    /// When each of `calendars` (anyone's address, for their primary calendar) is busy between
    /// `from` and `to` (`freebusy.query`): their busy spans, or `None` for a calendar Calendar
    /// won't say about.
    ///
    /// # Errors
    /// When Calendar can't be reached or refuses.
    pub async fn free_busy(
        &self,
        calendars: &[String],
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<(String, Option<Vec<(Timestamp, Timestamp)>>)>> {
        // One question per account the calendars are read through.
        let mut db = self.inner.db.clone();
        let rows = CalendarRow::all().exec(&mut db).await?;
        let holders: BTreeMap<String, String> =
            held(rows, &self.inner.account, |account| self.reads(account))
                .into_iter()
                .filter_map(|calendar| Some((calendar.entry.id?, calendar.account)))
                .collect();
        let mut asked: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for id in calendars {
            asked
                .entry(
                    holders
                        .get(id)
                        .map_or(self.inner.account.as_str(), String::as_str),
                )
                .or_default()
                .push(id.clone());
        }
        let mut busy = BTreeMap::new();
        for (account, ids) in asked {
            let request = gmail::calendar::FreeBusyRequest {
                time_min: Some(from.to_string()),
                time_max: Some(to.to_string()),
                items: ids
                    .iter()
                    .map(|id| gmail::calendar::FreeBusyRequestItem { id: id.clone() })
                    .collect(),
                ..gmail::calendar::FreeBusyRequest::default()
            };
            let client = self.client(account);
            let response = self
                .inner
                .api
                .request_with(&client, ATTEMPTS, |client| {
                    client.calendar().free_busy(&request)
                })
                .await?;
            for id in ids {
                let spans = response
                    .calendars
                    .get(&id)
                    .filter(|calendar| calendar.errors.is_empty())
                    .map(|calendar| {
                        calendar
                            .busy
                            .iter()
                            .filter_map(|period| {
                                Some((period.start.parse().ok()?, period.end.parse().ok()?))
                            })
                            .collect()
                    });
                busy.insert(id, spans);
            }
        }
        Ok(calendars
            .iter()
            .map(|id| (id.clone(), busy.remove(id).flatten()))
            .collect())
    }

    /// Person `email`'s events from `from` to `to` as their calendar shows me
    /// (`events.list` on their address, a page of up to 500 instances), through the calendar
    /// account that read it last, else through every account whose calendars are read, at once:
    /// the first answer with a public event, else the first answer. When Calendar won't list it
    /// through any, their busy times as events without details (`freebusy.query`); `None` when
    /// Calendar says neither.
    ///
    /// # Errors
    /// When Calendar can't be reached or refuses otherwise.
    pub async fn attendee_events(
        &self,
        email: &str,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Option<Vec<Event>>> {
        let query = EventsQuery {
            time_min: Some(from.to_string()),
            time_max: Some(to.to_string()),
            single_events: true,
            max_results: Some(ATTENDEE_PAGE),
            ..EventsQuery::default()
        };
        let list = |account: String| {
            let query = &query;
            async move {
                let client = self.client(&account);
                let listed = self
                    .inner
                    .api
                    .request_with(&client, ATTEMPTS, |client| {
                        client.calendar().events(email, query)
                    })
                    .await;
                (account, listed)
            }
        };
        let accounts = self.calendar_accounts();
        let previous = self
            .inner
            .attendee_accounts
            .read()
            .expect("attendee accounts are never poisoned")
            .get(email)
            .cloned();
        let mut answers = Vec::new();
        if let Some(account) = previous.filter(|account| accounts.contains(account)) {
            answers.push(list(account).await);
        }
        if !answers.iter().any(|(_, listed)| listed.is_ok()) {
            answers = futures::future::join_all(accounts.into_iter().map(list)).await;
        }
        let public = |events: &gmail::calendar::Events| {
            events
                .items
                .iter()
                .any(|event| event.visibility.as_deref() == Some("public"))
        };
        let found = answers
            .iter()
            .position(|(_, listed)| listed.as_ref().is_ok_and(public))
            .or_else(|| answers.iter().position(|(_, listed)| listed.is_ok()));
        let found = found.map(|index| answers.swap_remove(index));
        {
            let mut remembered = self
                .inner
                .attendee_accounts
                .write()
                .expect("attendee accounts are never poisoned");
            match &found {
                Some((account, _)) => remembered.insert(email.to_owned(), account.clone()),
                None => remembered.remove(email),
            };
        }
        if let Some((_, listed)) = found {
            return Ok(listed.ok().map(|page| page.items));
        }
        if let Some((_, Err(failure))) = answers.into_iter().find(|(_, listed)| !matches!(listed, Err(Failure::Permanent(error)) if matches!(error.status(), Some(403 | 404)))) {
            return Err(failure.into());
        }
        let at = |time: Timestamp| {
            Some(EventDateTime {
                date_time: Some(time.to_string()),
                ..EventDateTime::default()
            })
        };
        let busy = self
            .free_busy(&[email.to_owned()], from, to)
            .await?
            .into_iter()
            .next()
            .and_then(|(_, spans)| spans);
        Ok(busy.map(|spans| {
            spans
                .into_iter()
                .map(|(start, end)| Event {
                    start: at(start),
                    end: at(end),
                    ..Event::default()
                })
                .collect()
        }))
    }

    /// Series `id`'s `recurrence` lines on `calendar`, as Calendar has them now: what an
    /// instance's details describe.
    ///
    /// # Errors
    /// When Calendar can't be reached or refuses.
    pub async fn series_rules(&self, calendar: &str, id: &str) -> Result<Vec<String>> {
        Ok(self.fetch_event(calendar, id).await?.recurrence)
    }

    /// Event `id` on `calendar` as Calendar has it now (`events.get`): a series' master, or an
    /// instance.
    async fn fetch_event(&self, calendar: &str, id: &str) -> Result<Event> {
        let client = self.client_for(calendar).await?;
        Ok(self
            .inner
            .api
            .request_with(&client, ATTEMPTS, |client| {
                client.calendar().event(calendar, id)
            })
            .await?)
    }

    /// Adds `event` to `calendar` (`events.insert`, invitations sent to its guests when
    /// `notify`) and caches what Calendar answers. Its id is Exo's (random): taken means a retry whose earlier
    /// attempt Calendar made, only its answer lost, so the event as Calendar has it comes back.
    async fn insert_event(&self, calendar: &str, event: &Event, notify: bool) -> Result<Event> {
        let updates = if notify { "all" } else { "none" };
        let client = self.client_for(calendar).await?;
        let event = match self
            .inner
            .api
            .request_with(&client, ATTEMPTS, |client| {
                client
                    .calendar()
                    .insert_event(calendar, event, Some(updates))
            })
            .await
        {
            Ok(event) => event,
            Err(failure)
                if has_status(&failure, 409)
                    && let Some(id) = &event.id =>
            {
                self.inner
                    .api
                    .request_with(&client, ATTEMPTS, |client| {
                        client.calendar().event(calendar, id)
                    })
                    .await?
            }
            Err(failure) => return Err(failure.into()),
        };
        if event.recurrence.is_empty() {
            self.put_event(calendar, &event).await?;
        }
        Ok(event)
    }

    /// Changes the fields `patch` sets on event `id` of `calendar` (`events.patch`), telling the
    /// guests when `notify`, and caches what Calendar answers. A series' instances come back with
    /// the next fetch.
    async fn patch_event(
        &self,
        calendar: &str,
        id: &str,
        patch: &Event,
        notify: bool,
    ) -> Result<Event> {
        let updates = if notify { "all" } else { "none" };
        let client = self.client_for(calendar).await?;
        let event = self
            .inner
            .api
            .request_with(&client, ATTEMPTS, |client| {
                client
                    .calendar()
                    .patch_event(calendar, id, patch, Some(updates))
            })
            .await?;
        if event.recurrence.is_empty() {
            self.put_event(calendar, &event).await?;
        }
        Ok(event)
    }

    /// Deletes event `id` of `calendar` (`events.delete`), sending the guests a cancellation
    /// when `notify`, and drops it from the cache: the instance, or every instance of a series.
    async fn delete_event(&self, calendar: &str, id: &str, notify: bool) -> Result<()> {
        let updates = if notify { "all" } else { "none" };
        let client = self.client_for(calendar).await?;
        match self
            .inner
            .api
            .request_with(&client, ATTEMPTS, |client| {
                client.calendar().delete_event(calendar, id, Some(updates))
            })
            .await
        {
            // Already deleted: by an attempt whose answer was lost, or elsewhere.
            Err(failure) if has_status(&failure, 410) => {}
            result => result?,
        }
        let mut db = self.inner.db.clone();
        let rows = EventRow::filter(EventRow::fields().calendar_id().eq(calendar))
            .exec(&mut db)
            .await?;
        let gone: Vec<String> = rows
            .into_iter()
            .filter(|row| row.id == id || row.event.0.recurring_event_id.as_deref() == Some(id))
            .map(|row| row.id)
            .collect();
        let mut write = self.inner.begin(&mut db).await?;
        for ids in gone.chunks(500) {
            EventRow::filter(
                EventRow::fields()
                    .calendar_id()
                    .eq(calendar)
                    .and(EventRow::fields().id().in_list(ids)),
            )
            .delete()
            .exec(write.exec())
            .await?;
        }
        write.publish().await
    }

    /// The stored events of `calendars` on `days` (in `tz`), with the changes still queued for
    /// Calendar made to them, ordered by start.
    ///
    /// # Errors
    /// On database failures.
    pub async fn events(
        &self,
        calendars: &[String],
        days: Range<Date>,
        tz: &TimeZone,
    ) -> Result<Vec<StoredEvent>> {
        if calendars.is_empty() {
            return Ok(Vec::new());
        }
        self.inner.read_calendar(&days, tz);
        let mut db = self.inner.db.clone();
        let rows = overlapping(&mut db, calendars, &days, tz).await?;
        let mut events: Vec<StoredEvent> = rows
            .into_iter()
            .map(|row| StoredEvent {
                calendar_id: row.calendar_id,
                event: row.event.0,
                unsynced: false,
            })
            .collect();
        let shown = |calendar: &str, event: &Event| {
            calendars.iter().any(|id| id == calendar)
                && span(event).is_some_and(|span| overlaps(span, &days, tz))
        };
        let window = (midnight(days.start, tz), midnight(days.end, tz));
        for change in outbox::queued(&mut db).await? {
            change.apply(&mut events, &shown, Some(window));
        }
        events.sort_by_cached_key(|stored| {
            (
                span(&stored.event).map_or(i64::MAX, |(start, ..)| start),
                stored.calendar_id.clone(),
                stored.event.id.clone(),
            )
        });
        Ok(events)
    }
}

impl crate::store::Inner {
    /// Remembers that the calendar read `days` (in `tz`) this session, which eviction keeps the
    /// events of: the calendar shows days it fetched this session from the store.
    fn read_calendar(&self, days: &Range<Date>, tz: &TimeZone) {
        let span = (
            midnight(days.start, tz)
                .as_millisecond()
                .min(utc_midnight(days.start)),
            midnight(days.end, tz)
                .as_millisecond()
                .max(utc_midnight(days.end)),
        );
        let mut read = self
            .calendar_read
            .lock()
            .expect("calendar-read lock is never poisoned");
        if !read
            .iter()
            .any(|(from, to)| *from <= span.0 && span.1 <= *to)
        {
            read.push(span);
        }
    }
}

/// `rows`, the calendar accounts' lists, with each calendar once ([`Store::calendars`]' rule)
/// among the accounts read now (`reads`): the mail account's (`mail`) first, then the others'
/// by address, each in its list's order.
fn held(rows: Vec<CalendarRow>, mail: &str, reads: impl Fn(&str) -> bool) -> Vec<AccountCalendar> {
    let access = |entry: &CalendarListEntry| match entry.access_role.as_deref() {
        _ if entry.primary == Some(true) => 0,
        Some("owner") => 1,
        Some("writer") => 2,
        Some("reader") => 3,
        _ => 4,
    };
    let mut rows: Vec<CalendarRow> = rows.into_iter().filter(|row| reads(&row.account)).collect();
    rows.sort_by(|a, b| {
        (access(&a.entry.0), a.account != mail, &a.account).cmp(&(
            access(&b.entry.0),
            b.account != mail,
            &b.account,
        ))
    });
    let mut seen = HashSet::new();
    let mut kept: Vec<CalendarRow> = rows
        .into_iter()
        .filter(|row| seen.insert(row.id.clone()))
        .collect();
    kept.sort_by(|a, b| {
        (a.account != mail, &a.account, a.position).cmp(&(
            b.account != mail,
            &b.account,
            b.position,
        ))
    });
    kept.into_iter()
        .map(|row| AccountCalendar {
            account: row.account,
            entry: row.entry.0,
        })
        .collect()
}

/// Writes `event` (which has an id) over the cache's copy on `calendar`.
async fn put(exec: &mut dyn Executor, calendar: &str, event: &Event) -> Result<()> {
    let (Some(id), Some((start, end, all_day))) = (event.id.clone(), span(event)) else {
        return Ok(());
    };
    EventRow::filter(
        EventRow::fields()
            .calendar_id()
            .eq(calendar)
            .and(EventRow::fields().id().eq(&id)),
    )
    .delete()
    .exec(&mut *exec)
    .await?;
    EventRow::create()
        .calendar_id(calendar)
        .id(id)
        .start(start)
        .end(end)
        .all_day(all_day)
        .event(Json(event.clone()))
        .exec(exec)
        .await?;
    Ok(())
}

/// Rows of `calendars` overlapping `days`: timed events by instant in `tz`, all-day ones by
/// date.
async fn overlapping(
    exec: &mut dyn Executor,
    calendars: &[String],
    days: &Range<Date>,
    tz: &TimeZone,
) -> Result<Vec<EventRow>> {
    let fields = EventRow::fields();
    let (from, to) = (
        midnight(days.start, tz).as_millisecond(),
        midnight(days.end, tz).as_millisecond(),
    );
    let (first, last) = (utc_midnight(days.start), utc_midnight(days.end));
    let timed = fields
        .all_day()
        .eq(false)
        .and(fields.start().lt(to))
        .and(fields.end().gt(from));
    let all_day = fields
        .all_day()
        .eq(true)
        .and(fields.start().lt(last))
        .and(fields.end().gt(first));
    Ok(EventRow::filter(
        fields
            .calendar_id()
            .in_list(calendars)
            .and(timed.or(all_day)),
    )
    .exec(exec)
    .await?)
}

/// Whether an event spanning `span` ([`span`]) falls on `days` in `tz`, as [`overlapping`]
/// finds rows.
fn overlaps((start, end, all_day): (i64, i64, bool), days: &Range<Date>, tz: &TimeZone) -> bool {
    let (from, to) = if all_day {
        (utc_midnight(days.start), utc_midnight(days.end))
    } else {
        (
            midnight(days.start, tz).as_millisecond(),
            midnight(days.end, tz).as_millisecond(),
        )
    };
    start < to && end > from
}

/// An event's stored span: epoch milliseconds, and whether it's all-day.
fn span(event: &Event) -> Option<(i64, i64, bool)> {
    let (start, end) = (event.start.as_ref()?, event.end.as_ref()?);
    if let (Some(start), Some(end)) = (&start.date, &end.date) {
        return Some((
            utc_midnight(start.parse().ok()?),
            utc_midnight(end.parse().ok()?),
            true,
        ));
    }
    let start: Timestamp = start.date_time.as_deref()?.parse().ok()?;
    let end: Timestamp = end.date_time.as_deref()?.parse().ok()?;
    Some((start.as_millisecond(), end.as_millisecond(), false))
}

/// The start of `day` in `tz`.
fn midnight(day: Date, tz: &TimeZone) -> Timestamp {
    day.to_zoned(tz.clone()).map_or_else(
        |_| {
            day.to_zoned(TimeZone::UTC)
                .map(|zoned| zoned.timestamp())
                .unwrap_or_default()
        },
        |zoned| zoned.timestamp(),
    )
}

/// `day`'s UTC midnight, in milliseconds: how an all-day date is stored.
fn utc_midnight(day: Date) -> i64 {
    day.to_zoned(TimeZone::UTC)
        .map(|zoned| zoned.timestamp().as_millisecond())
        .unwrap_or_default()
}

/// Whether Calendar refused the call with HTTP `status`.
fn has_status(failure: &Failure, status: u16) -> bool {
    matches!(failure, Failure::Permanent(error) if error.status() == Some(status))
}
