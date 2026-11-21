//! How the palette orders its rows: [`rank_main`] for the main palette and [`rank_rows`] for
//! sub-palettes, both scoring with [`command_score`] and sorting as V8 does.

use std::borrow::Cow;

use crate::{command::Command, score::command_score};

/// A command that matched, with the title it's shown under.
pub(crate) struct Ranked<'a, A> {
    pub(crate) command: &'a Command<A>,
    /// The title, or `Title (Alias)` when an alias matched better.
    pub(crate) title: Cow<'a, str>,
    pub(crate) score: f64,
    natural_index: usize,
}

impl<'a, A> Ranked<'a, A> {
    /// A row shown as is, outside the ranking (a sub-palette's footer).
    pub(crate) fn unranked(command: &'a Command<A>) -> Self {
        Self {
            command,
            title: Cow::Borrowed(command.title.as_deref().unwrap_or_default()),
            score: 0.0,
            natural_index: 0,
        }
    }
}

/// Scores below this drop the row.
const MIN_SCORE: f64 = 0.0015;

/// Ranks the main palette's `candidates` (already in registration order, deduplicated and
/// filtered to the region) for `query`.
pub(crate) fn rank_main<'a, A>(candidates: &[&'a Command<A>], query: &str) -> Vec<Ranked<'a, A>> {
    let trimmed = query.trim();
    let lower_query = query.to_lowercase();
    let mut ranked: Vec<_> = candidates
        .iter()
        .enumerate()
        .filter_map(|(natural_index, &command)| score_main(command, query, trimmed, natural_index))
        .collect();
    v8_sort(&mut ranked, |a, b| compare_main(a, b, query, &lower_query));
    ranked
}

fn score_main<'a, A>(
    command: &'a Command<A>,
    query: &str,
    trimmed: &str,
    natural_index: usize,
) -> Option<Ranked<'a, A>> {
    if command.query_only && query.is_empty() {
        return None;
    }
    let base = command.title.as_deref().unwrap_or_default();
    let mut title = Cow::Borrowed(base);
    let mut score = None;
    if command.default_score != 0.0 && query.is_empty() {
        score = Some(command.default_score);
    } else if !query.is_empty() {
        let mut value = command_score(base, trimmed);
        if let Some(word) = &command.operative_word {
            let average = (value + command_score(word, trimmed)) / 2.0;
            if average > value {
                value = average;
            }
            if word.to_lowercase() == trimmed.to_lowercase() {
                value *= 2.0;
            }
        }
        if let Some((alias, alias_score)) = best_match(&command.aliases, trimmed)
            && alias_score > value
            && !base.to_lowercase().contains(query.to_lowercase().trim())
        {
            value = alias_score;
            if !command
                .exclude_alias_from_title
                .iter()
                .any(|excluded| excluded == alias)
            {
                title = Cow::Owned(format!("{base} ({alias})"));
            }
        }
        let mut scale = command.scale;
        // The one special case: `go to Sent` wins for "se", "sen", "ent", …
        if command.id == "sent" && "sent".contains(query) && query.encode_utf16().count() >= 2 {
            scale = Some(1.3);
        }
        let key_code = command.display_keys();
        if !key_code.is_empty() && key_code.join(",").to_lowercase() == query.to_lowercase() {
            value = 1.0;
        }
        if let Some(scale) = scale.filter(|&scale| scale != 0.0) {
            value *= scale;
        }
        if title.starts_with("Developer") {
            value /= 3.0;
        }
        score = Some(value);
    }
    if let Some(score) = score.filter(|&score| score > MIN_SCORE) {
        return Some(Ranked {
            command,
            title,
            score,
            natural_index,
        });
    }
    let (_, group) = best_match(&command.group_include_aliases, trimmed)?;
    (group > MIN_SCORE).then_some(Ranked {
        command,
        title,
        score: group / 2.0,
        natural_index,
    })
}

/// The alias that scores highest for `query` (the first of equals).
fn best_match<'a>(aliases: &'a [Cow<'static, str>], query: &str) -> Option<(&'a str, f64)> {
    let mut best: Option<(&str, f64)> = None;
    for alias in aliases {
        let score = command_score(alias, query);
        if best.is_none_or(|(_, best_score)| score > best_score) {
            best = Some((alias, score));
        }
    }
    best
}

