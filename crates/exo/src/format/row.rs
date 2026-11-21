//! The text a thread row shows, cut to fit: subjects and snippets against the estimated width of
//! the row's middle column, participants against the from column.

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use super::measure::{self, max_characters, split_utf16, utf16_len, width};
use super::names::{NameFormat, Person, display_name};

/// Subjects are cut to at most this many characters, however wide the row.
const MAX_SUBJECT: usize = 60;
/// Nor shorter than this.
const MIN_SUBJECT: usize = 10;

/// Where a search result's highlights start and end while it's cut: characters of their own, as
/// wide as any the width table doesn't have.
const MARK_START: char = '\u{e550}';
const MARK_END: char = '\u{e551}';
/// The same, to [`marked`] a subject with.
pub const MARKERS: [&str; 2] = ["\u{e550}", "\u{e551}"];
/// The tags a search result's excerpt comes in, which it's measured with.
const TAGS: [&str; 2] = ["<b>", "</b>"];

/// `text` with each of `marks` (byte ranges, in order) between `open` and `close`: a search
/// result's subject as it's cut ([`MARKERS`]), or its excerpt as it's measured (the `<b>` tags it
/// came in).
pub fn marked(text: &str, marks: &[Range<usize>], [open, close]: [&str; 2]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut done = 0;
    for range in marks {
        let Some(inner) = text.get(range.clone()).filter(|_| range.start >= done) else {
            continue;
        };
        out.push_str(&text[done..range.start]);
        out.push_str(open);
        out.push_str(inner);
        out.push_str(close);
        done = range.end;
    }
    out.push_str(&text[done..]);
    out
}

/// `Snippet.highlight`: a cut text's highlights, the markers taken out; a highlight followed by a
/// space, `@` or `.` and another runs on through both.
pub fn unmark(text: &str) -> (String, Vec<Range<usize>>) {
    let mut plain = String::with_capacity(text.len());
    let mut marks: Vec<Range<usize>> = Vec::new();
    let mut start = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            MARK_START => start = Some(plain.len()),
            MARK_END => {
                let mut ahead = chars.clone();
                if let (Some(between), Some(MARK_START)) = (ahead.next(), ahead.next())
                    && (between.is_whitespace() || between == '@' || between == '.')
                {
                    plain.push(between);
                    chars.next();
                    chars.next();
                    continue;
                }
                if let Some(start) = start.take().filter(|start| *start < plain.len()) {
                    marks.push(start..plain.len());
                }
            }
            c => plain.push(c),
        }
    }
    if let Some(start) = start.filter(|start| *start < plain.len()) {
        marks.push(start..plain.len());
    }
    (plain, marks)
}

/// The width budgeted for a label chip.
pub fn label_width(name: &str) -> f64 {
    width(name) * 0.83 + 8.0 + 6.0
}

fn labels_width(labels: &[String]) -> f64 {
    labels.iter().map(|label| label_width(label)).sum()
}

/// The subject as shown in a row whose middle column is `center_width` px wide, ending in `…`
/// when cut. A search result's comes [`marked`] with [`MARK_START`]/[`MARK_END`] around its
/// highlights, which count as characters; [`unmark`] it to show it.
pub fn subject(subject: &str, center_width: f64, labels: &[String]) -> String {
    let text = subject.trim();
    let available = center_width - 24.0 - labels_width(labels);
    let length = max_characters(text, available).clamp(MIN_SUBJECT, MAX_SUBJECT);
    let strict = width(split_utf16(text, length).0) + 6.0 > available;
    let mut cut = measure::truncate(text, length, strict);
    // Compared with the text as it came, whitespace untrimmed: a subject with a double space gets
    // an ellipsis even when nothing was cut.
    if utf16_len(&cut) != utf16_len(text) {
        cut.push('…');
    }
    cut
}

/// The snippet as shown after `shown_subject` (as [`subject`] gave it), and whether it was cut
/// (an ellipsis follows). `None` when not even three characters fit. A search result's
/// highlights (`marks`) are measured as the `<b>` tags it came in and cut as markers; [`unmark`] it
/// to show it.
pub fn snippet(
    snippet: &str,
    marks: &[Range<usize>],
    center_width: f64,
    labels: &[String],
    shown_subject: &str,
    done: bool,
) -> Option<(String, bool)> {
    let mut available = center_width - 24.0 - labels_width(labels) - width(shown_subject);
    if done {
        available -= 24.0;
    }
    let cleaned = clean_snippet(&marked(snippet, marks, TAGS));
    let length = max_characters(&cleaned, available);
    let teaser = if utf16_len(&cleaned) > 3 {
        format!("{}…", split_utf16(&cleaned, 3).0)
    } else {
        cleaned.clone()
    };
    if available <= width(&teaser) || length == 0 {
        return None;
    }
    let prepared = cleaned
        .replace(TAGS[0], MARKERS[0])
        .replace(TAGS[1], MARKERS[1]);
    let prepared = prepared.trim();
    let cut = measure::truncate(prepared, length, true);
    let truncated = utf16_len(&cut) < utf16_len(&measure::normalize_whitespace(prepared));
    Some((cut, truncated))
}

/// `Snippet.clean`: the snippet up to the first quoted reply, signature rule or footer.
pub fn clean_snippet(snippet: &str) -> String {
    static CUTS: LazyLock<[Regex; 4]> = LazyLock::new(|| {
        [
            r"[^a-zA-Z0-9]+You are receiving this",
            r"Sent (using|with|from|via) ",
            r"On ((Mon|Tue|Wed|Thu|Fri|Sat|Sun), )?(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)",
            r"\s?\{#HS:[\d-]+#\}\s?",
        ]
        .map(|pattern| Regex::new(pattern).expect("valid regex"))
    });
    let mut text = snippet.split("----").next().unwrap_or_default();
    text = text.split("____").next().unwrap_or_default();
    for cut in CUTS.iter() {
        text = cut.split(text).next().unwrap_or_default();
    }
    text.to_owned()
}

