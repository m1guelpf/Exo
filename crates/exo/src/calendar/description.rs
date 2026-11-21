//! An event's description as the event details show it: Calendar's boilerplate lines dropped and
//! newlines made `<br>`s, sanitized like a message (exo-mail's sanitizer, `<style>` forbidden),
//! then read into lines of text and links: blocks and `<br>`s break lines, list items are
//! indented 32px behind a bullet or number, `<b>`/`<strong>` are bold.

use std::sync::LazyLock;

use html5ever::{parse_document, tendril::TendrilSink as _};
use markup5ever_rcdom::{Handle, NodeData, RcDom};
use regex::Regex;

use super::details::{Line, Run};

/// How far a list sits in: `ul, ol { margin-left: 32px }`.
const LIST_INDENT: f32 = 32.0;

/// Boilerplate that descriptions carry: edit warnings, event links and divider lines.
static USELESS: LazyLock<[Regex; 5]> = LazyLock::new(|| {
    [
        r"Please do not edit this section of the description.",
        r"Do not edit this section of the description.",
        r"View your event at http(.+)($|\s)",
        r"(?m)^-::(~:{1,2})+-$",
        r"(?m)[-_.]{50,}",
    ]
    .map(|pattern| Regex::new(pattern).expect("valid regex"))
});

/// `description` without Calendar's boilerplate, as an invite's About has it: the first three
/// patterns drop their first match, the others every one; then trimmed.
pub(crate) fn clean(description: &str) -> String {
    let mut cleaned = description.to_owned();
    for (at, pattern) in USELESS.iter().enumerate() {
        cleaned = if at < 3 {
            pattern.replace(&cleaned, "")
        } else {
            pattern.replace_all(&cleaned, "")
        }
        .into_owned();
    }
    cleaned.trim().to_owned()
}

/// The lines `description` shows as; none when it has no text.
pub fn lines(description: &str) -> Vec<Line> {
    let mut cleaned = description.to_owned();
    // Newlines become `<br/>` inside the loop, after the first pattern.
    for pattern in USELESS.iter() {
        cleaned = pattern.replace(&cleaned, "").replace('\n', "<br/>");
    }
    let html = exo_mail::sanitize_description(cleaned.trim());
    let document = parse_document(RcDom::default(), Default::default()).one(html);
    let mut reader = Reader::default();
    reader.children(&document.document, &Style::default());
    reader.flush();
    while reader.lines.last().is_some_and(|line| line.runs.is_empty()) {
        reader.lines.pop();
    }
    if reader
        .lines
        .iter()
        .all(|line| line.runs.iter().all(|run| run.text.trim().is_empty()))
    {
        return Vec::new();
    }
    reader.lines
}

/// What the text being read is inside.
#[derive(Clone, Default)]
struct Style {
    link: Option<String>,
    bold: bool,
    indent: f32,
}

#[derive(Default)]
struct Reader {
    lines: Vec<Line>,
    current: Line,
    /// The lists being read: ordered or not, and items so far.
    lists: Vec<(bool, usize)>,
}

impl Reader {
    fn children(&mut self, node: &Handle, style: &Style) {
        for child in node.children.borrow().iter() {
            self.node(child, style);
        }
    }

