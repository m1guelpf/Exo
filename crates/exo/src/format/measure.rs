//! The text-width *estimate* and the "ideal" truncation, which together decide where list rows
//! cut subjects, snippets and participant lists.
//!
//! Rendered text isn't measured for this: the estimate sums a fixed per-character table, and
//! strings are cut at the cheapest nearby boundary. Both work in UTF-16 code units, like JavaScript
//! strings, so an emoji counts as two characters of the default width.

use std::sync::LazyLock;

use regex::Regex;

use super::widths::WIDTHS;

/// JavaScript's `\s`.
const JS_SPACE: &str = r"[\t\n\x0B\x0C\r \u{a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}]";
/// JavaScript's `\w`, which is ASCII-only.
const JS_WORD: &str = "[A-Za-z0-9_]";
/// Width of a character missing from the table.
const DEFAULT_WIDTH: f64 = 6.0;

fn advance(unit: u16) -> f64 {
    char::from_u32(u32::from(unit))
        .and_then(|c| WIDTHS.binary_search_by_key(&c, |(c, _)| *c).ok())
        .map_or(DEFAULT_WIDTH, |i| WIDTHS[i].1)
}

/// The estimated width of `text` in px, rounded up.
pub fn width(text: &str) -> f64 {
    let mut total = 0.0;
    for unit in text.encode_utf16() {
        total += advance(unit);
        total += 0.06;
    }
    total.ceil()
}

/// How many UTF-16 units of `text` fit in `max_width` px.
pub fn max_characters(text: &str, max_width: f64) -> usize {
    let mut total = 0.0;
    for (i, unit) in text.encode_utf16().enumerate() {
        let w = advance(unit);
        if total + w > max_width {
            return i;
        }
        total += w;
    }
    utf16_len(text)
}

/// The length of `text` as JavaScript counts it.
pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `text.substring(0, n)` / `text.substring(n)`, with `n` in UTF-16 units. A cut inside a
/// surrogate pair moves before it (JavaScript would leave half an emoji).
pub fn split_utf16(text: &str, n: usize) -> (&str, &str) {
    let mut units = 0;
    for (byte, c) in text.char_indices() {
        units += c.len_utf16();
        if units > n {
            return text.split_at(byte);
        }
    }
    (text, "")
}

/// `text.replace(/\s+/g, " ").trim()`.
pub fn normalize_whitespace(text: &str) -> String {
    static SPACES: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(&format!("{JS_SPACE}+")).expect("valid regex"));
    SPACES
        .replace_all(text, " ")
        .trim_matches(is_js_space)
        .to_owned()
}

fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\x0B' | '\x0C' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// Whitespace-normalized `text`, cut to at most `length` UTF-16 units at the cheapest boundary near
/// it. Non-strict truncation may overshoot `length` to finish a word.
pub fn truncate(text: &str, length: usize, strict: bool) -> String {
    let text = normalize_whitespace(text);
    if utf16_len(&text) <= length || length == 0 {
        return text;
    }
    split_ideally(&text, length, strict)
}

/// A place truncation may cut, and what cutting there costs.
struct Boundary {
    pattern: Regex,
    cost: f64,
    /// Whether the cut goes after the match rather than before it.
    split_after: bool,
}

