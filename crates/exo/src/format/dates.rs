//! The list's dates: the row timestamp and the date separators between rows.
//!
//! Both are computed in `now`'s time zone. Formats are the en-US ones.

use jiff::{Timestamp, ToSpan as _, Zoned, civil::Date, tz::TimeZone};

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn month_name(date: Date) -> &'static str {
    MONTHS[usize::try_from(date.month() - 1).expect("months are 1–12")]
}

/// `9:05 AM`.
pub fn time(at: &Zoned) -> String {
    let hour = match at.hour() % 12 {
        0 => 12,
        hour => hour,
    };
    let meridiem = if at.hour() < 12 { "AM" } else { "PM" };
    format!("{hour}:{:02} {meridiem}", at.minute())
}

/// Remind me's `someday` and `never` (`ca.d0H`, `ca.tmp`), which dates name.
pub const SOMEDAY: i64 = 4_000_000_000_000;
pub const NEVER: i64 = 5_000_000_000_000;

/// What dates call Remind me's `someday` and `never`.
pub(crate) fn sentinel(at: Timestamp) -> Option<&'static str> {
    match at.as_millisecond() {
        SOMEDAY => Some("Someday"),
        NEVER => Some("Never"),
        _ => None,
    }
}

/// A row's timestamp: the time today, `NOV 15` this year, `3/7/2024` before.
pub fn row(at: Timestamp, now: &Zoned) -> String {
    if let Some(name) = sentinel(at) {
        return name.into();
    }
    let at = at.to_zoned(now.time_zone().clone());
    if at.date() == now.date() {
        time(&at)
    } else if at.year() == now.year() {
        format!("{} {}", month_name(at.date())[..3].to_uppercase(), at.day())
    } else {
        format!("{}/{}/{}", at.month(), at.day(), at.year())
    }
}

/// A date without its weekday: `Today, 8:00 AM`, `November 12th, 10:30 AM` this year,
/// `November 12th, 2025` before.
pub fn exact_friendly_omit_day_of_week(at: Timestamp, now: &Zoned) -> String {
    let at = at.to_zoned(now.time_zone().clone());
    if at.date() == now.date() {
        format!("Today, {}", time(&at))
    } else if at.year() == now.year() {
        format!(
            "{} {}, {}",
            month_name(at.date()),
            ordinal(at.day()),
            time(&at)
        )
    } else {
        format!(
            "{} {}, {}",
            month_name(at.date()),
            ordinal(at.day()),
            at.year()
        )
    }
}

const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// The zone's abbreviation at `at` (`EST`, `PDT`).
fn zone_abbreviation(at: &Zoned) -> String {
    at.time_zone()
        .to_offset_info(at.timestamp())
        .abbreviation()
        .to_owned()
}

/// The expanded message header's date: `Tuesday, November 17 2026 at 8:31 AM EST`.
pub fn expanded(at: Timestamp, now: &Zoned) -> String {
    let at = at.to_zoned(now.time_zone().clone());
    let weekday =
        WEEKDAYS[usize::try_from(at.weekday().to_monday_zero_offset()).expect("weekdays are 0–6")];
    format!(
        "{weekday}, {} {} {} at {} {}",
        month_name(at.date()),
        at.day(),
        at.year(),
        time(&at),
        zone_abbreviation(&at)
    )
}

/// A message date's hover tooltip: `Tue, Nov 17 2026 at 8:31 AM EST`.
pub fn tooltip(at: Timestamp, now: &Zoned) -> String {
    let at = at.to_zoned(now.time_zone().clone());
    let weekday = &WEEKDAYS
        [usize::try_from(at.weekday().to_monday_zero_offset()).expect("weekdays are 0–6")][..3];
    format!(
        "{weekday}, {} {} {} at {} {}",
        &month_name(at.date())[..3],
        at.day(),
        at.year(),
        time(&at),
        zone_abbreviation(&at)
    )
}

