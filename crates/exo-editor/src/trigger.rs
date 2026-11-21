//! Inline triggers: the text before the caret that opens a dropdown. The editor finds them; the
//! caller resolves them (emoji search, snippets, contacts) and calls [`crate::Editor::replace`].

use std::sync::LazyLock;

use regex::Regex;

use crate::model::Pos;

/// Which dropdown a trigger opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerKind {
    /// `:smile` (emoji).
    Emoji,
    /// `;intro` (snippets).
    Snippet,
    /// `+ali`, `@ali`, `-ali` or `−ali` (contacts; the character is which).
    Contact(char),
}

/// An open inline trigger.
#[derive(Clone, Debug, PartialEq)]
pub struct Trigger {
    /// Its kind.
    pub kind: TriggerKind,
    /// What's typed after the trigger character (for contacts, trimmed).
    pub query: String,
    /// Where the trigger character is.
    pub start: Pos,
    /// The caret, where the trigger text ends.
    pub end: Pos,
    /// The trigger character's box on screen, where its dropdown anchors (each kind adds its own
    /// x offset: 50px for emoji, 9px for `+`, 5px for `@`, 12px for `-`). Set when the editor is
    /// drawn.
    pub anchor: egui::Rect,
}

/// `:` after whitespace, the start, `(`, `"` or an emoji.
static EMOJI: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:^|[\s&&[^\x{FEFF}]]|\(|"|\p{Extended_Pictographic}|[\x{2190}-\x{21FF}])(:[a-zA-Z0-9_+-]*)$"#).expect("valid regex")
});
/// `;` and words, or a bare `;` after whitespace or at the start.
static SNIPPET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(;[\x{FEFF}a-zA-Z0-9_+-]+(?:\s[a-zA-Z0-9_+-]+)*\s?|(?:[\s&&[^\x{FEFF}]]|^);)$")
        .expect("valid regex")
});
/// `tl;` isn't a snippet.
static NOT_SNIPPET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:^|\s)tl;").expect("valid regex"));
/// A trigger after a non-name character, then a name and an optional second word.
static CONTACT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:[^0-9a-zA-Z\x{C0}-\x{17F}_@\x{FEFF}]|^)([+@\-\x{2212}])([0-9a-zA-Z\x{C0}-\x{17F}][a-zA-Z\x{C0}-\x{17F}0-9_@.\-]{0,63}\s?(?:\s[a-zA-Z\x{C0}-\x{17F}0-9_.]{1,63})?)$").expect("valid regex")
});
/// `+1` and `-5` aren't mentions.
static NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:[^0-9a-zA-Z\x{C0}-\x{17F}_@\x{FEFF}]|^)[+\-\x{2212}][0-9]+\s?$")
        .expect("valid regex")
});

/// Whether typing `ch` can open a trigger (`editing` is set on its keydown).
pub fn is_trigger_char(ch: char) -> bool {
    matches!(ch, ':' | ';' | '+' | '@' | '-' | '\u{2212}')
}

/// The trigger at the end of `before` (a block's text, or a field's, up to the caret): its kind,
/// the character offset of the trigger character, and the query. The subject field runs it on
/// its value too.
pub fn find(before: &str) -> Option<(TriggerKind, usize, String)> {
    let offset = |byte: usize| before[..byte].chars().count();
    if let Some(group) = EMOJI.captures(before).and_then(|captures| captures.get(1)) {
        return Some((
            TriggerKind::Emoji,
            offset(group.start()),
            group.as_str()[1..].to_owned(),
        ));
    }
    if let Some(found) = SNIPPET.find(before)
        && !NOT_SNIPPET.is_match(before)
    {
        let start = found.start() + found.as_str().find(';').unwrap_or(0);
        return Some((
            TriggerKind::Snippet,
            offset(start),
            before[start + 1..].to_owned(),
        ));
    }
    if NUMBER.is_match(before) {
        return None;
    }
    let captures = CONTACT.captures(before)?;
    let (trigger, name) = (captures.get(1)?, captures.get(2)?);
    let ch = trigger.as_str().chars().next()?;
    Some((
        TriggerKind::Contact(ch),
        offset(trigger.start()),
        name.as_str().trim_end().to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_needs_a_boundary_before_the_colon() {
        assert_eq!(find("hi :smi"), Some((TriggerKind::Emoji, 3, "smi".into())));
        assert_eq!(find(":"), Some((TriggerKind::Emoji, 0, String::new())));
        assert_eq!(find("(:+1"), Some((TriggerKind::Emoji, 1, "+1".into())));
        assert_eq!(find("10:30"), None, "a time isn't an emoji");
    }

    #[test]
    fn snippets_and_their_exceptions() {
        assert_eq!(find(";"), Some((TriggerKind::Snippet, 0, String::new())));
        assert_eq!(
            find("see ;intro call"),
            Some((TriggerKind::Snippet, 4, "intro call".into()))
        );
        assert_eq!(find("tl;"), None);
        assert_eq!(find("tl;dr"), None);
    }

    #[test]
    fn mentions_but_not_numbers() {
        assert_eq!(
            find("cc +ali"),
            Some((TriggerKind::Contact('+'), 3, "ali".into()))
        );
        assert_eq!(
            find("@Jane Doe"),
            Some((TriggerKind::Contact('@'), 0, "Jane Doe".into()))
        );
        assert_eq!(find("call +1"), None);
        assert_eq!(find("a-b"), None, "a hyphen inside a word isn't a trigger");
        assert_eq!(
            find("x \u{2212}bob"),
            Some((TriggerKind::Contact('\u{2212}'), 2, "bob".into()))
        );
    }
}
