//! Emoji by name: what compose's `:` picker searches. Every emoji goes by its Unicode (CLDR) short
//! name as a shortcode (`grinning_face`), the face ones also without `_face` (`thinking`), and by
//! Slack's shortcode (`+1`, `tada`), which ranks lower. The data is Unicode's and iamcal's
//! (`crates/exo/data/emoji`).

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    sync::LazyLock,
};

use exo_store::EmojiSkinTone;
use regex::Regex;

/// The data, a line per emoji: `U\tgroup\tname\temoji\ttones` or `S\tname\temoji`.
const DATA: &str = include_str!("../data/emoji/emoji.tsv");
/// What an empty query shows, and names that win when the query starts them.
const DEFAULT: [&str; 8] = [
    "smile",
    "raised_hands",
    "heart",
    "thinking_face",
    "heart_eyes",
    "rocket",
    "sparkles",
    "tada",
];
/// Queries that start these keys favour names starting with their values.
const PREFIXES: [(&str, &str); 9] = [
    ("finger", "one_finger"),
    ("ok", "ok_hand"),
    ("party", "party"),
    ("slightly_", "slightly_smiling_face"),
    ("sm", "smile"),
    ("sweat_", "sweat_smile"),
    ("thumbs", "thumbs_up"),
    ("christmas", "christmas_tree"),
    ("chart", "chart_increasing"),
];
/// Slack's names score less when they aren't an exact match.
const SLACK_WEIGHT: f64 = 0.7;

/// A name and the emoji it gives.
struct Entry {
    emoji: String,
    slack: bool,
}

struct Index {
    /// Every name, in the data's order.
    names: Vec<(String, Entry)>,
    by_name: HashMap<String, usize>,
    /// An emoji's five skin tones, light to dark.
    tones: HashMap<String, Vec<String>>,
}

static INDEX: LazyLock<Index> = LazyLock::new(|| {
    let mut index = Index {
        names: Vec::new(),
        by_name: HashMap::new(),
        tones: HashMap::new(),
    };
    let add = |index: &mut Index, name: &str, emoji: &str, slack: bool| {
        if !index.by_name.contains_key(name) {
            index.by_name.insert(name.to_owned(), index.names.len());
            index.names.push((
                name.to_owned(),
                Entry {
                    emoji: emoji.to_owned(),
                    slack,
                },
            ));
        }
    };
    for line in DATA.lines().filter(|line| !line.starts_with('#')) {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["U", _group, name, emoji, rest @ ..] => {
                // A Unicode name takes over one already there.
                match index.by_name.get(*name) {
                    Some(at) => {
                        index.names[*at].1 = Entry {
                            emoji: (*emoji).to_owned(),
                            slack: false,
                        }
                    }
                    None => add(&mut index, name, emoji, false),
                }
                if let Some(tones) = rest
                    .first()
                    .map(|tones| tones.split(' ').map(str::to_owned).collect::<Vec<_>>())
                    .filter(|tones| tones.len() == 5)
                {
                    index.tones.insert((*emoji).to_owned(), tones);
                }
                if let Some(short) = name
                    .strip_suffix("_face")
                    .filter(|_| !name.ends_with("_with_face"))
                {
                    add(&mut index, short, emoji, false);
                }
            }
            ["S", name, emoji] => add(&mut index, name, emoji, true),
            _ => {}
        }
    }
    index
});

/// A search result: a name, and its emoji in the skin tone asked for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Found {
    pub name: String,
    pub emoji: String,
    score: f64,
}