/// The main palette's comparator, as a JavaScript comparator (negative: `a` first). It isn't a
/// total order (`follow` overrides scores pairwise), which is why the sort is [`v8_sort`].
fn compare_main<A>(a: &Ranked<'_, A>, b: &Ranked<'_, A>, query: &str, lower_query: &str) -> f64 {
    if b.command.follow.contains(&a.command.id) {
        return -1.0;
    }
    if a.command.follow.contains(&b.command.id) {
        return 1.0;
    }
    if a.score != b.score {
        return b.score - a.score;
    }
    if !query.is_empty()
        && let (Some(a_word), Some(b_word)) = (&a.command.operative_word, &b.command.operative_word)
    {
        let (a_word, b_word) = (a_word.to_lowercase(), b_word.to_lowercase());
        if a_word != b_word {
            if lower_query.contains(&a_word) {
                return -1.0;
            }
            if lower_query.contains(&b_word) {
                return 1.0;
            }
        }
    }
    let (a_default, b_default) = (a.command.default_score, b.command.default_score);
    if a_default != 0.0 && b_default != 0.0 && a_default != b_default {
        return b_default - a_default;
    }
    a.natural_index as f64 - b.natural_index as f64
}

/// Ranks a sub-palette's `rows`: everything for an empty query, otherwise rows whose first 200
/// characters match, best first, ties broken by `default_score`.
pub(crate) fn rank_rows<'a, A>(rows: &'a [Command<A>], query: &str) -> Vec<Ranked<'a, A>> {
    let mut ranked: Vec<_> = rows
        .iter()
        .enumerate()
        .filter_map(|(natural_index, command)| {
            let title = command.title.as_deref().unwrap_or_default();
            let score = if query.is_empty() {
                1.0
            } else {
                command_score(&utf16_prefix(title, 200), query)
            };
            (score > 0.0).then_some(Ranked {
                command,
                title: Cow::Borrowed(title),
                score,
                natural_index,
            })
        })
        .collect();
    v8_sort(&mut ranked, |a, b| {
        if a.score != b.score {
            b.score - a.score
        } else if a.command.default_score == b.command.default_score {
            a.natural_index as f64 - b.natural_index as f64
        } else {
            b.command.default_score - a.command.default_score
        }
    });
    ranked
}

/// `text.slice(0, units)`, in UTF-16 code units.
fn utf16_prefix(text: &str, units: usize) -> String {
    String::from_utf16_lossy(&text.encode_utf16().take(units).collect::<Vec<_>>())
}

/// `Array.prototype.sort` as V8 runs it on short arrays: find the leading run (reversing it if
/// strictly descending), then binary-insert the rest. With a comparator that isn't a total order,
/// the result depends on the algorithm, so this reproduces V8's instead of using `sort_by` (which
/// may also panic on such comparators). V8 merges runs of 32+ elements for arrays of 64 or more;
/// this keeps inserting, which only differs when `follow` contradicts scores in such long lists.
fn v8_sort<T>(items: &mut [T], compare: impl Fn(&T, &T) -> f64) {
    let length = items.len();
    if length < 2 {
        return;
    }
    let before = |a: &T, b: &T| compare(a, b) < 0.0;
    let mut run = 2;
    if before(&items[1], &items[0]) {
        while run < length && before(&items[run], &items[run - 1]) {
            run += 1;
        }
        items[..run].reverse();
    } else {
        while run < length && !before(&items[run], &items[run - 1]) {
            run += 1;
        }
    }
    for start in run..length {
        let (mut left, mut right) = (0, start);
        while left < right {
            let middle = left + (right - left) / 2;
            if before(&items[start], &items[middle]) {
                right = middle;
            } else {
                left = middle + 1;
            }
        }
        items[left..=start].rotate_right(1);
    }
}

#[cfg(test)]
mod catalogue;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v8_sort_matches_binary_insertion_with_a_descending_run() {
        let mut items = [5, 4, 3, 9, 1, 7];
        v8_sort(&mut items, |a, b| f64::from(*a) - f64::from(*b));
        assert_eq!(items, [1, 3, 4, 5, 7, 9]);
        let mut stable = [(1, 'a'), (0, 'b'), (1, 'c'), (0, 'd')];
        v8_sort(&mut stable, |a, b| f64::from(a.0) - f64::from(b.0));
        assert_eq!(stable, [(0, 'b'), (0, 'd'), (1, 'a'), (1, 'c')]);
    }

    #[test]
    fn aliases_decorate_the_title() {
        let commands = [Command::new("MARK_DONE", "Mark Done", ())
            .aliases(["Archive"])
            .key("e")
            .default_score(11.0)];
        let candidates: Vec<_> = commands.iter().collect();
        let ranked = rank_main(&candidates, "arch");
        assert_eq!(ranked[0].title, "Mark Done (Archive)");
        assert_eq!(ranked[0].command.id(), "MARK_DONE");
        let ranked = rank_main(&candidates, "e");
        assert_eq!(
            ranked[0].score, 1.0,
            "typing a row's single-key shortcut finds it"
        );
    }
}
