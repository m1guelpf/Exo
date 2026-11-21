//! Turning a parse into a date and its label: the three interpreters, and the time of day, span,
//! duration and time zone every one of them applies.

use std::rc::Rc;

use super::{
    Engine, MINUTE, Parsed, Parts, ZoneTime,
    date::JsDate,
    grammar::Interpreter,
    words::{Kind, Tag, Token, js_number},
    zones::{self, InvalidZone},
};

const HOUR: f64 = 60.0 * MINUTE;
const DAY: f64 = 24.0 * HOUR;
/// Someday: a date far enough out to sort last.
const SOMEDAY: i64 = 4_000_000_000_000;
const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
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
/// The unit pairs that read naturally (`2 hours 30 minutes`); any other costs confidence.
const NATURAL: [(&str, &str); 7] = [
    ("hours", "minutes"),
    ("days", "hours"),
    ("weeks", "days"),
    ("months", "days"),
    ("months", "weeks"),
    ("years", "weeks"),
    ("years", "months"),
];

pub(super) fn interpret(
    engine: &Engine,
    interpreter: Interpreter,
    tokens: Vec<Token>,
) -> Result<Parsed, InvalidZone> {
    match interpreter {
        Interpreter::Relative => relative(engine, tokens),
        Interpreter::Subsequent => subsequent(engine, tokens),
        Interpreter::Absolute => absolute(engine, tokens),
    }
}

/// JavaScript's truthiness for a number: not 0 and not NaN.
fn truthy(number: f64) -> bool {
    number != 0.0 && !number.is_nan()
}

/// `in 3 days`, `2h`, `a week`: now plus the amounts, at the typed or default time unless hours
/// or minutes were added, which keep the clock's.
fn relative(engine: &Engine, tokens: Vec<Token>) -> Result<Parsed, InvalidZone> {
    let mut date = engine.now.clone();
    let mut clock = Clock::new(engine, &tokens);
    let mut parts = Parts {
        date: true,
        time: true,
        ..Parts::default()
    };
    let (mut quantity, mut spelled_a) = (1.0, true);
    let (mut minutes, mut hours, mut days, mut weekdays, mut months, mut years) =
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    let mut label = String::new();
    let mut confidence = 1.0;
    let mut last_typed = None;
    let mut last_unit: Option<Rc<str>> = None;
    for (at, token) in tokens.iter().enumerate() {
        if !token.guessed {
            last_typed = Some(token.kind);
        }
        match token.kind {
            Kind::A => (quantity, spelled_a) = (1.0, true),
            Kind::All => parts.all_day = true,
            Kind::Quantity if !matches!(token.tag, Some(Tag::Duration | Tag::DurationAnd)) => {
                (quantity, spelled_a) = (token.parsed, false)
            }
            Kind::Unit => {
                let unit = &*token.value;
                if !quantity.is_nan() {
                    match unit {
                        "minutes" => minutes += quantity,
                        "hours" => hours += quantity,
                        "days" => weekdays += quantity,
                        "weeks" => days += 7.0 * quantity,
                        "fortnights" => {
                            confidence *= 0.1;
                            days += 14.0 * quantity;
                        }
                        "months" => months += quantity,
                        "years" => years += quantity,
                        _ => {}
                    }
                    label.push_str(if label.is_empty() {
                        "in "
                    } else if tokens.len() == at + 1 {
                        " and "
                    } else {
                        ", "
                    });
                    match (spelled_a, unit) {
                        (true, "hours") => label.push_str("an "),
                        (true, _) => label.push_str("a "),
                        (false, _) => label.push_str(&format!("{} ", js_number(quantity))),
                    }
                    let one = quantity == 1.0 || spelled_a;
                    if unit == "days" {
                        label.push_str(if engine.options.business_days_only {
                            "weekday"
                        } else {
                            "day"
                        });
                        if !one {
                            label.push('s');
                        }
                    } else {
                        label.push_str(if one {
                            unit.strip_suffix('s').unwrap_or(unit)
                        } else {
                            unit
                        });
                    }
                    spelled_a = true;
                }
                if let Some(last) = &last_unit
                    && !NATURAL.contains(&(&**last, unit))
                {
                    confidence *= 0.01;
                }
                last_unit = Some(token.value.clone());
            }
            _ => {}
        }
    }
    if last_typed == Some(Kind::And) {
        confidence *= 0.01;
    }
    if tokens
        .last()
        .is_some_and(|token| token.kind == Kind::Quantity)
    {
        confidence *= 0.01;
    }
    if label.is_empty() {
        confidence *= 0.0;
    }
    let target = (date.month() as f64 + months) % 12.0;
    date.set_month(date.month() as f64 + months + 12.0 * years);
    // The fraction of a month, to one decimal: half a month is half the previous month's days.
    let tenths: f64 = format!("{:.1}", months % 1.0)
        .get(2..)
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(0.0);
    if tenths != 0.0 {
        let mut previous = date.clone();
        previous.set_date(0.0);
        days = (tenths / 10.0 * previous.date() as f64).trunc();
    } else if date.month() as f64 != target {
        date.set_date(0.0);
    }
    if engine.options.business_days_only {
        date.set_date(date.date() as f64 + days);
        let mut left = weekdays;
        while left > 0.0 {
            date.set_date(date.date() as f64 + 1.0);
            if !matches!(date.weekday(), 0 | 6) {
                left -= 1.0;
            }
        }
    } else {
        date.set_date(date.date() as f64 + days + weekdays);
    }
    if truthy(minutes) || truthy(hours) {
        date.set_minutes(date.minutes() as f64 + minutes + 60.0 * hours);
        date.zero_seconds();
    } else {
        clock.apply(engine, &mut date)?;
    }
    let label = clock.format(label);
    Ok(clock.finish(
        engine,
        Parsed {
            label,
            date,
            confidence,
            duration: None,
            zone: None,
            parts,
            tokens,
        },
        true,
    ))
}