/// One participant of a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Participant {
    /// Who.
    pub person: Person,
    /// Whether it's the signed-in user.
    pub is_me: bool,
}

/// The from column's text: one person's full name, or first names. Given the column's
/// `column_width`, more than two people show as the first and last always, the ones a search
/// asked for (`searched`, its address and name words) too, and as many of the latest others as
/// fit, with ` .. ` standing for the rest; without one (draft recipients), everyone is listed.
pub fn participants(
    people: &[Participant],
    column_width: Option<f64>,
    searched: &[String],
) -> String {
    let name = |participant: &Participant| {
        let format = if people.len() == 1 {
            NameFormat::Full
        } else {
            NameFormat::First
        };
        display_name(
            &participant.person,
            participant.is_me,
            format,
            people.len() > 1,
        )
    };
    let names: Vec<String> = people.iter().map(name).collect();
    let Some(column_width) = column_width.filter(|_| names.len() > 2) else {
        return names.join(", ");
    };
    let last = names.len() - 1;
    // The address searched for, or a word of the name.
    let is_searched = |person: &Person| {
        searched.contains(&person.email)
            || person.name.split(' ').any(|word| {
                searched
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case(word))
            })
    };
    let always: f64 = names
        .iter()
        .zip(people)
        .enumerate()
        .filter(|(at, (_, participant))| {
            *at == 0 || *at == last || is_searched(&participant.person)
        })
        .map(|(_, (name, _))| width(name))
        .sum();
    let budget = column_width - always - width(" .. ");
    // The middle names, latest first, each shown while the running total still fits, or when
    // searched for.
    let mut used = 0.0;
    let mut shown: Vec<&String> = Vec::new();
    for (middle, participant) in names[1..last].iter().zip(&people[1..last]).rev() {
        used += width(&format!("{middle}, "));
        if is_searched(&participant.person) || used <= budget {
            shown.push(middle);
        }
    }
    shown.reverse();
    let mut text = names[0].clone();
    text.push_str(if used > budget { " .. " } else { ", " });
    for middle in shown {
        text.push_str(middle);
        text.push_str(", ");
    }
    text.push_str(&names[last]);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(name: &str) -> Participant {
        Participant {
            person: Person::new(name, format!("{}@x.co", name.to_lowercase())),
            is_me: false,
        }
    }

    #[test]
    fn snippets_stop_at_quoted_replies() {
        assert_eq!(
            clean_snippet("Sounds good. On Mon, Nov 16, 2026 at 5:54 PM Tom wrote:"),
            "Sounds good. "
        );
        assert_eq!(clean_snippet("Hi -- there"), "Hi -- there");
        assert_eq!(clean_snippet("Hi ---- sig"), "Hi ");
    }

    #[test]
    fn participants_overflow_with_two_dots() {
        let people = ["Tom", "Sam", "Oliver", "Lena", "Priya", "Daniel"].map(person);
        assert_eq!(
            participants(&people, Some(185.8), &[]),
            "Tom .. Oliver, Lena, Priya, Daniel"
        );
        assert_eq!(
            participants(&people, Some(1000.0), &[]),
            "Tom, Sam, Oliver, Lena, Priya, Daniel"
        );
        assert_eq!(
            participants(&people, None, &[]),
            "Tom, Sam, Oliver, Lena, Priya, Daniel"
        );
        assert_eq!(participants(&people[..1], Some(185.8), &[]), "Tom");
    }

    #[test]
    fn searched_people_always_show() {
        let people = ["Tom", "Sam", "Oliver", "Lena", "Priya", "Daniel"].map(person);
        assert_eq!(
            participants(&people, Some(185.8), &["sam".into()]),
            "Tom .. Sam, Lena, Priya, Daniel",
            "searched by name: Sam shows, and his room is taken from the others"
        );
        assert_eq!(
            participants(&people, Some(185.8), &["sam@x.co".into()]),
            "Tom .. Sam, Lena, Priya, Daniel",
            "or by address"
        );
        assert_eq!(
            participants(&people, Some(185.8), &["Sam@x.co".into()]),
            "Tom .. Oliver, Lena, Priya, Daniel",
            "an address matches as typed"
        );
    }

    #[test]
    fn highlights_count_where_a_result_is_cut() {
        let subject = "Invitation: Team offsite: Hudson @ Wed Dec 9, 2026 9:00am - 5:00pm (EST) (maya@brightmoor.co)";
        let at = subject.find("offsite").expect("in it");
        let offsite = [Range {
            start: at,
            end: at + 7,
        }];
        let cut = super::subject(&marked(subject, &offsite, MARKERS), 1000.0, &[]);
        let (shown, marks) = unmark(&cut);
        assert_eq!(
            shown, "Invitation: Team offsite: Hudson @ Wed Dec 9, 2026 9:00am…",
            "the markers take two of the 60 characters"
        );
        assert_eq!(marks, offsite);
        assert_eq!(
            unmark("\u{e550}sweet\u{e551} \u{e550}potato\u{e551} thing").1,
            [Range { start: 0, end: 12 }],
            "neighbours run on"
        );
    }

    #[test]
    fn long_subjects_are_cut_with_an_ellipsis() {
        let cut = subject(
            "Invitation: Coffee with Grace @ Wed Nov 25, 2026 9:00am - 9:45am (EST) (maya@brightmoor.co)",
            687.34,
            &[],
        );
        assert_eq!(
            cut,
            "Invitation: Coffee with Grace @ Wed Nov 25, 2026 9:00am - 9:45am…"
        );
    }
}
