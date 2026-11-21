//! Which day a date detected in a message means, read against the day the message was sent: what
//! its hover shows and its click starts an event on.

use jiff::{ToSpan as _, Zoned, civil::Date};

use crate::time_suggest;

/// The day `text` (a detected date's own text) means in a message sent at `sent`, if any.
pub(crate) fn resolve(text: &str, sent: &Zoned) -> Option<Date> {
    let parts = exo_mail::date_parts(text)?;
    let today = sent.date();
    if let Some(business) = &parts.business {
        return match business.to_lowercase().as_str() {
            "eod" | "eob" => Some(today),
            // The week's last day (Saturday), less a day.
            "eow" => today
                .checked_add((6 - i64::from(today.weekday().to_sunday_zero_offset()) - 1).days())
                .ok(),
            "eom" => Some(today.last_of_month()),
            "eoq" => Date::new(today.year(), ((today.month() - 1) / 3 + 1) * 3, 1)
                .ok()
                .map(|date| date.last_of_month()),
            "eoy" => Date::new(today.year(), 12, 31).ok(),
            _ => None,
        };
    }
    let month = parts.month.as_deref().map(|month| {
        let month = month.trim_end_matches('.').to_lowercase();
        if month == "sept" {
            "sep".to_owned()
        } else {
            month
        }
    });
    let mut has_year = parts.year.is_some();
    // Day, month and year, from the ISO timestamp or the written date.
    let mut found: Option<(Option<i16>, i8, i8)> = None;
    if let Some(iso) = &parts.iso_timestamp {
        has_year = true;
        found = Date::strptime("%Y-%m-%d", iso.get(..10)?)
            .ok()
            .map(|date| (Some(date.year()), date.month(), date.day()));
    } else if let Some(written) = parts.long_date.as_ref().or(parts.short_date.as_ref()) {
        let pieces: Vec<String> = written
            .replace(['(', ')'], "")
            .split(['.', ',', '/', '\\', '-'])
            .map(|piece| piece.trim().to_owned())
            .collect();
        let piece = |at: usize| pieces.get(at).filter(|piece| !piece.is_empty());
        has_year = piece(2).is_some();
        let year = piece(2).and_then(|year| year_of(year));
        found = if parts.long_date.is_some() && piece(0).is_some_and(|first| first.len() >= 3) {
            Some((
                piece(0).and_then(|year| year.parse().ok()),
                piece(1)?.parse().ok()?,
                piece(2)?.parse().ok()?,
            ))
        } else {
            // en-US writes the month first; a first number over 12 can only be a day.
            let (mut month, mut day): (i8, i8) = (piece(0)?.parse().ok()?, piece(1)?.parse().ok()?);
            if month > 12 && day <= 12 {
                (month, day) = (day, month);
            }
            Some((year, month, day))
        };
    }
    if let (Some(month), Some(dom)) = (&month, &parts.dom) {
        found = Some((
            parts.year.as_deref().and_then(year_of),
            month_number(month)?,
            dom.parse().ok()?,
        ));
    }
    if let Some((year, month, day)) = found {
        if has_year {
            return Date::new(year?, month, day).ok();
        }
        // Without a year: last year's within the past two weeks, else this year's unless it's more
        // than two weeks gone, then next year's.
        let on = |year: i16| Date::new(year, month, day).ok();
        let days = |date: Date| (date - today).get_days();
        let (previous, current, next) =
            (on(today.year() - 1), on(today.year()), on(today.year() + 1));
        if let Some(previous) = previous.filter(|date| (-14..=0).contains(&days(*date))) {
            return Some(previous);
        }
        return match current {
            Some(current) if days(current) < -14 => next.or(Some(current)),
            Some(current) => Some(current),
            None => next,
        };
    }
    // Weekdays and words (`tomorrow`, `next Friday`) go through the time parser, from the start
    // of the day; `last` is the coming one less a week.
    let midnight = sent.start_of_day().ok()?;
    let lower = text.to_lowercase();
    let (text, back) = match (
        parts.relative.as_deref().map(str::to_lowercase).as_deref(),
        parts.dow.is_some(),
    ) {
        (Some("last"), true) => (lower.replacen("last ", "", 1), true),
        _ => (lower, false),
    };
    let suggestion = time_suggest::suggest(&text, &time_suggest::Options::default(), &midnight)
        .into_iter()
        .next()?;
    let date = suggestion.start.to_zoned(sent.time_zone().clone()).date();
    if back {
        date.checked_sub(1.week()).ok()
    } else {
        Some(date)
    }
}

