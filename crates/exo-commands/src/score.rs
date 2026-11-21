//! Fuzzy matching for the palette: a port of `command-score`.
//!
//! A faithful Rust port of `command-score` (<https://github.com/superhuman/command-score>, MIT
//! license), so rows rank exactly as the JavaScript library ranks them. It works on UTF-16 code
//! units, like the JavaScript string operations it mirrors.

use std::collections::HashMap;

/// Characters that start a "word" after punctuation (`/[\\\/_+.#"@\[\(\{&]/`).
const GAPS: &[u8] = br#"\/_+.#"@[({&"#;

/// How well `abbreviation` matches `string`, from 0 (no match) to 1 (exact).
///
/// Matching is case-insensitive, with a tiny penalty for each case mismatch. Contiguous matches
/// score highest, then matches at the start of a word (after a space or hyphen, then after
/// punctuation), then matches inside a word; an abbreviation that covers only a prefix of the
/// string caps at 0.99.
pub fn command_score(string: &str, abbreviation: &str) -> f64 {
    Matcher {
        string: string.encode_utf16().collect(),
        abbreviation: abbreviation.encode_utf16().collect(),
        lower_string: normalize(string),
        lower_abbreviation: normalize(abbreviation),
        memo: HashMap::new(),
    }
    .score(0, 0)
}

/// Lower-cases and turns every whitespace or hyphen into a space.
fn normalize(text: &str) -> Vec<u16> {
    text.to_lowercase()
        .encode_utf16()
        .map(|unit| {
            if is_space(unit) {
                u16::from(b' ')
            } else {
                unit
            }
        })
        .collect()
}

/// JavaScript's `/[\s-]/`: ECMAScript white space and line terminators, or a hyphen.
fn is_space(unit: u16) -> bool {
    matches!(
        unit,
        0x09..=0x0d | 0x20 | 0x2d | 0xa0 | 0x1680 | 0x2000..=0x200a | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000 | 0xfeff
    )
}

fn is_gap(unit: u16) -> bool {
    u8::try_from(unit).is_ok_and(|byte| GAPS.contains(&byte))
}

struct Matcher {
    string: Vec<u16>,
    abbreviation: Vec<u16>,
    lower_string: Vec<u16>,
    lower_abbreviation: Vec<u16>,
    memo: HashMap<(usize, usize), f64>,
}

impl Matcher {
    /// The best score for matching `abbreviation[a..]` against `string[s..]`.
    fn score(&mut self, s: usize, a: usize) -> f64 {
        // JavaScript recurses past the end only in a case that can't arise (see the transposition
        // check below); `>=` keeps that impossible case from indexing out of bounds.
        if a >= self.abbreviation.len() {
            return if s == self.string.len() { 1.0 } else { 0.99 };
        }
        if let Some(&memoized) = self.memo.get(&(s, a)) {
            return memoized;
        }
        let wanted = self.lower_abbreviation[a];
        let mut best = 0.0;
        let mut found = self.index_of(wanted, s);
        while let Some(at) = found {
            let mut score = self.score(at + 1, a + 1);
            if score > best {
                if at == s {
                    // Contiguous: no penalty.
                } else if self.string.get(at - 1).copied().is_some_and(is_gap) {
                    score *= 0.8;
                    let skipped = self.count_between(s, at - 1, is_gap);
                    if skipped > 0 && s > 0 {
                        score *= 0.999_f64.powf(skipped as f64);
                    }
                } else if self.string.get(at - 1).copied().is_some_and(is_space) {
                    score *= 0.9;
                    let skipped = self.count_between(s, at - 1, is_space);
                    if skipped > 0 && s > 0 {
                        score *= 0.999_f64.powf(skipped as f64);
                    }
                } else {
                    score *= 0.3;
                    if s > 0 {
                        score *= 0.999_f64.powf((at - s) as f64);
                    }
                }
                if self.string.get(at) != self.abbreviation.get(a) {
                    score *= 0.9999;
                }
            }
            // Transpositions ("teh" for "the"). `None` stands for JavaScript's `charAt` past either
            // end, which returns "" (so two out-of-range characters compare equal).
            let before = at.checked_sub(1).and_then(|i| self.lower_string.get(i));
            let next = self.lower_abbreviation.get(a + 1);
            if (score < 0.1 && before == next) || (next == Some(&wanted) && before != Some(&wanted))
            {
                let transposed = self.score(at + 1, a + 2);
                if 0.1 * transposed > score {
                    score = 0.1 * transposed;
                }
            }
            if score > best {
                best = score;
            }
            found = self.index_of(wanted, at + 1);
        }
        self.memo.insert((s, a), best);
        best
    }

    /// How many of `string.slice(start, end)` satisfy `test` (`slice` clamps to the string).
    fn count_between(&self, start: usize, end: usize, test: fn(u16) -> bool) -> usize {
        let end = end.min(self.string.len());
        self.string
            .get(start..end)
            .map_or(0, |units| units.iter().filter(|&&unit| test(unit)).count())
    }

    /// Where `unit` next occurs in the lower-cased string, from `from` on.
    fn index_of(&self, unit: u16, from: usize) -> Option<usize> {
        self.lower_string
            .get(from..)?
            .iter()
            .position(|&u| u == unit)
            .map(|i| i + from)
    }
}

#[cfg(test)]
mod tests {
    use super::command_score;

    /// `(string, abbreviation, score)` computed by the JavaScript `command-score`, for palette
    /// titles and typical queries.
    const VECTORS: &str = include_str!("../tests/command_score.json");

    #[test]
    fn matches_the_javascript_implementation() {
        let vectors: Vec<(String, String, f64)> =
            serde_json::from_str(VECTORS).expect("valid vectors");
        assert!(vectors.len() > 700);
        for (string, abbreviation, expected) in vectors {
            let actual = command_score(&string, &abbreviation);
            assert!(
                (actual - expected).abs() < 1e-12,
                "command_score({string:?}, {abbreviation:?}) = {actual}, JavaScript gives {expected}"
            );
        }
    }

    #[test]
    fn ranks_like_the_readme() {
        assert_eq!(command_score("hello", "hello"), 1.0);
        assert!(command_score("hello world", "hw") > command_score("hello world", "ho"));
        assert_eq!(command_score("hello", "z"), 0.0);
    }
}
