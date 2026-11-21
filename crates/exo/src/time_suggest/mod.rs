//! The natural-language time parser: free text such as `tom 3pm`, `fri 10am for 1 hour`, `nov 5`
//! or `2 weeks` becomes up to three dated suggestions, each with its label. Jump to Date and the
//! calendar's When field use it.
//!
//! It is a small grammar engine: every word gets every reading its prefix allows, every rule
//! that can take those readings yields a parse, and each parse is scored for how plausible its
//! date and time are. Its answers, quirks included, hang on every step being exact, down to the
//! setter arithmetic of JavaScript's `Date` (`date`).

mod date;
mod grammar;
mod interpret;
mod words;
mod zones;

pub(crate) use zones::tables_for_search as zone_tables;

use std::{cell::RefCell, collections::HashMap};

use jiff::{Timestamp, Zoned, tz::TimeZone};

use date::JsDate;
use grammar::Rule;
use words::{Kind, Token, Vocabulary};
use zones::InvalidZone;

const MINUTE: f64 = 60_000.0;
/// A calendar suggestion's length when none was typed.
const DEFAULT_DURATION: f64 = 30.0 * MINUTE;

/// How a call site parses.
#[derive(Clone, Debug)]
pub struct Options {
    /// The calendar's grammar: durations (`for 30 min`), spans (`2-3pm`) and `all day`, and
    /// three variants of the best parse.
    pub calendar: bool,
    /// The time of day when none was typed, hours and minutes (7:00, Remind me's 8:00).
    pub default_time: (i8, i8),
    /// Keep dates in the past.
    pub allow_past: bool,
    /// No `on`/`in` in labels: `August 7th`, `Q3`.
    pub skip_prepositions: bool,
    /// Days are weekdays: `in 3 weekdays`.
    pub business_days_only: bool,
    /// The locale writes the month before the day, so `8/7` is August 7th. Exo's dates are en-US.
    pub month_first: bool,
    /// What blank input lists: each phrase's best parse.
    pub defaults: &'static [&'static str],
}

impl Default for Options {
    fn default() -> Self {
        Self {
            calendar: false,
            default_time: (7, 0),
            allow_past: false,
            skip_prepositions: false,
            business_days_only: false,
            month_first: true,
            defaults: &[],
        }
    }
}

/// A suggestion: a label and the time it stands for.
#[derive(Clone, Debug)]
pub struct Suggestion {
    /// What the picker shows: `tomorrow at 3 pm`, `on Friday`, `in 2 weeks`.
    pub label: String,
    pub start: Timestamp,
    /// The calendar's: when it ends, after a typed or default duration.
    pub end: Option<Timestamp>,
    /// A calendar suggestion for whole days (`all day`, `2 days`).
    pub all_day: bool,
    /// The time as typed in another zone (`2pm in new york`), when that moved it.
    pub zone: Option<ZoneTime>,
}

/// A time typed in another zone.
#[derive(Clone, Debug)]
pub struct ZoneTime {
    /// How the label names the zone: `PST`, `in London`.
    pub label: String,
    /// The zone; `None` reads as this one.
    pub tz: Option<TimeZone>,
    /// The start and end read with this zone's clock: their wall time here is the typed time there.
    pub start: Timestamp,
    pub end: Timestamp,
}

/// Up to three suggestions for `text` at `now`, in `now`'s time zone.
pub fn suggest(text: &str, options: &Options, now: &Zoned) -> Vec<Suggestion> {
    let engine = Engine {
        options,
        now: JsDate::new(now.timestamp().as_millisecond(), now.time_zone().clone()),
        offsets: RefCell::default(),
    };
    let parsed = if text.trim().is_empty() && !options.defaults.is_empty() {
        options
            .defaults
            .iter()
            .filter_map(|phrase| engine.suggest(phrase).ok()?.into_iter().next())
            .collect()
    } else {
        engine.suggest(text).unwrap_or_default()
    };
    parsed
        .into_iter()
        .filter(|parsed| {
            !options.calendar || parsed.duration.is_none_or(|duration| duration >= 0.0)
        })
        .map(|parsed| Suggestion {
            label: parsed.label,
            start: parsed.date.timestamp(),
            end: parsed
                .duration
                .map(|duration| millis_after(parsed.date.timestamp(), duration)),
            all_day: parsed.parts.all_day,
            zone: parsed.zone,
        })
        .collect()
}

