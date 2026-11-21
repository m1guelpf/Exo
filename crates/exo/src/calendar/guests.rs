//! Guests' calendars: each guest's events as their calendar shows me (read by [`super::sync`]),
//! the colour each takes, what their chip says, and the columns a day splits into while they're
//! overlaid.

use std::collections::BTreeMap;

use exo_store::StoredEvent;
use exo_ui::CalendarColor;
use gmail::calendar::Event as ApiEvent;
use jiff::{Timestamp, ToSpan, Zoned, civil::Date, tz::TimeZone};

use super::{
    event::{Event, Rsvp},
    sync::{Data, GuestEvents},
};

/// The most guests' calendars overlaid.
pub const MAX: usize = 7;
/// The colours guests take in turn, those of my shown calendars last.
const COLORS: [&str; 10] = [
    "blue", "pink", "green", "orange", "cobalt", "ochre", "red", "lime", "yellow", "magenta",
];

/// One of my calendars, as guests' colours avoid them: its id, colour and whether it shows.
pub type Mine = (String, CalendarColor, bool);

/// A surface's guests' colours, in the order they were given, and the calendar a hover
/// expanded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Colors {
    assigned: Vec<(String, CalendarColor)>,
    pub expanded: Option<String>,
}

impl Colors {
    /// `email`'s colour, once their calendar has loaded.
    pub fn of(&self, email: &str) -> Option<CalendarColor> {
        self.assigned
            .iter()
            .find(|(assigned, _)| assigned.eq_ignore_ascii_case(email))
            .map(|(_, color)| *color)
    }

    /// Gives each of `guests` whose calendar has loaded a colour: one of my calendars keeps its
    /// own; anyone else takes the colour after the last one given, my shown calendars' colours
    /// coming last.
    pub fn assign(&mut self, guests: &[String], data: &Data, mine: &[Mine]) {
        for email in guests {
            let loaded = data
                .guests
                .get(email)
                .is_some_and(|calendar| matches!(calendar.events, GuestEvents::Shown(_)));
            if !loaded || self.of(email).is_some() {
                continue;
            }
            let color = match mine.iter().find(|(id, ..)| id.eq_ignore_ascii_case(email)) {
                Some((_, color, _)) => *color,
                None => {
                    let shown: Vec<CalendarColor> = mine
                        .iter()
                        .filter(|(.., shown)| *shown)
                        .map(|(_, color, _)| *color)
                        .collect();
                    let ranked: Vec<CalendarColor> = COLORS
                        .iter()
                        .filter_map(|name| CalendarColor::from_name(name))
                        .filter(|color| !shown.contains(color))
                        .chain(
                            COLORS
                                .iter()
                                .filter_map(|name| CalendarColor::from_name(name))
                                .filter(|color| shown.contains(color)),
                        )
                        .collect();
                    let last = self
                        .assigned
                        .iter()
                        .rev()
                        .find(|(email, _)| {
                            !mine.iter().any(|(id, ..)| id.eq_ignore_ascii_case(email))
                        })
                        .map(|(_, color)| *color);
                    let next = last
                        .and_then(|last| ranked.iter().position(|color| *color == last))
                        .map_or(0, |at| (at + 1) % ranked.len());
                    ranked[next]
                }
            };
            self.assigned.push((email.clone(), color));
        }
    }

    /// The first [`MAX`] of `guests` whose calendars show, with their colours (offline, none).
    pub fn shown(
        &self,
        guests: &[String],
        data: &Data,
        online: bool,
    ) -> Vec<(String, CalendarColor)> {
        if !online {
            return Vec::new();
        }
        guests
            .iter()
            .filter(|email| {
                data.guests
                    .get(*email)
                    .is_some_and(|calendar| matches!(calendar.events, GuestEvents::Shown(_)))
            })
            .filter_map(|email| Some((email.clone(), self.of(email)?)))
            .take(MAX)
            .collect()
    }
}

/// What a guest's chip shows before their name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    /// Their calendar's colour.
    Swatch(CalendarColor),
    /// Their calendar is loading (`Loading availability`).
    Spinner,
    /// Their calendar isn't shown: an empty swatch.
    Empty,
}

