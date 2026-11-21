//! Words and their readings: splitting the input, and every reading each word can have by prefix.
//! `t` reads as `tomorrow`, `today`, `tuesday`, `thursday`, `the`, `this`, …; the grammar decides
//! which make sense.

use std::rc::Rc;

use super::zones;

/// What a reading is, named as the grammar names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Quantity,
    Hour,
    Day,
    /// The word `day` (`all day`).
    DayWord,
    Month,
    Minute,
    Year,
    Timezone,
    DurationUnit,
    Unit,
    Timespan,
    Dayname,
    Monthname,
    Quartername,
    All,
    Am,
    Th,
    This,
    A,
    And,
    The,
    Next,
    Tomorrow,
    Today,
    Tonight,
    Daylight,
    Colon,
    Of,
    Time,
    Someday,
    Slash,
    On,
    At,
    Span,
    In,
    For,
}

impl Kind {
    pub(super) fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "quantity" => Self::Quantity,
            "hour" => Self::Hour,
            "day" => Self::Day,
            "dayword" => Self::DayWord,
            "month" => Self::Month,
            "minute" => Self::Minute,
            "year" => Self::Year,
            "timezone" => Self::Timezone,
            "duration_unit" => Self::DurationUnit,
            "unit" => Self::Unit,
            "timespan" => Self::Timespan,
            "dayname" => Self::Dayname,
            "monthname" => Self::Monthname,
            "quartername" => Self::Quartername,
            "all" => Self::All,
            "am" => Self::Am,
            "th" => Self::Th,
            "this" => Self::This,
            "a" => Self::A,
            "and" => Self::And,
            "the" => Self::The,
            "next" => Self::Next,
            "tomorrow" => Self::Tomorrow,
            "today" => Self::Today,
            "tonight" => Self::Tonight,
            "daylight" => Self::Daylight,
            "colon" => Self::Colon,
            "of" => Self::Of,
            "time" => Self::Time,
            "someday" => Self::Someday,
            "slash" => Self::Slash,
            "on" => Self::On,
            "at" => Self::At,
            "span" => Self::Span,
            "in" => Self::In,
            "for" => Self::For,
            _ => return None,
        })
    }
}

/// A grammar element's tag, which tells the duration's quantities and units apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tag {
    Duration,
    DurationAnd,
    And,
}

/// One reading of a word, or a reading the grammar guessed past the end of the input.
#[derive(Clone, Debug)]
pub(super) struct Token {
    pub kind: Kind,
    /// The vocabulary's word (`monday`, `pm`, `hours`), or the typed number; `""` for a guessed
    /// quantity, which the interpreters mostly skip.
    pub value: Rc<str>,
    /// Its number: the typed value, a list position (Sunday 0, January 0, `qone` 0), or an
    /// hour such as `1530`.
    pub parsed: f64,
    /// A reading's own weight (spelled-out numbers), multiplied into the parse's.
    pub confidence: Option<f64>,
    /// Matched by an optional element.
    pub optional: bool,
    pub tag: Option<Tag>,
    /// Completed past the end of the input without a `>` asking for it.
    pub guessed: bool,
    /// Completed past the end of the input.
    pub suggested: bool,
}

impl Token {
    fn new(kind: Kind, value: impl Into<Rc<str>>, parsed: f64) -> Self {
        Self {
            kind,
            value: value.into(),
            parsed,
            confidence: None,
            optional: false,
            tag: None,
            guessed: false,
            suggested: false,
        }
    }
}