static BOUNDARIES: LazyLock<[Boundary; 5]> = LazyLock::new(|| {
    let boundary = |pattern: &str, cost, split_after| Boundary {
        pattern: Regex::new(pattern).expect("valid regex"),
        cost,
        split_after,
    };
    [
        // Sentence start, sentence end, disjunctive punctuation, word, anywhere.
        boundary(
            &format!(r#"({JS_SPACE}+[(\[{{<'"]+{JS_WORD})"#),
            20.0,
            false,
        ),
        boundary(
            &format!(r#"({JS_WORD}[.,!?)\]}}>'"]+{JS_SPACE}+)"#),
            20.0,
            true,
        ),
        boundary(
            &format!(r"({JS_SPACE}+[/\\~|.<>:;\-=#_¦«·»‐‑‒–—―‖•‣‹›])"),
            30.0,
            false,
        ),
        boundary("( )", 40.0, false),
        boundary("()", 90.0, false),
    ]
});

/// The head of `text` cut at the cheapest boundary near `target`: each boundary kind's cost plus
/// how far the cut lands from `target`.
fn split_ideally(text: &str, target: usize, strict: bool) -> String {
    let (window, _) = split_utf16(text, 2 * target);
    let mut best = f64::INFINITY;
    let mut head = String::new();
    for (i, boundary) in BOUNDARIES.iter().enumerate() {
        let (prev, next) = closest_cuts(window, boundary, target);
        let (prev_len, next_len) = (utf16_len(&prev) as f64, utf16_len(&next) as f64);
        let target = target as f64;
        let prev_cost = boundary.cost + 100.0 * (target - prev_len) / target;
        let next_cost = boundary.cost + 100.0 * (next_len - target) / target;
        if !strict && next_cost < prev_cost && next_cost <= best {
            head = next;
            best = next_cost;
        }
        if prev_cost <= best && (!prev.is_empty() || i != 0) {
            head = prev;
            best = prev_cost;
        }
    }
    head
}

/// The longest prefix ending at a boundary that is shorter than `target`, and the prefix one
/// boundary further.
fn closest_cuts(text: &str, boundary: &Boundary, target: usize) -> (String, String) {
    let parts = js_split(text, &boundary.pattern);
    let (mut prev, mut current) = (String::new(), String::new());
    let mut i = 0;
    while i < parts.len() && target > utf16_len(&prev) + utf16_len(&current) {
        prev.push_str(&current);
        current = if boundary.split_after {
            format!(
                "{}{}",
                parts[i],
                parts.get(i + 1).map_or("", String::as_str)
            )
        } else {
            format!(
                "{}{}",
                i.checked_sub(1).map_or("", |j| parts[j].as_str()),
                parts[i]
            )
        };
        i += 2;
    }
    let next = format!("{prev}{current}");
    (prev, next)
}

/// `String.prototype.split` with a one-group regex: the pieces between matches, each followed
/// by the group's text. Empty matches split between characters, as in JavaScript.
fn js_split(text: &str, pattern: &Regex) -> Vec<String> {
    let mut parts = Vec::new();
    let (mut piece_start, mut search_from) = (0, 0);
    while search_from < text.len() {
        let Some(captures) = pattern.captures_at(text, search_from) else {
            break;
        };
        let matched = captures.get(0).expect("group 0 always matches");
        if matched.end() == piece_start || matched.start() >= text.len() {
            // An empty match where the last piece ended: try one character further.
            search_from = matched.start()
                + text[matched.start()..]
                    .chars()
                    .next()
                    .map_or(1, char::len_utf8);
            continue;
        }
        parts.push(text[piece_start..matched.start()].to_owned());
        parts.push(
            captures
                .get(1)
                .map_or("", |group| group.as_str())
                .to_owned(),
        );
        piece_start = matched.end();
        search_from = piece_start;
    }
    parts.push(text[piece_start..].to_owned());
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_sums_the_table_plus_a_fudge_per_unit() {
        // 6.546875 + 0.06 per character, rounded up.
        assert_eq!(width("a"), 7.0);
        assert_eq!(width(" .. "), 11.0);
        // An emoji is two UTF-16 units of the default 6px.
        assert_eq!(width("😤"), 13.0);
    }

    #[test]
    fn max_characters_counts_units_that_fit() {
        assert_eq!(max_characters("aaaa", 13.1), 2);
        assert_eq!(max_characters("aa", 100.0), 2);
    }

    #[test]
    fn splits_like_javascript() {
        let space = Regex::new("( )").expect("valid");
        assert_eq!(js_split("a b", &space), ["a", " ", "b"]);
        let empty = Regex::new("()").expect("valid");
        assert_eq!(js_split("abc", &empty), ["a", "", "b", "", "c"]);
    }

    #[test]
    fn truncates_at_word_boundaries() {
        assert_eq!(truncate("Hello there world", 13, true), "Hello there");
        assert_eq!(truncate("short", 10, true), "short");
        assert_eq!(
            truncate("a  b", 10, true),
            "a b",
            "whitespace is normalized"
        );
    }
}
