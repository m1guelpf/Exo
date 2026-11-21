//! The parser's test vectors, then more edges, each at its now in Los Angeles, en-US. Each line is
//! `label → local time`, best first; the calendar's add the duration in minutes, `all-day` and the
//! typed zone's time.

use jiff::Zoned;

use super::*;

/// The now `at` in Los Angeles: [`NOW`], Tuesday 2026-11-17 at 10:00, or a vector's own.
fn now(at: &str) -> Zoned {
    format!("{at}[America/Los_Angeles]").parse().expect("now")
}

const NOW: &str = "2026-11-17T10:00";

/// Remind me's and Send later's options: 8:00 when no time is typed.
fn remind() -> Options {
    Options {
        default_time: (8, 0),
        ..Options::default()
    }
}

fn business() -> Options {
    Options {
        business_days_only: true,
        ..remind()
    }
}

fn calendar() -> Options {
    Options {
        calendar: true,
        allow_past: true,
        ..Options::default()
    }
}

fn jump() -> Options {
    Options {
        defaults: &["next week", "next month", "in 3 months"],
        allow_past: true,
        skip_prepositions: true,
        ..Options::default()
    }
}

fn show(text: &str, options: &Options, at_now: &str) -> Vec<String> {
    let now = now(at_now);
    let zone = now.time_zone().clone();
    let at = |ms: i64| {
        jiff::Timestamp::from_millisecond(ms)
            .expect("time")
            .to_zoned(zone.clone())
            .strftime("%a %Y-%m-%d %H:%M")
            .to_string()
    };
    suggest(text, options, &now)
        .into_iter()
        .map(|suggestion| {
            let mut line = format!(
                "{} → {}",
                suggestion.label,
                at(suggestion.start.as_millisecond())
            );
            if let Some(end) = suggestion.end.filter(|_| options.calendar) {
                let minutes = (end.as_millisecond() - suggestion.start.as_millisecond()) / 60_000;
                let all_day = if suggestion.all_day { ", all-day" } else { "" };
                let zone = suggestion
                    .zone
                    .as_ref()
                    .map(|zone| format!(", {} {}", zone.label, at(zone.start.as_millisecond())))
                    .unwrap_or_default();
                line.push_str(&format!(" ({minutes}m{all_day}{zone})"));
            }
            line
        })
        .collect()
}