/// An invite's When: `Wednesday, November 25th, 2026 at 9:00 AM – 9:45 AM EST` on one day,
/// `Monday, December 7th 2026 (all day)` for an all-day day, and both ends in full otherwise.
pub fn calendar_range(start: Timestamp, end: Timestamp, now: &Zoned) -> String {
    let zone = now.time_zone().clone();
    let (start, end) = (start.to_zoned(zone.clone()), end.to_zoned(zone));
    let weekday = |at: &Zoned| {
        WEEKDAYS[usize::try_from(at.weekday().to_monday_zero_offset()).expect("weekdays are 0–6")]
    };
    let day = |at: &Zoned| {
        format!(
            "{}, {} {}, {}",
            weekday(at),
            month_name(at.date()),
            ordinal(at.day()),
            at.year()
        )
    };
    if start.date() == end.date() {
        return format!(
            "{} at {} – {} {}",
            day(&start),
            time(&start),
            time(&end),
            zone_abbreviation(&end)
        );
    }
    let midnight = |at: &Zoned| at.hour() == 0 && at.minute() == 0;
    let all_day = midnight(&start) && midnight(&end);
    if all_day && start.date().tomorrow().ok() == Some(end.date()) {
        return format!(
            "{}, {} {} {} (all day)",
            weekday(&start),
            month_name(start.date()),
            ordinal(start.day()),
            start.year()
        );
    }
    if all_day {
        let last = end
            .date()
            .yesterday()
            .ok()
            .and_then(|date| date.to_zoned(end.time_zone().clone()).ok())
            .unwrap_or(end);
        return format!("{} – {} (all day)", day(&start), day(&last));
    }
    let full = |at: &Zoned| format!("{} at {} {}", day(at), time(at), zone_abbreviation(at));
    format!("{} – {}", full(&start), full(&end))
}

/// A calendar notification's times: notification's times: `10:00 – 10:30AM` on one day, else both
/// ends as `Tue, Nov 17 11:00PM`.
pub fn calendar_range_compact(start: Timestamp, end: Timestamp, tz: &TimeZone) -> String {
    let (start, end) = (start.to_zoned(tz.clone()), end.to_zoned(tz.clone()));
    // `{h}:{mm}`, and `{TT}` without a space.
    let clock = |at: &Zoned| {
        format!(
            "{}:{:02}",
            if at.hour() % 12 == 0 {
                12
            } else {
                at.hour() % 12
            },
            at.minute()
        )
    };
    let meridiem = |at: &Zoned| if at.hour() < 12 { "AM" } else { "PM" };
    if start.date() == end.date() {
        return format!("{} – {}{}", clock(&start), clock(&end), meridiem(&end));
    }
    let full = |at: &Zoned| {
        let weekday = &WEEKDAYS
            [usize::try_from(at.weekday().to_monday_zero_offset()).expect("weekdays are 0–6")][..3];
        format!(
            "{weekday}, {} {} {}{}",
            &month_name(at.date())[..3],
            at.day(),
            clock(at),
            meridiem(at)
        )
    };
    format!("{} – {}", full(&start), full(&end))
}

/// A calendar notification's `in 2 mins`: Sugar's `relative()` from `now` (`in 1 hour`,
/// `3 minutes ago`) with the first `minute` and `hours` shortened, and anything in seconds `now`.
pub fn relative_compact(at: Timestamp, now: Timestamp) -> String {
    let (count, unit, ahead) = adjusted_unit(at, now);
    let plural = if count == 1.0 { "" } else { "s" };
    let relative = if ahead {
        format!("in {count} {unit}{plural}")
    } else {
        format!("{count} {unit}{plural} ago")
    };
    if relative.contains("second") {
        return "now".to_owned();
    }
    relative
        .replacen("minute", "min", 1)
        .replacen("hours", "hrs", 1)
}

/// Sugar's `relative()`: `5 minutes ago`, `1 hour ago`, `3 days from now`.
pub fn relative(at: Timestamp, now: Timestamp) -> String {
    let (count, unit, ahead) = adjusted_unit(at, now);
    let plural = if count == 1.0 { "" } else { "s" };
    format!(
        "{count} {unit}{plural} {}",
        if ahead { "from now" } else { "ago" }
    )
}