/// `tomorrow`, `fri`, `next month`, `q3`, `2027`, `tonight`, a time alone: the next such day or
/// period, at the typed or default time.
fn subsequent(engine: &Engine, tokens: Vec<Token>) -> Result<Parsed, InvalidZone> {
    let now = &engine.now;
    let skip_prepositions = engine.options.skip_prepositions;
    let mut date = now.clone();
    let mut clock = Clock::new(engine, &tokens);
    let saw_time = clock.has_seen_time();
    let mut parts = Parts::default();
    let (mut next, mut dated, mut date_guessed) = (false, false, false);
    let mut label: Vec<String> = Vec::new();
    let add_days = |date: &mut JsDate, days: i64| date.set_date((date.date() + days) as f64);
    for token in &tokens {
        let value = &*token.value;
        match token.kind {
            Kind::Next => next = true,
            Kind::Hour if !token.suggested => parts.time = true,
            Kind::Timespan => {
                if next {
                    match value {
                        "week" => {
                            let ahead = 8 - date.weekday();
                            add_days(&mut date, ahead);
                        }
                        "month" => {
                            date.set_date(1.0);
                            date.set_month((date.month() + 1) as f64);
                        }
                        "quarter" => {
                            let quarter = (date.month() / 3 + 1) * 3 % 12;
                            if quarter == 0 {
                                date.set_full_year((date.year() + 1) as f64);
                            }
                            date.set_date(1.0);
                            date.set_month(quarter as f64);
                        }
                        "year" => {
                            date.set_date(1.0);
                            date.set_month(0.0);
                            date.set_full_year((date.year() + 1) as f64);
                        }
                        _ => {}
                    }
                    label.push(format!("next {value}"));
                }
                (dated, date_guessed) = (true, token.suggested);
            }
            Kind::Dayname => {
                let today = if now.weekday() == 0 { 7 } else { now.weekday() };
                let day = if token.parsed == 0.0 {
                    7
                } else {
                    token.parsed as i64
                };
                if next {
                    let ahead = if day >= today {
                        day - today + 7
                    } else if today == 7 {
                        if day == 6 { 6 } else { day + 7 }
                    } else if today - day > 3 {
                        day - today + 14
                    } else {
                        day - today + 7
                    };
                    add_days(&mut date, ahead);
                    label.push("next ".into());
                } else {
                    add_days(
                        &mut date,
                        if today < day {
                            day - today
                        } else {
                            7 - today + day
                        },
                    );
                    if !skip_prepositions {
                        label.push("on ".into());
                    }
                }
                label.push(capitalize(value));
                (dated, date_guessed) = (true, token.suggested);
            }
            Kind::Monthname => {
                let (this_month, month) = (now.month(), token.parsed as i64);
                date.set_date(1.0);
                date.set_month(month as f64);
                // Next year, or the year after for `next` a month more than 8 months back.
                let years = if next && this_month - month > 8 {
                    2
                } else {
                    i64::from(next || month <= this_month)
                };
                date.set_full_year((date.year() + years) as f64);
                if next {
                    label.extend(["next ".into(), capitalize(value)]);
                } else {
                    label.extend(["to ".into(), format!("{} ", capitalize(value))]);
                }
                (dated, date_guessed) = (true, token.suggested);
            }
            Kind::Quartername => {
                let (current, quarter) = (now.month() / 3 + 1, token.parsed as i64 + 1);
                date.set_date(1.0);
                date.set_month(((quarter - 1) * 3) as f64);
                if next || current >= quarter {
                    date.set_full_year((date.year() + 1) as f64);
                }
                if next {
                    label.extend(["next ".into(), format!("Q{quarter}")]);
                } else {
                    if !skip_prepositions {
                        label.push("in ".into());
                    }
                    label.push(format!("Q{quarter} "));
                }
                (dated, date_guessed) = (true, token.suggested);
            }
            Kind::Year => {
                date.set_full_year(value.parse().unwrap_or(f64::NAN));
                if !dated {
                    date.set_month(0.0);
                    date.set_date(1.0);
                }
                label.push(value.to_owned());
            }
            Kind::Tomorrow => {
                add_days(&mut date, 1);
                label.push("tomorrow".into());
                (dated, date_guessed) = (true, token.suggested);
            }
            Kind::Tonight | Kind::Today => {
                label.push("today".into());
                (dated, date_guessed) = (true, token.suggested);
            }
            Kind::Someday => {
                label.push(value.to_owned());
                date.set_time(SOMEDAY);
                parts.all_day = true;
            }
            Kind::All => parts.all_day = true,
            _ => {}
        }
    }
    clock.apply(engine, &mut date)?;
    parts.date |= dated && !date_guessed;
    if clock.has_seen_time() && !dated {
        // A time alone: today if it's still to come.
        if ((date.ms() - now.ms()).abs() as f64 / HOUR).ceil() > 24.0 {
            label.push("tomorrow".into());
            add_days(&mut date, 2);
        } else if date.ms() <= now.ms() {
            label.push("tomorrow".into());
            add_days(&mut date, 1);
        } else {
            label.push("today".into());
        }
    }
    if (parts.all_day || clock.all_day) && dated {
        date.set_hours(0.0);
        date.set_minutes(0.0);
    }
    // A time later today doesn't lose confidence for being hours away, so `3pm` beats
    // `tomorrow at 3 pm`.
    let mut confidence = 1.0;
    if saw_time && label.last().is_some_and(|last| last == "today") && date.ms() >= now.ms() {
        confidence /= engine.date_confidence(&date);
    }
    let label = clock.format(label.concat());
    Ok(clock.finish(
        engine,
        Parsed {
            label,
            date,
            confidence,
            duration: None,
            zone: None,
            parts,
            tokens,
        },
        dated,
    ))
}