/// The marker for `email`: their colour while their calendar is overlaid, a spinner while it
/// loads, else an empty swatch.
pub fn marker(email: &str, shown: &[(String, CalendarColor)], data: &Data, online: bool) -> Marker {
    if let Some((_, color)) = shown
        .iter()
        .find(|(shown, _)| shown.eq_ignore_ascii_case(email))
    {
        return Marker::Swatch(*color);
    }
    if online
        && data
            .guests
            .get(email)
            .is_some_and(|calendar| calendar.loading)
    {
        Marker::Spinner
    } else {
        Marker::Empty
    }
}

/// A guest's marker in `rect`: a swatch in their calendar's colour, an empty swatch's 1px
/// outline at `.3`, or Classic's spinner at `.25`.
pub fn paint_marker(ui: &egui::Ui, rect: egui::Rect, marker: Marker) {
    let palette = exo_ui::palette(ui.ctx());
    let theme = exo_ui::theme(ui.ctx());
    let rect = exo_ui::pixel_snap(rect);
    match marker {
        Marker::Swatch(color) => {
            ui.painter()
                .rect_filled(rect, egui::CornerRadius::same(2), color.guest_swatch(theme));
        }
        Marker::Empty => {
            ui.painter().rect_stroke(
                rect,
                egui::CornerRadius::same(2),
                egui::Stroke::new(1.0, palette.ink(0.3, 0.3)),
                egui::StrokeKind::Inside,
            );
        }
        Marker::Spinner => exo_ui::paint_spinner(
            ui.painter(),
            rect,
            ui.input(|input| input.time),
            palette.ink(0.25, 0.25),
        ),
    }
}

/// `(busy)` when one of `email`'s events overlaps `start`–`end` on the start's day, else `(OOO)`
/// when an out-of-office one does; nothing for free time, an unanswered invite, or offline.
pub fn busy_status(
    email: &str,
    data: &Data,
    (start, end): (&Zoned, &Zoned),
    online: bool,
) -> Option<&'static str> {
    if !online {
        return None;
    }
    let events = events_of(data, email)?;
    let day = start.date();
    let mut statuses = events.iter().filter_map(|event| {
        status(
            event,
            email,
            day,
            start.timestamp(),
            end.timestamp(),
            start.time_zone(),
        )
    });
    let (busy, ooo) = statuses
        .by_ref()
        .fold((false, false), |(busy, ooo), status| {
            (busy || status == "busy", ooo || status == "OOO")
        });
    if busy {
        Some("(busy)")
    } else {
        ooo.then_some("(OOO)")
    }
}

/// The busy status one of a guest's events on `day` gives them.
fn status(
    event: &ApiEvent,
    email: &str,
    day: Date,
    start: Timestamp,
    end: Timestamp,
    tz: &TimeZone,
) -> Option<&'static str> {
    let (from, to, _) = span(event, tz)?;
    let on_day = day
        .to_zoned(tz.clone())
        .ok()
        .zip(
            day.tomorrow()
                .ok()
                .and_then(|next| next.to_zoned(tz.clone()).ok()),
        )
        .is_some_and(|(midnight, next)| from < next.timestamp() && to > midnight.timestamp());
    if !on_day
        || event.transparency.as_deref() == Some("transparent")
        || super::event::my_attendance(event, email) == Some(Rsvp::NeedsAction)
    {
        return None;
    }
    (from < end && to > start).then_some(if event.event_type.as_deref() == Some("outOfOffice") {
        "OOO"
    } else {
        "busy"
    })
}

/// A guest's events as listed, without those they declined or that leave them free.
fn events_of<'a>(data: &'a Data, email: &str) -> Option<Vec<&'a ApiEvent>> {
    let GuestEvents::Shown(events) = &data.guests.get(email)?.events else {
        return None;
    };
    Some(
        events
            .iter()
            .filter(|event| {
                super::event::my_attendance(event, email) != Some(Rsvp::Declined)
                    && (is_all_day(event) || event.transparency.as_deref() != Some("transparent"))
            })
            .collect(),
    )
}