/// How far `at` is from `now` in Sugar's terms: the largest unit it's at least one of, rounded to a
/// tenth then down, and whether it's ahead.
fn adjusted_unit(at: Timestamp, now: Timestamp) -> (f64, &'static str, bool) {
    // Sugar's units, largest first (a month is 30.4375 days, a year 365.25).
    const UNITS: [(&str, f64); 7] = [
        ("year", 31_557_600_000.0),
        ("month", 2_629_800_000.0),
        ("week", 604_800_000.0),
        ("day", 86_400_000.0),
        ("hour", 3_600_000.0),
        ("minute", 60_000.0),
        ("second", 1000.0),
    ];
    let ms = at.as_millisecond() - now.as_millisecond();
    let away = ms.unsigned_abs() as f64;
    let (unit, count) = UNITS
        .iter()
        .map(|(unit, size)| (*unit, ((away / size * 10.0).round() / 10.0).floor()))
        .find(|(_, count)| *count >= 1.0)
        .unwrap_or(("millisecond", away));
    (count, unit, ms > 0)
}

/// `1st`, `2nd`, `3rd`, `4th`, `11th`, `21st`.
pub(crate) fn ordinal(day: i8) -> String {
    let suffix = match (day % 10, day % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{day}{suffix}")
}

/// A day: `Today`, `Yesterday`, `November 12th`, `November 12th, 2025`.
fn friendly_day(date: Date, today: Date) -> String {
    if date == today {
        "Today".into()
    } else if date.tomorrow().ok() == Some(today) {
        "Yesterday".into()
    } else if date.year() == today.year() {
        format!("{} {}", month_name(date), ordinal(date.day()))
    } else {
        format!(
            "{} {}, {}",
            month_name(date),
            ordinal(date.day()),
            date.year()
        )
    }
}

/// A month: `this month`, `October`, `October 2025`.
fn friendly_month(date: Date, today: Date) -> String {
    if (date.year(), date.month()) == (today.year(), today.month()) {
        "this month".into()
    } else if date.year() == today.year() {
        month_name(date).into()
    } else {
        format!("{} {}", month_name(date), date.year())
    }
}

/// One entry of a segmented list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment<T> {
    /// A date separator.
    Separator(String),
    /// A list item.
    Item(T),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Day,
    Month,
    Merged,
}

struct Separator {
    label: String,
    /// The last instant the separator covers; it sits above items at or before it.
    until: Timestamp,
    kind: Kind,
}

/// The last instant of `date` in `now`'s zone.
fn end_of_day(date: Date, now: &Zoned) -> Timestamp {
    date.to_zoned(now.time_zone().clone())
        .and_then(|start| start.checked_add(1.day()))
        .map_or(Timestamp::MAX, |next| next.timestamp() - 1.millisecond())
}

/// A separator for each of the last seven days, then one per month, back to `oldest`.
fn separators(oldest: Timestamp, now: &Zoned) -> Vec<Separator> {
    let today = now.date();
    let days_ago = |n: i64| {
        today
            .checked_sub(n.days())
            .expect("dates near now are in range")
    };
    let mut separators = Vec::new();
    for n in 0..7 {
        let day = days_ago(n);
        let until = end_of_day(day, now);
        if until <= oldest {
            break;
        }
        separators.push(Separator {
            label: friendly_day(day, today),
            until,
            kind: Kind::Day,
        });
    }
    let week_ago = days_ago(7);
    let prefix = if week_ago.month() == today.month() {
        "Earlier "
    } else if week_ago.month() == days_ago(6).month() {
        "Earlier in "
    } else {
        ""
    };
    let mut until = end_of_day(week_ago, now);
    if until <= oldest {
        return separators;
    }
    separators.push(Separator {
        label: format!("{prefix}{}", friendly_month(week_ago, today)),
        until,
        kind: Kind::Month,
    });
    // Each earlier month, labelled at its last day.
    let mut month_end = week_ago;
    for _ in 1..600 {
        month_end = month_end
            .first_of_month()
            .yesterday()
            .expect("dates near now are in range");
        until = end_of_day(month_end, now);
        if until <= oldest {
            break;
        }
        separators.push(Separator {
            label: friendly_month(month_end, today),
            until,
            kind: Kind::Month,
        });
    }
    separators
}

