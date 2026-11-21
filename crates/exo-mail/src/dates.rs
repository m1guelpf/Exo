//! Dates in message text, marked for the "Create Event" hover.

use std::sync::LazyLock;

use regex::Regex;

use crate::dom::{self, NodeRef};

/// What each capture group of [`DATE`] means (`ns.groups`).
const GROUPS: &[Option<Group>] = {
    use Group::*;
    &[
        None,
        Some(IsoTimestamp),
        Some(Relative),
        Some(Dow),
        Some(LongDate),
        Some(ShortDate),
        Some(Dom),
        Some(Month),
        Some(Year),
        Some(Relative),
        Some(Dow),
        Some(Other),
        Some(Dom),
        Some(Other),
        Some(Month),
        Some(Relative),
        Some(Dow),
        Some(Month),
        Some(Dom),
        Some(Other),
        Some(Month),
        Some(Dom),
        Some(Month),
        Some(Dom),
        Some(Year),
        Some(Dom),
        Some(Month),
        Some(Dow),
        Some(Dom),
        Some(Month),
        Some(Year),
        Some(Dom),
        Some(Other),
        Some(Month),
        Some(Year),
        Some(Dom),
        Some(Month),
        Some(Year),
        Some(LongDate),
        Some(ShortDate),
        Some(Relative),
        Some(Dow),
        Some(Other),
        Some(Other),
        Some(Other),
        Some(Other),
        Some(Business),
    ]
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    IsoTimestamp,
    Relative,
    Dow,
    LongDate,
    ShortDate,
    Dom,
    Month,
    Year,
    /// `eod`, `eow`…: not used to filter.
    Business,
    /// Joiners, `today`, parts of day: not used to filter.
    Other,
}

/// The lookarounds need a backtracking engine. Its default budget is per search, which a long
/// stretch of text without dates uses up, so it's lifted: the pattern only backtracks a bounded
/// amount at each position.
static DATE: LazyLock<fancy_regex::Regex> = LazyLock::new(|| {
    let r2 = r"[0-9]{1,2}";
    let r3 = r"[0-9]{2,4}";
    let r4 = r"(?:(on|this|next|last)\s+)";
    let r5 = r"(today|tonight|tomorrow)";
    let r6 = r"(?:\s?(the|of)\s+)";
    let r8 = r"(monday|tuesday|wednesday|thursday|friday|saturday|sunday|mon|tues?|weds?|thur?|thurs|fri|sat|sun)";
    let r7 = r"(morning|afternoon|evening|night)";
    let r9 = r"(jan\.?|january|feb\.?|february|mar\.?|march|apr\.?|april|may\.?|jun\.?|june|jul\.?|july|aug\.?|august|sept?\.?|september|oct\.?|october|nov\.?|november|dec\.?|december)";
    let ne = r"([0-9]{1,2})(?:rd|st|nd|th|ʳᵈ|ˢᵗ|ⁿᵈ|ᵗʰ)?";
    let nt = r"[./\\]";
    let nr = r"[./\\-]";
    let nn = r"[./\\\-\s,]*";
    let no = format!(r"((?:{r2}{nt}{r2}|{r3}{nt}{r2})(?!\s*(?:[ap]m|day|week|mon|year|:(?!\s))))");
    let ni = format!(r"((?:{r3}{nr}{r2}{nr}{r2}|{r2}{nr}{r2}{nr}{r3})(?!{nr}[0-9]))");
    let na = format!(r"({r2}){nn}{r9}{nn}({r3})?");
    let iso = r"([0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}(?::[0-9]{2}(?:\.[0-9]+)?)?(?:Z|[+-][0-9]{2}:[0-9]{2}))";
    fancy_regex::RegexBuilder::new(&format!(
        r"(?i)(?<![A-Za-z0-9_.-])(?:{iso}|{r4}?{r8}[,\s]+(?:{ni}|{no}|{na})|{r4}?{r8}[,\s]+{r6}?{ne}{r6}?{r9}?|{r4}?{r8}[,\s]+{r9}[,\s]+{ne}|{r5}[,\s]+{r9}\s+{ne}|{r9}\s+{ne}(?:[,\s])*({r3})?(?!:)|{ne}\s+{r9}\s+{r8}|{ne}\s+{r9}(?:[,\s])*({r3})?(?!:)|{ne}{r6}{r9}(?:[,\s]*({r3}))?(?!:)|{na}|{ni}|{no}|{r4}?{r8}|this\s+{r7}|{r5}\s+{r7}|{r5}|(eod|eob|eow|eom|eoq|eoy))(?![A-Za-z0-9_-])"
    ))
    .backtrack_limit(usize::MAX)
    .build()
    .expect("valid regex")
});

