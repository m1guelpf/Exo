//! The calendar's changes until Calendar has them: new events, edits, deletions and answers,
//! queued in the store, so they're made offline, outlast a quit and go when the connection is
//! back.
//!
//! [`Store::queue_calendar_change`] writes a row, and [`Store::events`] shows it at once: every
//! read lays the queued changes over Calendar's copy. A change can be held first, Undo's ten
//! seconds, and [`Store::unqueue_calendar_change`] takes back one that hasn't gone. The caller
//! sends the changes whose turn it is ([`Store::due_calendar_changes`],
//! [`Store::send_calendar_change`]): one queue per event and its series, so a change waits for
//! those before it to the same event and for no other. Being offline, signed out or rate limited
//! postpones a change; an answer retrying won't change gives it up, and the calendar shows
//! Calendar's copy again.

use std::collections::{HashMap, HashSet};

use gmail::calendar::{Event, EventDateTime};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use toasty::{Executor, Json};

use super::{StoredEvent, put};
use crate::db::{CalendarChangeRow, EventRow};
use crate::error::{Error, Result};
use crate::store::Store;

/// A change to the calendar, as queued for Calendar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CalendarChange {
    /// Adds `event`, whose id is chosen, to `calendar`, inviting its guests.
    Create { calendar: String, event: Event },
    /// Sets the fields `patch` has on event `id` of `calendar` (an instance, or a series),
    /// telling the guests when `notify`. For a series, `shift` moves its start and end by the
    /// instance's. `shown` is how the cached events it changes look meanwhile.
    Edit {
        calendar: String,
        id: String,
        patch: Event,
        notify: bool,
        shift: Option<(SignedDuration, SignedDuration)>,
        shown: Vec<Event>,
    },
    /// Deletes event `id` of `calendar` (an instance, or a series and all its instances).
    Delete {
        calendar: String,
        id: String,
        notify: bool,
    },
    /// Ends series `series` before its instance at `from`.
    Truncate {
        calendar: String,
        series: String,
        from: Cut,
        notify: bool,
    },
    /// Edits series `series` from its instance at `from` on: the series ends before it, and a new
    /// one (`patch`'s id) takes over with the series' rule and what `patch` sets, starting at
    /// `patch`'s start (else the instance's, `start` and `end`). From the series' first instance,
    /// the series is edited. `shown` is how the instances it changes look meanwhile.
    Split {
        calendar: String,
        series: String,
        from: Cut,
        patch: Event,
        start: EventDateTime,
        end: EventDateTime,
        notify: bool,
        shown: Vec<Event>,
    },
    /// Sets my answer (`accepted`, `tentative`, `declined`) to `event` on `calendar`, whose id is
    /// my address: to the instance, or to its whole series when `all`.
    Respond {
        calendar: String,
        event: Event,
        status: String,
        all: bool,
    },
    /// Answers an invite on `calendar` (its event as the `.ics` describes it), from conversation
    /// `thread`: the calendar's event with its UID, imported first when there's none.
    RespondToInvite {
        calendar: String,
        invite: Event,
        status: String,
        thread: Option<String>,
    },
}

/// Where "this and following" cuts a series: an all-day series at an instance's day, another
/// at its start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    /// The instance's day.
    Day(jiff::civil::Date),
    /// The instance's start.
    At(Timestamp),
}

/// A queued change whose turn it is, for [`Store::send_calendar_change`].
#[derive(Clone, Debug)]
pub struct QueuedCalendarChange {
    id: u64,
    /// What it was queued under.
    pub token: u64,
    pub change: CalendarChange,
}

/// How sending a queued change went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CalendarOutcome {
    /// Calendar has it, and the cache has Calendar's answer.
    Sent,
    /// Offline, signed out or rate limited: it waits, and goes again later.
    Postponed,
    /// Calendar refused it, with this error: it's given up and its overlay is gone. `gone` says
    /// the event it's for has been deleted or cancelled.
    Refused { error: String, gone: bool },
}

impl Cut {
    /// Whether an instance starting at `start` stays: it comes before the cut (or can't be read).
    fn keeps(self, start: &EventDateTime) -> bool {
        match self {
            Cut::Day(day) => start
                .date
                .as_deref()
                .and_then(|date| date.parse::<jiff::civil::Date>().ok())
                .is_none_or(|date| date < day),
            Cut::At(at) => start
                .timestamp(&TimeZone::UTC)
                .is_none_or(|start| start < at),
        }
    }