/// `aug 7`, `8/7`, `the 25th`, `2026-12-01`: the date as written, next year when it has passed
/// and no year was typed.
fn absolute(engine: &Engine, tokens: Vec<Token>) -> Result<Parsed, InvalidZone> {
    let now = &engine.now;
    let mut date = now.clone();
    let mut clock = Clock::new(engine, &tokens);
    let mut parts = Parts {
        date: true,
        ..Parts::default()
    };
    let mut positions: Vec<(Kind, usize)> = Vec::new();
    let (mut day, mut dayname, mut suffix) = (None, None, None);
    let (mut year_typed, mut day_set, mut overflowed) = (false, false, false);
    let mut confidence = 1.0;
    for (at, token) in tokens.iter().enumerate() {
        if token.value.is_empty() {
            continue;
        }
        let parsed = token.parsed;
        match positions.iter_mut().find(|(kind, _)| *kind == token.kind) {
            Some(position) => position.1 = at + 1,
            None => positions.push((token.kind, at + 1)),
        }
        match token.kind {
            Kind::Year => {
                let mut year = if parsed < 2000.0 {
                    parsed + 2000.0
                } else {
                    parsed
                };
                if overflowed && date.month() == 0 {
                    year += 1.0;
                }
                let before = date.date();
                date.set_full_year(year);
                overflowed |= date.date() != before;
                year_typed = !token.guessed;
            }
            Kind::Month if overflowed => date.set_month(parsed + 1.0),
            Kind::Month => {
                if !day_set {
                    date.set_date(1.0);
                }
                date.set_month(parsed);
                overflowed |= date.month() as f64 != parsed;
            }
            Kind::Day => {
                day = Some(parsed);
                date.set_date(parsed);
                overflowed |= date.date() as f64 != parsed;
                day_set = true;
            }
            Kind::Dayname => dayname = Some(parsed),
            Kind::Th => suffix = Some(token.value.clone()),
            Kind::Hour => parts.time = true,
            Kind::All => parts.all_day = true,
            _ => {}
        }
    }
    let position = |kind: Kind| {
        positions
            .iter()
            .find(|(other, _)| *other == kind)
            .map(|(_, at)| *at)
    };
    if overflowed {
        date.set_date(0.0);
    }
    if date.ms() < now.ms() {
        if position(Kind::Day).is_some() && position(Kind::Month).is_some() && !year_typed {
            let before = date.date();
            date.set_full_year((date.year() + 1) as f64);
            if date.date() != before {
                date.set_date(0.0);
            }
        } else if position(Kind::Day).is_none() && position(Kind::Month).is_none() {
            date.set_date((date.date() + 1) as f64);
        }
    }
    if day.is_some_and(|day| date.date() as f64 != day) {
        confidence *= 0.1;
    }
    if dayname.is_some_and(|dayname| !dayname.is_nan() && date.weekday() as f64 != dayname) {
        confidence *= 0.1;
    }
    if year_typed && position(Kind::Day).is_none() {
        date.set_date(1.0);
        date.set_month(0.0);
    }
    let ordinal = ordinal_suffix(date.date());
    if suffix.as_deref() == Some(ordinal) {
        confidence *= 12.0;
    }
    let day_text = format!("{}{ordinal}", date.date());
    let preposition = if engine.options.skip_prepositions {
        ""
    } else {
        "on"
    };
    let month = MONTHS[date.month() as usize];
    let mut label = match dayname {
        Some(_) => format!("{preposition} {} ", WEEKDAYS[date.weekday() as usize]),
        None => format!("{preposition} "),
    };
    let day_first = position(Kind::Dayname).is_some()
        || matches!((position(Kind::Day), position(Kind::Month)), (Some(day), Some(month)) if day < month);
    label.push_str(&if day_first {
        format!("{day_text} {month}")
    } else {
        format!("{month} {day_text}")
    });
    if year_typed || date.year() != now.year() {
        label.push_str(&format!(" {}", date.year()));
    }
    if (date.ms() - now.ms()) as f64 > 3.0 * DAY {
        confidence *= 0.8;
    }
    if position(Kind::Hour).is_some() || position(Kind::Minute).is_some() {
        confidence *= 0.1;
    }
    let label = clock.format(label).trim().to_owned();
    clock.apply(engine, &mut date)?;
    Ok(clock.finish(
        engine,
        Parsed {
            label,
            date,
            confidence,
            duration: None,
            zone: None,
            parts,
            tokens,
        },
        day_set,
    ))
}

