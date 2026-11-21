//! Expansion of the RRULE subset fixtures and clients use: `FREQ=DAILY|WEEKLY|MONTHLY|YEARLY`
//! with `INTERVAL`, `COUNT`, `UNTIL`, (weekly) `BYDAY` and (monthly) one `BYDAY` with its week
//! (`3TU`, `-1FR`). Wall-clock times are kept across DST
//! changes by stepping in the event's own time zone; an all-day series' instances are all-day.

use gmail::calendar::{Event, EventDateTime};
use jiff::civil::Weekday;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan, Zoned};

/// Most instances generated per series, so open-ended rules stay bounded.
const MAX_INSTANCES: usize = 730;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

/// A rule's `UNTIL`: a date, or a UTC date-time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Until {
    Date(jiff::civil::Date),
    Time(Timestamp),
}

struct Rule {
    freq: Freq,
    interval: i64,
    count: Option<usize>,
    until: Option<Until>,
    by_day: Vec<Weekday>,
    /// A monthly rule's week of the month: 1 to 4, or -1 for the last.
    nth: Option<i8>,
}

/// Whether every rule's `UNTIL` takes its series' start's type, a date for an all-day series
/// and a date-time otherwise, as RFC 5545 §3.3.10 requires.
pub fn valid(event: &Event) -> bool {
    let all_day = event
        .start
        .as_ref()
        .is_some_and(|start| start.date.is_some());
    event
        .recurrence
        .iter()
        .filter_map(|line| line.strip_prefix("RRULE:"))
        .flat_map(|rule| rule.split(';'))
        .filter_map(|part| part.strip_prefix("UNTIL="))
        .all(|until| {
            matches!(
                (parse_until(until), all_day),
                (Some(Until::Date(_)), true) | (Some(Until::Time(_)), false)
            )
        })
}

/// The instances of a recurring `event` starting before `limit`, as Calendar returns them with
/// `singleEvents=true`.
pub fn expand(
    event: &Event,
    calendar_tz: &TimeZone,
    limit: Timestamp,
) -> impl Iterator<Item = Event> {
    let instances = instances(event, calendar_tz, limit).unwrap_or_default();
    let master = event.clone();
    let all_day = master
        .start
        .as_ref()
        .is_some_and(|start| start.date.is_some());
    instances.into_iter().map(move |(start, end)| {
        let tz_name = master.start.as_ref().and_then(|s| s.time_zone.clone());
        let at = |time: &Zoned| {
            if all_day {
                EventDateTime {
                    date: Some(time.date().to_string()),
                    ..EventDateTime::default()
                }
            } else {
                EventDateTime {
                    date: None,
                    date_time: Some(time.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string()),
                    time_zone: tz_name.clone(),
                }
            }
        };
        let suffix = if all_day {
            start.strftime("%Y%m%d").to_string()
        } else {
            start.timestamp().strftime("%Y%m%dT%H%M%SZ").to_string()
        };
        Event {
            id: master.id.as_ref().map(|id| format!("{id}_{suffix}")),
            recurring_event_id: master.id.clone(),
            original_start_time: Some(at(&start)),
            start: Some(at(&start)),
            end: Some(at(&end)),
            recurrence: Vec::new(),
            ..master.clone()
        }
    })
}