    /// The rule's `UNTIL`: the day before for an all-day series, whose `UNTIL` is a date (RFC
    /// 5545 §3.3.10 has it take the start's type; Calendar's own all-day example ends
    /// `UNTIL=20150628`), else the second before, in UTC.
    fn until(self) -> String {
        match self {
            Cut::Day(day) => day
                .yesterday()
                .unwrap_or(day)
                .strftime("%Y%m%d")
                .to_string(),
            Cut::At(at) => at
                .checked_sub(SignedDuration::from_secs(1))
                .unwrap_or(at)
                .strftime("%Y%m%dT%H%M%SZ")
                .to_string(),
        }
    }
}

impl CalendarChange {
    /// The calendar it changes.
    #[must_use]
    pub fn calendar(&self) -> &str {
        match self {
            Self::Create { calendar, .. }
            | Self::Edit { calendar, .. }
            | Self::Delete { calendar, .. }
            | Self::Truncate { calendar, .. }
            | Self::Split { calendar, .. }
            | Self::Respond { calendar, .. }
            | Self::RespondToInvite { calendar, .. } => calendar,
        }
    }

    /// Its queue: the event's, which an instance shares with its series (`{series}_{start}`).
    fn queue(&self) -> String {
        let id = match self {
            Self::Create { event, .. } => event.id.as_deref().unwrap_or_default(),
            Self::Edit { id, .. }
            | Self::Delete { id, .. }
            | Self::Truncate { series: id, .. }
            | Self::Split { series: id, .. } => id,
            Self::Respond { event, .. } => event.id.as_deref().unwrap_or_default(),
            Self::RespondToInvite { invite, .. } => {
                return format!(
                    "{}/{}",
                    self.calendar(),
                    invite.ical_uid.as_deref().unwrap_or_default()
                );
            }
        };
        format!("{}/{}", self.calendar(), id.split('_').next().unwrap_or(id))
    }

    /// `events` as they are once this is made. A new event, or one an edit moves, joins them
    /// when `shown` says it belongs; a new series, its instances starting in `window`.
    pub(crate) fn apply(
        &self,
        events: &mut Vec<StoredEvent>,
        shown: &dyn Fn(&str, &Event) -> bool,
        window: Option<(Timestamp, Timestamp)>,
    ) {
        let on = |stored: &StoredEvent, calendar: &str| stored.calendar_id == calendar;
        match self {
            Self::Create { calendar, event } if !event.recurrence.is_empty() => {
                if let Some((from, to)) = window {
                    for instance in super::recurrence::instances(event, from, to) {
                        upsert(events, calendar, &instance, true, shown);
                    }
                }
            }
            Self::Create { calendar, event } => upsert(events, calendar, event, true, shown),
            Self::Edit {
                calendar,
                shown: changed,
                ..
            } => {
                for event in changed {
                    upsert(events, calendar, event, false, shown);
                }
            }
            Self::Delete { calendar, id, .. } => events.retain(|stored| {
                !on(stored, calendar)
                    || (stored.event.id.as_deref() != Some(id)
                        && stored.event.recurring_event_id.as_deref() != Some(id))
            }),
            Self::Truncate {
                calendar,
                series,
                from,
                ..
            } => truncate(events, calendar, series, *from),
            Self::Split {
                calendar,
                series,
                from,
                shown: changed,
                ..
            } => {
                truncate(events, calendar, series, *from);
                for event in changed {
                    upsert(events, calendar, event, false, shown);
                }
            }
            Self::Respond {
                calendar,
                event,
                status,
                all,
            } => {
                let series = event.recurring_event_id.as_deref().filter(|_| *all);
                for stored in events.iter_mut().filter(|stored| on(stored, calendar)) {
                    if stored.event.id == event.id
                        || (series.is_some()
                            && stored.event.recurring_event_id.as_deref() == series)
                    {
                        answer(&mut stored.event, calendar, status);
                    }
                }
            }
            Self::RespondToInvite {
                calendar,
                invite,
                status,
                ..
            } => {
                for stored in events.iter_mut().filter(|stored| {
                    on(stored, calendar)
                        && stored.event.ical_uid.is_some()
                        && stored.event.ical_uid == invite.ical_uid
                }) {
                    answer(&mut stored.event, calendar, status);
                }
            }
        }
    }
}

