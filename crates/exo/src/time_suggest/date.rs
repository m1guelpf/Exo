//! JavaScript's `Date` in a time zone: an instant read and written as wall time, with the
//! setters' overflow (`setDate(0)` is the previous month's last day, `setMonth(12)` next
//! January, `setHours(25)` one in the morning tomorrow) and their truncation of fractions. The
//! parser does all its arithmetic with these setters, so its answers, quirks included, hang on
//! their semantics.

use jiff::{Timestamp, civil, tz::TimeZone};

const HOUR: i64 = 3_600_000;
const MINUTE: i64 = 60_000;
const DAY: i64 = 24 * HOUR;

/// A `Date`: milliseconds since the epoch, seen in `tz`.
#[derive(Clone, Debug)]
pub(super) struct JsDate {
    ms: i64,
    tz: TimeZone,
}

/// A `Date`'s wall time, split as its getters read it.
struct Fields {
    year: i64,
    /// 0 to 11.
    month: i64,
    date: i64,
    /// Milliseconds into the day.
    time: i64,
}

impl JsDate {
    pub(super) const fn new(ms: i64, tz: TimeZone) -> Self {
        Self { ms, tz }
    }

    /// The instant whose wall time in `tz` is `wall` (`new Date("2026-11-17T15:00:00")`).
    pub(super) fn from_wall(wall: civil::DateTime, tz: TimeZone) -> Self {
        let mut date = Self::new(0, tz);
        let time = (i64::from(wall.hour()) * 60 + i64::from(wall.minute())) * MINUTE
            + i64::from(wall.second()) * 1000;
        date.set_local(
            i64::from(wall.year()),
            i64::from(wall.month()) - 1,
            i64::from(wall.day()),
            time,
        );
        date
    }

    pub(super) const fn ms(&self) -> i64 {
        self.ms
    }

    pub(super) const fn time_zone(&self) -> &TimeZone {
        &self.tz
    }

    pub(super) fn timestamp(&self) -> Timestamp {
        Timestamp::from_millisecond(self.ms).unwrap_or_default()
    }

    /// `setTime`.
    pub(super) fn set_time(&mut self, ms: i64) {
        self.ms = ms;
    }

    fn local(&self) -> i64 {
        let offset = Timestamp::from_millisecond(self.ms)
            .map_or(0, |at| i64::from(self.tz.to_offset(at).seconds()));
        self.ms + offset * 1000
    }

    fn fields(&self) -> Fields {
        let local = self.local();
        let (year, month, date) = civil_from_days(local.div_euclid(DAY));
        Fields {
            year,
            month: month - 1,
            date,
            time: local.rem_euclid(DAY),
        }
    }

    pub(super) fn year(&self) -> i64 {
        self.fields().year
    }

    /// `getMonth`: 0 to 11.
    pub(super) fn month(&self) -> i64 {
        self.fields().month
    }

    /// `getDate`: the day of the month.
    pub(super) fn date(&self) -> i64 {
        self.fields().date
    }

    /// `getDay`: 0 (Sunday) to 6.
    pub(super) fn weekday(&self) -> i64 {
        (self.local().div_euclid(DAY) + 4).rem_euclid(7)
    }

    pub(super) fn minutes(&self) -> i64 {
        self.fields().time % HOUR / MINUTE
    }

    pub(super) fn set_full_year(&mut self, year: f64) {
        let fields = self.fields();
        self.set_local(js_int(year), fields.month, fields.date, fields.time);
    }

    pub(super) fn set_month(&mut self, month: f64) {
        let fields = self.fields();
        self.set_local(fields.year, js_int(month), fields.date, fields.time);
    }

    pub(super) fn set_date(&mut self, date: f64) {
        let fields = self.fields();
        self.set_local(fields.year, fields.month, js_int(date), fields.time);
    }

    pub(super) fn set_hours(&mut self, hours: f64) {
        let fields = self.fields();
        let time = js_int(hours) * HOUR + fields.time % HOUR;
        self.set_local(fields.year, fields.month, fields.date, time);
    }