/// Words near a date that make a bare one more likely to be meant.
static CONTEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(^|\s)(day|week|month|year|on|at|free|available|[0-9]{1,2}(?:am|pm|:d{2}))(?-u:\b)",
    )
    .expect("valid regex")
});
/// Reply headers, whose dates aren't plans.
static HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(On[^\n]{10,100}wrote:|---\s*Forwarded message[^\n]*|Sent:[\s0-9A-Za-z_]{3})")
        .expect("valid regex")
});

/// A byte range of the searched text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    start: usize,
    end: usize,
}

impl Span {
    /// Whether the span overlaps or abuts any of `spans`, which are in order, ends increasing.
    fn touches(self, spans: &[Span]) -> bool {
        let first = spans.partition_point(|span| span.end < self.start);
        spans.get(first).is_some_and(|span| span.start <= self.end)
    }
}

/// The parts of a detected date that say which day it is, as the date regex's named groups
/// capture them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DateParts {
    /// `2026-11-25T14:00Z`.
    pub iso_timestamp: Option<String>,
    /// `11/25`, `25.11`.
    pub short_date: Option<String>,
    /// `2026-11-25`, `11/25/2026`.
    pub long_date: Option<String>,
    /// `Nov`, `november`.
    pub month: Option<String>,
    /// The day of the month's digits.
    pub dom: Option<String>,
    pub year: Option<String>,
    /// `on`, `this`, `next`, `last`.
    pub relative: Option<String>,
    /// `Friday`, `fri`.
    pub dow: Option<String>,
    /// `eod`, `eob`, `eow`, `eom`, `eoq`, `eoy`.
    pub business: Option<String>,
}

/// The parts of the first date in `text`, a detected date's own text, found the way
/// `extract` finds it (`find(text, true)`: no context needed).
pub fn date_parts(text: &str) -> Option<DateParts> {
    let captures = DATE.captures(text).ok()??;
    let group = |wanted: Group| last_group(&captures, wanted).map(str::to_owned);
    let parts = DateParts {
        iso_timestamp: group(Group::IsoTimestamp),
        short_date: group(Group::ShortDate),
        long_date: group(Group::LongDate),
        month: group(Group::Month),
        dom: group(Group::Dom),
        year: group(Group::Year),
        relative: group(Group::Relative),
        dow: group(Group::Dow),
        business: group(Group::Business),
    };
    // `find(text, true)` still drops a short date without a slash and without a weekday.
    let dropped = parts.iso_timestamp.is_none()
        && parts.long_date.is_none()
        && parts
            .short_date
            .as_ref()
            .is_some_and(|short| parts.dow.is_none() && !short.contains('/'));
    (!dropped).then_some(parts)
}

/// The capture of group `wanted`: a later group of the same name wins.
fn last_group<'t>(captures: &fancy_regex::Captures<'t>, wanted: Group) -> Option<&'t str> {
    GROUPS
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, g)| **g == Some(wanted))
        .filter_map(|(i, _)| captures.get(i))
        .map(|m| m.as_str())
        .find(|s| !s.is_empty())
}

/// The dates in `text` that get marked, as byte ranges, in order: what compose's editor wraps in
/// `span.sh-date`.
pub fn find_dates(text: &str) -> Vec<std::ops::Range<usize>> {
    find(text)
        .into_iter()
        .map(|span| span.start..span.end)
        .collect()
}

/// The dates in `text` that get marked, in order.
fn find(text: &str) -> Vec<Span> {
    let spans = |regex: &Regex, after: usize| -> Vec<Span> {
        regex
            .find_iter(text)
            .map(|found| Span {
                start: found.start(),
                end: found.end() + after,
            })
            .collect()
    };
    let context = spans(&CONTEXT, 20);
    let headers = spans(&HEADER, 0);
    DATE.captures_iter(text)
        .map_while(Result::ok)
        .filter_map(|captures| {
            let whole = captures.get(0)?;
            let date = Span {
                start: whole.start(),
                end: whole.end(),
            };
            if date.touches(&headers) {
                return None;
            }
            let group = |wanted: Group| last_group(&captures, wanted);
            let keep = if group(Group::IsoTimestamp).is_some() || group(Group::LongDate).is_some() {
                true
            } else if let Some(short) = group(Group::ShortDate) {
                group(Group::Dow).is_some()
                    || (short.contains('/')
                        && (date.touches(&context)
                            || text[date.end..]
                                .strip_prefix(':')
                                .is_some_and(|rest| rest.starts_with(char::is_whitespace))))
            } else {
                let concrete = group(Group::Dom).is_some() && group(Group::Month).is_some();
                let short_weekday = group(Group::Dow).is_some_and(|dow| dow.chars().count() <= 5);
                concrete
                    || !short_weekday
                    || group(Group::Relative).is_some()
                    || group(Group::Dom).is_some()
                    || date.end < text.len()
            };
            keep.then_some(date)
        })
        .collect()
}