const DAYNAMES: [&str; 7] = [
    "sunday",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
];
const MONTHNAMES: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];
/// One to twenty.
const NUMBERS: [&str; 20] = [
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
    "twenty",
];
/// The tens, and `hundred`.
const TENS: [&str; 10] = [
    "ten", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety", "hundred",
];
/// Hours by name, as four digits.
const HOUR_WORDS: [(&str, &str); 9] = [
    ("dawn", "0600"),
    ("breakfast", "0800"),
    ("midday", "1200"),
    ("noon", "1200"),
    ("lunch", "1200"),
    ("teatime", "1500"),
    ("dinner", "1900"),
    ("dusk", "2100"),
    ("midnight", "2400"),
];
/// Ordinals for days.
const ORDINALS: [&str; 20] = [
    "first",
    "second",
    "third",
    "fourth",
    "fifth",
    "sixth",
    "seventh",
    "eighth",
    "ninth",
    "tenth",
    "eleventh",
    "twelfth",
    "thirteenth",
    "fourteenth",
    "fifteenth",
    "sixteenth",
    "seventeenth",
    "eighteenth",
    "nineteenth",
    "twentieth",
];
/// Months by number.
const MONTH_NUMBERS: [&str; 21] = [
    "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "01", "02", "03", "04", "05",
    "06", "07", "08", "09",
];
/// Hours by number.
const HOUR_NUMBERS: [&str; 34] = [
    "1", "2", "3", "4", "5", "6", "7", "8", "9", "00", "01", "02", "03", "04", "05", "06", "07",
    "08", "09", "10", "11", "12", "13", "14", "15", "16", "17", "18", "19", "20", "21", "22", "23",
    "24",
];
/// Days by number.
const DAY_NUMBERS: [&str; 40] = [
    "1", "2", "3", "4", "5", "6", "7", "8", "9", "01", "02", "03", "04", "05", "06", "07", "08",
    "09", "10", "11", "12", "13", "14", "15", "16", "17", "18", "19", "20", "21", "22", "23", "24",
    "25", "26", "27", "28", "29", "30", "31",
];

/// The suggestors, in the order each word's readings are listed.
const SUGGESTORS: [Kind; 35] = [
    Kind::Quantity,
    Kind::Hour,
    Kind::Day,
    Kind::Month,
    Kind::Minute,
    Kind::Year,
    Kind::Timezone,
    Kind::DurationUnit,
    Kind::Unit,
    Kind::Timespan,
    Kind::Dayname,
    Kind::Monthname,
    Kind::Quartername,
    Kind::All,
    Kind::Am,
    Kind::Th,
    Kind::This,
    Kind::A,
    Kind::And,
    Kind::The,
    Kind::Next,
    Kind::Tomorrow,
    Kind::Today,
    Kind::Tonight,
    Kind::Daylight,
    Kind::Colon,
    Kind::Of,
    Kind::Time,
    Kind::Someday,
    Kind::Slash,
    Kind::On,
    Kind::At,
    Kind::Span,
    Kind::In,
    Kind::For,
];

/// The words a plain vocabulary suggestor matches, by mode.
fn vocabulary(kind: Kind, calendar: bool) -> &'static [&'static str] {
    match kind {
        Kind::DurationUnit => &[
            "minutes", "hours", "mins", "hrs", "days", "wks", "weeks", "months", "years",
        ],
        Kind::Unit => &[
            "minutes",
            "mins",
            "hours",
            "hrs",
            "days",
            "weeks",
            "fortnights",
            "months",
            "years",
        ],
        Kind::Timespan => &["week", "month", "quarter", "year"],
        Kind::Dayname => &DAYNAMES,
        Kind::Monthname => &MONTHNAMES,
        Kind::Quartername => &["qone", "qtwo", "qthree", "qfour"],
        Kind::All => &["all", "full"],
        Kind::Am => &["am", "pm"],
        Kind::Th => &["st", "nd", "rd", "th"],
        Kind::This => &["this"],
        Kind::A => &["a", "an"],
        Kind::And => &["and", "&"],
        Kind::The => &["the"],
        Kind::Next => &["next"],
        Kind::Tomorrow => &["tomorrow", "tomorow", "tmrw"],
        Kind::Today => &["today"],
        Kind::Tonight => &["tonight"],
        Kind::Daylight => &["morning", "afternoon", "evening", "night"],
        Kind::Colon => &[":"],
        Kind::Of => &["of"],
        Kind::Time => &["time"],
        Kind::Someday => &["whenever", "meh", "shrug"],
        Kind::Slash => &["/", "\\", "-", "–"],
        Kind::On if calendar => &["on", "this", "until"],
        Kind::On => &["on", "this", "until", "to"],
        Kind::At if calendar => &["at", "@", "from"],
        Kind::At => &["at", "@"],
        Kind::Span if calendar => &["to", "til", "until", "-", "–"],
        Kind::In if calendar => &["in", "after"],
        Kind::In => &["in", "after", "for"],
        Kind::For if calendar => &["for"],
        _ => &[],
    }
}