fn is_all_day(event: &ApiEvent) -> bool {
    event
        .start
        .as_ref()
        .is_some_and(|start| start.date.is_some())
}

/// An event's start and end, and whether it's all-day.
fn span(event: &ApiEvent, tz: &TimeZone) -> Option<(Timestamp, Timestamp, bool)> {
    let at = |value: &gmail::calendar::EventDateTime| match &value.date {
        Some(date) => date
            .parse::<Date>()
            .ok()?
            .to_zoned(tz.clone())
            .ok()
            .map(|at| at.timestamp()),
        None => value.date_time.as_deref()?.parse().ok(),
    };
    Some((
        at(event.start.as_ref()?)?,
        at(event.end.as_ref()?)?,
        is_all_day(event),
    ))
}

/// A calendar as a column of a day: mine or a guest's, its events laid out in it, collapsed to
/// bare bars unless it's the expanded one.
#[derive(Clone, Debug)]
pub struct Column {
    /// Its calendar: a guest's address, or `None` for mine.
    pub guest: Option<String>,
    pub collapsed: bool,
    pub events: Vec<Event>,
}

/// The guests' columns over `days`, each of `shown`'s events as a block in their colour, all-day
/// ones that keep them busy as the whole of each day; those that leave them free stay all-day,
/// for the all-day row.
pub fn guest_events(
    data: &Data,
    shown: &[(String, CalendarColor)],
    days: &[Date],
    tz: &TimeZone,
) -> Vec<(String, Vec<Event>)> {
    shown
        .iter()
        .map(|(email, color)| {
            let colors = BTreeMap::from([(email.clone(), *color)]);
            let mut blocks = Vec::new();
            for event in events_of(data, email).unwrap_or_default() {
                let stored = StoredEvent {
                    calendar_id: email.clone(),
                    event: event.clone(),
                    unsynced: false,
                };
                let Some(mut block) = Event::new(&stored, None, &colors, email, tz) else {
                    continue;
                };
                // `Busy` for an attendee's event without details.
                if event.summary.as_deref().is_none_or(str::is_empty) {
                    block.title = "Busy".to_owned();
                }
                if !block.all_day {
                    blocks.push(block);
                    continue;
                }
                // An all-day event that keeps them busy fills each of its days, to 23:59:59.
                if event.event_type.as_deref() != Some("outOfOffice")
                    && event.transparency.as_deref() == Some("transparent")
                {
                    blocks.push(block);
                    continue;
                }
                for day in days.iter().filter(|day| block.on(**day)) {
                    let Ok(midnight) = day.to_zoned(tz.clone()) else {
                        continue;
                    };
                    let start = if block.start.date() == *day {
                        block.start.clone()
                    } else {
                        midnight.clone()
                    };
                    let Ok(end) = midnight.checked_add(86_399.seconds()) else {
                        continue;
                    };
                    let end = if block.end.date() == *day && block.end < end {
                        block.end.clone()
                    } else {
                        end
                    };
                    blocks.push(Event {
                        key: format!("{}_{day}", block.key),
                        start,
                        end,
                        all_day: false,
                        ..block.clone()
                    });
                }
            }
            (email.clone(), blocks)
        })
        .collect()
}

/// The bars for one calendar's timed `events` on the day from `midnight`: each run of
/// overlapping answered events as one bar, then each run of unanswered ones in what's left.
pub fn merged(events: &[&Event], midnight: &Zoned) -> Vec<Event> {
    let Ok(end_of_day) = midnight.checked_add(86_399.seconds()) else {
        return Vec::new();
    };
    let mut gaps = vec![(midnight.clone(), end_of_day)];
    let mut merged = Vec::new();
    for unanswered in [false, true] {
        let mut runs: Vec<&Event> = events
            .iter()
            .copied()
            .filter(|event| (event.rsvp == Some(Rsvp::NeedsAction)) == unanswered)
            .collect();
        runs.sort_by_key(|event| event.start.timestamp());
        let mut bars: Vec<Event> = Vec::new();
        for event in runs {
            match bars.last_mut() {
                Some(last) if last.end > event.start => {
                    if event.end > last.end {
                        last.end = event.end.clone();
                    }
                }
                _ => bars.push(Event {
                    title: String::new(),
                    time_text: None,
                    ..event.clone()
                }),
            }
        }
        // Each bar clipped to the time no earlier bar took.
        let mut kept = Vec::new();
        for bar in bars {
            for (from, to) in &gaps {
                let (start, end) = (
                    bar.start.clone().max(from.clone()),
                    bar.end.clone().min(to.clone()),
                );
                if start < end {
                    kept.push(Event {
                        key: format!("{}_{}", bar.key, start.timestamp().as_millisecond()),
                        start,
                        end,
                        ..bar.clone()
                    });
                }
            }
        }
        gaps = subtract(&gaps, &kept);
        merged.extend(kept);
    }
    merged
}