/// What a parse says it has.
#[derive(Clone, Copy, Debug, Default)]
struct Parts {
    date: bool,
    time: bool,
    duration: bool,
    all_day: bool,
}

impl Parts {
    fn len(self) -> usize {
        [self.date, self.time, self.duration, self.all_day]
            .into_iter()
            .filter(|part| *part)
            .count()
    }
}

/// An interpreted parse.
#[derive(Clone, Debug)]
struct Parsed {
    label: String,
    date: JsDate,
    confidence: f64,
    /// The calendar's duration, milliseconds.
    duration: Option<f64>,
    zone: Option<ZoneTime>,
    parts: Parts,
    tokens: Vec<Token>,
}

/// One call's state.
struct Engine<'a> {
    options: &'a Options,
    now: JsDate,
    /// Zones' offsets from here in hours, by token, from the first date that needed one.
    offsets: RefCell<HashMap<String, f64>>,
}

impl Engine<'_> {
    const fn calendar(&self) -> bool {
        self.options.calendar
    }

    /// The suggestions for `text`, past the blank-input defaults.
    fn suggest(&self, text: &str) -> Result<Vec<Parsed>, InvalidZone> {
        let text = preprocess(text);
        // `330` and `1530` are also read as `3:30` and `15:30`, whose best two lead.
        let compact = compact_times(&text);
        let early = match compact.filter(|compact| *compact != text) {
            Some(compact) => self.parse(&compact)?.into_iter().take(2).collect(),
            None => Vec::new(),
        };
        let mut results: Vec<Parsed> = self.parse(&text)?.into_iter().take(3).collect();
        if !early.is_empty() {
            let mut merged = early;
            merged.extend(results.into_iter().take(1));
            dedup_by(&mut merged, |a, b| a.date.ms() == b.date.ms());
            dedup_by(&mut merged, |a, b| a.label == b.label);
            results = merged;
        }
        if self.calendar() {
            results = variants(results);
        }
        Ok(results)
    }

    /// Every parse of `text` under every rule, interpreted, scored and filtered.
    fn parse(&self, text: &str) -> Result<Vec<Parsed>, InvalidZone> {
        let vocabulary = Vocabulary {
            calendar: self.calendar(),
            year: self.now.year(),
        };
        let words = words::tokenize(text, &vocabulary);
        let mut results = Vec::new();
        let mut best = f64::NEG_INFINITY;
        for rule in grammar::rules(self.calendar(), self.options.month_first) {
            for tokens in grammar::parses(rule, &words, &vocabulary) {
                let mut parsed = interpret::interpret(self, rule.interpreter, tokens)?;
                // In this order: the products round differently in another.
                parsed.confidence *= rule.confidence;
                parsed.confidence *= self.date_confidence(&parsed.date);
                parsed.confidence *= token_confidence(&parsed.tokens);
                best = best.max(if parsed.confidence.is_nan() {
                    0.0
                } else {
                    parsed.confidence
                });
                if !overlaps_textually(rule, &parsed) {
                    results.push(parsed);
                }
            }
        }
        Ok(self.output(results, best))
    }

    /// The parses worth showing: in the future unless the past is allowed, within a third of the
    /// best parse's confidence, one per label (the earliest), then most confident first, one per
    /// date (and duration, in the calendar).
    fn output(&self, mut results: Vec<Parsed>, best: f64) -> Vec<Parsed> {
        results.retain(|parsed| {
            (self.options.allow_past || parsed.date.ms() >= self.now.ms())
                && parsed.confidence > best / 3.0
        });
        results.sort_by_key(|parsed| parsed.date.ms());
        dedup_by(&mut results, |a, b| a.label == b.label);
        results.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
        if self.calendar() {
            dedup_by(&mut results, |a, b| {
                a.date.ms() == b.date.ms() && a.duration == b.duration
            });
        } else {
            dedup_by(&mut results, |a, b| a.date.ms() == b.date.ms());
        }
        results
    }

    /// How likely a date this far away is meant: the next few days most, the past hardly at all.
    fn date_confidence(&self, date: &JsDate) -> f64 {
        let mut days = (date.ms() - self.now.ms()) as f64 / 86_400_000.0;
        if self.options.allow_past {
            days = days.abs();
        }
        let high = |days: f64| 0.8 * (-0.08 * days).exp();
        let low = |days: f64| days.powf(0.25) * (1.0 - (days + 6.0).powf(0.25)).exp();
        if days < 0.0 {
            1e-5
        } else if days < 5.0 {
            high(days)
        } else {
            low(days) * high(5.0) / low(5.0)
        }
    }

    /// The offset in hours from the zone `token` names to this one at `date`.
    fn zone_offset(&self, date: &JsDate, token: &str) -> Result<f64, InvalidZone> {
        if let Some(offset) = self.offsets.borrow().get(token) {
            return Ok(*offset);
        }
        let at = date.ms() - date.ms().rem_euclid(1000);
        let here = date.time_zone();
        let there = zones::resolve(token).tz?.unwrap_or_else(|| here.clone());
        let wall = JsDate::new(at, here.clone())
            .timestamp()
            .to_zoned(there)
            .datetime();
        let offset = (at - JsDate::from_wall(wall, here.clone()).ms()) as f64 / (60.0 * MINUTE);
        self.offsets.borrow_mut().insert(token.to_owned(), offset);
        Ok(offset)
    }
}