/// What the suggestors need to know.
pub(super) struct Vocabulary {
    /// Calendar mode: its prepositions, no `forty`, short zone names.
    pub calendar: bool,
    /// Now's year, which years are typed around.
    pub year: i64,
}

impl Vocabulary {
    /// Every reading of `word` (with `number` its [`parse_float`]) by the suggestor for `kind`.
    /// A grammar element past the end of the input is completed with `readings(kind, "", 0)`.
    pub(super) fn readings(&self, kind: Kind, word: &str, number: f64) -> Vec<Token> {
        match kind {
            Kind::Quantity => self.quantities(word, number),
            Kind::Hour => hours(word, number),
            Kind::Day => days(word, number),
            Kind::Month if number.is_nan() || word.is_empty() => self
                .readings(Kind::Monthname, word, number)
                .into_iter()
                .map(|token| Token {
                    kind: Kind::Month,
                    ..token
                })
                .collect(),
            Kind::Month => prefixed(word, &MONTH_NUMBERS)
                .map(|value| Token::new(Kind::Month, value, parse_int(value) - 1.0))
                .collect(),
            Kind::Minute => minutes(word, number),
            Kind::Year => self.years(word, number),
            Kind::Timezone => {
                // A single letter in the calendar only reads as an abbreviation of up to four.
                let longest = if self.calendar && word.len() == 1 {
                    4
                } else {
                    usize::MAX
                };
                zones::names()
                    .iter()
                    .enumerate()
                    .filter(|(_, name)| name.len() <= longest && name.starts_with(word))
                    .map(|(at, name)| Token::new(Kind::Timezone, name.as_str(), at as f64))
                    .collect()
            }
            Kind::DayWord => Vec::new(),
            _ => {
                let words = vocabulary(kind, self.calendar);
                prefixed(word, words)
                    .map(|value| {
                        let value = match value {
                            "hrs" => "hours",
                            "mins" => "minutes",
                            value => value,
                        };
                        Token::new(
                            kind,
                            value,
                            words
                                .iter()
                                .position(|word| *word == value)
                                .map_or(-1.0, |at| at as f64),
                        )
                    })
                    .collect()
            }
        }
    }

    /// Numbers under 1000 as typed, or spelled out (`two`, `thirty`; `forty` isn't one in the
    /// calendar), which cost confidence: a lot when the word was complete and another matched.
    fn quantities(&self, word: &str, number: f64) -> Vec<Token> {
        if number.is_nan() && !word.is_empty() {
            let exact = NUMBERS.contains(&word);
            let mut spelled: Vec<(&str, f64)> = NUMBERS
                .iter()
                .enumerate()
                .map(|(at, name)| (*name, at as f64 + 1.0))
                .collect();
            spelled.extend(
                TENS.iter()
                    .enumerate()
                    .skip(2)
                    .filter(|(at, _)| !(self.calendar && *at == 3))
                    .map(|(at, name)| (*name, (at as f64 + 1.0) * 10.0)),
            );
            return spelled
                .into_iter()
                .filter(|(name, _)| name.starts_with(word))
                .map(|(name, parsed)| Token {
                    confidence: Some(if exact && word != name { 5e-4 } else { 0.05 }),
                    ..Token::new(Kind::Quantity, name, parsed)
                })
                .collect();
        }
        if !number.is_nan() && number < 1000.0 && word.len() < 4 && !word.starts_with('0') {
            return vec![Token::new(Kind::Quantity, word, number)];
        }
        Vec::new()
    }

    /// Two- and four-digit years near now's: `2027`, or what `2`, `20`, `27` could become.
    fn years(&self, word: &str, number: f64) -> Vec<Token> {
        if number.is_nan() {
            return Vec::new();
        }
        let near = word.len() == 4 && (number - self.year as f64).abs() < 10.0;
        let short = self.year % 100;
        let candidates: Vec<String> = if near {
            vec![word.to_owned()]
        } else {
            [
                short,
                short + 1,
                short + 2,
                self.year,
                self.year + 1,
                self.year + 2,
            ]
            .iter()
            .map(i64::to_string)
            .collect()
        };
        candidates
            .into_iter()
            .filter(|value| near || value.starts_with(word))
            .map(|value| {
                let parsed = parse_int(&value);
                Token::new(Kind::Year, value, parsed)
            })
            .collect()
    }
}