/// Wraps every detected date that lies within one text node in `<span class="exo-date">`,
/// except inside links. Spans quoted from earlier mail go first: Exo's, and the `sh-date` ones
/// some senders' mail carries.
pub(crate) fn decorate(root: &NodeRef) {
    let existing = dom::select(root, |n| {
        dom::is(n, "span") && (dom::has_class(n, "exo-date") || dom::has_class(n, "sh-date"))
    });
    for parent in dom::unwrap_all(&existing) {
        dom::normalize(&parent);
    }
    let texts: Vec<(NodeRef, String)> = dom::descendants(root)
        .filter_map(|node| Some((node.clone(), dom::text(&node)?)))
        .collect();
    // The text nodes are searched joined with spaces.
    let joined = texts
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let dates = find(&joined);
    let mut decorated = Vec::new();
    let mut start = 0;
    for (node, text) in texts {
        let node_start = start;
        start += text.len() + 1;
        let in_link = dom::parent(&node)
            .and_then(|parent| dom::tag(&parent).map(str::to_ascii_lowercase))
            .is_some_and(|tag| matches!(tag.as_str(), "a" | "title" | "style" | "script"));
        if in_link {
            continue;
        }
        let first = dates.partition_point(|date| date.start < node_start);
        let mut pieces = Vec::new();
        let mut cursor = 0;
        for date in dates[first..]
            .iter()
            .take_while(|date| date.end <= node_start + text.len())
        {
            let (from, to) = (date.start - node_start, date.end - node_start);
            if from > cursor {
                pieces.push(dom::text_node(&text[cursor..from]));
            }
            let span = dom::element("span", &[("class", "exo-date")]);
            dom::append(&span, &dom::text_node(&text[from..to]));
            pieces.push(span);
            cursor = to;
        }
        if pieces.is_empty() {
            continue;
        }
        if cursor < text.len() {
            pieces.push(dom::text_node(&text[cursor..]));
        }
        decorated.push((node, pieces));
    }
    dom::replace_all(decorated);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dates(text: &str) -> Vec<&str> {
        find(text)
            .into_iter()
            .map(|d| &text[d.start..d.end])
            .collect()
    }

    #[test]
    fn finds_dates_but_not_numbers_or_quote_headers() {
        assert_eq!(
            dates("Heads up: today's design crit moves to 3pm"),
            ["today"]
        );
        assert_eq!(
            dates("scorecards in by tomorrow noon? deadline on Friday, so"),
            ["tomorrow", "on Friday"]
        );
        assert_eq!(dates("Wed Nov 25, 2026 9:00am"), ["Wed Nov 25"]);
        assert_eq!(dates("version 1.2.3 and 3/4 cup"), Vec::<&str>::new());
        assert_eq!(
            dates("On Mon, Nov 16, 2026 at 5:54 PM Tom <t@x.co> wrote:"),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn reads_a_dates_parts() {
        assert_eq!(
            GROUPS.len(),
            DATE.captures_len(),
            "a group for every capture"
        );
        let parts = date_parts("Wed Nov 25").expect("a date");
        assert_eq!(
            (
                parts.dow.as_deref(),
                parts.month.as_deref(),
                parts.dom.as_deref()
            ),
            (Some("Wed"), Some("Nov"), Some("25"))
        );
        assert_eq!(
            date_parts("EOW")
                .and_then(|parts| parts.business)
                .as_deref(),
            Some("EOW")
        );
        assert_eq!(
            date_parts("next Friday")
                .and_then(|parts| parts.relative)
                .as_deref(),
            Some("next")
        );
    }

    #[test]
    fn searches_long_text_to_the_end() {
        // The backtracking engine used to give up on long stretches without a date, and the
        // search then retried the same position forever.
        let text = "x ".repeat(20_000) + "see you tomorrow";
        assert_eq!(dates(&text), ["tomorrow"]);
    }

    #[test]
    fn wraps_dates_within_text_nodes() {
        let document = dom::parse("<p>Let's meet tomorrow, <a href=x>or Friday</a></p>");
        let body = dom::body(&document).unwrap();
        decorate(&body);
        assert_eq!(
            dom::inner_html(&body),
            "<p>Let's meet <span class=\"exo-date\">tomorrow</span>, <a href=\"x\">or Friday</a></p>"
        );
    }

    #[test]
    fn redetects_dates_in_quoted_sh_date_spans() {
        let document = dom::parse(
            "<p>Heads up: <span class=\"sh-date\" data-date-isostring=\"2026-11-17\">today</span>'s crit</p>",
        );
        let body = dom::body(&document).unwrap();
        decorate(&body);
        assert_eq!(
            dom::inner_html(&body),
            "<p>Heads up: <span class=\"exo-date\">today</span>'s crit</p>"
        );
    }
}