/// What a date detected in a draft says moved `days` on from the day it means today, counted from
/// `now`'s day: `today`, `tomorrow`, its weekday up to six days on (`on Friday` if it said `on`
/// before a weekday or word), `next Friday` up to ten, else `November 25th` (`, 2027` in another
/// year), as for a day gone by.
pub(crate) fn shift(text: &str, days: i64, now: &Zoned) -> Option<String> {
    let day = resolve(text, now)?.checked_add(days.days()).ok()?;
    let parts = exo_mail::date_parts(text)?;
    // `uses_on` comes only from the parse of weekdays and words, and only from a lower-case `on`.
    let by_words = parts.business.is_none()
        && parts.iso_timestamp.is_none()
        && parts.long_date.is_none()
        && parts.short_date.is_none()
        && !(parts.month.is_some() && parts.dom.is_some());
    let uses_on = by_words && parts.relative.as_deref() == Some("on");
    let today = now.date();
    let weekday = day.strftime("%A").to_string();
    Some(match (day - today).get_days() {
        0 => "today".to_owned(),
        1 => "tomorrow".to_owned(),
        2..=6 if uses_on => format!("on {weekday}"),
        2..=6 => weekday,
        7..=10 => format!("next {weekday}"),
        _ if day.year() != today.year() => format!(
            "{} {}, {}",
            day.strftime("%B"),
            crate::format::dates::ordinal(day.day()),
            day.year()
        ),
        _ => format!(
            "{} {}",
            day.strftime("%B"),
            crate::format::dates::ordinal(day.day())
        ),
    })
}

/// A written year: two digits are this century's.
fn year_of(year: &str) -> Option<i16> {
    let parsed: i16 = year.parse().ok()?;
    Some(if year.len() <= 2 {
        2000 + parsed
    } else {
        parsed
    })
}

fn month_number(month: &str) -> Option<i8> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    MONTHS
        .iter()
        .position(|name| month.starts_with(name))
        .map(|at| at as i8 + 1)
}

#[cfg(test)]
mod tests {
    use jiff::civil::Weekday;

    use super::*;

    fn on(text: &str) -> Option<String> {
        let sent: Zoned = "2026-11-17T09:00:00[America/New_York]"
            .parse()
            .expect("valid");
        resolve(text, &sent).map(|date| date.to_string())
    }

    #[test]
    fn reads_days_against_the_message() {
        assert_eq!(on("tomorrow").as_deref(), Some("2026-11-18"));
        assert_eq!(on("today").as_deref(), Some("2026-11-17"));
        assert_eq!(on("Wed Nov 25").as_deref(), Some("2026-11-25"));
        assert_eq!(on("Dec 9").as_deref(), Some("2026-12-09"));
        assert_eq!(
            on("Nov 3").as_deref(),
            Some("2026-11-03"),
            "two weeks back is still this year"
        );
        assert_eq!(
            on("Oct 1").as_deref(),
            Some("2027-10-01"),
            "further back is next year's"
        );
        assert_eq!(on("11/25/2026").as_deref(), Some("2026-11-25"));
        assert_eq!(on("2026-12-01").as_deref(), Some("2026-12-01"));
        assert_eq!(on("EOW").as_deref(), Some("2026-11-20"));
        assert_eq!(on("EOM").as_deref(), Some("2026-11-30"));
        let friday = on("on Friday").expect("a day");
        assert_eq!(
            friday.parse::<Date>().expect("a date").weekday(),
            Weekday::Friday
        );
        let last = on("last Friday").expect("a day");
        assert_eq!(last, "2026-11-13");
    }

    #[test]
    fn shifts_a_date_and_writes_it_relative_to_today() {
        let now: Zoned = "2026-11-17T09:00:00[America/New_York]"
            .parse()
            .expect("valid");
        let shift = |text: &str, days: i64| shift(text, days, &now);
        assert_eq!(shift("today", 1).as_deref(), Some("tomorrow"));
        assert_eq!(shift("tomorrow", -1).as_deref(), Some("today"));
        assert_eq!(shift("tomorrow", 1).as_deref(), Some("Thursday"));
        assert_eq!(shift("on Friday", 1).as_deref(), Some("on Saturday"));
        assert_eq!(
            shift("On Friday", 1).as_deref(),
            Some("Saturday"),
            "the `on` check is case-sensitive"
        );
        assert_eq!(shift("Monday", 1).as_deref(), Some("next Tuesday"));
        assert_eq!(shift("Nov 25", 1).as_deref(), Some("next Thursday"));
        assert_eq!(shift("Nov 27", 1).as_deref(), Some("November 28th"));
        assert_eq!(shift("today", -1).as_deref(), Some("November 16th"));
        assert_eq!(shift("Dec 31", 1).as_deref(), Some("January 1st, 2027"));
        assert_eq!(shift("EOD", 1).as_deref(), Some("tomorrow"));
    }
}