/// `gaps` without the time `taken` covers.
fn subtract(gaps: &[(Zoned, Zoned)], taken: &[Event]) -> Vec<(Zoned, Zoned)> {
    let mut left = gaps.to_vec();
    for event in taken {
        left = left
            .into_iter()
            .flat_map(|(from, to)| {
                if event.end <= from || event.start >= to {
                    return vec![(from, to)];
                }
                [(from, event.start.clone()), (event.end.clone(), to)]
                    .into_iter()
                    .filter(|(from, to)| from < to)
                    .collect()
            })
            .collect();
    }
    left
}

/// The columns of the day from `midnight`: mine (when `mine` is given) then each guest's. With
/// three or more, all but the expanded one (mine unless a hover chose another) collapse to bars.
pub fn columns(
    mine: Option<Vec<&Event>>,
    guests: &[(String, Vec<Event>)],
    expanded: Option<&str>,
    midnight: &Zoned,
) -> Vec<Column> {
    let day = midnight.date();
    let count = usize::from(mine.is_some()) + guests.len();
    let collapse = count >= 3;
    let expanded = expanded.filter(|email| guests.iter().any(|(guest, _)| guest == email));
    let mut columns = Vec::new();
    if let Some(mine) = mine {
        let collapsed = collapse && expanded.is_some();
        let events = if collapsed {
            merged(&mine, midnight)
        } else {
            mine.into_iter().cloned().collect()
        };
        columns.push(Column {
            guest: None,
            collapsed,
            events,
        });
    }
    for (email, events) in guests {
        let on: Vec<&Event> = events
            .iter()
            .filter(|event| !event.all_day && event.on(day))
            .collect();
        let collapsed = collapse && expanded != Some(email.as_str());
        let events = if collapsed {
            merged(&on, midnight)
        } else {
            on.into_iter().cloned().collect()
        };
        columns.push(Column {
            guest: Some(email.clone()),
            collapsed,
            events,
        });
    }
    columns
}

/// The columns' widths as shares of the day, timed and all-day alike, from which are
/// `collapsed`: one alone takes it all; otherwise each open one 50% and the collapsed ones the
/// rest between them.
pub fn widths(collapsed: &[bool]) -> Vec<f32> {
    if collapsed.len() == 1 {
        return vec![1.0];
    }
    let open = collapsed.iter().filter(|collapsed| !**collapsed).count() as f32;
    let shut = collapsed.len() as f32 - open;
    // `flex-basis: 50%` shrinks to fit when open ones alone overflow.
    let open_width = if shut == 0.0 {
        1.0 / open
    } else {
        0.5_f32.min(1.0 / open)
    };
    let rest = if shut == 0.0 {
        0.0
    } else {
        (1.0 - open * open_width) / shut
    };
    collapsed
        .iter()
        .map(|collapsed| if *collapsed { rest } else { open_width })
        .collect()
}

/// A column of a day's all-day row: its calendar (a guest's address, `None` for mine), whether
/// it's collapsed, and its events.
pub type AllDayColumn<'a> = (Option<String>, bool, Vec<&'a Event>);

