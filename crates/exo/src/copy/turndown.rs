//! HTML to Markdown, as `turndown` (MIT, Dom Christie) converts it with its default options: setext
//! headings, `*` bullets, `_` emphasis, `**` strong, indented code, `* * *` rules, and a two-space
//! line break.
//!
//! It runs on its own small tree, since the serializer edits the message before converting it
//! and turndown edits it again (whitespace collapsing).

use html5ever::{parse_document, tendril::TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

/// `blockElements`.
const BLOCK: &[&str] = &[
    "ADDRESS",
    "ARTICLE",
    "ASIDE",
    "AUDIO",
    "BLOCKQUOTE",
    "BODY",
    "CANVAS",
    "CENTER",
    "DD",
    "DIR",
    "DIV",
    "DL",
    "DT",
    "FIELDSET",
    "FIGCAPTION",
    "FIGURE",
    "FOOTER",
    "FORM",
    "FRAMESET",
    "H1",
    "H2",
    "H3",
    "H4",
    "H5",
    "H6",
    "HEADER",
    "HGROUP",
    "HR",
    "HTML",
    "ISINDEX",
    "LI",
    "MAIN",
    "MENU",
    "NAV",
    "NOFRAMES",
    "NOSCRIPT",
    "OL",
    "OUTPUT",
    "P",
    "PRE",
    "SECTION",
    "TABLE",
    "TBODY",
    "TD",
    "TFOOT",
    "TH",
    "THEAD",
    "TR",
    "UL",
];
/// `voidElements`.
const VOID: &[&str] = &[
    "AREA", "BASE", "BR", "COL", "COMMAND", "EMBED", "HR", "IMG", "INPUT", "KEYGEN", "LINK",
    "META", "PARAM", "SOURCE", "TRACK", "WBR",
];
/// `meaningfulWhenBlankElements`.
const MEANINGFUL_WHEN_BLANK: &[&str] = &[
    "A", "TABLE", "THEAD", "TBODY", "TFOOT", "TH", "TD", "IFRAME", "SCRIPT", "AUDIO", "VIDEO",
];

/// A node of the tree; `name` is upper-case, as the DOM has it.
#[derive(Clone, Debug)]
pub(super) enum Kind {
    Element {
        name: String,
        attributes: Vec<(String, String)>,
    },
    Text(String),
    /// Comments and the like, which whitespace collapsing removes.
    Other,
}

#[derive(Clone, Debug)]
struct Node {
    kind: Kind,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// A mutable HTML tree. Node 0 is the root, whose children are the converted content.
#[derive(Clone, Debug)]
pub(super) struct Tree {
    nodes: Vec<Node>,
}

/// JavaScript's `\s`.
fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// HTML's ASCII whitespace, which turndown collapses (`[ \r\n\t]`).
fn is_ascii_space(c: char) -> bool {
    matches!(c, ' ' | '\r' | '\n' | '\t')
}

impl Tree {
    /// `html`'s body, under a root.
    pub(super) fn parse(html: &str) -> Self {
        let dom = parse_document(RcDom::default(), Default::default()).one(html);
        let mut tree = Self {
            nodes: vec![Node {
                kind: Kind::Element {
                    name: "X-TURNDOWN".into(),
                    attributes: Vec::new(),
                },
                parent: None,
                children: Vec::new(),
            }],
        };
        if let Some(body) = find(&dom.document, "body") {
            for child in body.children.borrow().iter() {
                tree.import(child, 0);
            }
        }
        tree
    }

    fn import(&mut self, handle: &Handle, parent: usize) {
        let kind = match &handle.data {
            NodeData::Element { name, attrs, .. } => Kind::Element {
                name: name.local.to_uppercase(),
                attributes: attrs
                    .borrow()
                    .iter()
                    .map(|attr| (attr.name.local.to_string(), attr.value.to_string()))
                    .collect(),
            },
            NodeData::Text { contents } => Kind::Text(contents.borrow().to_string()),
            _ => Kind::Other,
        };
        let id = self.push(kind, parent);
        for child in handle.children.borrow().iter() {
            self.import(child, id);
        }
    }

    pub(super) fn push(&mut self, kind: Kind, parent: usize) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node {
            kind,
            parent: Some(parent),
            children: Vec::new(),
        });
        self.nodes[parent].children.push(id);
        id
    }

    pub(super) fn kind(&self, id: usize) -> &Kind {
        &self.nodes[id].kind
    }

    pub(super) fn name(&self, id: usize) -> Option<&str> {
        match &self.nodes[id].kind {
            Kind::Element { name, .. } => Some(name),
            _ => None,
        }
    }

    pub(super) fn attribute(&self, id: usize, name: &str) -> Option<&str> {
        match &self.nodes[id].kind {
            Kind::Element { attributes, .. } => attributes
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str()),
            _ => None,
        }
    }

    fn parent(&self, id: usize) -> Option<usize> {
        self.nodes[id].parent
    }

    fn children(&self, id: usize) -> &[usize] {
        &self.nodes[id].children
    }

    fn sibling(&self, id: usize, offset: isize) -> Option<usize> {
        let parent = self.parent(id)?;
        let siblings = self.children(parent);
        let at = siblings.iter().position(|child| *child == id)?;
        siblings.get(at.checked_add_signed(offset)?).copied()
    }

    /// Every element named one of `names`, in document order (`querySelectorAll`).
    pub(super) fn elements(&self, names: &[&str]) -> Vec<usize> {
        let mut found = Vec::new();
        let mut stack = vec![0];
        while let Some(id) = stack.pop() {
            if id != 0 && self.name(id).is_some_and(|name| names.contains(&name)) {
                found.push(id);
            }
            stack.extend(self.children(id).iter().rev());
        }
        found
    }

    /// `textContent`.
    pub(super) fn text(&self, id: usize) -> String {
        match &self.nodes[id].kind {
            Kind::Text(text) => text.clone(),
            Kind::Other => String::new(),
            Kind::Element { .. } => self
                .children(id)
                .iter()
                .map(|child| self.text(*child))
                .collect(),
        }
    }

    fn has_descendant(&self, id: usize, names: &[&str]) -> bool {
        self.children(id).iter().any(|child| {
            self.name(*child).is_some_and(|name| names.contains(&name))
                || self.has_descendant(*child, names)
        })
    }

    /// Takes `id` out of the tree, which it leaves with its subtree.
    pub(super) fn detach(&mut self, id: usize) {
        if let Some(parent) = self.nodes[id].parent.take() {
            self.nodes[parent].children.retain(|child| *child != id);
        }
    }

    /// Puts `new` (a detached node) where `old` is.
    pub(super) fn replace(&mut self, old: usize, new: usize) {
        let Some(parent) = self.nodes[old].parent.take() else {
            return;
        };
        if let Some(at) = self.nodes[parent]
            .children
            .iter()
            .position(|child| *child == old)
        {
            self.nodes[parent].children[at] = new;
        }
        self.detach(new);
        self.nodes[new].parent = Some(parent);
    }

    /// A new element with no parent yet.
    pub(super) fn create(&mut self, name: &str) -> usize {
        self.nodes.push(Node {
            kind: Kind::Element {
                name: name.into(),
                attributes: Vec::new(),
            },
            parent: None,
            children: Vec::new(),
        });
        self.nodes.len() - 1
    }

    /// Sets an element's text: its children become `text`, with `<br>` for line breaks.
    pub(super) fn set_inner_text(&mut self, id: usize, text: &str) {
        for child in std::mem::take(&mut self.nodes[id].children) {
            self.nodes[child].parent = None;
        }
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                let br = self.create("BR");
                self.nodes[br].parent = Some(id);
                self.nodes[id].children.push(br);
            }
            if !line.is_empty() {
                self.push(Kind::Text(line.to_owned()), id);
            }
        }
    }

    fn is_block(&self, id: usize) -> bool {
        self.name(id).is_some_and(|name| BLOCK.contains(&name))
    }

    fn is_pre(&self, id: usize) -> bool {
        self.name(id) == Some("PRE")
    }

    /// Runs of HTML whitespace become one space, and spaces at the edges of blocks and next to
    /// other spaces go.
    fn collapse_whitespace(&mut self) {
        if self.children(0).is_empty() {
            return;
        }
        let mut previous_text: Option<usize> = None;
        let mut keep_leading = false;
        let mut previous: Option<usize> = None;
        let mut node = self.next(None, 0);
        while node != 0 {
            match self.nodes[node].kind.clone() {
                Kind::Text(text) => {
                    let mut text = collapse(&text);
                    let after_space = previous_text.is_none_or(|previous| matches!(&self.nodes[previous].kind, Kind::Text(text) if text.ends_with(' ')));
                    if after_space && !keep_leading && text.starts_with(' ') {
                        text.remove(0);
                    }
                    if text.is_empty() {
                        node = self.remove(node);
                        continue;
                    }
                    self.nodes[node].kind = Kind::Text(text);
                    previous_text = Some(node);
                }
                Kind::Element { name, .. } => {
                    if self.is_block(node) || name == "BR" {
                        if let Some(previous) = previous_text {
                            self.trim_trailing_space(previous);
                        }
                        previous_text = None;
                        keep_leading = false;
                    } else if VOID.contains(&name.as_str()) || self.is_pre(node) {
                        previous_text = None;
                        keep_leading = true;
                    } else if previous_text.is_some() {
                        keep_leading = false;
                    }
                }
                Kind::Other => {
                    node = self.remove(node);
                    continue;
                }
            }
            let next = self.next(previous, node);
            previous = Some(node);
            node = next;
        }
        if let Some(previous) = previous_text {
            self.trim_trailing_space(previous);
            if matches!(&self.nodes[previous].kind, Kind::Text(text) if text.is_empty()) {
                self.remove(previous);
            }
        }
    }

    fn trim_trailing_space(&mut self, id: usize) {
        if let Kind::Text(text) = &mut self.nodes[id].kind
            && text.ends_with(' ')
        {
            text.pop();
        }
    }

    /// The node after `current` in a walk that doesn't enter `<pre>`, having come from
    /// `previous` (`next`).
    fn next(&self, previous: Option<usize>, current: usize) -> usize {
        let up = previous.is_some_and(|previous| self.parent(previous) == Some(current))
            || self.is_pre(current);
        let parent = self.parent(current).unwrap_or(0);
        if up {
            self.sibling(current, 1).unwrap_or(parent)
        } else {
            self.children(current)
                .first()
                .copied()
                .or_else(|| self.sibling(current, 1))
                .unwrap_or(parent)
        }
    }

    /// Removes `id` and returns where the walk goes next (`remove`).
    fn remove(&mut self, id: usize) -> usize {
        let next = self.sibling(id, 1).or_else(|| self.parent(id)).unwrap_or(0);
        self.detach(id);
        next
    }
}

