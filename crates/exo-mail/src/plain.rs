//! Plain-text bodies as the thread view shows them: paragraphs, `>` quotes as nested blockquotes,
//! and short lines kept as lines.

use std::sync::LazyLock;

use regex::Regex;

/// A line starting with a letter or digit (not punctuation, a symbol or a list marker like
/// `1.`) continues the previous line's paragraph unless that line is short.
static CONTINUATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[^\p{P}\p{S}\p{Z}]").expect("valid regex"));

/// Converts a text/plain body to the HTML it renders as.
pub(crate) fn to_html(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut depth = 0;
    let mut blank_before = false;
    for raw in text.split('\n') {
        let prefix_len = raw.len() - raw.trim_start_matches(['>', ' ']).len();
        let level = raw[..prefix_len].chars().filter(|c| *c == '>').count();
        let line = raw[prefix_len..]
            .strip_suffix('\r')
            .unwrap_or(&raw[prefix_len..]);
        if line.trim().is_empty() {
            blank_before = true;
            continue;
        }
        let escaped = escape(line);
        if level > depth {
            lines.push(format!("</p>{}<p>", "<blockquote>".repeat(level - depth)));
            depth = level;
        } else if level < depth {
            lines.push(format!("</p>{}<p>", "</blockquote>".repeat(depth - level)));
            depth = level;
        } else if blank_before {
            lines.push("</p><p>".to_owned());
        } else if let Some(previous) = lines.last()
            && (previous.encode_utf16().count() < 50 || !continues(line))
        {
            lines.push("<br/>".to_owned());
        }
        blank_before = false;
        lines.push(escaped);
    }
    format!("<p>{}</p>", lines.join("\n"))
}

/// `/^[^\p{P}\p{S}\p{Z}](?![.)])/u`: starts like a sentence, not like `a)` or `1.`.
fn continues(line: &str) -> bool {
    let mut chars = line.chars();
    CONTINUATION.is_match(line) && {
        chars.next();
        !matches!(chars.next(), Some('.' | ')'))
    }
}

/// lodash's `escape`.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_paragraphs_lines_and_quotes() {
        assert_eq!(
            to_html(
                "Hi Ann,\n\nThanks for the notes.\nSee you <soon>.\n\n> On Mon you wrote:\n> old"
            ),
            "<p>Hi Ann,\n</p><p>\nThanks for the notes.\n<br/>\nSee you &lt;soon&gt;.\n</p><blockquote><p>\nOn Mon you wrote:\n<br/>\nold</p>"
        );
    }
}