/// The all-day row's columns while guests are overlaid: for each day, mine (every all-day event
/// of mine on it) then each guest's (their all-day events that leave them free, on it). With two
/// guests or more all but the expanded one (mine unless a hover chose another) collapse.
pub fn all_day_columns<'a>(
    mine: &[&'a Event],
    guests: &'a [(String, Vec<Event>)],
    expanded: Option<&str>,
    days: &[Date],
) -> Vec<Vec<AllDayColumn<'a>>> {
    let collapse = guests.len() >= 2;
    let expanded = expanded.filter(|email| guests.iter().any(|(guest, _)| guest == email));
    days.iter()
        .map(|day| {
            let mut columns = vec![(
                None,
                collapse && expanded.is_some(),
                mine.iter()
                    .copied()
                    .filter(|event| event.all_day && event.on(*day))
                    .collect(),
            )];
            columns.extend(guests.iter().map(|(email, events)| {
                (
                    Some(email.clone()),
                    collapse && expanded != Some(email.as_str()),
                    events
                        .iter()
                        .filter(|event| event.all_day && event.on(*day))
                        .collect(),
                )
            }));
            columns
        })
        .collect()
}

/// The surface whose guests are overlaid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Surface {
    Form,
    Share,
    Meet,
}

/// Guests' busy times as Find Time counts them: their events that don't leave them
/// free, all-day ones as their days.
pub fn busy(data: &Data, email: &str, tz: &TimeZone) -> Vec<(Timestamp, Timestamp)> {
    events_of(data, email)
        .unwrap_or_default()
        .into_iter()
        .filter(|event| event.transparency.as_deref() != Some("transparent"))
        .filter_map(|event| span(event, tz))
        .map(|(start, end, _)| (start, end))
        .collect()
}

impl crate::Mail {
    /// The guests overlaid on the calendar now, with the column a hover expanded: the open
    /// form's, else Share Availability's, else the week's Meet's (the right pane only the
    /// form's). Asks for their calendars over the time that surface covers, and gives the
    /// loaded ones their colours.
    pub(super) fn calendar_overlay(
        &mut self,
        now: &Zoned,
        pane: bool,
        days: &[Date],
    ) -> (Vec<(String, Vec<Event>)>, Option<String>) {
        let Some((surface, guests, from)) = self.calendar_overlaid(now, pane) else {
            return (Vec::new(), None);
        };
        let tz = now.time_zone().clone();
        let (from, to) = match surface {
            // This week's start to 30 days from now.
            Surface::Share => (
                from,
                now.checked_add(30.days()).map_or(from, |to| to.timestamp()),
            ),
            // The anchor's Sunday and 20 days after, or for Meet the weeks Find Time looks
            // through.
            Surface::Form => (from, from + jiff::SignedDuration::from_hours(21 * 24)),
            Surface::Meet => (
                from,
                from + jiff::SignedDuration::from_hours(7 * 24 * (super::meet::WEEKS + 1)),
            ),
        };
        self.account.calendar().load_guests(&guests, from, to);
        let data = self.account.calendar().data();
        let mine: Vec<Mine> = self
            .calendar_list()
            .into_iter()
            .map(|calendar| (calendar.id, calendar.color, calendar.shown))
            .collect();
        let online = self.snapshot.connection.is_online();
        let Some(colors) = self.calendar_overlay_colors(surface) else {
            return (Vec::new(), None);
        };
        colors.assign(&guests, &data, &mine);
        let shown = colors.shown(&guests, &data, online);
        let expanded = colors.expanded.clone();
        (guest_events(&data, &shown, days, &tz), expanded)
    }