fn find(handle: &Handle, name: &str) -> Option<Handle> {
    if let NodeData::Element { name: element, .. } = &handle.data
        && &*element.local == name
    {
        return Some(handle.clone());
    }
    handle
        .children
        .borrow()
        .iter()
        .find_map(|child| find(child, name))
}

/// `[ \r\n\t]+` → one space.
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for c in text.chars() {
        if is_ascii_space(c) {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

/// Converts `tree`'s content to Markdown.
pub(super) fn turndown(mut tree: Tree) -> String {
    tree.collapse_whitespace();
    let output = process(&tree, 0, false);
    // No line breaks or tabs at the start, nor whitespace at the end.
    output
        .trim_start_matches(['\t', '\r', '\n'])
        .trim_end_matches(|c: char| matches!(c, '\t' | '\r' | '\n') || is_space(c))
        .to_owned()
}

fn process(tree: &Tree, id: usize, code: bool) -> String {
    let mut output = String::new();
    for &child in tree.children(id) {
        let replacement = match tree.kind(child) {
            Kind::Text(text) if code => text.clone(),
            Kind::Text(text) => escape(text),
            Kind::Element { name, .. } => replacement(tree, child, code || name == "CODE"),
            Kind::Other => String::new(),
        };
        output = join(&output, &replacement);
    }
    output
}

/// Node `id` as Markdown: its rule's, with the whitespace at its edges moved outside.
fn replacement(tree: &Tree, id: usize, code: bool) -> String {
    let mut content = process(tree, id, code);
    let (leading, trailing) = flanking_whitespace(tree, id);
    if !leading.is_empty() || !trailing.is_empty() {
        content = content.trim_matches(is_space).to_owned();
    }
    format!("{leading}{}{trailing}", rule(tree, id, &content))
}

fn is_blank(tree: &Tree, id: usize) -> bool {
    let name = tree.name(id).unwrap_or_default();
    !VOID.contains(&name)
        && !MEANINGFUL_WHEN_BLANK.contains(&name)
        && tree.text(id).chars().all(is_space)
        && !tree.has_descendant(id, VOID)
        && !tree.has_descendant(id, MEANINGFUL_WHEN_BLANK)
}

/// The whitespace at the edges of an inline element's text, which moves outside its Markdown; ASCII
/// whitespace already next to a space is dropped.
fn flanking_whitespace(tree: &Tree, id: usize) -> (String, String) {
    if tree.is_block(id) {
        return (String::new(), String::new());
    }
    let text = tree.text(id);
    let chars: Vec<char> = text.chars().collect();
    let ascii_lead = chars.iter().take_while(|c| is_ascii_space(**c)).count();
    let lead = ascii_lead
        + chars[ascii_lead..]
            .iter()
            .take_while(|c| is_space(**c))
            .count();
    let mut leading: String = chars[..lead].iter().collect();
    let (trailing_non_ascii, trailing_ascii) = if lead == chars.len() {
        (String::new(), String::new())
    } else {
        let ascii_trail = chars
            .iter()
            .rev()
            .take_while(|c| is_ascii_space(**c))
            .count();
        let trail = ascii_trail
            + chars[..chars.len() - ascii_trail]
                .iter()
                .rev()
                .take_while(|c| is_space(**c))
                .count();
        (
            chars[chars.len() - trail..chars.len() - ascii_trail]
                .iter()
                .collect::<String>(),
            chars[chars.len() - ascii_trail..]
                .iter()
                .collect::<String>(),
        )
    };
    let mut trailing = format!("{trailing_non_ascii}{trailing_ascii}");
    if ascii_lead > 0 && flanked(tree, id, -1) {
        leading = chars[ascii_lead..lead].iter().collect();
    }
    if !trailing_ascii.is_empty() && flanked(tree, id, 1) {
        trailing = trailing_non_ascii;
    }
    (leading, trailing)
}

/// Whether the sibling before (-1) or after (1) ends or starts with a space.
fn flanked(tree: &Tree, id: usize, side: isize) -> bool {
    let Some(sibling) = tree.sibling(id, side) else {
        return false;
    };
    let edge = |text: &str| {
        if side < 0 {
            text.ends_with(' ')
        } else {
            text.starts_with(' ')
        }
    };
    match tree.kind(sibling) {
        Kind::Text(text) => edge(text),
        Kind::Element { .. } if !tree.is_block(sibling) => edge(&tree.text(sibling)),
        _ => false,
    }
}

/// The first rule that applies to element `id`, applied to its converted `content`.
fn rule(tree: &Tree, id: usize, content: &str) -> String {
    let block = tree.is_block(id);
    if is_blank(tree, id) {
        return if block { "\n\n".into() } else { String::new() };
    }
    let name = tree.name(id).unwrap_or_default();
    match name {
        "P" => format!("\n\n{content}\n\n"),
        "BR" => "  \n".into(),
        "H1" | "H2" => {
            let underline =
                if name == "H1" { "=" } else { "-" }.repeat(content.encode_utf16().count());
            format!("\n\n{content}\n{underline}\n\n")
        }
        "H3" | "H4" | "H5" | "H6" => {
            let level = name[1..].parse::<usize>().unwrap_or(3);
            format!("\n\n{} {content}\n\n", "#".repeat(level))
        }
        "BLOCKQUOTE" => {
            let quoted = content
                .trim_matches('\n')
                .lines()
                .map(|line| format!("> {line}"))
                .collect::<Vec<_>>()
                .join("\n");
            format!("\n\n{quoted}\n\n")
        }
        "UL" | "OL" => {
            let parent = tree.parent(id);
            let last_in_item = parent.is_some_and(|parent| {
                tree.name(parent) == Some("LI")
                    && tree
                        .children(parent)
                        .iter()
                        .rev()
                        .find(|child| tree.name(**child).is_some())
                        == Some(&id)
            });
            if last_in_item {
                format!("\n{content}")
            } else {
                format!("\n\n{content}\n\n")
            }
        }
        "LI" => list_item(tree, id, content),
        "PRE"
            if tree
                .children(id)
                .first()
                .is_some_and(|first| tree.name(*first) == Some("CODE")) =>
        {
            let code = tree
                .children(id)
                .first()
                .map(|first| tree.text(*first))
                .unwrap_or_default();
            format!("\n\n    {}\n\n", code.replace('\n', "\n    "))
        }
        "HR" => "\n\n* * *\n\n".into(),
        "EM" | "I" if !content.trim_matches(is_space).is_empty() => format!("_{content}_"),
        "STRONG" | "B" if !content.trim_matches(is_space).is_empty() => format!("**{content}**"),
        "EM" | "I" | "STRONG" | "B" => String::new(),
        "CODE"
            if !(tree
                .parent(id)
                .is_some_and(|parent| tree.name(parent) == Some("PRE"))
                && tree.sibling(id, -1).or(tree.sibling(id, 1)).is_none()) =>
        {
            code(content)
        }
        _ if block => format!("\n\n{content}\n\n"),
        _ => content.to_owned(),
    }
}

/// A list item: `*` or its number, and its content indented under it.
fn list_item(tree: &Tree, id: usize, content: &str) -> String {
    let content = content.trim_start_matches('\n');
    let content = match content.trim_end_matches('\n') {
        trimmed if trimmed.len() < content.len() => format!("{trimmed}\n"),
        _ => content.to_owned(),
    };
    let content = content.replace('\n', "\n    ");
    let parent = tree.parent(id);
    let prefix = match parent.filter(|parent| tree.name(*parent) == Some("OL")) {
        Some(list) => {
            let index = tree
                .children(list)
                .iter()
                .filter(|child| tree.name(**child).is_some())
                .position(|child| *child == id)
                .unwrap_or(0);
            let start = tree
                .attribute(list, "start")
                .and_then(|start| start.trim().parse::<i64>().ok());
            format!(
                "{}.  ",
                start.map_or(index as i64 + 1, |start| start + index as i64)
            )
        }
        None => "*   ".into(),
    };
    let newline = tree.sibling(id, 1).is_some() && !content.ends_with('\n');
    format!("{prefix}{content}{}", if newline { "\n" } else { "" })
}

/// The `code` rule: backticks, more of them than any run inside.
fn code(content: &str) -> String {
    if content.is_empty() {
        return String::new();
    }
    let content = content.replace("\r\n", " ").replace(['\r', '\n'], " ");
    // `/^`|^ .*?[^ ].* $|`$/`.
    let padded = content.starts_with('`')
        || content.ends_with('`')
        || (content.len() > 1
            && content.starts_with(' ')
            && content.ends_with(' ')
            && content.chars().any(|c| c != ' '));
    let extra = if padded { " " } else { "" };
    let mut delimiter = "`".to_owned();
    let runs: Vec<usize> = content
        .split(|c| c != '`')
        .map(str::len)
        .filter(|len| *len > 0)
        .collect();
    while runs.contains(&delimiter.len()) {
        delimiter.push('`');
    }
    format!("{delimiter}{extra}{content}{extra}{delimiter}")
}

/// Markdown characters in text get a backslash. The anchored rules only apply at the very start of
/// the text.
fn escape(text: &str) -> String {
    let mut text = text.replace('\\', "\\\\").replace('*', "\\*");
    if text.starts_with('-') {
        text.insert(0, '\\');
    }
    if text.starts_with("+ ") {
        text.insert(0, '\\');
    }
    if text.starts_with('=') {
        text.insert(0, '\\');
    }
    let hashes = text.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && text[hashes..].starts_with(' ') {
        text.insert(0, '\\');
    }
    text = text.replace('`', "\\`");
    if text.starts_with("~~~") {
        text.insert(0, '\\');
    }
    text = text.replace('[', "\\[").replace(']', "\\]");
    if text.starts_with('>') {
        text.insert(0, '\\');
    }
    text = text.replace('_', "\\_");
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && text[digits..].starts_with(". ") {
        text.insert(digits, '\\');
    }
    text
}

/// `join`: the newlines between two pieces are the larger of their own, at most two.
fn join(output: &str, replacement: &str) -> String {
    let head = output.trim_end_matches('\n');
    let tail = replacement.trim_start_matches('\n');
    let newlines = (output.len() - head.len())
        .max(replacement.len() - tail.len())
        .min(2);
    format!("{head}{}{tail}", "\n".repeat(newlines))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(html: &str) -> String {
        turndown(Tree::parse(html))
    }

    #[test]
    fn blocks_and_inline_formatting() {
        assert_eq!(
            md("<p>Hello <b>world</b></p><p>Again</p>"),
            "Hello **world**\n\nAgain"
        );
        assert_eq!(md("<div>one<br>two</div>"), "one  \ntwo");
        assert_eq!(md("<h1>Title</h1><h3>Sub</h3>"), "Title\n=====\n\n### Sub");
        assert_eq!(md("<ul><li>a</li><li>b</li></ul>"), "*   a\n*   b");
        assert_eq!(
            md("<ol start=\"3\"><li>a</li><li>b</li></ol>"),
            "3.  a\n4.  b"
        );
        assert_eq!(md("<blockquote><p>quoted</p></blockquote>"), "> quoted");
        assert_eq!(md("<p>a</p><hr><p>b</p>"), "a\n\n* * *\n\nb");
        assert_eq!(md("<i> spaced </i>out"), "_spaced_ out");
        assert_eq!(
            md("<pre><code>let x = 1;\nlet y;</code></pre>"),
            "    let x = 1;\n    let y;"
        );
        assert_eq!(md("use <code>a`b</code> here"), "use ``a`b`` here");
    }

    #[test]
    fn whitespace_collapses_like_a_browser() {
        assert_eq!(
            md("<div>\n  Hello,\n\n  world  </div>\n<div> next </div>"),
            "Hello, world\n\nnext"
        );
        assert_eq!(md("<span>a</span> <span>b</span>"), "a b");
    }

    #[test]
    fn markdown_characters_are_escaped() {
        assert_eq!(
            md("<p>1. not a list *really* [x] under_score</p>"),
            "1\\. not a list \\*really\\* \\[x\\] under\\_score"
        );
        assert_eq!(md("<p>- dash</p><p># hash</p>"), "\\- dash\n\n\\# hash");
    }

    #[test]
    fn tables_become_paragraphs() {
        assert_eq!(md("<table><tr><td>a</td><td>b</td></tr></table>"), "a\n\nb");
    }
}