/// `events` without series `series`' instances of `calendar` from `from` on.
fn truncate(events: &mut Vec<StoredEvent>, calendar: &str, series: &str, from: Cut) {
    events.retain(|stored| {
        stored.calendar_id != calendar
            || stored.event.recurring_event_id.as_deref() != Some(series)
            || stored
                .event
                .start
                .as_ref()
                .is_none_or(|start| from.keeps(start))
    });
}

/// `event` in place of the copy of it in `events`, or among them when `shown` says so.
fn upsert(
    events: &mut Vec<StoredEvent>,
    calendar: &str,
    event: &Event,
    unsynced: bool,
    shown: &dyn Fn(&str, &Event) -> bool,
) {
    match events
        .iter_mut()
        .find(|stored| stored.calendar_id == calendar && stored.event.id == event.id)
    {
        Some(stored) => stored.event = event.clone(),
        None if shown(calendar, event) => events.push(StoredEvent {
            calendar_id: calendar.to_owned(),
            event: event.clone(),
            unsynced,
        }),
        None => {}
    }
}

/// Sets my answer to `event`: the attendee that's me, or has my calendar's address.
fn answer(event: &mut Event, calendar: &str, status: &str) {
    for attendee in &mut event.attendees {
        if attendee.is_self == Some(true)
            || attendee
                .email
                .as_deref()
                .is_some_and(|email| email.eq_ignore_ascii_case(calendar))
        {
            attendee.response_status = Some(status.to_owned());
        }
    }
}

/// Every queued change, oldest first.
pub(crate) async fn queued(exec: &mut dyn Executor) -> Result<Vec<CalendarChange>> {
    let rows = CalendarChangeRow::all()
        .order_by(CalendarChangeRow::fields().id().asc())
        .exec(exec)
        .await?;
    Ok(rows.into_iter().map(|row| row.change.0).collect())
}

impl Store {
    /// Queues `change` under `token` for Calendar, shown by [`events`](Self::events) at once and
    /// sent once `hold` has passed. A deletion of a new event still queued takes it back
    /// instead, so Calendar and its guests never hear of it. Not while
    /// [`send_calendar_change`](Self::send_calendar_change) is sending that event's change.
    ///
    /// # Errors
    /// On database failures.
    pub async fn queue_calendar_change(
        &self,
        token: u64,
        change: CalendarChange,
        hold: Option<Timestamp>,
    ) -> Result<()> {
        let mut db = self.inner.db.clone();
        let mut write = self.inner.begin(&mut db).await?;
        let queue = change.queue();
        if !unqueued_new(write.exec(), &queue, &change).await? {
            CalendarChangeRow::create()
                .token(token)
                .queue(queue)
                .change(Json(change))
                .failures(0)
                .not_before(hold)
                .exec(write.exec())
                .await?;
        }
        write.publish().await
    }

    /// Takes back the change queued under `token`, unless it's gone, and for a new event what
    /// was queued for it since (its edits). Returns whether it was there.
    ///
    /// # Errors
    /// On database failures.
    pub async fn unqueue_calendar_change(&self, token: u64) -> Result<bool> {
        let mut db = self.inner.db.clone();
        let mut write = self.inner.begin(&mut db).await?;
        let rows = CalendarChangeRow::filter(CalendarChangeRow::fields().token().eq(token))
            .exec(write.exec())
            .await?;
        if rows.is_empty() {
            return Ok(false);
        }
        for row in rows
            .iter()
            .filter(|row| matches!(row.change.0, CalendarChange::Create { .. }))
        {
            let queue = CalendarChangeRow::fields().queue().eq(&row.queue);
            CalendarChangeRow::filter(queue.and(CalendarChangeRow::fields().id().gt(row.id)))
                .delete()
                .exec(write.exec())
                .await?;
        }
        CalendarChangeRow::filter(CalendarChangeRow::fields().token().eq(token))
            .delete()
            .exec(write.exec())
            .await?;
        write.publish().await?;
        Ok(true)
    }