    fn node(&mut self, node: &Handle, style: &Style) {
        match &node.data {
            NodeData::Text { contents } => self.text(&contents.borrow(), style),
            NodeData::Element { name, attrs, .. } => {
                let tag: &str = &name.local;
                let attr = |key: &str| {
                    attrs
                        .borrow()
                        .iter()
                        .find(|attr| &*attr.name.local == key)
                        .map(|attr| attr.value.to_string())
                };
                let mut inner = style.clone();
                match tag {
                    "br" => return self.flush_line(style),
                    "a" => inner.link = attr("href"),
                    "b" | "strong" => inner.bold = true,
                    "ul" | "ol" => inner.indent += LIST_INDENT,
                    "script" | "style" | "head" | "title" => return,
                    _ => {}
                }
                let block = matches!(
                    tag,
                    "p" | "div"
                        | "li"
                        | "ul"
                        | "ol"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "blockquote"
                        | "pre"
                        | "table"
                        | "tr"
                );
                if block {
                    self.flush();
                }
                if matches!(tag, "ul" | "ol") {
                    self.lists.push((tag == "ol", 0));
                }
                if tag == "li" {
                    let marker = match self.lists.last_mut() {
                        Some((true, count)) => {
                            *count += 1;
                            format!("{count}. ")
                        }
                        _ => "• ".to_owned(),
                    };
                    self.current.indent = inner.indent;
                    self.current.runs.push(Run {
                        text: marker,
                        ..Run::default()
                    });
                }
                self.children(node, &inner);
                if matches!(tag, "ul" | "ol") {
                    self.lists.pop();
                }
                if block {
                    self.flush();
                }
            }
            _ => self.children(node, style),
        }
    }

    /// Text, its whitespace collapsed (`white-space: normal`).
    fn text(&mut self, text: &str, style: &Style) {
        let mut collapsed = String::new();
        let mut space = self
            .current
            .runs
            .last()
            .is_none_or(|run| run.text.ends_with(' '));
        for c in text.chars() {
            if c.is_whitespace() {
                if !space {
                    collapsed.push(' ');
                }
                space = true;
            } else {
                collapsed.push(c);
                space = false;
            }
        }
        if collapsed.is_empty() {
            return;
        }
        if self.current.runs.is_empty() {
            self.current.indent = style.indent;
        }
        match self.current.runs.last_mut() {
            Some(run) if run.link == style.link && run.bold == style.bold => {
                run.text.push_str(&collapsed)
            }
            _ => self.current.runs.push(Run {
                text: collapsed,
                link: style.link.clone(),
                bold: style.bold,
            }),
        }
    }

    /// Ends the line, even an empty one (a `<br>`).
    fn flush_line(&mut self, style: &Style) {
        let mut line = std::mem::take(&mut self.current);
        if line.runs.is_empty() {
            line.indent = style.indent;
        }
        self.lines.push(line);
    }

    /// Ends the line if it has text (a block's edge).
    fn flush(&mut self) {
        if !self.current.runs.is_empty() {
            self.lines.push(std::mem::take(&mut self.current));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|line| line.runs.iter().map(|run| run.text.as_str()).collect())
            .collect()
    }

    #[test]
    fn plain_text_keeps_its_lines() {
        let lines = lines("Agenda:\n- metrics\n\nThanks");
        assert_eq!(texts(&lines), ["Agenda:", "- metrics", "", "Thanks"]);
    }

    #[test]
    fn html_reads_into_blocks_links_and_lists() {
        let lines = lines(
            "<p>Join <a href=\"https://example.com/x\">here</a></p><ul><li>One</li><li><b>Two</b></li></ul><ol><li>A</li><li>B</li></ol>",
        );
        assert_eq!(
            texts(&lines),
            ["Join here", "• One", "• Two", "1. A", "2. B"]
        );
        assert_eq!(
            lines[0].runs[1].link.as_deref(),
            Some("https://example.com/x")
        );
        assert_eq!(lines[1].indent, LIST_INDENT);
        assert!(lines[2].runs[1].bold);
    }

    #[test]
    fn scripts_styles_and_unsafe_links_are_dropped() {
        let lines = lines("<style>p{}</style><script>x()</script><a href=\"javascript:x()\">y</a>");
        assert_eq!(texts(&lines), ["y"]);
        assert_eq!(lines[0].runs[0].link, None);
    }

    #[test]
    fn boilerplate_alone_is_no_description() {
        assert!(lines("Please do not edit this section of the description.\n\n").is_empty());
        // Newlines become `<br/>`s after the first pattern, so the line-anchored ones only match
        // a whole description.
        assert_eq!(texts(&lines("Agenda\n-::~:~::-")), ["Agenda", "-::~:~::-"]);
    }
}