/// `text` with its first lower-case letter upper-cased.
fn capitalize(text: &str) -> String {
    match text.find(|ch: char| ch.is_ascii_lowercase()) {
        Some(at) => format!(
            "{}{}{}",
            &text[..at],
            text[at..=at].to_ascii_uppercase(),
            &text[at + 1..]
        ),
        None => text.to_owned(),
    }
}

/// A day's ordinal suffix: 1st, 2nd, 3rd, 21st, 22nd, 23rd, 31st, else th.
fn ordinal_suffix(day: i64) -> &'static str {
    match day {
        1 | 21 | 31 => "st",
        2 | 22 => "nd",
        3 | 23 => "rd",
        _ => "th",
    }
}

/// One end of a span, or the time when there's none.
#[derive(Clone, Default)]
struct Endpoint {
    hours: Option<f64>,
    minutes: Option<f64>,
    am_pm: Option<Rc<str>>,
    /// How many digits the hour was typed with (`1530` is 4).
    hour_digits: Option<usize>,
    daylight: Option<Rc<str>>,
}

/// What a parse said about the time.
#[derive(Default)]
struct Seen {
    span: bool,
    day: bool,
    /// Hours or minutes as amounts (`in 2 hours`).
    amount: bool,
    am_pm: bool,
    hours: bool,
    minutes: bool,
    daylight: bool,
    timezone: bool,
    main_duration_unit: bool,
    duration_for: bool,
}