/// `items` (newest first, with their sort time) interleaved with date separators. Empty groups are
/// dropped; runs of sparse days within the last week merge into `Last 7 days`; the list never
/// starts with a separator.
pub fn segment<T>(items: Vec<(Timestamp, T)>, now: &Zoned) -> Vec<Segment<T>> {
    let Some(oldest) = items.last().map(|(at, _)| *at) else {
        return Vec::new();
    };
    // Merge separators into the list: a separator goes before the first item at or before it.
    let mut entries: Vec<Entry<T>> = Vec::new();
    let mut pending = separators(oldest, now).into_iter().peekable();
    for (at, item) in items {
        while let Some(separator) = pending.next_if(|separator| separator.until >= at) {
            entries.push(Entry::Separator(separator));
        }
        entries.push(Entry::Item(item));
    }
    // A separator directly followed by another covers nothing.
    let mut i = 0;
    while i + 1 < entries.len() {
        if matches!(
            (&entries[i], &entries[i + 1]),
            (Entry::Separator(_), Entry::Separator(_))
        ) {
            entries.remove(i);
        } else {
            i += 1;
        }
    }
    merge_sparse_days(&mut entries);
    if matches!(entries.first(), Some(Entry::Separator(_))) {
        entries.remove(0);
    }
    entries
        .into_iter()
        .map(|entry| match entry {
            Entry::Separator(separator) => Segment::Separator(separator.label),
            Entry::Item(item) => Segment::Item(item),
        })
        .collect()
}

enum Entry<T> {
    Separator(Separator),
    Item(T),
}

