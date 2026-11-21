//! Grapheme clusters, which the caret moves over and Delete removes whole, as Chromium does
//! (UAX #29's extended clusters: flags, emoji sequences, Hangul syllables, a letter and its
//! marks). Offsets are in characters, as [`crate::Pos`] counts them.

use std::sync::LazyLock;

use regex::Regex;

/// One extended grapheme cluster (UAX #29's table 1b).
static CLUSTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        \r\n | [\p{gcb=Control}\p{gcb=CR}\p{gcb=LF}]
        | \p{gcb=Prepend}*
          (?: \p{gcb=RI}\p{gcb=RI}
            | \p{Extended_Pictographic} (?:\p{gcb=Extend}*\x{200D}\p{Extended_Pictographic})*
            | \p{gcb=L}* (?:\p{gcb=V}+ | \p{gcb=LV}\p{gcb=V}* | \p{gcb=LVT}) \p{gcb=T}* | \p{gcb=L}+ | \p{gcb=T}+
            | [^\p{gcb=Control}\p{gcb=CR}\p{gcb=LF}] )
          [\p{gcb=Extend}\p{gcb=ZWJ}\p{gcb=SpacingMark}]*",
    )
    .expect("valid regex")
});

/// What Backspace deletes whole rather than one character at a time: emoji sequences, flags,
/// keycaps (Blink's `BackspaceStateMachine`).
static EMOJI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[\p{Extended_Pictographic}\p{gcb=RI}\x{20E3}\x{FE0F}]").expect("valid regex")
});

/// The clusters of `text`: `(start, end)` in characters.
pub(crate) fn clusters(text: &str) -> Vec<(usize, usize)> {
    let mut at = 0;
    CLUSTER
        .find_iter(text)
        .map(|found| {
            let start = at;
            at += found.as_str().chars().count();
            (start, at)
        })
        .collect()
}

/// The end of the cluster after `offset` (Delete, →).
pub(crate) fn next(text: &str, offset: usize) -> usize {
    clusters(text)
        .into_iter()
        .map(|(_, end)| end)
        .find(|end| *end > offset)
        .unwrap_or(offset)
}

/// The start of the cluster before `offset` (←).
pub(crate) fn previous(text: &str, offset: usize) -> usize {
    clusters(text)
        .into_iter()
        .map(|(start, _)| start)
        .take_while(|start| *start < offset)
        .last()
        .unwrap_or(0)
}

/// Where Backspace at `offset` deletes from: an emoji cluster's start, else one character
/// back, so a letter keeps its base when its accent goes (Chromium's backward deletion).
pub(crate) fn backspace(text: &str, offset: usize) -> usize {
    let start = previous(text, offset);
    let cluster: String = text.chars().skip(start).take(offset - start).collect();
    if EMOJI.is_match(&cluster) {
        start
    } else {
        offset.saturating_sub(1)
    }
}

/// `offset` moved off the inside of a cluster: to the nearer of its ends, the start on a tie.
pub(crate) fn snap(text: &str, offset: usize) -> usize {
    match clusters(text)
        .into_iter()
        .find(|(start, end)| *start < offset && offset < *end)
    {
        Some((start, end)) if end - offset < offset - start => end,
        Some((start, _)) => start,
        None => offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clusters_hold_what_shows_as_one_character() {
        let text =
            "e\u{301}🇺🇸👨\u{200d}👩\u{200d}👧👍🏽1\u{fe0f}\u{20e3}\u{1100}\u{1161}\u{915}\u{93f}a";
        let lengths: Vec<usize> = clusters(text)
            .into_iter()
            .map(|(start, end)| end - start)
            .collect();
        assert_eq!(lengths, [2, 2, 5, 2, 3, 2, 2, 1]);
    }

    #[test]
    fn backspace_takes_emoji_whole_and_marks_alone() {
        assert_eq!(backspace("a🇺🇸", 3), 1);
        assert_eq!(backspace("a👍🏽", 3), 1);
        assert_eq!(backspace("ae\u{301}", 3), 2, "the accent goes, the e stays");
        assert_eq!(backspace("ab", 2), 1);
    }

    #[test]
    fn moves_and_snaps_by_cluster() {
        let text = "xe\u{301}🇺🇸";
        assert_eq!(next(text, 1), 3);
        assert_eq!(next(text, 3), 5);
        assert_eq!(previous(text, 5), 3);
        assert_eq!(previous(text, 3), 1);
        assert_eq!(snap(text, 2), 1);
        assert_eq!(snap(text, 4), 3);
        assert_eq!(snap(text, 3), 3);
    }
}