/// Hours: names (`noon` is `1200`), four digits as `HHMM`, three as `HHM0` and `HHM5`, `1` and
/// `2` with their afternoon hours, and any other number with its prefix completions and, up to
/// 12, its afternoon hour.
fn hours(word: &str, number: f64) -> Vec<Token> {
    let token = |value: String| {
        let parsed = parse_int(&value);
        Token::new(Kind::Hour, value, parsed)
    };
    if number.is_nan() && !word.is_empty() {
        return HOUR_WORDS
            .iter()
            .filter(|(name, _)| name.starts_with(word))
            .map(|(_, hour)| token((*hour).to_owned()))
            .collect();
    }
    match word.len() {
        4.. => (number <= 2400.0 && number % 100.0 < 60.0)
            .then(|| token(word.to_owned()))
            .into_iter()
            .collect(),
        3 if number > 240.0 => Vec::new(),
        3 if number == 240.0 => vec![token(format!("{word}0"))],
        3 => vec![token(format!("{word}0")), token(format!("{word}5"))],
        _ if word == "1" || word == "2" => {
            vec![token(js_number(number)), token(js_number(number + 12.0))]
        }
        _ => {
            let mut values: Vec<String> =
                prefixed(word, &HOUR_NUMBERS).map(str::to_owned).collect();
            if number <= 12.0 && number > 0.0 {
                values.push(js_number(number + 12.0));
            }
            values.into_iter().map(token).collect()
        }
    }
}

/// Days: numbers, ordinals (`fifth`, at a cost), and the word `day`.
fn days(word: &str, number: f64) -> Vec<Token> {
    if number.is_nan() && !word.is_empty() {
        if "day".starts_with(word) {
            return vec![Token::new(Kind::DayWord, "day", 0.0)];
        }
        return ORDINALS
            .iter()
            .enumerate()
            .filter(|(_, name)| name.starts_with(word))
            .map(|(at, name)| Token {
                confidence: Some(0.05),
                ..Token::new(Kind::Day, *name, at as f64 + 1.0)
            })
            .collect();
    }
    if number.is_nan() {
        return Vec::new();
    }
    prefixed(word, &DAY_NUMBERS)
        .map(|value| Token::new(Kind::Day, value, parse_int(value)))
        .collect()
}

/// Minutes: two digits under 60 as typed; one digit `n` under 6 as `n0` and `n5`, else `0n`.
fn minutes(word: &str, number: f64) -> Vec<Token> {
    if number.is_nan() || word.len() > 2 || number >= 60.0 || number != number.round() {
        return Vec::new();
    }
    let values = match word.len() {
        2 => vec![word.to_owned()],
        0 => vec!["00".to_owned(), "30".to_owned()],
        _ if number < 6.0 => vec![format!("{word}0"), format!("{word}5")],
        _ => vec![format!("0{word}")],
    };
    values
        .into_iter()
        .map(|value| {
            let parsed = parse_int(&value);
            Token::new(Kind::Minute, value, parsed)
        })
        .collect()
}

fn prefixed<'a>(word: &'a str, words: &'a [&'a str]) -> impl Iterator<Item = &'a str> {
    words
        .iter()
        .copied()
        .filter(move |candidate| candidate.starts_with(word))
}

/// Every reading of every word of `text`. A number that isn't the last word only reads as itself
/// (or an hour 12 later); the last may be a prefix of more (`3` → `30`).
pub(super) fn tokenize(text: &str, vocabulary: &Vocabulary) -> Vec<Vec<Token>> {
    let words = join_places(split(text));
    let mut after_quantity = false;
    words
        .iter()
        .enumerate()
        .map(|(at, word)| {
            let number = parse_float(word);
            let mut word = if word == "weds" { "wed" } else { word.as_str() };
            if at > 0 && after_quantity && (word == "m" || word == "mi") {
                word = "minutes";
            }
            after_quantity = false;
            let last = at + 1 == words.len();
            let mut readings = Vec::new();
            for kind in SUGGESTORS {
                for token in vocabulary.readings(kind, word, number) {
                    after_quantity |= token.kind == Kind::Quantity;
                    let value = parse_float(&token.value);
                    let hour = kind == Kind::Hour && (value == number || value == number + 12.0);
                    if number.is_nan() || value == number || last || hour {
                        readings.push(token);
                    }
                }
            }
            readings
        })
        .collect()
}