/// The time of day, span, duration and time zone of a parse.
#[derive(Default)]
struct Clock {
    seen: Seen,
    start: Endpoint,
    end: Endpoint,
    all_day: bool,
    zone: Option<ZoneTime>,
    zone_token: Option<Rc<str>>,
    quantity: Option<f64>,
    unit: Option<Rc<str>>,
    and_quantity: Option<f64>,
    and_unit: Option<Rc<str>>,
    hours: f64,
    minutes: f64,
    /// Milliseconds; 0 or NaN for none.
    duration: f64,
}

impl Clock {
    fn new(engine: &Engine, tokens: &[Token]) -> Self {
        let mut clock = Self::default();
        for token in tokens.iter().filter(|token| !token.value.is_empty()) {
            let in_span = clock.seen.span;
            let value = &token.value;
            let end = if in_span {
                &mut clock.end
            } else {
                &mut clock.start
            };
            match token.kind {
                Kind::For => clock.seen.duration_for = true,
                Kind::Span => clock.seen.span = true,
                Kind::Hour => {
                    let (hours, minutes) = if value.len() == 4 {
                        ((token.parsed / 100.0).floor(), token.parsed % 100.0)
                    } else {
                        (token.parsed, 0.0)
                    };
                    clock.seen.hours |= !in_span;
                    clock.seen.minutes |= truthy(minutes);
                    (end.hours, end.minutes, end.hour_digits) =
                        (Some(hours), Some(minutes), Some(value.len()));
                }
                Kind::Minute => {
                    end.minutes = Some(token.parsed);
                    clock.seen.minutes |= !in_span;
                }
                Kind::Am => {
                    end.am_pm = Some(value.clone());
                    clock.seen.am_pm |= !in_span;
                }
                Kind::Timezone => {
                    clock.zone_token = Some(value.clone());
                    clock.seen.timezone = true;
                }
                Kind::Dayname | Kind::Day => clock.seen.day |= !token.guessed,
                Kind::Tomorrow | Kind::This | Kind::Today => clock.seen.day = true,
                Kind::Tonight => {
                    end.daylight = Some("night".into());
                    clock.seen.daylight = true;
                    clock.seen.day = true;
                }
                Kind::Quantity => match token.tag {
                    Some(Tag::Duration) => clock.quantity = Some(token.parsed),
                    Some(Tag::DurationAnd) => clock.and_quantity = Some(token.parsed),
                    _ => {}
                },
                Kind::Unit if &**value == "days" && !token.guessed => clock.seen.day = true,
                Kind::Unit => clock.seen.amount |= matches!(&**value, "minutes" | "hours"),
                Kind::DurationUnit => {
                    if token.tag == Some(Tag::And) {
                        clock.and_unit = Some(value.clone());
                    } else {
                        clock.seen.main_duration_unit = !token.suggested;
                        clock.unit = Some(value.clone());
                    }
                    clock.all_day |=
                        matches!(&**value, "days" | "weeks" | "months" | "years" | "wks");
                }
                Kind::Daylight => {
                    end.daylight = Some(value.clone());
                    clock.seen.daylight = true;
                }
                Kind::All => {
                    clock.all_day = true;
                    clock.seen.main_duration_unit = true;
                }
                _ => {}
            }
        }
        let (hours, minutes, duration) = if clock.seen.span {
            clock.span()
        } else {
            let (hours, minutes) = clock.time(engine);
            (hours, Some(minutes), clock.parse_duration())
        };
        clock.hours = hours;
        clock.duration = duration;
        clock.minutes = if hours != 24.0 || engine.calendar() {
            minutes.unwrap_or(0.0).round()
        } else {
            0.0
        };
        clock
    }