    /// A guest's event in the week the details show: `key`'s, among the overlaid guests', in its
    /// guest's colour, with no answer of mine and nothing to change. An all-day one that keeps
    /// them busy goes by each of its days' blocks.
    pub(super) fn calendar_guest_detailed(
        &self,
        key: &str,
        now: &Zoned,
    ) -> Option<(Event, ApiEvent)> {
        let (surface, guests, _) = self.calendar_overlaid(now, false)?;
        let colors = match surface {
            Surface::Form => &self.calendar.form.as_ref()?.guest_colors,
            Surface::Share => &self.calendar.week.as_ref()?.share.as_ref()?.colors,
            Surface::Meet => &self.calendar.week.as_ref()?.meet.colors,
        };
        let data = self.account.calendar().data();
        for (email, color) in colors.shown(&guests, &data, self.snapshot.connection.is_online()) {
            for event in events_of(&data, &email).unwrap_or_default() {
                let stored = StoredEvent {
                    calendar_id: email.clone(),
                    event: event.clone(),
                    unsynced: false,
                };
                let Some(mut block) = Event::new(
                    &stored,
                    None,
                    &BTreeMap::from([(email.clone(), color)]),
                    &email,
                    now.time_zone(),
                ) else {
                    continue;
                };
                let day_of = |rest: &str| {
                    rest.strip_prefix('_')
                        .and_then(|day| day.parse::<Date>().ok())
                };
                let matches = block.key == key
                    || (block.all_day
                        && key
                            .strip_prefix(block.key.as_str())
                            .and_then(day_of)
                            .is_some_and(|day| block.on(day)));
                if !matches {
                    continue;
                }
                if event.summary.as_deref().is_none_or(str::is_empty) {
                    block.title = "Busy".to_owned();
                }
                block.editable = false;
                block.rsvp = None;
                return Some((block, event.clone()));
            }
        }
        None
    }

    /// The overlaid surface, its guests and the start of the time their calendars are read over.
    fn calendar_overlaid(
        &self,
        now: &Zoned,
        pane: bool,
    ) -> Option<(Surface, Vec<String>, Timestamp)> {
        let sunday = |day: Date| {
            super::week_start(day, false)
                .to_zoned(now.time_zone().clone())
                .ok()
                .map(|at| at.timestamp())
        };
        if let Some(form) = &self.calendar.form {
            let guests: Vec<String> = form
                .values
                .guests
                .iter()
                .filter(|guest| !guest.me)
                .map(|guest| guest.email.clone())
                .collect();
            let anchor = if form.week {
                form.values
                    .when
                    .as_ref()
                    .map_or(now.date(), |(start, _)| start.date())
            } else {
                self.calendar
                    .form_day
                    .or(self.calendar.day.map(|day| day.date))
                    .unwrap_or(now.date())
            };
            return Some((Surface::Form, guests, sunday(anchor)?));
        }
        if pane {
            return None;
        }
        let week = self.calendar.week.as_ref()?;
        if let Some(share) = &week.share {
            let guests = share
                .attendees
                .iter()
                .filter(|attendee| attendee.visible && !attendee.person.organizer)
                .map(|attendee| attendee.address.email.clone())
                .collect();
            return Some((
                Surface::Share,
                guests,
                self.calendar_week_start(now.date())
                    .to_zoned(now.time_zone().clone())
                    .ok()?
                    .timestamp(),
            ));
        }
        let guests = week
            .meet
            .guests
            .iter()
            .map(|guest| guest.email.clone())
            .collect();
        let first = self
            .calendar_week_start(week.active)
            .min(self.calendar_week_start(now.date()));
        Some((
            Surface::Meet,
            guests,
            first.to_zoned(now.time_zone().clone()).ok()?.timestamp(),
        ))
    }

    fn calendar_overlay_colors(&mut self, surface: Surface) -> Option<&mut Colors> {
        match surface {
            Surface::Form => self
                .calendar
                .form
                .as_mut()
                .map(|form| &mut form.guest_colors),
            Surface::Share => self
                .calendar
                .week
                .as_mut()
                .and_then(|week| week.share.as_mut())
                .map(|share| &mut share.colors),
            Surface::Meet => self
                .calendar
                .week
                .as_mut()
                .map(|week| &mut week.meet.colors),
        }
    }

    /// The open form's guests' markers and busy statuses at its time, by address.
    pub(super) fn calendar_guest_chips(&self) -> BTreeMap<String, (Marker, Option<&'static str>)> {
        let Some(form) = &self.calendar.form else {
            return BTreeMap::new();
        };
        let data = self.account.calendar().data();
        let online = self.snapshot.connection.is_online();
        let guests: Vec<String> = form
            .values
            .guests
            .iter()
            .map(|guest| guest.email.clone())
            .collect();
        let shown = form.guest_colors.shown(&guests, &data, online);
        let when = form.values.when.as_ref();
        guests
            .into_iter()
            .map(|email| {
                let busy =
                    when.and_then(|(start, end)| busy_status(&email, &data, (start, end), online));
                let marker = marker(&email, &shown, &data, online);
                (email, (marker, busy))
            })
            .collect()
    }

