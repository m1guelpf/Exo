//! Dates for times ahead: the `exact-friendly` family a scheduled message's header shows, the
//! compact one a time picker's rows end with, and the compose form's `Send: {date}`.
//!
//! Sugar's tokens: `{Weekday}` Monday, `{Dow}` Mon, `{dow}` mon, `{Month}` November, `{Mon}` Nov,
//! `{d}` 7, `{dd}` 07, `{ord}` 7th, `{h}:{mm} {TT}` 8:00 AM. "Within a week" is before the start
//! of the day a week from now. All in `now`'s time zone. Remind me's `someday` and `never` are
//! `Someday` and `Never`.

use jiff::{Timestamp, ToSpan as _, Zoned};

use super::dates::{ordinal, sentinel, time};

const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
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

/// A date in `now`'s zone, with what the formats ask of it.
struct Day {
    at: Zoned,
    today: bool,
    tomorrow: bool,
    within_week: bool,
    this_month: bool,
    this_year: bool,
}

impl Day {
    fn new(at: Timestamp, now: &Zoned) -> Self {
        let at = at.to_zoned(now.time_zone().clone());
        let week = now
            .date()
            .checked_add(7.days())
            .ok()
            .and_then(|day| day.to_zoned(now.time_zone().clone()).ok());
        Self {
            today: at.date() == now.date(),
            tomorrow: now.date().tomorrow().ok() == Some(at.date()),
            within_week: week.is_none_or(|week| at < week),
            this_month: (at.year(), at.month()) == (now.year(), now.month()),
            this_year: at.year() == now.year(),
            at,
        }
    }

    fn weekday(&self) -> &'static str {
        WEEKDAYS
            [usize::try_from(self.at.weekday().to_monday_zero_offset()).expect("weekdays are 0–6")]
    }

    fn dow(&self) -> &'static str {
        &self.weekday()[..3]
    }

    fn month(&self) -> &'static str {
        MONTHS[usize::try_from(self.at.month() - 1).expect("months are 1–12")]
    }

    fn mon(&self) -> &'static str {
        &self.month()[..3]
    }

    fn time(&self) -> String {
        time(&self.at)
    }

    /// The time with a no-break space: `8:00\u{a0}AM`.
    fn time_nbsp(&self) -> String {
        self.time().replace(' ', "\u{a0}")
    }
}

/// The full form: `8:00 AM`, `Wednesday November 18th, 8:00 AM`, `December 4th, 8:00 AM`,
/// `January 4th, 2027, 8:00 AM`.
pub fn exact_friendly(at: Timestamp, now: &Zoned) -> String {
    if let Some(name) = sentinel(at) {
        return name.into();
    }
    let day = Day::new(at, now);
    if day.today {
        day.time()
    } else if day.within_week {
        format!(
            "{} {} {}, {}",
            day.weekday(),
            day.month(),
            ordinal(day.at.day()),
            day.time()
        )
    } else if day.this_year {
        format!("{} {}, {}", day.month(), ordinal(day.at.day()), day.time())
    } else {
        format!(
            "{} {}, {}, {}",
            day.month(),
            ordinal(day.at.day()),
            day.at.year(),
            day.time()
        )
    }
}

/// Without the time: `8:00 AM`, `Wednesday, November 18th`, `December 4th`, `January 4th, 2027`.
pub fn exact_friendly_omit_time(at: Timestamp, now: &Zoned) -> String {
    if let Some(name) = sentinel(at) {
        return name.into();
    }
    let day = Day::new(at, now);
    if day.today {
        day.time()
    } else if day.within_week {
        format!(
            "{}, {} {}",
            day.weekday(),
            day.month(),
            ordinal(day.at.day())
        )
    } else if day.this_year {
        format!("{} {}", day.month(), ordinal(day.at.day()))
    } else {
        format!(
            "{} {}, {}",
            day.month(),
            ordinal(day.at.day()),
            day.at.year()
        )
    }
}

/// The compact form: `8:00 AM`, `wed, 8:00 AM`, `fri, Dec 04, 8:00 AM`,
/// `mon, Jan 04, 2027, 8:00 AM`. The pickers show it upper-cased.
pub fn exact_friendly_compact(at: Timestamp, now: &Zoned) -> String {
    if let Some(name) = sentinel(at) {
        return name.into();
    }
    let day = Day::new(at, now);
    let dow = day.dow().to_lowercase();
    if day.today {
        day.time()
    } else if day.within_week {
        format!("{dow}, {}", day.time())
    } else if day.this_year {
        format!("{dow}, {} {:02}, {}", day.mon(), day.at.day(), day.time())
    } else {
        format!(
            "{dow}, {} {:02}, {}, {}",
            day.mon(),
            day.at.day(),
            day.at.year(),
            day.time()
        )
    }
}