fn instances(
    event: &Event,
    calendar_tz: &TimeZone,
    limit: Timestamp,
) -> Option<Vec<(Zoned, Zoned)>> {
    let rule = event
        .recurrence
        .iter()
        .find_map(|line| line.strip_prefix("RRULE:"))
        .and_then(parse)?;
    let start_time = event.start.as_ref()?;
    let tz = start_time
        .time_zone
        .as_deref()
        .and_then(|name| TimeZone::get(name).ok())
        .unwrap_or_else(|| calendar_tz.clone());
    let first = start_time.timestamp(&tz)?.to_zoned(tz.clone());
    let last = event.end.as_ref()?.timestamp(&tz)?.to_zoned(tz.clone());
    // An all-day series lasts whole days, whatever DST does to them.
    let span = match start_time.date {
        Some(_) => (last.date() - first.date()).get_days().days(),
        None => last
            .timestamp()
            .duration_since(first.timestamp())
            .try_into()
            .ok()?,
    };

    let weekdays = if rule.by_day.is_empty() {
        vec![first.weekday()]
    } else {
        rule.by_day.clone()
    };
    let mut out = Vec::new();
    let mut day = first.clone();
    while out.len() < MAX_INSTANCES {
        let ended = match rule.until {
            Some(Until::Date(until)) => day.date() > until,
            Some(Until::Time(until)) => day.timestamp() > until,
            None => false,
        };
        if day.timestamp() >= limit || ended {
            break;
        }
        if rule.count.is_some_and(|count| out.len() >= count) {
            break;
        }
        let days = (day.date() - first.date()).get_days();
        let matches = match rule.freq {
            Freq::Daily => i64::from(days) % rule.interval == 0,
            Freq::Weekly => {
                let week_start = |date: jiff::civil::Date| {
                    date - i64::from(date.weekday().to_monday_zero_offset()).days()
                };
                let weeks = (week_start(day.date()) - week_start(first.date())).get_days() / 7;
                weekdays.contains(&day.weekday()) && i64::from(weeks) % rule.interval == 0
            }
            Freq::Monthly => {
                let months = i64::from(day.year() - first.year()) * 12
                    + i64::from(day.month() - first.month());
                let on_day = match rule.nth {
                    Some(-1) => day.weekday() == weekdays[0] && day.day() + 7 > day.days_in_month(),
                    Some(nth) => day.weekday() == weekdays[0] && (day.day() - 1) / 7 + 1 == nth,
                    None => day.day() == first.day(),
                };
                on_day && months % rule.interval == 0
            }
            Freq::Yearly => {
                day.month() == first.month()
                    && day.day() == first.day()
                    && i64::from(day.year() - first.year()) % rule.interval == 0
            }
        };
        if matches {
            let end = day.checked_add(span).ok()?;
            out.push((day.clone(), end));
        }
        day = day.checked_add(1.day()).ok()?;
    }
    Some(out)
}

fn parse(rule: &str) -> Option<Rule> {
    let mut parsed = Rule {
        freq: Freq::Daily,
        interval: 1,
        count: None,
        until: None,
        by_day: Vec::new(),
        nth: None,
    };
    let mut freq = None;
    for part in rule.split(';') {
        let (key, value) = part.split_once('=')?;
        match key {
            "FREQ" => {
                freq = Some(match value {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    _ => return None,
                });
            }
            "INTERVAL" => parsed.interval = value.parse().ok().filter(|&n: &i64| n > 0)?,
            "COUNT" => parsed.count = value.parse().ok(),
            "UNTIL" => parsed.until = parse_until(value),
            "BYDAY" => {
                parsed.by_day = value
                    .split(',')
                    .filter_map(|token| {
                        let at = token.len().checked_sub(2)?;
                        if at > 0 {
                            parsed.nth = token[..at].parse().ok();
                        }
                        Some(match &token[at..] {
                            "MO" => Weekday::Monday,
                            "TU" => Weekday::Tuesday,
                            "WE" => Weekday::Wednesday,
                            "TH" => Weekday::Thursday,
                            "FR" => Weekday::Friday,
                            "SA" => Weekday::Saturday,
                            "SU" => Weekday::Sunday,
                            _ => return None,
                        })
                    })
                    .collect();
            }
            _ => {}
        }
    }
    parsed.freq = freq?;
    Some(parsed)
}