/// The names `query` fuzzily matches (`command-score`), best first, one per emoji, at most `limit`;
/// the default eight for an empty query. `tone` dresses the emoji that take one.
pub(crate) fn search(query: &str, limit: usize, tone: Option<EmojiSkinTone>) -> Vec<Found> {
    let query = query
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_");
    let index = &*INDEX;
    let dress = |emoji: &str| qualify(&with_tone(emoji, tone));
    if query.is_empty() {
        return DEFAULT
            .iter()
            .filter_map(|name| {
                index.by_name.get(*name).map(|at| Found {
                    name: (*name).to_owned(),
                    emoji: index.names[*at].1.emoji.clone(),
                    score: 1.0,
                })
            })
            .collect();
    }
    let mut found: Vec<Found> = index
        .names
        .iter()
        .filter_map(|(name, entry)| {
            let mut score = exo_commands::command_score(name, &query);
            if score < 1.0 && entry.slack {
                score *= SLACK_WEIGHT;
            }
            (score > 0.0).then(|| Found {
                name: name.clone(),
                emoji: dress(&entry.emoji),
                score,
            })
        })
        .collect();
    found.sort_by(|a, b| {
        custom(&query, a, b).then_with(|| {
            if a.score == b.score {
                collate(&a.name, &b.name)
            } else {
                b.score.total_cmp(&a.score)
            }
        })
    });
    let mut seen = HashSet::new();
    found.retain(|found| seen.insert(found.emoji.clone()));
    found.truncate(limit);
    found
}

/// The emoji named exactly `name` (no skin tone), for `:name:`.
pub(crate) fn get(name: &str) -> Option<String> {
    INDEX
        .by_name
        .get(name)
        .map(|at| qualify(&INDEX.names[*at].1.emoji))
}

/// Each `:name:` of an emoji in pasted plain text becomes the emoji, except after a `://` in the
/// same word (a URL's path).
pub(crate) fn replace_codes(text: &str) -> String {
    static CODE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i):([a-z0-9_+-]+):").expect("valid regex"));
    let substitute = |text: &str| {
        CODE.replace_all(text, |captures: &regex::Captures<'_>| {
            get(&captures[1]).unwrap_or_else(|| captures[0].to_owned())
        })
        .into_owned()
    };
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(|ch: char| !ch.is_whitespace()) {
        out.push_str(&rest[..start]);
        let word_end = rest[start..]
            .find(char::is_whitespace)
            .map_or(rest.len(), |end| start + end);
        let word = &rest[start..word_end];
        match word.find("://") {
            Some(at) => {
                out.push_str(&substitute(&word[..=at]));
                out.push_str(&word[at + 1..]);
            }
            None => out.push_str(&substitute(word)),
        }
        rest = &rest[word_end..];
    }
    out.push_str(rest);
    out
}

/// Whether `emoji` comes in skin tones.
pub(crate) fn takes_tone(emoji: &str) -> bool {
    tones(emoji).is_some()
}

fn tones(emoji: &str) -> Option<&'static Vec<String>> {
    INDEX
        .tones
        .get(emoji)
        .or_else(|| INDEX.tones.get(&emoji.replace('\u{fe0f}', "")))
}

/// `emoji` in skin tone `tone`, if it comes in them.
fn with_tone(emoji: &str, tone: Option<EmojiSkinTone>) -> String {
    let at = match tone {
        None | Some(EmojiSkinTone::None) => return emoji.to_owned(),
        Some(EmojiSkinTone::Light) => 0,
        Some(EmojiSkinTone::MediumLight) => 1,
        Some(EmojiSkinTone::Medium) => 2,
        Some(EmojiSkinTone::MediumDark) => 3,
        Some(EmojiSkinTone::Dark) => 4,
    };
    tones(emoji).map_or_else(|| emoji.to_owned(), |tones| tones[at].clone())
}

/// An emoji character that shows as text by default gets the emoji variation selector, unless it
/// has one or a skin tone after it.
fn qualify(emoji: &str) -> String {
    static EMOJI: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\p{Emoji}$").expect("valid regex"));
    static PRESENTATION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\p{Emoji_Presentation}$").expect("valid regex"));
    static MODIFIER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\p{Emoji_Modifier}$").expect("valid regex"));
    let chars: Vec<char> = emoji.chars().collect();
    let mut out = String::with_capacity(emoji.len() + 3);
    for (at, ch) in chars.iter().enumerate() {
        out.push(*ch);
        let text = ch.to_string();
        if !EMOJI.is_match(&text) || PRESENTATION.is_match(&text) || MODIFIER.is_match(&text) {
            continue;
        }
        let next = chars.get(at + 1);
        if !next.is_some_and(|next| *next == '\u{fe0f}' || MODIFIER.is_match(&next.to_string())) {
            out.push('\u{fe0f}');
        }
    }
    out
}