/// Options, input, and the expected lines.
type Vector = (fn() -> Options, &'static str, &'static [&'static str]);
/// Options, now, input, and the expected lines.
type VectorAt = (
    fn() -> Options,
    &'static str,
    &'static str,
    &'static [&'static str],
);

/// Checks every vector at [`NOW`] and reports all the ones that differ.
fn check(vectors: &[Vector]) {
    let vectors: Vec<VectorAt> = vectors
        .iter()
        .map(|(options, text, expected)| (*options, NOW, *text, *expected))
        .collect();
    check_at(&vectors);
}

/// Checks vectors with their own nows.
fn check_at(vectors: &[VectorAt]) {
    let failures: Vec<String> = vectors
        .iter()
        .filter_map(|(options, now, text, expected)| {
            let got = show(text, &options(), now);
            (got != *expected)
                .then(|| format!("`{text}` at {now}:\n  expected {expected:?}\n  got      {got:?}"))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} differ:\n{}",
        failures.len(),
        vectors.len(),
        failures.join("\n")
    );
}

/// Times of day, alone or with a day.
#[test]
fn times() {
    check(&[
        (remind, "tomorrow", &["tomorrow → Wed 2026-11-18 08:00"]),
        (remind, "tom", &["tomorrow → Wed 2026-11-18 08:00"]),
        (remind, "tmrw", &["tomorrow → Wed 2026-11-18 08:00"]),
        (
            remind,
            "tom 3pm",
            &["tomorrow at 3 pm → Wed 2026-11-18 15:00"],
        ),
        (
            remind,
            "tomorrow at 3",
            &["tomorrow at 3 pm → Wed 2026-11-18 15:00"],
        ),
        (
            remind,
            "tomorrow morning",
            &["tomorrow morning at 8 am → Wed 2026-11-18 08:00"],
        ),
        (
            remind,
            "tomorrow afternoon",
            &["tomorrow afternoon at 1 pm → Wed 2026-11-18 13:00"],
        ),
        (
            remind,
            "tomorrow evening",
            &["tomorrow evening at 8 pm → Wed 2026-11-18 20:00"],
        ),
        (
            remind,
            "tomorrow night",
            &["tomorrow night at 8 pm → Wed 2026-11-18 20:00"],
        ),
        (
            remind,
            "tonight",
            &["tonight at 8 pm → Tue 2026-11-17 20:00"],
        ),
        (
            remind,
            "today",
            &[
                "today at 12 pm → Tue 2026-11-17 12:00",
                "today at 1 pm → Tue 2026-11-17 13:00",
                "today at 2 pm → Tue 2026-11-17 14:00",
            ],
        ),
        (
            remind,
            "this afternoon",
            &["this afternoon at 1 pm → Tue 2026-11-17 13:00"],
        ),
        (
            remind,
            "af",
            &["this afternoon at 1 pm → Tue 2026-11-17 13:00"],
        ),
        (
            remind,
            "this evening",
            &["this evening at 8 pm → Tue 2026-11-17 20:00"],
        ),
        (
            remind,
            "morning",
            &["tomorrow morning at 8 am → Wed 2026-11-18 08:00"],
        ),
        (
            remind,
            "3pm",
            &[
                "today at 3 pm → Tue 2026-11-17 15:00",
                "tomorrow at 3 pm → Wed 2026-11-18 15:00",
            ],
        ),
        (
            remind,
            "3 pm",
            &[
                "today at 3 pm → Tue 2026-11-17 15:00",
                "tomorrow at 3 pm → Wed 2026-11-18 15:00",
            ],
        ),
        (
            remind,
            "3:30",
            &[
                "today at 3:30 pm → Tue 2026-11-17 15:30",
                "tomorrow at 3:30 pm → Wed 2026-11-18 15:30",
            ],
        ),
        (
            remind,
            "330",
            &[
                "today at 3:30 pm → Tue 2026-11-17 15:30",
                "tomorrow at 3:30 pm → Wed 2026-11-18 15:30",
            ],
        ),
        (
            remind,
            "1530",
            &[
                "today at 3:30 pm → Tue 2026-11-17 15:30",
                "tomorrow at 3:30 pm → Wed 2026-11-18 15:30",
            ],
        ),
        (remind, "9am", &["tomorrow at 9 am → Wed 2026-11-18 09:00"]),
        (remind, "8 am", &["tomorrow at 8 am → Wed 2026-11-18 08:00"]),
        (
            remind,
            "8am tomorrow",
            &["tomorrow at 8 am → Wed 2026-11-18 08:00"],
        ),
        (
            remind,
            "8 am tomorrow",
            &["tomorrow at 8 am → Wed 2026-11-18 08:00"],
        ),
        (
            remind,
            "at 8",
            &[
                "tomorrow at 8 am → Wed 2026-11-18 08:00",
                "today at 8 pm → Tue 2026-11-17 20:00",
                "tomorrow at 8 pm → Wed 2026-11-18 20:00",
            ],
        ),
        (
            remind,
            "8:15",
            &[
                "tomorrow at 8:15 am → Wed 2026-11-18 08:15",
                "today at 8:15 pm → Tue 2026-11-17 20:15",
                "tomorrow at 8:15 pm → Wed 2026-11-18 20:15",
            ],
        ),
        (
            remind,
            "at 5:45pm",
            &[
                "today at 5:45 pm → Tue 2026-11-17 17:45",
                "tomorrow at 5:45 pm → Wed 2026-11-18 17:45",
            ],
        ),
        (
            remind,
            "tomorrow 830",
            &[
                "tomorrow at 8:30 am → Wed 2026-11-18 08:30",
                "tomorrow at 8:30 pm → Wed 2026-11-18 20:30",
            ],
        ),
        (
            remind,
            "noon",
            &[
                "today at 12 pm → Tue 2026-11-17 12:00",
                "tomorrow at 12 pm → Wed 2026-11-18 12:00",
            ],
        ),
        (
            remind,
            "lunch",
            &[
                "today at 12 pm → Tue 2026-11-17 12:00",
                "tomorrow at 12 pm → Wed 2026-11-18 12:00",
            ],
        ),
        (
            remind,
            "noon tomorrow",
            &["tomorrow at 12 pm → Wed 2026-11-18 12:00"],
        ),
        (
            remind,
            "dinner",
            &[
                "today at 7 pm → Tue 2026-11-17 19:00",
                "tomorrow at 7 pm → Wed 2026-11-18 19:00",
            ],
        ),
        (
            remind,
            "midnight",
            &[
                "today at midnight → Tue 2026-11-17 23:59",
                "tomorrow at midnight → Wed 2026-11-18 23:59",
            ],
        ),
        (
            remind,
            "after lunch",
            &["this afternoon at 12 pm → Tue 2026-11-17 12:00"],
        ),
    ]);
}

/// Bare numbers and amounts from now.
#[test]
fn relative() {
    check(&[
        (
            remind,
            "1",
            &[
                "in 1 minute → Tue 2026-11-17 10:01",
                "in 1 hour → Tue 2026-11-17 11:00",
                "in 1 day → Wed 2026-11-18 08:00",
            ],
        ),
        (
            remind,
            "2",
            &[
                "in 2 minutes → Tue 2026-11-17 10:02",
                "in 2 hours → Tue 2026-11-17 12:00",
                "in 2 days → Thu 2026-11-19 08:00",
            ],
        ),
        (
            remind,
            "in 2",
            &[
                "in 2 minutes → Tue 2026-11-17 10:02",
                "in 2 hours → Tue 2026-11-17 12:00",
                "in 2 days → Thu 2026-11-19 08:00",
            ],
        ),
        (
            remind,
            "3",
            &[
                "in 3 minutes → Tue 2026-11-17 10:03",
                "in 3 hours → Tue 2026-11-17 13:00",
                "in 3 days → Fri 2026-11-20 08:00",
            ],
        ),
        (
            remind,
            "7",
            &[
                "in 7 minutes → Tue 2026-11-17 10:07",
                "in 7 hours → Tue 2026-11-17 17:00",
                "in 7 days → Tue 2026-11-24 08:00",
            ],
        ),
        (
            remind,
            "9",
            &[
                "in 9 minutes → Tue 2026-11-17 10:09",
                "in 9 hours → Tue 2026-11-17 19:00",
                "in 9 days → Thu 2026-11-26 08:00",
            ],
        ),
        (
            remind,
            "12",
            &[
                "in 12 minutes → Tue 2026-11-17 10:12",
                "in 12 hours → Tue 2026-11-17 22:00",
                "in 12 days → Sun 2026-11-29 08:00",
            ],
        ),
        (
            remind,
            "a",
            &[
                "in a minute → Tue 2026-11-17 10:01",
                "in an hour → Tue 2026-11-17 11:00",
                "in a day → Wed 2026-11-18 08:00",
            ],
        ),
        (
            remind,
            "in",
            &[
                "in a minute → Tue 2026-11-17 10:01",
                "in an hour → Tue 2026-11-17 11:00",
                "in a day → Wed 2026-11-18 08:00",
            ],
        ),
        (remind, "in 2 hours", &["in 2 hours → Tue 2026-11-17 12:00"]),
        (remind, "2 hours", &["in 2 hours → Tue 2026-11-17 12:00"]),
        (remind, "2h", &["in 2 hours → Tue 2026-11-17 12:00"]),
        (
            remind,
            "in 30 min",
            &["in 30 minutes → Tue 2026-11-17 10:30"],
        ),
        (remind, "30 mins", &["in 30 minutes → Tue 2026-11-17 10:30"]),
        (remind, "30m", &["in 30 minutes → Tue 2026-11-17 10:30"]),
        (remind, "an hour", &["in an hour → Tue 2026-11-17 11:00"]),
        (
            remind,
            "in 1.5 hours",
            &["in 1.5 hours → Tue 2026-11-17 11:30"],
        ),
        (
            remind,
            "1 hour and 30 minutes",
            &["in 1 hour and 30 minutes → Tue 2026-11-17 11:30"],
        ),
        (remind, "3 days", &["in 3 days → Fri 2026-11-20 08:00"]),
        (remind, "in 3 days", &["in 3 days → Fri 2026-11-20 08:00"]),
        (
            business,
            "3 days",
            &["in 3 weekdays → Fri 2026-11-20 08:00"],
        ),
        (remind, "1.5 days", &["in 1.5 days → Wed 2026-11-18 08:00"]),
        (remind, "a week", &["in a week → Tue 2026-11-24 08:00"]),
        (remind, "2 weeks", &["in 2 weeks → Tue 2026-12-01 08:00"]),
        (
            remind,
            "fortnight",
            &["in a fortnight → Tue 2026-12-01 08:00"],
        ),
        (
            remind,
            "2 weeks 3 days",
            &["in 2 weeks and 3 days → Fri 2026-12-04 08:00"],
        ),
        (remind, "in a month", &["in a month → Thu 2026-12-17 08:00"]),
        (remind, "1 month", &["in 1 month → Thu 2026-12-17 08:00"]),
        (remind, "6 months", &["in 6 months → Mon 2027-05-17 08:00"]),
        (remind, "a year", &["in a year → Wed 2027-11-17 08:00"]),
        (
            remind,
            "next",
            &[
                "next week → Mon 2026-11-23 08:00",
                "next month → Tue 2026-12-01 08:00",
                "next quarter → Fri 2027-01-01 08:00",
            ],
        ),
    ]);
}

/// Weekdays and named periods.
#[test]
fn weekdays() {
    check(&[
        (
            remind,
            "mon",
            &[
                "on Monday → Mon 2026-11-23 08:00",
                "in a month → Thu 2026-12-17 08:00",
            ],
        ),
        (remind, "monday", &["on Monday → Mon 2026-11-23 08:00"]),
        (
            remind,
            "next monday",
            &["next Monday → Mon 2026-11-23 08:00"],
        ),
        (remind, "next mon", &["next Monday → Mon 2026-11-23 08:00"]),
        (remind, "tue", &["on Tuesday → Tue 2026-11-24 08:00"]),
        (remind, "tuesday", &["on Tuesday → Tue 2026-11-24 08:00"]),
        (remind, "next tue", &["next Tuesday → Tue 2026-11-24 08:00"]),
        (remind, "wed", &["on Wednesday → Wed 2026-11-18 08:00"]),
        (remind, "on friday", &["on Friday → Fri 2026-11-20 08:00"]),
        (remind, "next fri", &["next Friday → Fri 2026-11-27 08:00"]),
        (
            remind,
            "fri 2pm",
            &["on Friday at 2 pm → Fri 2026-11-20 14:00"],
        ),
        (
            remind,
            "friday morning",
            &["on Friday morning at 8 am → Fri 2026-11-20 08:00"],
        ),
        (
            remind,
            "monday 9am",
            &["on Monday at 9 am → Mon 2026-11-23 09:00"],
        ),
        (
            remind,
            "9 mon",
            &[
                "in 9 months → Tue 2027-08-17 08:00",
                "on Monday at 9 am → Mon 2026-11-23 09:00",
            ],
        ),
        (
            remind,
            "mon 9",
            &[
                "in a month and 9 days → Sat 2026-12-26 08:00",
                "in a month and 9 weeks → Thu 2027-02-18 08:00",
            ],
        ),
        (remind, "sat", &["on Saturday → Sat 2026-11-21 08:00"]),
        (remind, "sun", &["on Sunday → Sun 2026-11-22 08:00"]),
        (remind, "next week", &["next week → Mon 2026-11-23 08:00"]),
        (remind, "next month", &["next month → Tue 2026-12-01 08:00"]),
        (
            remind,
            "next quarter",
            &["next quarter → Fri 2027-01-01 08:00"],
        ),
        (remind, "next year", &["next year → Fri 2027-01-01 08:00"]),
        (remind, "q1", &["in Q1 2027 → Fri 2027-01-01 08:00"]),
        (
            remind,
            "q3",
            &[
                "in Q3 2027 → Thu 2027-07-01 08:00",
                "in Q3 2028 → Sat 2028-07-01 08:00",
            ],
        ),
        (
            remind,
            "next december",
            &["next December → Wed 2027-12-01 08:00"],
        ),
        (
            remind,
            "dec 2027",
            &["to December 2027 → Wed 2027-12-01 08:00"],
        ),
        (remind, "whenever", &["whenever → Tue 2096-10-02 08:00"]),
    ]);
}

/// Dates as written.
#[test]
fn dates() {
    check(&[
        (
            remind,
            "aug 7",
            &["on August 7th 2027 → Sat 2027-08-07 08:00"],
        ),
        (
            remind,
            "august 7th",
            &["on August 7th 2027 → Sat 2027-08-07 08:00"],
        ),
        (
            remind,
            "8/7",
            &["on August 7th 2027 → Sat 2027-08-07 08:00"],
        ),
        (
            remind,
            "7 aug",
            &["on 7th August 2027 → Sat 2027-08-07 08:00"],
        ),
        (
            remind,
            "7th aug",
            &["on 7th August 2027 → Sat 2027-08-07 08:00"],
        ),
        (remind, "7/8", &["on July 8th 2027 → Thu 2027-07-08 08:00"]),
        (
            remind,
            "12/25",
            &["on December 25th → Fri 2026-12-25 08:00"],
        ),
        (
            remind,
            "11/20",
            &["on November 20th → Fri 2026-11-20 08:00"],
        ),
        (
            remind,
            "11/16",
            &["on November 16th 2027 → Tue 2027-11-16 08:00"],
        ),
        (remind, "11/17", &[]),
        (
            remind,
            "11/20/2026",
            &["on November 20th 2026 → Fri 2026-11-20 08:00"],
        ),
        (
            remind,
            "2026-12-01",
            &[
                "on December 1st 2026 at 8 pm → Tue 2026-12-01 20:00",
                "on December 1st 2026 → Tue 2026-12-01 08:00",
            ],
        ),
        (
            remind,
            "aug 7 2027",
            &[
                "on August 7th 2027 at 8:27 pm → Sat 2027-08-07 20:27",
                "on August 7th 2027 → Sat 2027-08-07 08:00",
            ],
        ),
        (
            remind,
            "the 25th",
            &[
                "on 25th November → Wed 2026-11-25 08:00",
                "on 25th December → Fri 2026-12-25 08:00",
                "on 25th January 2027 → Mon 2027-01-25 08:00",
            ],
        ),
        (
            remind,
            "25th",
            &[
                "on 25th November → Wed 2026-11-25 08:00",
                "on 25th December → Fri 2026-12-25 08:00",
                "on 25th January 2027 → Mon 2027-01-25 08:00",
            ],
        ),
        (
            remind,
            "dec",
            &[
                "on December 1st → Tue 2026-12-01 08:00",
                "on December 2nd → Wed 2026-12-02 08:00",
                "on December 3rd → Thu 2026-12-03 08:00",
            ],
        ),
        (
            remind,
            "december",
            &[
                "on December 1st → Tue 2026-12-01 08:00",
                "on December 2nd → Wed 2026-12-02 08:00",
                "on December 3rd → Thu 2026-12-03 08:00",
            ],
        ),
        (
            remind,
            "2027",
            &[
                "today at 8:27 pm → Tue 2026-11-17 20:27",
                "tomorrow at 8:27 pm → Wed 2026-11-18 20:27",
                "2027 → Fri 2027-01-01 08:00",
            ],
        ),
    ]);
}

/// Times in another zone, moved to this one.
#[test]
fn time_zones() {
    check(&[
        (
            remind,
            "3pm pst",
            &["today at 3 pm PST → Tue 2026-11-17 15:00"],
        ),
        (
            remind,
            "5pm est",
            &["today at 5 pm EST → Tue 2026-11-17 14:00"],
        ),
        (
            remind,
            "9am london",
            &["tomorrow at 9 am in London → Wed 2026-11-18 01:00"],
        ),
        (
            remind,
            "9am new york",
            &["tomorrow at 9 am in New York → Wed 2026-11-18 06:00"],
        ),
    ]);
}

/// Words the grammar has no reading for (Remind me shows its fuzzy-matched rows instead).
#[test]
fn no_reading() {
    check(&[
        (remind, "someday", &[]),
        (remind, "never", &[]),
        (remind, "weekend", &[]),
        (remind, "this weekend", &[]),
        (remind, "later", &[]),
        (remind, "later today", &[]),
        (remind, "a few hours", &[]),
        (remind, "eod", &[]),
        (remind, "half", &[]),
        (remind, "3 weekdays", &[]),
        (remind, "blah", &[]),
        (remind, "x", &[]),
    ]);
}

/// The calendar's When: durations, spans, all day, and the best parse's three variants.
#[test]
fn calendar_when() {
    check(&[
        (
            calendar,
            "tomorrow 2 pm 30 min new york",
            &[
                "tomorrow at 2 pm in New York for 30 minutes → Wed 2026-11-18 11:00 (30m, in New York Wed 2026-11-18 14:00)",
            ],
        ),
        (
            calendar,
            "tomorrow 2pm",
            &[
                "tomorrow at 2 pm → Wed 2026-11-18 14:00 (30m)",
                "tomorrow at 2 pm → Wed 2026-11-18 14:00 (60m)",
                "tomorrow at 2 pm → Wed 2026-11-18 14:00 (15m)",
            ],
        ),
        (
            calendar,
            "tomorrow",
            &[
                "tomorrow → Wed 2026-11-18 10:00 (30m)",
                "tomorrow → Wed 2026-11-18 13:00 (30m)",
                "tomorrow → Wed 2026-11-18 00:00 (1440m, all-day)",
            ],
        ),
        (
            calendar,
            "fri 10am for 1 hour",
            &["on Friday at 10 am for 1 hour → Fri 2026-11-20 10:00 (60m)"],
        ),
        (
            calendar,
            "3pm to 5pm",
            &[
                "today at 3 pm for 2 hours → Tue 2026-11-17 15:00 (120m)",
                "today at 3 pm for 2 hours → Wed 2026-11-18 15:00 (120m)",
                "today at 3 pm for 2 hours → Thu 2026-11-19 15:00 (120m)",
            ],
        ),
        (
            calendar,
            "2-3pm",
            &[
                "today at 2 pm for 1 hour → Tue 2026-11-17 14:00 (60m)",
                "today at 2 pm for 1 hour → Wed 2026-11-18 14:00 (60m)",
                "today at 2 pm for 1 hour → Thu 2026-11-19 14:00 (60m)",
            ],
        ),
        (
            calendar,
            "monday all day",
            &["on Monday all day → Mon 2026-11-23 00:00 (1440m, all-day)"],
        ),
        (
            calendar,
            "30 min",
            &[
                "tomorrow for 30 minutes → Wed 2026-11-18 10:00 (30m)",
                "tomorrow for 30 minutes → Wed 2026-11-18 13:00 (30m)",
                "tomorrow for 30 minutes → Wed 2026-11-18 17:00 (30m)",
            ],
        ),
        (
            calendar,
            "lunch tomorrow",
            &[
                "tomorrow at 12 pm → Wed 2026-11-18 12:00 (30m)",
                "tomorrow at 12 pm → Wed 2026-11-18 12:00 (60m)",
                "tomorrow at 12 pm → Wed 2026-11-18 12:00 (15m)",
            ],
        ),
        (
            calendar,
            "11/16 3pm",
            &[
                "on November 16th 2027 at 3 pm → Tue 2027-11-16 15:00 (30m)",
                "on November 16th 2027 at 3 pm → Tue 2027-11-16 15:00 (60m)",
                "on November 16th 2027 at 3 pm → Tue 2027-11-16 15:00 (15m)",
            ],
        ),
        (calendar, "next week", &[]),
        (calendar, "yesterday 3pm", &[]),
    ]);
}

/// Jump to Date: the defaults when blank, no prepositions, the past allowed.
#[test]
fn jump_to_date() {
    check(&[
        (
            jump,
            "",
            &[
                "next week → Mon 2026-11-23 07:00",
                "next month → Tue 2026-12-01 07:00",
                "in 3 months → Wed 2027-02-17 07:00",
            ],
        ),
        (jump, "nov 5", &["November 5th 2027 → Fri 2027-11-05 07:00"]),
        (jump, "2 weeks", &["in 2 weeks → Tue 2026-12-01 07:00"]),
        (
            jump,
            "2028",
            &[
                "today at 8:28 pm → Tue 2026-11-17 20:28",
                "tomorrow at 8:28 pm → Wed 2026-11-18 20:28",
                "2028 → Sat 2028-01-01 07:00",
            ],
        ),
        (jump, "last friday", &[]),
    ]);
}

/// More edges: Sunday's `next`, partial words, month ends, a clock change, spans, durations, zones
/// the tables can't resolve.
#[test]
fn edges() {
    check_at(&[
        (
            remind,
            "2026-11-22T10:00",
            "sat",
            &["on Saturday → Sat 2026-11-28 08:00"],
        ),
        (
            remind,
            "2026-11-22T10:00",
            "sun",
            &["on Sunday → Sun 2026-11-29 08:00"],
        ),
        (
            remind,
            "2026-11-22T10:00",
            "next sat",
            &["next Saturday → Sat 2026-11-28 08:00"],
        ),
        (
            remind,
            "2026-11-22T10:00",
            "next sun",
            &["next Sunday → Sun 2026-11-29 08:00"],
        ),
        (
            remind,
            "2026-11-22T10:00",
            "next mon",
            &["next Monday → Mon 2026-11-30 08:00"],
        ),
        (
            remind,
            "2026-11-22T10:00",
            "next fri",
            &["next Friday → Fri 2026-12-04 08:00"],
        ),
        (
            remind,
            NOW,
            "next sun",
            &["next Sunday → Sun 2026-11-29 08:00"],
        ),
        (
            remind,
            NOW,
            "next wed",
            &["next Wednesday → Wed 2026-11-25 08:00"],
        ),
        (
            remind,
            NOW,
            "t",
            &[
                "tomorrow → Wed 2026-11-18 08:00",
                "on Thursday → Thu 2026-11-19 08:00",
                "on Tuesday → Tue 2026-11-24 08:00",
            ],
        ),
        (
            remind,
            NOW,
            "ne",
            &[
                "next week → Mon 2026-11-23 08:00",
                "next month → Tue 2026-12-01 08:00",
                "next quarter → Fri 2027-01-01 08:00",
            ],
        ),
        (
            remind,
            NOW,
            "next f",
            &["next Friday → Fri 2026-11-27 08:00"],
        ),
        (
            remind,
            NOW,
            "in 2 h",
            &["in 2 hours → Tue 2026-11-17 12:00"],
        ),
        (
            remind,
            NOW,
            "aug 7 20",
            &[
                "on August 7th 2027 → Sat 2027-08-07 08:00",
                "on August 7th 2028 → Mon 2028-08-07 08:00",
            ],
        ),
        (
            remind,
            NOW,
            "fri 10a",
            &["on Friday at 10 am → Fri 2026-11-20 10:00"],
        ),
        (
            remind,
            NOW,
            "2 weeks and",
            &["in 2 weeks and a day → Thu 2026-12-03 08:00"],
        ),
        (
            remind,
            NOW,
            "1.5 years",
            &["in 1.5 years → Sun 2028-04-30 08:00"],
        ),
        (
            remind,
            NOW,
            "1.5 months",
            &["in 1.5 months → Fri 2027-01-01 08:00"],
        ),
        (
            remind,
            NOW,
            "feb 30",
            &["on February 28th 2027 → Sun 2027-02-28 08:00"],
        ),
        (
            remind,
            NOW,
            "apr 31",
            &["on April 30th 2027 → Fri 2027-04-30 08:00"],
        ),
        (
            remind,
            NOW,
            "5/5/27",
            &["on May 5th 2027 → Wed 2027-05-05 08:00"],
        ),
        (
            remind,
            NOW,
            "tuesday the 24th",
            &["on Tuesday 24th November → Tue 2026-11-24 08:00"],
        ),
        (
            remind,
            NOW,
            "mon 23 nov",
            &["on Monday 23rd November → Mon 2026-11-23 08:00"],
        ),
        (
            remind,
            NOW,
            "twenty minutes",
            &["in 20 minutes → Tue 2026-11-17 10:20"],
        ),
        (remind, NOW, "3pm gmt", &[]),
        (remind, NOW, "9am eastern time", &[]),
        (
            remind,
            NOW,
            "9am la",
            &[
                "tomorrow at 9 am in Los Angeles → Wed 2026-11-18 09:00",
                "tomorrow at 9 am in Laos → Tue 2026-11-17 18:00",
                "tomorrow at 9 am in Lahore → Tue 2026-11-17 20:00",
            ],
        ),
        (remind, NOW, "9am kansas city, mo", &[]),
        (
            remind,
            NOW,
            "midnight pst",
            &["today at midnight PST → Wed 2026-11-18 00:00"],
        ),
        (
            remind,
            "2026-01-31T23:30",
            "in a month",
            &["in a month → Sat 2026-02-28 08:00"],
        ),
        (
            remind,
            "2026-01-31T23:30",
            "next month",
            &["next month → Sun 2026-02-01 08:00"],
        ),
        (
            remind,
            "2026-01-31T23:30",
            "tomorrow",
            &["tomorrow → Sun 2026-02-01 08:00"],
        ),
        (
            remind,
            "2026-12-31T18:00",
            "q1",
            &["in Q1 2027 → Fri 2027-01-01 08:00"],
        ),
        (
            remind,
            "2026-12-31T18:00",
            "jan 1",
            &[
                "on January 1st 2027 → Fri 2027-01-01 08:00",
                "on January 10th 2027 → Sun 2027-01-10 08:00",
                "on January 11th 2027 → Mon 2027-01-11 08:00",
            ],
        ),
        (
            remind,
            "2026-12-31T18:00",
            "next week",
            &["next week → Mon 2027-01-04 08:00"],
        ),
        (
            remind,
            "2026-03-08T01:30",
            "2:30am",
            &[
                "today at 2:30 am → Sun 2026-03-08 03:30",
                "tomorrow at 2:30 am → Mon 2026-03-09 02:30",
            ],
        ),
        (
            remind,
            "2026-03-08T01:30",
            "3am",
            &[
                "today at 3 am → Sun 2026-03-08 03:00",
                "tomorrow at 3 am → Mon 2026-03-09 03:00",
            ],
        ),
        (
            calendar,
            NOW,
            "8pm to 1am",
            &[
                "today at 8 pm for 5 hours → Tue 2026-11-17 20:00 (300m)",
                "today at 8 pm for 5 hours → Wed 2026-11-18 20:00 (300m)",
                "today at 8 pm for 5 hours → Thu 2026-11-19 20:00 (300m)",
            ],
        ),
        (
            calendar,
            NOW,
            "11 to 1",
            &[
                "today at 11 am for 2 hours → Tue 2026-11-17 11:00 (120m)",
                "today at 11 am for 2 hours → Wed 2026-11-18 11:00 (120m)",
                "today at 11 am for 2 hours → Thu 2026-11-19 11:00 (120m)",
            ],
        ),
        (
            calendar,
            NOW,
            "11am to 1",
            &[
                "today at 11 am for 2 hours → Tue 2026-11-17 11:00 (120m)",
                "today at 11 am for 2 hours → Wed 2026-11-18 11:00 (120m)",
                "today at 11 am for 2 hours → Thu 2026-11-19 11:00 (120m)",
            ],
        ),
        (
            calendar,
            NOW,
            "10:30-11:15",
            &[
                "today at 10:30 am for 45 minutes → Tue 2026-11-17 10:30 (45m)",
                "today at 10:30 am for 45 minutes → Wed 2026-11-18 10:30 (45m)",
                "today at 10:30 am for 45 minutes → Thu 2026-11-19 10:30 (45m)",
            ],
        ),
        (
            calendar,
            NOW,
            "noon to 2",
            &[
                "today at 12 pm for 2 hours → Tue 2026-11-17 12:00 (120m)",
                "today at 12 pm for 2 hours → Wed 2026-11-18 12:00 (120m)",
                "today at 12 pm for 2 hours → Thu 2026-11-19 12:00 (120m)",
            ],
        ),
        (
            calendar,
            NOW,
            "all day",
            &[
                "tomorrow all day → Wed 2026-11-18 00:00 (1440m, all-day)",
                "tomorrow all day → Thu 2026-11-19 00:00 (1440m, all-day)",
                "tomorrow all day → Fri 2026-11-20 00:00 (1440m, all-day)",
            ],
        ),
        (
            calendar,
            NOW,
            "2 days",
            &[
                "tomorrow all day → Wed 2026-11-18 00:00 (2880m, all-day)",
                "tomorrow all day → Thu 2026-11-19 00:00 (2880m, all-day)",
                "tomorrow all day → Fri 2026-11-20 00:00 (2880m, all-day)",
            ],
        ),
        (
            calendar,
            NOW,
            "for 90 min",
            &[
                "tomorrow for 1 hour and 30 minutes → Wed 2026-11-18 10:00 (90m)",
                "tomorrow for 1 hour and 30 minutes → Wed 2026-11-18 13:00 (90m)",
                "tomorrow for 1 hour and 30 minutes → Wed 2026-11-18 17:00 (90m)",
            ],
        ),
        (
            calendar,
            NOW,
            "dec 25 all day",
            &["on December 25th all day → Fri 2026-12-25 00:00 (1440m, all-day)"],
        ),
        (
            calendar,
            NOW,
            "monday for 3 days",
            &["on Monday all day → Mon 2026-11-23 00:00 (4320m, all-day)"],
        ),
        (
            calendar,
            NOW,
            "tomorrow 2pm for 45 min",
            &["tomorrow at 2 pm for 45 minutes → Wed 2026-11-18 14:00 (45m)"],
        ),
        (
            calendar,
            NOW,
            "fri 1-2pm",
            &["on Friday at 1 pm for 1 hour → Fri 2026-11-20 13:00 (60m)"],
        ),
        (
            calendar,
            NOW,
            "9am london",
            &[
                "tomorrow at 9 am in London → Wed 2026-11-18 01:00 (30m, in London Wed 2026-11-18 09:00)",
                "tomorrow at 9 am in London → Thu 2026-11-19 01:00 (30m, in London Thu 2026-11-19 09:00)",
                "tomorrow at 9 am in London → Fri 2026-11-20 01:00 (30m, in London Fri 2026-11-20 09:00)",
            ],
        ),
        (
            calendar,
            NOW,
            "in 2 days at 3pm",
            &[
                "in 2 days at 3 pm → Thu 2026-11-19 15:00 (30m)",
                "in 2 days at 3 pm → Thu 2026-11-19 15:00 (60m)",
                "in 2 days at 3 pm → Thu 2026-11-19 15:00 (15m)",
            ],
        ),
        (
            calendar,
            NOW,
            "t",
            &[
                "tomorrow → Wed 2026-11-18 10:00 (30m)",
                "tomorrow → Wed 2026-11-18 13:00 (30m)",
                "tomorrow → Wed 2026-11-18 00:00 (1440m, all-day)",
            ],
        ),
        (
            jump,
            NOW,
            "t",
            &[
                "tomorrow → Wed 2026-11-18 07:00",
                "Thursday → Thu 2026-11-19 07:00",
                "Tuesday → Tue 2026-11-24 07:00",
            ],
        ),
        (
            jump,
            NOW,
            "jan",
            &[
                "January 1st 2027 → Fri 2027-01-01 07:00",
                "January 2nd 2027 → Sat 2027-01-02 07:00",
                "January 3rd 2027 → Sun 2027-01-03 07:00",
            ],
        ),
        (jump, NOW, "last week", &[]),
        (
            jump,
            NOW,
            "mon",
            &[
                "Monday → Mon 2026-11-23 07:00",
                "in a month → Thu 2026-12-17 07:00",
            ],
        ),
        (
            jump,
            NOW,
            "q3",
            &[
                "Q3 2026 → Wed 2026-07-01 07:00",
                "Q3 2027 → Thu 2027-07-01 07:00",
            ],
        ),
    ]);
}