/// The input as the engine reads it: `1` and `a` (and blank) as `in 1`/`in a`, aliases
/// (`tod`, `tn`, `af`, `12 noon`, `q3`), `next mon` as Monday.
fn preprocess(text: &str) -> String {
    let mut chars = text.chars();
    let first = chars.next();
    let mut text =
        if matches!(first, None | Some('a' | '1' | '2' | ' ')) && chars.all(|ch| ch == ' ') {
            format!("in {text}")
        } else {
            text.to_owned()
        };
    for (from, to) in [
        ("12 noon", "noon"),
        ("12 midnight", "midnight"),
        ("0000", "midnight"),
        ("000", "midnight"),
        ("q1", "qone"),
        ("q2", "qtwo"),
        ("q3", "qthree"),
        ("q4", "qfour"),
    ] {
        text = text.to_lowercase().replacen(from, to, 1);
    }
    match text.to_lowercase().trim() {
        "tod" | "toda" => text = "today".to_owned(),
        "nig" | "nigh" | "night" | "tn" => text = "tonight".to_owned(),
        _ => {}
    }
    if "after".starts_with(text.as_str()) && text != "a" {
        text = "afternoon".to_owned();
    }
    if text.trim().eq_ignore_ascii_case("next mon") {
        text = "next monday".to_owned();
    }
    text
}