    /// The start's time, with the default for what wasn't typed: a one-digit hour before 8 is in
    /// the afternoon, a daylight word alone has its hour.
    fn time(&self, engine: &Engine) -> (f64, f64) {
        let start = &self.start;
        let (default_hours, default_minutes) = engine.options.default_time;
        let mut hours = start.hours.unwrap_or(f64::from(default_hours));
        let minutes = start.minutes.unwrap_or(f64::from(default_minutes));
        if start.hour_digits == Some(1) && hours < 8.0 {
            hours += 12.0;
        }
        if let Some(daylight) = start.daylight.as_deref()
            && start.hours.is_none()
        {
            hours = match daylight {
                "morning" => 8.0,
                "afternoon" => 13.0,
                _ => 20.0,
            };
        }
        if let Some(am_pm) = start.am_pm.as_deref() {
            hours = adjust_for_am_pm(hours, am_pm);
        }
        (hours, minutes)
    }

    /// `2-3pm`, `3pm to 5pm`: the start and the duration, with AM or PM inferred so that the end
    /// comes after the start.
    fn span(&self) -> (f64, Option<f64>, f64) {
        let mut ends = [self.start.clone(), self.end.clone()];
        for end in &mut ends {
            if end.hour_digits.is_some_and(|digits| digits > 2) {
                continue;
            }
            if end.hours.is_some_and(|hours| hours > 12.0) {
                end.am_pm = Some("pm".into());
            }
            let hours = end.hours.unwrap_or(0.0) % 12.0;
            end.hours = Some(
                end.am_pm
                    .as_deref()
                    .map_or(hours, |am_pm| adjust_for_am_pm(hours, am_pm)),
            );
        }
        let millis =
            |end: &Endpoint| end.hours.unwrap_or(0.0) * HOUR + end.minutes.unwrap_or(0.0) * MINUTE;
        let [mut start, mut end] = ends;
        let shift =
            |end: &mut Endpoint, hours: f64| end.hours = Some(end.hours.unwrap_or(0.0) + hours);
        match (start.am_pm.is_some(), end.am_pm.is_some()) {
            (true, false) => {
                for _ in 0..2 {
                    if millis(&end) < millis(&start) {
                        shift(&mut end, 12.0);
                    }
                }
            }
            (false, true) => {
                if millis(&end) <= millis(&start) {
                    shift(&mut end, 24.0);
                }
                if millis(&end) - millis(&start) > 12.0 * HOUR {
                    shift(&mut start, 12.0);
                }
            }
            (true, true) if millis(&end) <= millis(&start) => shift(&mut end, 24.0),
            _ => {}
        }
        (
            start.hours.unwrap_or(0.0),
            start.minutes,
            millis(&end) - millis(&start),
        )
    }

    /// `for 30 min`, `2 days`: a bare number over 5 is minutes, else hours; all-day durations round
    /// up to whole days.
    fn parse_duration(&self) -> f64 {
        if self.all_day {
            let quantity = self
                .quantity
                .filter(|quantity| truthy(*quantity))
                .unwrap_or(1.0)
                .ceil();
            let mut days = (days_per(self.unit.as_deref().unwrap_or("days")) * quantity).ceil();
            if let (Some(and_quantity), Some(and_unit)) = (
                self.and_quantity.filter(|quantity| truthy(*quantity)),
                self.and_unit.as_deref(),
            ) {
                days += (days_per(and_unit) * and_quantity).ceil();
            }
            return days * DAY;
        }
        let quantity = self.quantity.unwrap_or(0.0);
        let unit = self
            .unit
            .as_deref()
            .unwrap_or(if quantity > 5.0 { "minutes" } else { "hours" });
        let mut millis = to_millis(quantity, unit);
        if let Some(and_quantity) = self.and_quantity.filter(|quantity| truthy(*quantity)) {
            millis += to_millis(and_quantity, self.and_unit.as_deref().unwrap_or("minutes"));
        }
        millis
    }