/// The rules that rank before the score: a default name the query starts wins; so does a name
/// starting with a prefix rule's value when the query starts its key.
fn custom(query: &str, a: &Found, b: &Found) -> Ordering {
    for name in DEFAULT {
        if name.starts_with(query) && (a.name == name || b.name == name) {
            return if a.name == name {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
    }
    for (key, value) in PREFIXES {
        if key.starts_with(query) && a.name.starts_with(value) != b.name.starts_with(value) {
            return if a.name.starts_with(value) {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
    }
    Ordering::Equal
}

/// Shortcodes in order: punctuation before digits before letters, as the root collation orders
/// them.
fn collate(a: &str, b: &str) -> Ordering {
    let key = |ch: char| {
        (
            if ch.is_ascii_alphabetic() {
                2
            } else if ch.is_ascii_digit() {
                1
            } else {
                0
            },
            ch,
        )
    };
    a.chars().map(key).cmp(b.chars().map(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(query: &str, limit: usize) -> Vec<String> {
        search(query, limit, None)
            .into_iter()
            .map(|found| found.name)
            .collect()
    }

    #[test]
    fn pasted_shortcodes_become_emoji_but_not_in_urls() {
        assert_eq!(
            replace_codes("ship it :rocket: :nope:\n:smile:"),
            "ship it 🚀 :nope:\n😄"
        );
        assert_eq!(
            replace_codes("see:smile:https://x.co/:rocket:"),
            "see😄https://x.co/:rocket:"
        );
    }

    #[test]
    fn an_empty_query_shows_the_eight_defaults() {
        assert_eq!(names("", 8), DEFAULT);
        assert_eq!(search("", 8, None)[0].emoji, "😄");
    }

    #[test]
    fn the_rules_put_the_favourites_first() {
        assert_eq!(
            names("sm", 1),
            ["smile"],
            "the default `smile` wins its prefix"
        );
        assert_eq!(names("thumbs", 2), ["thumbs_up", "thumbs_down"]);
        assert_eq!(names("ok", 1), ["ok_hand"]);
        assert_eq!(names("rock", 1), ["rocket"]);
        assert_eq!(names("+1", 1), ["+1"]);
    }

    #[test]
    fn one_row_per_emoji_and_faces_go_without_face() {
        let found = search("thinking", 8, None);
        assert_eq!(found[0].emoji, "🤔");
        assert_eq!(found.iter().filter(|found| found.emoji == "🤔").count(), 1);
        assert_eq!(get("thinking").as_deref(), Some("🤔"));
        assert_eq!(get("thinking_face").as_deref(), Some("🤔"));
        assert_eq!(get("Thinking"), None, "names are lower case");
    }

    #[test]
    fn skin_tones_dress_the_emoji_that_take_them() {
        let wave = search("waving_hand", 1, Some(EmojiSkinTone::MediumDark));
        assert_eq!(wave[0].emoji, "👋🏾");
        assert!(takes_tone("👋") && !takes_tone("🚀"));
        assert_eq!(
            search("rocket", 1, Some(EmojiSkinTone::Dark))[0].emoji,
            "🚀"
        );
    }

    #[test]
    fn text_style_characters_get_the_emoji_selector() {
        assert_eq!(qualify("\u{2764}"), "\u{2764}\u{fe0f}");
        assert_eq!(qualify("\u{2764}\u{fe0f}"), "\u{2764}\u{fe0f}");
        assert_eq!(qualify("👍🏽"), "👍🏽");
    }
}