/// A time typed in another zone: `Wed, Nov 18, 8:00 AM EST`.
pub fn time_suggestor(at: Timestamp, now: &Zoned) -> String {
    let day = Day::new(at, now);
    let zone = day
        .at
        .time_zone()
        .to_offset_info(day.at.timestamp())
        .abbreviation()
        .to_owned();
    format!(
        "{}, {} {}, {} {zone}",
        day.dow(),
        day.mon(),
        day.at.day(),
        day.time()
    )
}

/// The compose form's date formats, widest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposeFormat {
    /// `compose-form-friendly`.
    Friendly,
    /// `compose-form-compact`.
    Compact,
    /// `compose-form-extra-compact`.
    ExtraCompact,
    /// `compose-form-most-compact`.
    MostCompact,
}

/// The date in `Send: {date}`, its time's space a no-break one.
pub fn compose_form(at: Timestamp, now: &Zoned, format: ComposeFormat) -> String {
    let day = Day::new(at, now);
    let ord = ordinal(day.at.day());
    let d = day.at.day();
    if day.today {
        return day.time_nbsp();
    }
    match format {
        ComposeFormat::MostCompact if day.tomorrow || (day.within_week && !day.this_month) => {
            format!("{}…", day.dow())
        }
        ComposeFormat::MostCompact => format!("{} {d}…", day.mon()),
        _ if day.tomorrow => format!("{} {}", day.dow(), day.time_nbsp()),
        ComposeFormat::Friendly if day.within_week => {
            format!("{} {} {ord}, {}", day.dow(), day.mon(), day.time_nbsp())
        }
        ComposeFormat::Friendly => format!("{} {} {ord}", day.dow(), day.mon()),
        ComposeFormat::Compact if day.within_week => {
            format!("{} {} {d}, {}", day.dow(), day.mon(), day.time_nbsp())
        }
        ComposeFormat::Compact => format!("{} {} {d}", day.dow(), day.mon()),
        ComposeFormat::ExtraCompact if day.within_week => {
            format!("{} {d}, {}", day.mon(), day.time_nbsp())
        }
        ComposeFormat::ExtraCompact => format!("{} {d}", day.mon()),
    }
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
    fn friendly_dates() {
        let now = now();
        assert_eq!(exact_friendly(at("2026-11-17T13:00"), &now), "1:00 PM");
        assert_eq!(
            exact_friendly(at("2026-11-18T08:00"), &now),
            "Wednesday November 18th, 8:00 AM"
        );
        assert_eq!(
            exact_friendly(at("2026-11-24T08:00"), &now),
            "November 24th, 8:00 AM",
            "a week from now isn't within the week"
        );
        assert_eq!(
            exact_friendly(at("2027-01-04T08:00"), &now),
            "January 4th, 2027, 8:00 AM"
        );
        assert_eq!(
            exact_friendly_omit_time(at("2026-11-23T08:00"), &now),
            "Monday, November 23rd"
        );
        assert_eq!(
            exact_friendly_omit_time(at("2026-12-04T08:00"), &now),
            "December 4th"
        );
        assert_eq!(
            exact_friendly_compact(at("2026-11-18T08:00"), &now).to_uppercase(),
            "WED, 8:00 AM"
        );
        assert_eq!(
            exact_friendly_compact(at("2026-12-04T08:00"), &now).to_uppercase(),
            "FRI, DEC 04, 8:00 AM"
        );
        assert_eq!(
            time_suggestor(at("2026-11-18T08:00"), &now),
            "Wed, Nov 18, 8:00 AM EST"
        );
    }

    #[test]
    fn compose_form_dates() {
        let now = now();
        let nbsp = |text: &str| text.replace('_', "\u{a0}");
        assert_eq!(
            compose_form(at("2026-11-17T15:00"), &now, ComposeFormat::Friendly),
            nbsp("3:00_PM")
        );
        assert_eq!(
            compose_form(at("2026-11-18T08:00"), &now, ComposeFormat::Friendly),
            nbsp("Wed 8:00_AM")
        );
        assert_eq!(
            compose_form(at("2026-11-20T08:00"), &now, ComposeFormat::Friendly),
            nbsp("Fri Nov 20th, 8:00_AM")
        );
        assert_eq!(
            compose_form(at("2026-11-20T08:00"), &now, ComposeFormat::Compact),
            nbsp("Fri Nov 20, 8:00_AM")
        );
        assert_eq!(
            compose_form(at("2026-11-20T08:00"), &now, ComposeFormat::ExtraCompact),
            nbsp("Nov 20, 8:00_AM")
        );
        assert_eq!(
            compose_form(at("2026-11-20T08:00"), &now, ComposeFormat::MostCompact),
            "Nov 20…"
        );
        assert_eq!(
            compose_form(at("2026-11-18T08:00"), &now, ComposeFormat::MostCompact),
            "Wed…"
        );
        assert_eq!(
            compose_form(at("2026-12-01T08:00"), &now, ComposeFormat::Friendly),
            "Tue Dec 1st"
        );
        assert_eq!(
            compose_form(at("2026-12-01T08:00"), &now, ComposeFormat::MostCompact),
            "Dec 1…"
        );
    }
}