    /// The queued changes whose turn it is, oldest first: each queue's first, unless it's held
    /// or backing off (whatever it waits for when `held_too`). Also when the next that waits is
    /// due.
    ///
    /// # Errors
    /// On database failures.
    pub async fn due_calendar_changes(
        &self,
        held_too: bool,
    ) -> Result<(Vec<QueuedCalendarChange>, Option<Timestamp>)> {
        let mut db = self.inner.db.clone();
        let rows = CalendarChangeRow::all()
            .order_by(CalendarChangeRow::fields().id().asc())
            .exec(&mut db)
            .await?;
        let now = Timestamp::now();
        let mut queues = HashSet::new();
        let (mut due, mut next) = (Vec::new(), None::<Timestamp>);
        for row in rows
            .into_iter()
            .filter(|row| queues.insert(row.queue.clone()))
        {
            match row.not_before.filter(|at| *at > now && !held_too) {
                Some(at) => next = Some(next.map_or(at, |next| next.min(at))),
                None => due.push(QueuedCalendarChange {
                    id: row.id,
                    token: row.token,
                    change: row.change.0,
                }),
            }
        }
        Ok((due, next))
    }

    /// Queued changes Calendar doesn't have yet.
    ///
    /// # Errors
    /// On database failures.
    pub async fn calendar_pending(&self) -> Result<usize> {
        let mut db = self.inner.db.clone();
        Ok(
            usize::try_from(CalendarChangeRow::all().count().exec(&mut db).await?)
                .unwrap_or(usize::MAX),
        )
    }

    /// Sends `queued` to Calendar, caching its answer, and drops it from the queue; or postpones
    /// it, or gives it up.
    ///
    /// # Errors
    /// On database failures.
    pub async fn send_calendar_change(
        &self,
        queued: &QueuedCalendarChange,
    ) -> Result<CalendarOutcome> {
        let failure = match self.make(&queued.change).await {
            Ok(()) => return self.dequeue(queued.id, CalendarOutcome::Sent).await,
            Err(failure) => failure,
        };
        let until = match failure {
            // Calendar's quota is its own: only its own pushback holds a change.
            Error::Throttled { until } => Some(until),
            Error::Offline(_) | Error::Unauthorized(_) => None,
            Error::Gmail(error) => {
                log::warn!("Calendar refused a change, giving it up: {error}");
                let gone = matches!(error.status(), Some(404 | 410));
                return self
                    .dequeue(
                        queued.id,
                        CalendarOutcome::Refused {
                            error: error.to_string(),
                            gone,
                        },
                    )
                    .await;
            }
            Error::EventCancelled => {
                log::warn!("giving up an answer to a cancelled event");
                return self
                    .dequeue(
                        queued.id,
                        CalendarOutcome::Refused {
                            error: Error::EventCancelled.to_string(),
                            gone: true,
                        },
                    )
                    .await;
            }
            error => return Err(error),
        };
        let mut db = self.inner.db.clone();
        let mut write = self.inner.begin(&mut db).await?;
        let failures = CalendarChangeRow::filter_by_id(queued.id)
            .first()
            .exec(write.exec())
            .await?
            .map_or(0, |row| row.failures)
            .saturating_add(1);
        let not_before = until.unwrap_or_else(|| self.inner.api.postpone(failures));
        CalendarChangeRow::update_by_id(queued.id)
            .failures(failures)
            .not_before(not_before)
            .exec(write.exec())
            .await?;
        write.commit().await?;
        Ok(CalendarOutcome::Postponed)
    }

    /// Drops change `id` from its queue, which ended in `outcome`.
    async fn dequeue(&self, id: u64, outcome: CalendarOutcome) -> Result<CalendarOutcome> {
        let mut db = self.inner.db.clone();
        let mut write = self.inner.begin(&mut db).await?;
        CalendarChangeRow::filter_by_id(id)
            .delete()
            .exec(write.exec())
            .await?;
        write.publish().await?;
        Ok(outcome)
    }