    /// Sets `date`'s time: midnight is 23:59, a night before 6 is the next day's. A time zone moves
    /// it by the zone's offset from here (the first one seen for that name).
    fn apply(&mut self, engine: &Engine, date: &mut JsDate) -> Result<(), InvalidZone> {
        if self.seen.hours && self.hours == 24.0 {
            date.set_hours(23.0);
            date.set_minutes(59.0);
        } else {
            if self.start.daylight.as_deref() == Some("night") && self.hours < 6.0 {
                date.set_date((date.date() + 1) as f64);
            }
            date.set_hours(self.hours);
            date.set_minutes(self.minutes);
        }
        if let Some(token) = self.zone_token.clone() {
            let typed = date.clone();
            let offset = engine.zone_offset(date, &token)?;
            let whole = offset.trunc();
            date.set_hours(self.hours + whole);
            date.set_minutes(self.minutes + (offset - whole) * 60.0);
            if offset != 0.0 {
                let zone = zones::resolve(&token);
                let length = if truthy(self.duration) {
                    self.duration
                } else {
                    super::DEFAULT_DURATION
                };
                self.zone = Some(ZoneTime {
                    label: zone.label,
                    tz: zone.tz?,
                    start: typed.timestamp(),
                    end: super::millis_after(typed.timestamp(), length),
                });
            }
        }
        date.zero_seconds();
        Ok(())
    }

    /// The label's time: ` at 3 pm`, ` this afternoon at 1 pm`, ` at midnight`, ` PST`,
    /// ` for 1 hour and 30 minutes`, ` all day`.
    fn format(&self, mut label: String) -> String {
        if self.all_day {
            return label + " all day";
        }
        let zone = self
            .zone_token
            .as_deref()
            .map(|token| zones::resolve(token).label);
        let zone_suffix = zone.map(|zone| format!(" {zone}")).unwrap_or_default();
        if self.seen.hours && self.hours == 24.0 {
            return format!("{label} at midnight{zone_suffix}");
        }
        if self.seen.daylight {
            let daylight = self.start.daylight.as_deref().unwrap_or("null");
            label = match label.as_str() {
                "today" if daylight == "night" => "tonight".to_owned(),
                "today" => format!("this {daylight}"),
                _ => format!("{label} {daylight}"),
            };
        }
        if self.seen.daylight || self.seen.hours || self.seen.minutes {
            let hour = if self.hours % 12.0 != 0.0 {
                self.hours % 12.0
            } else {
                12.0
            };
            let minutes = if self.seen.minutes || truthy(self.minutes) {
                format!(":{:02}", self.minutes as i64)
            } else {
                String::new()
            };
            let am_pm = if self.hours < 12.0 { " am" } else { " pm" };
            label = format!(
                "{label} at {}{minutes}{am_pm}{zone_suffix}",
                js_number(hour)
            );
        }
        if truthy(self.duration) && self.duration > 0.0 {
            let total = self.duration / MINUTE;
            let (hours, minutes) = ((total / 60.0).floor(), total % 60.0);
            let mut parts = Vec::new();
            if truthy(hours) {
                parts.push(format!(
                    "{} hour{}",
                    js_number(hours),
                    if hours > 1.0 { "s" } else { "" }
                ));
            }
            if truthy(minutes) {
                parts.push(format!(
                    "{} minute{}",
                    js_number(minutes),
                    if minutes > 1.0 { "s" } else { "" }
                ));
            }
            label = format!("{label} for {}", parts.join(" and "));
        }
        label
    }

    /// Completes an interpreter's parse: its duration in the calendar, all day at midnight
    /// (when `midnight`: it has a day), the time zone it was typed in, and the time's
    /// plausibility.
    fn finish(self, engine: &Engine, mut parsed: Parsed, midnight: bool) -> Parsed {
        let parts = &mut parsed.parts;
        if engine.calendar() {
            parts.duration = truthy(self.duration);
            parsed.duration = Some(if parts.duration {
                self.duration
            } else {
                super::DEFAULT_DURATION
            });
        }
        parts.all_day |= self.all_day;
        if parts.all_day && midnight {
            parsed.date.set_hours(0.0);
            parsed.date.set_minutes(0.0);
        }
        parsed.confidence *= self.confidence(engine);
        parsed.zone = self.zone;
        parsed
    }

    fn has_seen_time(&self) -> bool {
        self.seen.hours
            || self.seen.minutes
            || self.seen.am_pm
            || self.seen.timezone
            || self.seen.daylight
    }

    fn confidence(&self, engine: &Engine) -> f64 {
        self.time_confidence(engine) * self.duration_confidence()
    }