/// `UNTIL=20261231T235959Z` or `UNTIL=20261231`, the last day the series may start on.
fn parse_until(value: &str) -> Option<Until> {
    if value.len() == 8 {
        return jiff::civil::Date::strptime("%Y%m%d", value)
            .ok()
            .map(Until::Date);
    }
    jiff::civil::DateTime::strptime("%Y%m%dT%H%M%SZ", value)
        .ok()?
        .to_zoned(TimeZone::UTC)
        .ok()
        .map(|zoned| Until::Time(zoned.timestamp()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weekly(rule: &str) -> Event {
        Event {
            id: Some("abc".into()),
            start: Some(EventDateTime {
                date_time: Some("2026-10-27T10:00:00-04:00".into()),
                time_zone: Some("America/New_York".into()),
                ..EventDateTime::default()
            }),
            end: Some(EventDateTime {
                date_time: Some("2026-10-27T10:30:00-04:00".into()),
                time_zone: Some("America/New_York".into()),
                ..EventDateTime::default()
            }),
            recurrence: vec![rule.into()],
            ..Event::default()
        }
    }

    #[test]
    fn keeps_wall_clock_across_dst() {
        let tz = TimeZone::get("America/New_York").unwrap();
        let instances: Vec<Event> =
            expand(&weekly("RRULE:FREQ=WEEKLY;COUNT=3"), &tz, Timestamp::MAX).collect();
        let starts: Vec<_> = instances
            .iter()
            .map(|e| e.start.clone().unwrap().date_time.unwrap())
            .collect();
        // DST ends on 2026-11-01: still 10:00 local, now at -05:00.
        assert_eq!(
            starts,
            [
                "2026-10-27T10:00:00-04:00",
                "2026-11-03T10:00:00-05:00",
                "2026-11-10T10:00:00-05:00"
            ]
        );
        assert_eq!(instances[1].id.as_deref(), Some("abc_20261103T150000Z"));
        assert_eq!(instances[1].recurring_event_id.as_deref(), Some("abc"));
    }

    #[test]
    fn an_all_day_series_has_all_day_instances_and_ends_on_a_date() {
        let day = |date: &str| EventDateTime {
            date: Some(date.into()),
            ..EventDateTime::default()
        };
        let all_day = |rule: &str| Event {
            id: Some("abc".into()),
            start: Some(day("2026-10-27")),
            end: Some(day("2026-10-28")),
            recurrence: vec![rule.into()],
            ..Event::default()
        };
        let tz = TimeZone::get("America/New_York").unwrap();
        let series = all_day("RRULE:FREQ=WEEKLY;UNTIL=20261110");
        let instances: Vec<Event> = expand(&series, &tz, Timestamp::MAX).collect();
        let days: Vec<_> = instances
            .iter()
            .map(|e| {
                (
                    e.start.clone().unwrap().date.unwrap(),
                    e.end.clone().unwrap().date.unwrap(),
                )
            })
            .collect();
        // Across the end of DST (Nov 1) too, each lasts the day.
        assert_eq!(
            days,
            [
                ("2026-10-27".to_owned(), "2026-10-28".to_owned()),
                ("2026-11-03".to_owned(), "2026-11-04".to_owned()),
                ("2026-11-10".to_owned(), "2026-11-11".to_owned())
            ]
        );
        assert_eq!(instances[1].id.as_deref(), Some("abc_20261103"));
        assert!(valid(&series));
        assert!(
            !valid(&all_day("RRULE:FREQ=WEEKLY;UNTIL=20261110T045959Z")),
            "an all-day series ends on a date"
        );
        assert!(
            !valid(&weekly("RRULE:FREQ=WEEKLY;UNTIL=20261110")),
            "a timed one at a time"
        );
    }

    #[test]
    fn weekly_by_day_with_until() {
        let tz = TimeZone::get("America/New_York").unwrap();
        let count = expand(
            &weekly("RRULE:FREQ=WEEKLY;BYDAY=TU,TH;UNTIL=20261106T000000Z"),
            &tz,
            Timestamp::MAX,
        )
        .count();
        assert_eq!(count, 4); // Oct 27, Oct 29, Nov 3, Nov 5
    }
}