    /// Makes `change` at Calendar, and caches what it changed: Calendar's answer, or for a series,
    /// its instances as they were shown until the next fetch brings Calendar's.
    async fn make(&self, change: &CalendarChange) -> Result<()> {
        match change {
            CalendarChange::Create { calendar, event } => {
                self.insert_event(calendar, event, true).await.map(drop)
            }
            CalendarChange::Edit {
                calendar,
                id,
                patch,
                notify,
                shift,
                ..
            } => {
                let mut patch = patch.clone();
                if let Some(shift) = shift {
                    shifted(&mut patch, &self.fetch_event(calendar, id).await?, *shift);
                }
                let answer = self.patch_event(calendar, id, &patch, *notify).await?;
                if answer.recurrence.is_empty() {
                    Ok(())
                } else {
                    self.fold_into_cache(change).await
                }
            }
            CalendarChange::Delete {
                calendar,
                id,
                notify,
            } => self.delete_event(calendar, id, *notify).await,
            CalendarChange::Split {
                calendar,
                series,
                from,
                patch,
                start,
                end,
                notify,
                ..
            } => {
                let master = self.fetch_event(calendar, series).await?;
                let (start, end) = (
                    patch.start.clone().unwrap_or_else(|| start.clone()),
                    patch.end.clone().unwrap_or_else(|| end.clone()),
                );
                if master.start.as_ref().is_none_or(|first| !from.keeps(first)) {
                    // From the first instance, the series itself changes.
                    let patch = Event {
                        id: None,
                        start: Some(start),
                        end: Some(end),
                        ..patch.clone()
                    };
                    self.patch_event(calendar, series, &patch, *notify).await?;
                    return self.fold_into_cache(change).await;
                }
                let truncated = Event {
                    recurrence: ended(&master.recurrence, *from),
                    ..Event::default()
                };
                self.patch_event(calendar, series, &truncated, *notify)
                    .await?;
                self.insert_event(
                    calendar,
                    &remainder(&master, patch, start, end, *from),
                    *notify,
                )
                .await?;
                self.fold_into_cache(change).await
            }
            CalendarChange::Truncate {
                calendar,
                series,
                from,
                notify,
            } => {
                let master = self.fetch_event(calendar, series).await?;
                if master.start.as_ref().is_none_or(|first| !from.keeps(first)) {
                    return self.delete_event(calendar, series, *notify).await;
                }
                let patch = Event {
                    recurrence: ended(&master.recurrence, *from),
                    ..Event::default()
                };
                self.patch_event(calendar, series, &patch, *notify).await?;
                self.fold_into_cache(change).await
            }
            CalendarChange::Respond {
                calendar,
                event,
                status,
                all,
            } => {
                let series = event.recurring_event_id.as_deref().filter(|_| *all);
                self.respond(
                    calendar,
                    series.or(event.id.as_deref()).unwrap_or_default(),
                    status,
                )
                .await?;
                if series.is_some() {
                    self.fold_into_cache(change).await
                } else {
                    Ok(())
                }
            }
            CalendarChange::RespondToInvite {
                calendar,
                invite,
                status,
                ..
            } => {
                let uid = invite.ical_uid.clone().unwrap_or_default();
                let event = match self.event_by_uid(calendar, &uid).await? {
                    Some(event) if event.status.as_deref() == Some("cancelled") => {
                        return Err(Error::EventCancelled);
                    }
                    Some(event) => event,
                    None => self.import_event(calendar, invite).await?,
                };
                self.respond(
                    calendar,
                    event
                        .recurring_event_id
                        .as_deref()
                        .or(event.id.as_deref())
                        .unwrap_or_default(),
                    status,
                )
                .await?;
                self.fold_into_cache(change).await
            }
        }
    }

    /// Makes `change` to the cached events, as Calendar now has it.
    async fn fold_into_cache(&self, change: &CalendarChange) -> Result<()> {
        let calendar = change.calendar();
        let mut db = self.inner.db.clone();
        let rows = EventRow::filter(EventRow::fields().calendar_id().eq(calendar))
            .exec(&mut db)
            .await?;
        let before: HashMap<String, Event> =
            rows.into_iter().map(|row| (row.id, row.event.0)).collect();
        let mut after: Vec<StoredEvent> = before
            .values()
            .map(|event| StoredEvent {
                calendar_id: calendar.to_owned(),
                event: event.clone(),
                unsynced: false,
            })
            .collect();
        change.apply(&mut after, &|_, _| false, None);
        let kept: HashSet<&str> = after
            .iter()
            .filter_map(|stored| stored.event.id.as_deref())
            .collect();
        let gone: Vec<String> = before
            .keys()
            .filter(|id| !kept.contains(id.as_str()))
            .cloned()
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
        for stored in &after {
            if stored.event.id.as_ref().and_then(|id| before.get(id)) != Some(&stored.event) {
                put(write.exec(), calendar, &stored.event).await?;
            }
        }
        write.publish().await
    }
}

