//! The `text/plain` alternative of a sent message, derived from its HTML in the style of
//! `jaytaylor/html2text`: blocks on their own lines, paragraphs and headings set off by a blank
//! line, links as `text ( url )`, list items as `* `, quotes as `> `.

use html5ever::{parse_document, tendril::TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

/// `html` as plain text, with CRLF line ends.
pub(crate) fn from_html(html: &str) -> String {
    let dom = parse_document(RcDom::default(), Default::default()).one(html);
    let mut out = Writer::default();
    out.node(&dom.document);
    out.finish()
}

/// The text of `html` the user wrote: quotes, the signature and quoted messages left out.
pub(crate) fn authored(html: &str) -> String {
    let dom = parse_document(RcDom::default(), Default::default()).one(html);
    let mut out = Writer {
        authored: true,
        ..Writer::default()
    };
    out.node(&dom.document);
    out.finish().replace("\r\n", "\n")
}

/// Classes whose contents the user didn't write: the revealed signature and quote, as Exo
/// marks them, and Gmail's.
const NOT_AUTHORED: [&str; 4] = [
    "sh-signature",
    "gmail_signature",
    "sh-quoted-content",
    "gmail_quote",
];

#[derive(Default)]
struct Writer {
    /// Leaves out what the user didn't write ([`authored`]).
    authored: bool,
    lines: Vec<String>,
    line: String,
    /// Blank lines wanted before the next text.
    gap: usize,
    /// `> ` levels.
    quote: usize,
    /// Inside `<pre>`: whitespace kept.
    pre: bool,
}

impl Writer {
    fn node(&mut self, node: &Handle) {
        match &node.data {
            NodeData::Text { contents } => self.text(&contents.borrow()),
            NodeData::Element { name, attrs, .. } => {
                let tag: &str = &name.local;
                let attr = |wanted: &str| {
                    attrs
                        .borrow()
                        .iter()
                        .find(|attr| &*attr.name.local == wanted)
                        .map(|attr| attr.value.to_string())
                };
                if self.authored
                    && (tag == "blockquote"
                        || attr("class").is_some_and(|class| {
                            class
                                .split_whitespace()
                                .any(|class| NOT_AUTHORED.contains(&class))
                        }))
                {
                    return;
                }
                match tag {
                    "head" | "style" | "script" | "title" => {}
                    "br" => self.newline(),
                    "hr" => {
                        self.block(1);
                        self.text("--------");
                        self.block(1);
                    }
                    "img" => {
                        if let Some(alt) = attr("alt").filter(|alt| !alt.trim().is_empty()) {
                            self.text(&alt);
                        }
                    }
                    "a" => {
                        let start = self.flat().len();
                        self.children(node);
                        let text = self.flat()[start..].trim().to_owned();
                        if let Some(href) = attr("href").filter(|href| !href.is_empty()) {
                            let href = href.strip_prefix("mailto:").unwrap_or(&href).to_owned();
                            if text.is_empty() {
                                self.text(&href);
                            } else if text != href {
                                self.text(&format!(" ( {href} )"));
                            }
                        }
                    }
                    "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "table" => {
                        self.block(2);
                        self.children(node);
                        self.block(2);
                    }
                    "blockquote" => {
                        self.block(1);
                        self.quote += 1;
                        self.children(node);
                        self.quote -= 1;
                        self.block(1);
                    }
                    "pre" => {
                        self.block(1);
                        self.pre = true;
                        self.children(node);
                        self.pre = false;
                        self.block(1);
                    }
                    "li" => {
                        self.block(1);
                        self.text("* ");
                        self.children(node);
                        self.block(1);
                    }
                    "div" | "ul" | "ol" | "tr" | "section" | "article" | "header" | "footer"
                    | "body" | "html" => {
                        self.block(1);
                        self.children(node);
                        self.block(1);
                    }
                    "td" | "th" => {
                        self.children(node);
                        self.text(" ");
                    }
                    _ => self.children(node),
                }
            }
            _ => self.children(node),
        }
    }

    fn children(&mut self, node: &Handle) {
        for child in node.children.borrow().iter() {
            self.node(child);
        }
    }

    /// What's on the current line, for links to see their text.
    fn flat(&self) -> &str {
        &self.line
    }

    fn text(&mut self, text: &str) {
        let text = text.replace('\u{a0}', " ");
        if self.pre {
            for (index, part) in text.split('\n').enumerate() {
                if index > 0 {
                    self.newline();
                }
                self.push(part);
            }
            return;
        }
        let mut collapsed = String::with_capacity(text.len());
        let mut space = false;
        for ch in text.chars() {
            if ch.is_whitespace() {
                space = true;
            } else {
                if space
                    && (!collapsed.is_empty() || !self.line.is_empty() && !self.line.ends_with(' '))
                {
                    collapsed.push(' ');
                }
                space = false;
                collapsed.push(ch);
            }
        }
        if space && !collapsed.is_empty() {
            collapsed.push(' ');
        }
        if !collapsed.is_empty() {
            self.push(&collapsed);
        }
    }

    fn push(&mut self, text: &str) {
        if self.line.is_empty() {
            for _ in 0..self.gap {
                self.lines.push(self.prefix().trim_end().to_owned());
            }
            self.gap = 0;
        }
        self.line.push_str(text);
    }

    fn prefix(&self) -> String {
        "> ".repeat(self.quote)
    }

    fn newline(&mut self) {
        let line = std::mem::take(&mut self.line);
        self.lines
            .push(format!("{}{}", self.prefix(), line.trim_end()));
    }

    /// Ends the current line, if it has text, and asks for `blank - 1` blank lines before more.
    fn block(&mut self, blank: usize) {
        if !self.line.trim().is_empty() {
            self.newline();
        } else {
            self.line.clear();
        }
        if !self.lines.is_empty() {
            self.gap = self.gap.max(blank - 1);
        }
    }

    fn finish(mut self) -> String {
        if !self.line.trim().is_empty() {
            self.newline();
        }
        while self.lines.last().is_some_and(|line| line.trim().is_empty()) {
            self.lines.pop();
        }
        self.lines.join("\r\n")
    }
}

#[cfg(test)]
mod tests {
    use super::{authored, from_html};

    #[test]
    fn blocks_become_lines_and_links_show_their_address() {
        let html = "<div><div><div><html><head></head><body><div>Hi Ann,</div><div><br></div><div>See <a href=\"https://x.com/a\">the doc</a>.</div></body></html></div><div></div><br/></div></div>";
        assert_eq!(
            from_html(html),
            "Hi Ann,\r\n\r\nSee the doc ( https://x.com/a )."
        );
    }

    #[test]
    fn quotes_and_lists_are_marked() {
        let html = "<div>On Tue, Ann wrote:<br><blockquote><div>First</div><div>Second</div></blockquote><ul><li>one</li><li>two</li></ul></div>";
        assert_eq!(
            from_html(html),
            "On Tue, Ann wrote:\r\n> First\r\n> Second\r\n* one\r\n* two"
        );
    }

    #[test]
    fn authored_text_leaves_out_revealed_signatures_and_quotes() {
        let html = "<div>See attached</div><div class=\"sh-signature\"><div>Maya</div></div><div class=\"sh-quoted-content\"><div>the deck</div></div>";
        assert_eq!(authored(html), "See attached");
    }
}