    pub(super) fn set_minutes(&mut self, minutes: f64) {
        let fields = self.fields();
        let time = fields.time / HOUR * HOUR + js_int(minutes) * MINUTE + fields.time % MINUTE;
        self.set_local(fields.year, fields.month, fields.date, time);
    }

    /// `setSeconds(0)` then `setMilliseconds(0)`.
    pub(super) fn zero_seconds(&mut self) {
        let fields = self.fields();
        self.set_local(
            fields.year,
            fields.month,
            fields.date,
            fields.time / MINUTE * MINUTE,
        );
    }

    /// `MakeDate(MakeDay(year, month, date), time)` read as wall time in the zone (`UTC(t)`): a
    /// time a clock change skips moves on by the change, one it repeats is the earlier.
    fn set_local(&mut self, year: i64, month: i64, date: i64, time: i64) {
        let days =
            days_from_civil(year + month.div_euclid(12), month.rem_euclid(12) + 1, 1) + date - 1;
        self.ms = utc(days * DAY + time, &self.tz);
    }
}

/// `ToIntegerOrInfinity`, as the setters apply it to their arguments.
fn js_int(value: f64) -> i64 {
    value.trunc() as i64
}

/// The instant whose wall time in `tz` is `local` milliseconds past the epoch's midnight.
fn utc(local: i64, tz: &TimeZone) -> i64 {
    let (year, month, day) = civil_from_days(local.div_euclid(DAY));
    let time = local.rem_euclid(DAY);
    let wall = i16::try_from(year).ok().and_then(|year| {
        civil::DateTime::new(
            year,
            month as i8,
            day as i8,
            (time / HOUR) as i8,
            (time % HOUR / MINUTE) as i8,
            (time % MINUTE / 1000) as i8,
            (time % 1000 * 1_000_000) as i32,
        )
        .ok()
    });
    wall.and_then(|wall| tz.to_ambiguous_timestamp(wall).compatible().ok())
        .map_or(local, |at| at.as_millisecond())
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The date `days` after 1970-01-01, month 1 to 12 (Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn la() -> TimeZone {
        TimeZone::get("America/Los_Angeles").expect("zone")
    }

    fn at(text: &str) -> JsDate {
        let wall: civil::DateTime = text.parse().expect("date");
        JsDate::new(
            wall.to_zoned(la())
                .expect("exists")
                .timestamp()
                .as_millisecond(),
            la(),
        )
    }

    fn show(date: &JsDate) -> String {
        date.timestamp()
            .to_zoned(la())
            .strftime("%Y-%m-%d %H:%M")
            .to_string()
    }

    #[test]
    fn setters_overflow_as_dates_do() {
        let mut date = at("2026-03-31T10:00");
        date.set_month(1.0);
        assert_eq!(show(&date), "2026-03-03 10:00", "Feb 31st is Mar 3rd");
        date.set_date(0.0);
        assert_eq!(show(&date), "2026-02-28 10:00");
        date.set_month(12.5);
        assert_eq!(show(&date), "2027-01-28 10:00", "fractions truncate");
        date.set_hours(25.0);
        assert_eq!(show(&date), "2027-01-29 01:00");
        date.set_minutes(-30.0);
        assert_eq!(show(&date), "2027-01-29 00:30");
        assert_eq!((date.weekday(), date.minutes()), (5, 30));
    }

    #[test]
    fn wall_times_a_clock_change_skips_move_on() {
        let mut date = at("2026-03-08T00:00");
        date.set_hours(2.0);
        assert_eq!(show(&date), "2026-03-08 03:00");
        let mut date = at("2026-11-01T00:00");
        date.set_hours(1.0);
        assert_eq!(
            date.timestamp().to_string(),
            "2026-11-01T08:00:00Z",
            "the earlier 1 am (PDT)"
        );
    }
}