/// The input with each word that starts with an `HMM` or `HHMM` time (hours under 24,
/// minutes under 60) written `H:MM`, or `None` when no word starts with three or four digits.
fn compact_times(text: &str) -> Option<String> {
    fn leading(word: &str) -> Option<&str> {
        let digits = word
            .find(|ch: char| !ch.is_ascii_digit())
            .unwrap_or(word.len());
        matches!(digits, 3 | 4).then(|| &word[..digits])
    }
    text.split_whitespace()
        .any(|word| leading(word).is_some())
        .then(|| {
            text.split(' ')
                .map(|word| {
                    let Some(digits) = leading(word) else {
                        return word.to_owned();
                    };
                    let (hours, minutes) = digits.split_at(digits.len() - 2);
                    match (hours.parse::<u8>(), minutes.parse::<u8>()) {
                        (Ok(hours_value), Ok(minutes_value))
                            if hours_value < 24 && minutes_value < 60 =>
                        {
                            word.replacen(digits, &format!("{hours}:{minutes}"), 1)
                        }
                        _ => word.to_owned(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
}

/// "after … pm" parsed as a time zone's preposition with no AM/PM typed.
fn overlaps_textually(rule: &Rule, parsed: &Parsed) -> bool {
    rule.am_then_in
        && parsed.label.contains("pm")
        && parsed.tokens.iter().any(|token| &*token.value == "after")
        && !parsed.tokens.iter().any(|token| token.kind == Kind::Am)
}

/// How sure the words are: each reading's own weight, less for words the grammar guessed or that
/// an optional element took.
fn token_confidence(tokens: &[Token]) -> f64 {
    let (mut count, mut guessed, mut optional, mut weight) = (0.0, 0.0, 0.0, 1.0);
    for token in tokens {
        count += 1.0;
        guessed += 0.75 * f64::from(u8::from(token.guessed));
        optional += 0.9 * f64::from(u8::from(token.optional));
        weight *= token
            .confidence
            .filter(|weight| *weight != 0.0 && !weight.is_nan())
            .unwrap_or(1.0);
    }
    weight * (count - guessed - optional) / count
}

/// Keeps the first of each run of `same` items, wherever they are.
fn dedup_by<T>(items: &mut Vec<T>, same: impl Fn(&T, &T) -> bool) {
    let mut kept: Vec<T> = Vec::with_capacity(items.len());
    for item in items.drain(..) {
        if !kept.iter().any(|other| same(other, &item)) {
            kept.push(item);
        }
    }
    *items = kept;
}

/// The calendar's three options from its best parse: a date and time for 30 minutes, an hour or 15
/// minutes; a date at 10, 13 and all day; a time today, tomorrow and the day after; … The labels
/// stay the best parse's.
fn variants(results: Vec<Parsed>) -> Vec<Parsed> {
    let Some(best) = results.first() else {
        return results;
    };
    let parts = best.parts;
    if parts.len() >= 3 && parts.date && parts.duration && (parts.all_day || parts.time) {
        return results;
    }
    let mut options = [best.clone(), best.clone(), best.clone()];
    let at = |parsed: &mut Parsed, hours: f64| {
        parsed.date.set_hours(hours);
        parsed.date.set_minutes(0.0);
    };
    let later = |parsed: &mut Parsed, days: i64| {
        parsed.date.set_date((parsed.date.date() + days) as f64);
        if let Some(zone) = &mut parsed.zone {
            let mut start =
                JsDate::new(zone.start.as_millisecond(), parsed.date.time_zone().clone());
            start.set_date((start.date() + days) as f64);
            zone.start = start.timestamp();
        }
    };
    let or_default = |parsed: &mut Parsed| {
        parsed.duration = Some(
            parsed
                .duration
                .filter(|duration| *duration != 0.0 && !duration.is_nan())
                .unwrap_or(DEFAULT_DURATION),
        )
    };
    let [a, b, c] = &mut options;
    if parts.date && parts.time && !parts.all_day {
        a.duration = Some(30.0 * MINUTE);
        b.duration = Some(60.0 * MINUTE);
        c.duration = Some(15.0 * MINUTE);
    } else if parts.date && parts.duration && !parts.all_day {
        at(a, 10.0);
        at(b, 13.0);
        at(c, 17.0);
    } else if parts.len() == 1 && parts.time || parts.len() == 2 && parts.time && parts.duration {
        for parsed in [&mut *a, &mut *b, &mut *c] {
            or_default(parsed);
        }
        later(b, 1);
        later(c, 2);
    } else if parts.len() == 1 && parts.duration {
        at(a, 10.0);
        at(b, 13.0);
        at(c, 17.0);
    } else if parts.len() == 2 && parts.all_day && parts.duration {
        for (days, parsed) in [&mut *a, &mut *b, &mut *c].into_iter().enumerate() {
            or_default(parsed);
            later(parsed, days as i64);
            at(parsed, 0.0);
            parsed.parts.all_day = true;
        }
    } else {
        // A date alone, or anything else: 10:00, 13:00, and the whole day.
        or_default(a);
        at(a, 10.0);
        or_default(b);
        at(b, 13.0);
        c.duration = Some(24.0 * 60.0 * MINUTE);
        at(c, 0.0);
        c.parts.all_day = true;
    }
    for parsed in &mut options {
        if let (Some(zone), Some(duration)) = (&mut parsed.zone, parsed.duration) {
            zone.end = millis_after(zone.start, duration);
        }
    }
    options.into()
}

fn millis_after(start: Timestamp, millis: f64) -> Timestamp {
    Timestamp::from_millisecond(start.as_millisecond() + millis as i64).unwrap_or(start)
}

#[cfg(test)]
mod tests;