/// Takes back the new event `queue` still holds, with whatever was queued for it since, when
/// `change` deletes it. Returns whether it did.
async fn unqueued_new(
    exec: &mut dyn Executor,
    queue: &str,
    change: &CalendarChange,
) -> Result<bool> {
    let CalendarChange::Delete { calendar, id, .. } = change else {
        return Ok(false);
    };
    let rows = CalendarChangeRow::filter(CalendarChangeRow::fields().queue().eq(queue))
        .exec(&mut *exec)
        .await?;
    let created = rows.iter().any(|row| matches!(&row.change.0, CalendarChange::Create { calendar: on, event } if on == calendar && event.id.as_deref() == Some(id.as_str())));
    if created {
        CalendarChangeRow::filter(CalendarChangeRow::fields().queue().eq(queue))
            .delete()
            .exec(exec)
            .await?;
    }
    Ok(created)
}

/// `recurrence` with each rule ending before `from` (`UNTIL`, in place of a count or an
/// earlier end).
fn ended(recurrence: &[String], from: Cut) -> Vec<String> {
    let until = from.until();
    recurrence
        .iter()
        .map(|line| match line.strip_prefix("RRULE:") {
            Some(rule) => {
                let mut parts: Vec<String> = rule
                    .split(';')
                    .filter(|part| !part.starts_with("COUNT=") && !part.starts_with("UNTIL="))
                    .map(str::to_owned)
                    .collect();
                parts.push(format!("UNTIL={until}"));
                format!("RRULE:{}", parts.join(";"))
            }
            None => line.clone(),
        })
        .collect()
}

/// The new series that takes over series `master` from `from`: the master's fields with
/// `patch`'s, a new id (`patch`'s) and Calendar's own fields left to it, starting at `start`, its
/// rule counting only what's left.
fn remainder(
    master: &Event,
    patch: &Event,
    start: EventDateTime,
    end: EventDateTime,
    from: Cut,
) -> Event {
    let mut event = Event {
        id: patch.id.clone(),
        summary: patch.summary.clone().or_else(|| master.summary.clone()),
        description: patch
            .description
            .clone()
            .or_else(|| master.description.clone()),
        location: patch.location.clone().or_else(|| master.location.clone()),
        attendees: if patch.attendees.is_empty() {
            master.attendees.clone()
        } else {
            patch.attendees.clone()
        },
        conference_data: match &patch.conference_data {
            Some(serde_json::Value::Null) => None,
            Some(data) => Some(data.clone()),
            None => master.conference_data.clone(),
        },
        color_id: master.color_id.clone(),
        transparency: master.transparency.clone(),
        visibility: master.visibility.clone(),
        reminders: master.reminders.clone(),
        guests_can_invite_others: master.guests_can_invite_others,
        guests_can_modify: master.guests_can_modify,
        guests_can_see_other_guests: master.guests_can_see_other_guests,
        recurrence: master.recurrence.clone(),
        ..Event::default()
    };
    let all_day = start.date.is_some();
    let first = master
        .start
        .as_ref()
        .and_then(|time| super::recurrence::zoned(time, &TimeZone::UTC));
    let new_start = super::recurrence::zoned(&start, &TimeZone::UTC);
    if let (Some(rule), Some((first, _)), Some((new_start, _))) = (
        super::recurrence::Recurrence::from_rules(&master.recurrence),
        first,
        new_start,
    ) {
        let day = match from {
            Cut::Day(day) => day,
            Cut::At(at) => at.to_zoned(first.time_zone().clone()).date(),
        };
        let line = rule.from_day(&first, day).rule(&new_start, all_day);
        event.recurrence = master
            .recurrence
            .iter()
            .filter(|line| !line.starts_with("RRULE"))
            .cloned()
            .chain([line])
            .collect();
    }
    event.start = Some(start);
    event.end = Some(end);
    event
}

/// Sets `patch`'s start and end to series `master`'s, moved by `shift` (an instance's move,
/// applied to every instance), in the master's own time zone.
fn shifted(patch: &mut Event, master: &Event, (start, end): (SignedDuration, SignedDuration)) {
    let moved = |time: Option<&EventDateTime>, by: SignedDuration| -> Option<EventDateTime> {
        let time = time?;
        let at: Timestamp = time.date_time.as_deref()?.parse().ok()?;
        let zone = time
            .time_zone
            .as_deref()
            .and_then(|name| TimeZone::get(name).ok())
            .unwrap_or(TimeZone::UTC);
        let at = at.checked_add(by).ok()?.to_zoned(zone);
        Some(EventDateTime {
            date_time: Some(at.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string()),
            ..time.clone()
        })
    };
    patch.start = moved(master.start.as_ref(), start).or(patch.start.take());
    patch.end = moved(master.end.as_ref(), end).or(patch.end.take());
}