    /// How plausible the time is: round minutes, daytime hours, a daylight word that fits its hour,
    /// a typed day.
    fn time_confidence(&self, engine: &Engine) -> f64 {
        let seen = &self.seen;
        let start = &self.start;
        let mut confidence = 1.0;
        if !seen.hours && (seen.minutes || seen.am_pm) {
            confidence *= 0.0;
        }
        if seen.daylight {
            match start.daylight.as_deref() {
                Some("morning") if self.hours > 12.0 => confidence *= 0.0,
                Some("afternoon") if self.hours < 12.0 || self.hours > 20.0 => confidence *= 0.0,
                Some("evening") if self.hours < 15.0 => confidence *= 0.0,
                Some("evening") if self.hours != 20.0 => confidence *= 0.9,
                Some("night") if self.hours > 6.0 && self.hours < 15.0 => confidence *= 0.0,
                _ => {}
            }
        }
        if let Some(token) = &self.zone_token {
            confidence *= match (seen.hours || seen.daylight, zones::is_well_known(token)) {
                (false, _) => 0.0,
                (true, true) => 0.025,
                (true, false) => 0.01,
            };
        }
        if seen.amount && seen.hours {
            confidence *= 0.0;
        }
        if (seen.hours || seen.daylight) && !seen.day {
            confidence *= 0.1;
        }
        let default_minutes = f64::from(engine.options.default_time.1);
        if ![0.0, 15.0, 30.0, 45.0, default_minutes].contains(&self.minutes) {
            confidence *= 0.1;
        }
        if self.hours < 7.0 || self.hours > 19.0 {
            confidence *= 0.5;
        }
        if start.hour_digits == Some(4) {
            return 0.1 * confidence;
        }
        if seen.hours || seen.daylight {
            return confidence
                * (0.1
                    + 0.15
                        * (f64::from(u8::from(seen.minutes)) + f64::from(u8::from(seen.am_pm))));
        }
        confidence
    }

    /// How plausible the duration is.
    fn duration_confidence(&self) -> f64 {
        let seen = &self.seen;
        if self.all_day {
            if !seen.main_duration_unit || seen.amount || seen.hours {
                return 0.0;
            }
            return if self.unit.as_deref() == Some("days") {
                1.0
            } else {
                0.05
            };
        }
        let mut confidence = 1.0;
        let duration = self.duration;
        if truthy(duration) {
            if duration <= 0.0 {
                confidence *= 0.0;
            }
            if duration >= 12.0 * HOUR {
                confidence *= 0.4;
            }
            let bare = !(self.has_seen_time()
                || seen.span
                || seen.duration_for
                || seen.main_duration_unit);
            if bare && self.quantity.unwrap_or(0.0) <= 12.0 {
                confidence *= 0.0;
            }
        }
        let pm_after_noon =
            |end: &Endpoint| end.hours.is_some_and(|hours| hours > 12.0) && end.am_pm.is_some();
        if pm_after_noon(&self.start) || pm_after_noon(&self.end) {
            confidence *= 0.0;
        }
        let quantity = self.quantity.is_some_and(truthy);
        let and_quantity = self.and_quantity.is_some_and(truthy);
        if !quantity && and_quantity {
            confidence *= 0.0;
        }
        if (!quantity && self.unit.is_some()) || (!and_quantity && self.and_unit.is_some()) {
            confidence *= 0.0;
        }
        confidence
    }
}

fn adjust_for_am_pm(hours: f64, am_pm: &str) -> f64 {
    match am_pm {
        "pm" if hours < 12.0 => hours + 12.0,
        "am" if hours >= 12.0 => hours - 12.0,
        _ => hours,
    }
}

/// `quantity` `unit`s in milliseconds; NaN for a unit it doesn't know (`wks`).
fn to_millis(quantity: f64, unit: &str) -> f64 {
    quantity
        * match unit {
            "minutes" => MINUTE,
            "hours" => HOUR,
            "days" => DAY,
            "weeks" => 7.0 * DAY,
            "months" => 28.0 * DAY,
            "years" => 12.0 * 28.0 * DAY,
            _ => f64::NAN,
        }
}

/// The days one `unit` of an all-day duration counts for (an hour 24, a minute 0.4); NaN for `wks`.
fn days_per(unit: &str) -> f64 {
    match unit {
        "minutes" => 24.0 / 60.0,
        "hours" => 24.0,
        "days" => 1.0,
        "weeks" => 7.0,
        "months" => 30.0,
        "years" => 365.0,
        _ => f64::NAN,
    }
}