    /// Meet's guests' markers, by address.
    pub(super) fn calendar_meet_markers(&self) -> BTreeMap<String, Marker> {
        let Some(week) = &self.calendar.week else {
            return BTreeMap::new();
        };
        let data = self.account.calendar().data();
        let online = self.snapshot.connection.is_online();
        let guests: Vec<String> = week
            .meet
            .guests
            .iter()
            .map(|guest| guest.email.clone())
            .collect();
        let shown = week.meet.colors.shown(&guests, &data, online);
        guests
            .into_iter()
            .map(|email| {
                let marker = marker(&email, &shown, &data, online);
                (email, marker)
            })
            .collect()
    }

    /// A hover expanded a column of the overlaid surface.
    pub(super) fn calendar_expand(&mut self, guest: Option<String>, pane: bool) {
        let now = self.now.clone();
        let Some((surface, ..)) = self.calendar_overlaid(&now, pane) else {
            return;
        };
        if let Some(colors) = self.calendar_overlay_colors(surface) {
            colors.expanded = guest;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::sync::GuestCalendar;

    fn loaded(emails: &[&str]) -> Data {
        let mut data = Data::default();
        for email in emails {
            data.guests.insert(
                (*email).to_owned(),
                GuestCalendar {
                    range: (Timestamp::UNIX_EPOCH, Timestamp::UNIX_EPOCH),
                    loading: false,
                    events: GuestEvents::Shown(Vec::new()),
                },
            );
        }
        data
    }

    #[test]
    fn guests_take_the_next_colour_after_my_shown_calendars_colours() {
        let guests = [
            "a@x.co".to_owned(),
            "b@x.co".to_owned(),
            "c@x.co".to_owned(),
        ];
        let data = loaded(&["a@x.co", "b@x.co", "c@x.co"]);
        let blue = CalendarColor::from_name("blue").expect("blue");
        let mine = [("me@x.co".to_owned(), blue, true)];
        let mut colors = Colors::default();
        colors.assign(&guests, &data, &mine);
        let names: Vec<&str> = guests
            .iter()
            .map(|email| colors.of(email).expect("a colour").name())
            .collect();
        assert_eq!(names, ["pink", "green", "orange"]);
    }

    #[test]
    fn a_calendar_of_mine_keeps_its_colour() {
        let guests = ["team@x.co".to_owned()];
        let data = loaded(&["team@x.co"]);
        let lime = CalendarColor::from_name("lime").expect("lime");
        let mut colors = Colors::default();
        colors.assign(&guests, &data, &[("team@x.co".to_owned(), lime, false)]);
        assert_eq!(colors.of("team@x.co"), Some(lime));
    }

    fn event(
        start: &str,
        end: &str,
        response: &str,
        kind: Option<&str>,
        transparent: bool,
    ) -> ApiEvent {
        let at = |time: &str| {
            Some(gmail::calendar::EventDateTime {
                date_time: Some(format!("2026-11-17T{time}:00Z")),
                ..Default::default()
            })
        };
        ApiEvent {
            start: at(start),
            end: at(end),
            event_type: kind.map(str::to_owned),
            transparency: transparent.then(|| "transparent".to_owned()),
            attendees: vec![gmail::calendar::EventAttendee {
                email: Some("a@x.co".into()),
                response_status: Some(response.into()),
                is_self: Some(true),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn a_chip_says_busy_over_out_of_office_and_ignores_free_and_unanswered_time() {
        let mut data = Data::default();
        let events = vec![
            event("11:00", "12:00", "accepted", None, true),
            event("11:00", "12:00", "needsAction", None, false),
            event("13:00", "18:00", "accepted", Some("outOfOffice"), false),
            event("15:00", "16:00", "accepted", None, false),
        ];
        data.guests.insert(
            "a@x.co".into(),
            GuestCalendar {
                range: (Timestamp::UNIX_EPOCH, Timestamp::UNIX_EPOCH),
                loading: false,
                events: GuestEvents::Shown(events),
            },
        );
        let at = |time: &str| {
            format!("2026-11-17T{time}:00Z")
                .parse::<Timestamp>()
                .expect("a time")
                .to_zoned(TimeZone::UTC)
        };
        let status =
            |from: &str, to: &str| busy_status("a@x.co", &data, (&at(from), &at(to)), true);
        assert_eq!(status("11:15", "11:45"), None, "free and unanswered");
        assert_eq!(status("13:30", "14:00"), Some("(OOO)"));
        assert_eq!(
            status("14:30", "15:30"),
            Some("(busy)"),
            "busy wins over out of office"
        );
        assert_eq!(
            busy_status("a@x.co", &data, (&at("14:30"), &at("15:30")), false),
            None,
            "offline"
        );
    }

    #[test]
    fn three_columns_collapse_all_but_the_expanded_one() {
        let tz = TimeZone::UTC;
        let midnight = jiff::civil::date(2026, 11, 17).to_zoned(tz).expect("a day");
        let guests = vec![
            ("a@x.co".to_owned(), Vec::new()),
            ("b@x.co".to_owned(), Vec::new()),
        ];
        let columns = columns(Some(Vec::new()), &guests, None, &midnight);
        assert_eq!(
            columns
                .iter()
                .map(|column| column.collapsed)
                .collect::<Vec<_>>(),
            [false, true, true]
        );
        assert_eq!(widths(&[false, true, true]), [0.5, 0.25, 0.25]);
        let two = super::columns(Some(Vec::new()), &guests[..1], None, &midnight);
        assert_eq!(
            two.iter()
                .map(|column| column.collapsed)
                .collect::<Vec<_>>(),
            [false, false]
        );
        assert_eq!(widths(&[false, false]), [0.5, 0.5]);
    }

    #[test]
    fn a_guest_s_free_all_day_event_goes_to_the_all_day_row_and_a_busy_one_fills_the_day() {
        let tz = TimeZone::UTC;
        let day = |date: &str| {
            Some(gmail::calendar::EventDateTime {
                date: Some(date.to_owned()),
                ..Default::default()
            })
        };
        let all_day = |id: &str, transparent: bool| ApiEvent {
            id: Some(id.into()),
            summary: Some(id.into()),
            start: day("2026-11-17"),
            end: day("2026-11-18"),
            transparency: transparent.then(|| "transparent".to_owned()),
            ..Default::default()
        };
        let mut data = loaded(&["a@x.co", "b@x.co"]);
        if let Some(calendar) = data.guests.get_mut("a@x.co") {
            calendar.events = GuestEvents::Shown(vec![
                all_day("Design week", true),
                all_day("Conference", false),
            ]);
        }
        let pink = CalendarColor::from_name("pink").expect("pink");
        let tuesday = jiff::civil::date(2026, 11, 17);
        let guests = guest_events(
            &data,
            &[("a@x.co".to_owned(), pink), ("b@x.co".to_owned(), pink)],
            &[tuesday],
            &tz,
        );
        let midnight = tuesday.to_zoned(tz).expect("a day");
        let timed = columns(Some(Vec::new()), &guests[..1], None, &midnight);
        assert_eq!(
            timed[1]
                .events
                .iter()
                .map(|event| event.title.as_str())
                .collect::<Vec<_>>(),
            ["Conference"],
            "the busy one fills the day"
        );
        let rows = all_day_columns(&[], &guests, None, &[tuesday]);
        let row: Vec<(Option<&str>, bool, Vec<&str>)> = rows[0]
            .iter()
            .map(|(guest, collapsed, events)| {
                (
                    guest.as_deref(),
                    *collapsed,
                    events.iter().map(|event| event.title.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            row,
            [
                (None, false, vec![]),
                (Some("a@x.co"), true, vec!["Design week"]),
                (Some("b@x.co"), true, vec![])
            ],
            "two guests: theirs collapse while mine is open"
        );
    }
}