/// The input's words, lower-cased: numbers (with one decimal, `1.5`), letter runs, and
/// `: / \ - @ & –`; anything else separates.
fn split(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.to_lowercase().chars().collect();
    let mut words = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let digits = chars[at..]
            .iter()
            .take_while(|ch| ch.is_ascii_digit())
            .count();
        let after = at + digits;
        let decimal = chars.get(after) == Some(&'.')
            && chars.get(after + 1).is_some_and(char::is_ascii_digit)
            && !chars.get(after + 2).is_some_and(char::is_ascii_digit);
        let len = if decimal {
            digits + 2
        } else if digits > 0 {
            digits
        } else if chars[at].is_ascii_lowercase() {
            chars[at..]
                .iter()
                .take_while(|ch| ch.is_ascii_lowercase())
                .count()
        } else if matches!(chars[at], ':' | '/' | '\\' | '-' | '@' | '&' | '–') {
            1
        } else {
            at += 1;
            continue;
        };
        words.push(chars[at..at + len].iter().collect());
        at += len;
    }
    words
}

/// Joins the first multi-word zone name in `words` (`new york` → `new_york`) and drops a `time`
/// (or a prefix of it) after a place (`london time`).
fn join_places(mut words: Vec<String>) -> Vec<String> {
    let first = words
        .iter()
        .filter(|word| *word != "st")
        .find_map(|word| zones::starting_with(word).map(|names| (word.clone(), names)));
    let time_follows = |words: &[String], at: usize| {
        words
            .get(at)
            .is_some_and(|word| "time".starts_with(word.as_str()))
    };
    if let Some((first, names)) = first {
        let start = words
            .iter()
            .position(|word| *word == first)
            .unwrap_or_default();
        let mut joined = first;
        let mut len = 1;
        for word in &words[start + 1..] {
            let longer = format!("{joined}_{word}");
            if !names.iter().any(|name| name.contains(&longer)) {
                break;
            }
            joined = longer;
            len += 1;
        }
        if zones::is_place(&joined) && time_follows(&words, start + len) {
            words.remove(start + len);
        }
        words.splice(start..start + len, [joined]);
    } else if words.len() >= 2
        && zones::is_place(&words[words.len() - 2])
        && time_follows(&words, words.len() - 1)
    {
        words.pop();
    }
    words
}

/// A word's or a reading's value as a number: its leading decimal number, else NaN.
fn parse_float(text: &str) -> f64 {
    let digits = text
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(text.len());
    let fraction = text[digits..].strip_prefix('.').map_or(0, |rest| {
        rest.find(|ch: char| !ch.is_ascii_digit())
            .unwrap_or(rest.len())
    });
    let len = if fraction > 0 {
        digits + 1 + fraction
    } else {
        digits
    };
    text[..len].parse().unwrap_or(f64::NAN)
}

/// The number `text`'s leading digits make, else NaN.
fn parse_int(text: &str) -> f64 {
    let digits = text
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(text.len());
    text[..digits].parse().unwrap_or(f64::NAN)
}

/// A number as JavaScript writes it (`3`, `1.5`).
pub(super) fn js_number(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e21 {
        format!("{}", number as i64)
    } else {
        format!("{number}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_words_and_joins_places() {
        assert_eq!(split("Tom 3PM, 1.5h"), ["tom", "3", "pm", "1.5", "h"]);
        assert_eq!(
            split("1.55 .5 2026-12-01 @noon"),
            ["1", "55", ".5", "2026", "-", "12", "-", "01", "@", "noon"]
        );
        assert_eq!(
            join_places(split("9am new york time")),
            ["9", "am", "new_york"]
        );
        assert_eq!(join_places(split("3pm london t")), ["3", "pm", "london"]);
    }

    #[test]
    fn numbers_parse_as_javascript_does() {
        assert_eq!(parse_float("1.50"), 1.5);
        assert_eq!(parse_float(".5"), 0.5);
        assert!(parse_float("qone").is_nan());
        assert_eq!(parse_int("0800"), 800.0);
        assert_eq!(js_number(1.5), "1.5");
        assert_eq!(js_number(30.0), "30");
    }
}