#[cfg(test)]
mod tests {
    use jiff::civil::Date;

    use super::*;

    fn at(text: &str) -> EventDateTime {
        EventDateTime {
            date_time: Some(text.into()),
            time_zone: Some("America/New_York".into()),
            ..EventDateTime::default()
        }
    }

    #[test]
    fn a_series_moves_by_its_instance() {
        let master = Event {
            start: Some(at("2026-11-03T11:00:00-05:00")),
            end: Some(at("2026-11-03T11:30:00-05:00")),
            ..Event::default()
        };
        let mut patch = Event::default();
        shifted(
            &mut patch,
            &master,
            (SignedDuration::from_mins(60), SignedDuration::from_mins(90)),
        );
        assert_eq!(
            patch.start.and_then(|start| start.date_time).as_deref(),
            Some("2026-11-03T12:00:00-05:00")
        );
        assert_eq!(
            patch.end.and_then(|end| end.date_time).as_deref(),
            Some("2026-11-03T13:00:00-05:00")
        );
    }

    #[test]
    fn a_series_ends_before_an_instance() {
        let from = Cut::At("2026-11-24T16:00:00Z".parse().expect("time"));
        let rules = ended(
            &[
                "RRULE:FREQ=WEEKLY;COUNT=10;BYDAY=TU".to_owned(),
                "EXDATE:20261110T160000Z".to_owned(),
            ],
            from,
        );
        assert_eq!(
            rules,
            [
                "RRULE:FREQ=WEEKLY;BYDAY=TU;UNTIL=20261124T155959Z",
                "EXDATE:20261110T160000Z"
            ]
        );
        assert_eq!(
            ended(
                &["RRULE:FREQ=WEEKLY".to_owned()],
                Cut::Day(Date::constant(2026, 11, 17))
            ),
            ["RRULE:FREQ=WEEKLY;UNTIL=20261116"],
            "an all-day series ends on a date"
        );
    }

    #[test]
    fn queued_changes_show_on_the_calendar() {
        let event = |id: &str, series: Option<&str>| StoredEvent {
            calendar_id: "me".into(),
            event: Event {
                id: Some(id.into()),
                recurring_event_id: series.map(Into::into),
                ..Event::default()
            },
            unsynced: false,
        };
        let everywhere = |_: &str, _: &Event| true;
        let mut events = vec![
            event("a", None),
            event("s_1", Some("s")),
            event("s_2", Some("s")),
        ];
        CalendarChange::Delete {
            calendar: "me".into(),
            id: "s".into(),
            notify: false,
        }
        .apply(&mut events, &everywhere, None);
        assert_eq!(events.len(), 1, "a series takes its instances");
        let renamed = Event {
            id: Some("a".into()),
            summary: Some("Renamed".into()),
            ..Event::default()
        };
        CalendarChange::Edit {
            calendar: "me".into(),
            id: "a".into(),
            patch: Event::default(),
            notify: false,
            shift: None,
            shown: vec![renamed.clone()],
        }
        .apply(&mut events, &everywhere, None);
        assert_eq!(events[0].event, renamed);
        let new = CalendarChange::Create {
            calendar: "me".into(),
            event: Event {
                id: Some("b".into()),
                ..Event::default()
            },
        };
        new.apply(&mut events, &|_, _| false, None);
        assert_eq!(events.len(), 1, "a new event off the days shown isn't");
        new.apply(&mut events, &everywhere, None);
        assert_eq!(
            (events.len(), events[1].unsynced),
            (2, true),
            "a new event on them is, and Calendar hasn't got it"
        );
    }

    #[test]
    fn an_instance_shares_its_series_queue() {
        let respond = |id: &str| CalendarChange::Respond {
            calendar: "me".into(),
            event: Event {
                id: Some(id.into()),
                ..Event::default()
            },
            status: "accepted".into(),
            all: false,
        };
        let delete = CalendarChange::Delete {
            calendar: "me".into(),
            id: "s".into(),
            notify: false,
        };
        assert_eq!(respond("s_20261117T150000Z").queue(), delete.queue());
        assert_ne!(respond("t").queue(), delete.queue());
    }
}
