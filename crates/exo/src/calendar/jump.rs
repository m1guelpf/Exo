//! Jump to Date: a sub-palette that reads the typed date with the time parser and moves the
//! calendar there.

use exo_commands::{Command, SubPalette, command_score};
use jiff::{Zoned, civil::Date};

use super::Action;
use crate::{
    commands::Action as AppAction,
    time_suggest::{self, Options},
};

/// The palette for dates read at `now`.
pub(super) fn palette(now: Zoned) -> SubPalette<AppAction> {
    // The header has the palette's default icon (`data/icon` in Classic).
    SubPalette::ordered(
        "CHANGE_CALENDAR_DAY",
        "Jump to Date",
        "data/icon",
        move |query| rows(query, &now),
    )
    .placeholder("Try: nov 5, 2 weeks, 2028")
    .empty_note("No matching date found")
}

/// The parser's first five dates, each with its date as en-US writes it. With a query, only the
/// labels it fuzzy matches stay, best first, so `8/7` (`August 7th 2027`) lists nothing.
fn rows(query: &str, now: &Zoned) -> Vec<Command<AppAction>> {
    let options = Options {
        defaults: &["next week", "next month", "in 3 months"],
        allow_past: true,
        skip_prepositions: true,
        ..Options::default()
    };
    let mut scored: Vec<(f64, time_suggest::Suggestion)> =
        time_suggest::suggest(query, &options, now)
            .into_iter()
            .take(5)
            .filter_map(|suggestion| {
                let score = if query.is_empty() {
                    1.0
                } else {
                    command_score(&suggestion.label.to_lowercase(), &query.to_lowercase())
                };
                (score > 0.0).then_some((score, suggestion))
            })
            .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored
        .into_iter()
        .enumerate()
        .map(|(at, (_, suggestion))| {
            let date = suggestion.start.to_zoned(now.time_zone().clone()).date();
            Command::new(
                format!("CHANGE_CALENDAR_DAY_{at}"),
                suggestion.label,
                AppAction::Calendar(Action::JumpTo(date)),
            )
            .side_title(locale_date(date))
        })
        .collect()
}

/// `date` as en-US writes it in short: `11/23/2026`.
fn locale_date(date: Date) -> String {
    format!("{}/{}/{}", date.month(), date.day(), date.year())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_the_parsed_dates_the_query_matches() {
        let jump = palette(
            "2026-11-17T10:00[America/Los_Angeles]"
                .parse()
                .expect("now"),
        );
        assert_eq!(jump.titles(""), ["next week", "next month", "in 3 months"]);
        assert_eq!(jump.titles("nov 5"), ["November 5th 2027"]);
        assert_eq!(jump.titles("2 weeks"), ["in 2 weeks"]);
        assert_eq!(
            jump.titles("8/7"),
            ["No matching date found"],
            "`August 7th 2027` doesn't match what was typed"
        );
        assert_eq!(jump.titles("last friday"), ["No matching date found"]);
        assert_eq!(locale_date(jiff::civil::date(2026, 12, 1)), "12/1/2026");
    }
}