/// From the first pair of consecutive day separators (other than `Today`) with at most two
/// items between them, every day separator up to the first month separator is replaced by one
/// `Last 7 days` where the first of the pair was.
fn merge_sparse_days<T>(entries: &mut Vec<Entry<T>>) {
    let mut merged_at = None;
    let mut doomed = Vec::new();
    let mut previous_day: Option<usize> = None;
    let mut items_since = 0;
    for (i, entry) in entries.iter().enumerate() {
        let Entry::Separator(separator) = entry else {
            items_since += 1;
            continue;
        };
        if separator.kind == Kind::Month {
            break;
        }
        if separator.kind != Kind::Day || separator.label == "Today" {
            continue;
        }
        if merged_at.is_some() {
            doomed.push(i);
        } else if let Some(previous) = previous_day.filter(|_| items_since <= 2) {
            merged_at = Some(previous);
            doomed.extend([previous, i]);
        }
        previous_day = Some(i);
        items_since = 0;
    }
    let Some(at) = merged_at else {
        return;
    };
    let Entry::Separator(first) = &entries[at] else {
        unreachable!("merges start at a separator")
    };
    let merged = Separator {
        label: "Last 7 days".into(),
        until: first.until,
        kind: Kind::Merged,
    };
    // Everything doomed besides `at` comes after it, so removing those keeps `at` in place.
    for i in doomed.into_iter().rev().filter(|&i| i != at) {
        entries.remove(i);
    }
    entries[at] = Entry::Separator(merged);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Zoned {
        "2026-11-17T09:00:00[America/New_York]"
            .parse()
            .expect("valid time")
    }

    fn at(text: &str) -> Timestamp {
        format!("{text}[America/New_York]")
            .parse::<Zoned>()
            .expect("valid time")
            .timestamp()
    }

    #[test]
    fn exact_friendly_dates_leave_out_the_weekday() {
        assert_eq!(
            exact_friendly_omit_day_of_week(at("2026-11-17T08:00:00"), &now()),
            "Today, 8:00 AM"
        );
        assert_eq!(
            exact_friendly_omit_day_of_week(at("2026-11-12T10:30:00"), &now()),
            "November 12th, 10:30 AM"
        );
        assert_eq!(
            exact_friendly_omit_day_of_week(at("2025-11-12T10:30:00"), &now()),
            "November 12th, 2025"
        );
    }

    #[test]
    fn row_dates() {
        assert_eq!(row(at("2026-11-17T08:10"), &now()), "8:10 AM");
        assert_eq!(row(at("2026-11-17T00:05"), &now()), "12:05 AM");
        assert_eq!(row(at("2026-11-16T17:54"), &now()), "NOV 16");
        assert_eq!(row(at("2025-03-07T12:00"), &now()), "3/7/2025");
    }

    #[test]
    fn message_header_dates() {
        assert_eq!(
            expanded(at("2026-11-17T08:31"), &now()),
            "Tuesday, November 17 2026 at 8:31 AM EST"
        );
        assert_eq!(
            tooltip(at("2026-07-04T14:05"), &now()),
            "Sat, Jul 4 2026 at 2:05 PM EDT"
        );
    }

    #[test]
    fn invite_ranges() {
        assert_eq!(
            calendar_range(at("2026-11-25T09:00"), at("2026-11-25T09:45"), &now()),
            "Wednesday, November 25th, 2026 at 9:00 AM – 9:45 AM EST"
        );
        assert_eq!(
            calendar_range(at("2026-12-07T00:00"), at("2026-12-08T00:00"), &now()),
            "Monday, December 7th 2026 (all day)"
        );
        assert_eq!(
            calendar_range(at("2026-12-09T00:00"), at("2026-12-12T00:00"), &now()),
            "Wednesday, December 9th, 2026 – Friday, December 11th, 2026 (all day)"
        );
    }

    #[test]
    fn notification_ranges() {
        let tz = now().time_zone().clone();
        assert_eq!(
            calendar_range_compact(at("2026-11-17T10:00"), at("2026-11-17T10:30"), &tz),
            "10:00 – 10:30AM"
        );
        assert_eq!(
            calendar_range_compact(at("2026-11-17T11:30"), at("2026-11-17T12:15"), &tz),
            "11:30 – 12:15PM"
        );
        assert_eq!(
            calendar_range_compact(at("2026-11-17T23:00"), at("2026-11-18T01:00"), &tz),
            "Tue, Nov 17 11:00PM – Wed, Nov 18 1:00AM"
        );
    }

    #[test]
    fn relative_times_as_sugar_rounds_them() {
        let start = at("2026-11-17T10:00");
        let before = |seconds: i64| start - jiff::SignedDuration::from_secs(seconds);
        // Two minutes' lead, with the notifications' 30 s nudge: 150 s, and up to 30 s late.
        assert_eq!(
            relative_compact(start + 30.seconds(), before(120)),
            "in 2 mins"
        );
        assert_eq!(
            relative_compact(start + 30.seconds(), before(91)),
            "in 2 mins"
        );
        assert_eq!(
            relative_compact(start, before(110)),
            "in 1 min",
            "1.8 rounds to a tenth, then down"
        );
        assert_eq!(
            relative_compact(start, before(57)),
            "in 1 min",
            "0.95 rounds up to 1"
        );
        assert_eq!(relative_compact(start, before(50)), "now");
        assert_eq!(relative_compact(start, start), "now");
        assert_eq!(relative_compact(start, start + 180.seconds()), "3 mins ago");
        assert_eq!(relative_compact(start, before(3600)), "in 1 hour");
        assert_eq!(relative_compact(start, before(2 * 3600)), "in 2 hrs");
        assert_eq!(relative_compact(start, before(86_400)), "in 1 day");
        assert_eq!(relative_compact(start, before(7 * 86_400)), "in 1 week");
    }

    #[test]
    fn ordinals() {
        assert_eq!(
            [1, 2, 3, 4, 11, 12, 13, 21, 22, 31].map(ordinal),
            [
                "1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "31st"
            ]
        );
    }

    #[test]
    fn segments_like_the_harness_inbox() {
        // Eight today, ten yesterday, then two days of two, then older mail.
        let mut items = Vec::new();
        items.extend((0..8).map(|i| (at("2026-11-17T08:00"), format!("today {i}"))));
        items.extend((0..10).map(|i| (at("2026-11-16T12:00"), format!("yesterday {i}"))));
        items.extend((0..2).map(|i| (at("2026-11-15T12:00"), format!("15th {i}"))));
        items.push((at("2026-11-14T12:00"), "14th".into()));
        items.extend((0..2).map(|i| (at("2026-11-13T12:00"), format!("13th {i}"))));
        items.push((at("2026-11-09T12:00"), "9th".into()));
        items.push((at("2026-10-20T12:00"), "october".into()));
        let labels: Vec<_> = segment(items, &now())
            .into_iter()
            .filter_map(|segment| match segment {
                Segment::Separator(label) => Some(label),
                Segment::Item(_) => None,
            })
            .collect();
        assert_eq!(
            labels,
            ["Yesterday", "Last 7 days", "Earlier this month", "October"]
        );
    }
}
