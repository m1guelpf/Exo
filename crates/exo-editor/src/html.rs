//! HTML in and out: a draft body loaded and saved as Squire does it, and the paste cleanup.
//!
//! [`write`] produces what Chromium's `innerHTML` gives for the DOM Squire would hold: block
//! elements with their attributes, inline elements nested as recorded, `<br>` in empty lines,
//! and spaces as Chromium's editing leaves them (`&nbsp;` where a plain space would collapse).
//! [`parse`] reads that back exactly, and reads foreign HTML (Gmail drafts, quoted replies,
//! signatures) the way Squire keeps it: unknown structure as containers, text directly inside a
//! container as a line of its own. Reading doesn't sanitize: the editor runs what it loads or
//! pastes through `exo_mail::sanitize_fragment` first.

use html5ever::{
    parse_document,
    serialize::{SerializeOpts, TraversalScope, serialize},
    tendril::TendrilSink as _,
};
use markup5ever_rcdom::{Handle, NodeData, RcDom, SerializableHandle};

use crate::model::{Block, Container, Content, Document, Element, Piece, Run};

/// Elements laid out as blocks (Squire's blocks and containers, plus the HTML block-level
/// elements pasted mail uses).
const BLOCK_TAGS: [&str; 33] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "center",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "ul",
];
/// Kept whole, as markup the editor doesn't edit inside.
const OPAQUE_TAGS: [&str; 1] = ["table"];
/// Left out with their contents: what Squire's clean-up drops, foreign content (SVG, MathML)
/// the editor can't show, and what runs or embeds other content, in case unsanitized markup
/// gets here.
const DROPPED_TAGS: [&str; 21] = [
    "applet",
    "base",
    "embed",
    "frame",
    "frameset",
    "head",
    "iframe",
    "link",
    "math",
    "meta",
    "noembed",
    "noframes",
    "noscript",
    "object",
    "plaintext",
    "script",
    "style",
    "svg",
    "template",
    "title",
    "xmp",
];
/// How deep elements nest: Chromium's parser (`kMaximumHTMLParserDOMTreeDepth`) adds deeper ones
/// as siblings, so Squire never holds more. It also bounds the reader's recursion.
const MAX_DEPTH: usize = 512;

/// Reads a draft body as Squire holds it once loaded. It doesn't sanitize.
pub fn parse(html: &str) -> Document {
    let (_document, root) = fragment(html);
    let mut reader = Reader::default();
    reader.container(&root, &[]);
    Document::new(reader.blocks)
}

/// Whether every top-level node of `html` is a `<blockquote>` or has no text.
pub(crate) fn is_blockquoted(html: &str) -> bool {
    fn text(node: &Handle) -> String {
        match &node.data {
            NodeData::Text { contents } => contents.borrow().to_string(),
            _ => node.children.borrow().iter().map(text).collect(),
        }
    }
    let (_document, root) = fragment(html);
    let children = root.children.borrow();
    children.iter().all(|child| {
        matches!(&child.data, NodeData::Element { name, .. } if &*name.local == "blockquote")
            || text(child).is_empty()
    })
}

/// Reads pasted HTML with the paste cleanup: only colour, weight, style and decoration survive as
/// inline styles, and the result is normalized the way Squire cleans what it inserts. It doesn't
/// sanitize.
pub(crate) fn parse_paste(html: &str) -> Document {
    let (_document, root) = fragment(html);
    let mut reader = Reader {
        paste: true,
        ..Reader::default()
    };
    reader.container(&root, &[]);
    Document::new(reader.blocks)
}

/// Parses `html` as a page's body would be (a `div`'s `innerHTML` parses the same way for the
/// content the editor holds): the document, which must outlive any use of the tree (dropping it
/// empties every node), and its `<body>`.
fn fragment(html: &str) -> (Handle, Handle) {
    let document = parse_document(RcDom::default(), Default::default())
        .one(html)
        .document;
    let root = find(&document, "html").unwrap_or_else(|| document.clone());
    let body = find(&root, "body").unwrap_or(root);
    prune(&body);
    (document, body)
}

/// Takes [`DROPPED_TAGS`] out of `root`, tables included, and gives elements deeper than
/// [`MAX_DEPTH`] their children as siblings. Iterative, so hostile nesting can't overflow the
/// stack.
fn prune(root: &Handle) {
    let mut levels = vec![(root.clone(), 0)];
    while let Some((node, depth)) = levels.pop() {
        let mut pending: std::collections::VecDeque<Handle> =
            std::mem::take(&mut *node.children.borrow_mut()).into();
        let mut kept = Vec::with_capacity(pending.len());
        while let Some(child) = pending.pop_front() {
            if let NodeData::Element { name, .. } = &child.data {
                if DROPPED_TAGS.contains(&&*name.local) {
                    continue;
                }
                if depth >= MAX_DEPTH {
                    for grandchild in std::mem::take(&mut *child.children.borrow_mut())
                        .into_iter()
                        .rev()
                    {
                        pending.push_front(grandchild);
                    }
                } else {
                    levels.push((child.clone(), depth + 1));
                }
            }
            kept.push(child);
        }
        *node.children.borrow_mut() = kept;
    }
}

fn find(node: &Handle, tag: &str) -> Option<Handle> {
    node.children
        .borrow()
        .iter()
        .find(|child| matches!(&child.data, NodeData::Element { name, .. } if &*name.local == tag))
        .cloned()
}

#[derive(Default)]
struct Reader {
    blocks: Vec<Block>,
    next_id: u32,
    paste: bool,
}

impl Reader {
    fn container(&mut self, node: &Handle, path: &[Container]) {
        let mut inline: Vec<Run> = Vec::new();
        for child in node.children.borrow().iter() {
            match &child.data {
                NodeData::Element { name, .. }
                    if is_block(&name.local) || OPAQUE_TAGS.contains(&&*name.local) =>
                {
                    self.flush_loose(&mut inline, path);
                    self.block(child, path);
                }
                NodeData::Element { name, .. } if DROPPED_TAGS.contains(&&*name.local) => {}
                NodeData::Element { .. } | NodeData::Text { .. } => {
                    self.inline(child, &[], &mut inline)
                }
                _ => {}
            }
        }
        self.flush_loose(&mut inline, path);
    }

    /// Text and inline elements directly inside a container: a line with no block element.
    fn flush_loose(&mut self, inline: &mut Vec<Run>, path: &[Container]) {
        let runs = std::mem::take(inline);
        let mut block = Block::div(path.to_vec(), runs);
        block.element = Element::new("");
        if finish(&mut block) || block.trailing_break {
            self.blocks.push(block);
        }
    }

    fn block(&mut self, node: &Handle, path: &[Container]) {
        let element = element_of(node, self.paste);
        if OPAQUE_TAGS.contains(&element.tag.as_str()) {
            let mut block = Block::div(path.to_vec(), Vec::new());
            block.element = Element::new("");
            block.content = Content::Raw {
                html: outer_html(node),
                text: text_of(node),
            };
            self.blocks.push(block);
            return;
        }
        if element.is("hr") {
            let mut block = Block::div(path.to_vec(), Vec::new());
            block.element = Element::new("");
            block.content = Content::Rule;
            self.blocks.push(block);
            return;
        }
        let children = node.children.borrow();
        let has_blocks = children.iter().any(|child| matches!(&child.data, NodeData::Element { name, .. } if is_block(&name.local) || OPAQUE_TAGS.contains(&&*name.local)));
        // `<div><hr></div>`: Squire's rule line.
        let only_rule = children
            .iter()
            .filter(|child| !is_blank(child))
            .collect::<Vec<_>>();
        if let [single] = only_rule.as_slice()
            && matches!(&single.data, NodeData::Element { name, .. } if &*name.local == "hr")
        {
            self.blocks.push(Block {
                path: path.to_vec(),
                element,
                content: Content::Rule,
                trailing_break: false,
            });
            return;
        }
        drop(children);
        let is_container =
            has_blocks || matches!(element.tag.as_str(), "blockquote" | "ul" | "ol" | "dl");
        if is_container {
            self.next_id += 1;
            let mut inner = path.to_vec();
            inner.push(Container {
                id: self.next_id,
                element,
            });
            let before = self.blocks.len();
            self.container(node, &inner);
            if self.blocks.len() == before {
                // An empty container still holds an empty line, as the browser shows it.
                let mut block = Block::empty(inner);
                block.element = Element::new("");
                self.blocks.push(block);
            }
            return;
        }
        let mut runs = Vec::new();
        for child in node.children.borrow().iter() {
            self.inline(child, &[], &mut runs);
        }
        let mut block = Block {
            path: path.to_vec(),
            element,
            content: Content::Inline(runs),
            trailing_break: false,
        };
        finish(&mut block);
        self.blocks.push(block);
    }

    fn inline(&mut self, node: &Handle, marks: &[Element], out: &mut Vec<Run>) {
        match &node.data {
            NodeData::Text { contents } => {
                let text = contents.borrow().replace('\u{FEFF}', "");
                if !text.is_empty() {
                    out.push(Run::text(text, marks.to_vec()));
                }
            }
            NodeData::Element { name, .. } => {
                let tag = &*name.local;
                if DROPPED_TAGS.contains(&tag) {
                    return;
                }
                match tag {
                    "br" => out.push(Run {
                        piece: Piece::Break,
                        marks: marks.to_vec(),
                    }),
                    "img" => out.push(Run {
                        piece: Piece::Image(element_of(node, false)),
                        marks: marks.to_vec(),
                    }),
                    "wbr" => {}
                    _ => {
                        let mut inner = marks.to_vec();
                        let element = element_of(node, self.paste);
                        if !element.tag.is_empty()
                            && !(self.paste && element.is("span") && element.attrs.is_empty())
                        {
                            inner.push(element);
                        }
                        for child in node.children.borrow().iter() {
                            self.inline(child, &inner, out);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn is_block(tag: &str) -> bool {
    BLOCK_TAGS.contains(&tag)
}

fn is_blank(node: &Handle) -> bool {
    matches!(&node.data, NodeData::Text { contents } if contents.borrow().chars().all(char::is_whitespace))
}

/// The element `node` is, with Squire's tag normalization (`strong` → `b`, `em` → `i`, `ins` →
/// `u`, `strike` → `s`) and, for pastes, the attribute cleanup.
fn element_of(node: &Handle, paste: bool) -> Element {
    let NodeData::Element { name, attrs, .. } = &node.data else {
        return Element::new("");
    };
    let tag = match &*name.local {
        "strong" => "b",
        "em" => "i",
        "ins" => "u",
        "strike" => "s",
        other => other,
    };
    let mut element = Element::new(tag);
    for attr in attrs.borrow().iter() {
        element
            .attrs
            .push((attr.name.local.to_string(), attr.value.to_string()));
    }
    if paste {
        clean_pasted(&mut element);
    }
    element
}

/// The paste's attribute rules: theme classes and `tabindex` go; non-table elements
/// keep only `color`, bold `font-weight`, `font-style` and `text-decoration` in their style.
fn clean_pasted(element: &mut Element) {
    const TABLE: [&str; 8] = [
        "table", "colgroup", "col", "tbody", "tr", "td", "thead", "th",
    ];
    element.attrs.retain(|(name, _)| {
        !matches!(
            name.as_str(),
            "tabindex" | "autoplay" | "data-slate-fragment" | "id" | "contenteditable"
        )
    });
    if let Some(class) = element.attr("class") {
        let kept: Vec<&str> = class
            .split_whitespace()
            .filter(|class| {
                !class.starts_with("Theme-")
                    && !matches!(*class, "ShadowHTML" | "Exo-quoting-ellipsis" | "exo-date")
            })
            .collect();
        let kept = kept.join(" ");
        if kept.is_empty() {
            element.attrs.retain(|(name, _)| name != "class");
        } else {
            element.set_attr("class", &kept);
        }
    }
    if TABLE.contains(&element.tag.as_str()) {
        return;
    }
    let style = element.attr("style").unwrap_or_default().to_owned();
    {
        let mut kept: Vec<String> = style
            .split(';')
            .filter_map(|declaration| {
                let (property, value) = declaration.split_once(':')?;
                let (property, value) = (property.trim().to_ascii_lowercase(), value.trim());
                match property.as_str() {
                    "color" => Some(format!("color:{value}")),
                    "font-style" if value != "normal" => Some(format!("font-style:{value}")),
                    "font-weight"
                        if matches!(value, "bold" | "bolder")
                            || value.parse::<u16>().is_ok_and(|weight| weight > 500) =>
                    {
                        Some("font-weight:700".to_owned())
                    }
                    property if property.starts_with("text-decoration") => {
                        Some(format!("{property}:{value}"))
                    }
                    _ => None,
                }
            })
            .collect();
        // Pasted paragraphs lose their margins (`P` and `.MsoNormal` get `margin:0px`).
        if element.is("p")
            || element
                .attr("class")
                .is_some_and(|class| class.contains("MsoNormal"))
        {
            kept.push("margin:0px".to_owned());
        }
        if kept.is_empty() {
            element.attrs.retain(|(name, _)| name != "style");
        } else {
            element.set_attr("style", &kept.join(";"));
        }
    }
}

/// Collapses whitespace as CSS lays it out, and moves the closing placeholder `<br>` into the
/// block's flag. Returns whether anything visible is left.
fn finish(block: &mut Block) -> bool {
    let code = block.in_code();
    let Content::Inline(runs) = &mut block.content else {
        return true;
    };
    if code {
        *runs = code_lines(std::mem::take(runs));
    } else {
        collapse_whitespace(runs);
    }
    if matches!(
        runs.last(),
        Some(Run {
            piece: Piece::Break,
            ..
        })
    ) {
        runs.pop();
        block.trailing_break = true;
    }
    crate::model::normalize(runs);
    !runs.is_empty()
}

/// `white-space: pre-wrap` (a `pre`): white space stays, and each `\n` is a line break.
fn code_lines(runs: Vec<Run>) -> Vec<Run> {
    let mut out = Vec::with_capacity(runs.len());
    for run in runs {
        let Piece::Text(text) = &run.piece else {
            out.push(run);
            continue;
        };
        for (index, line) in text.replace("\r\n", "\n").split('\n').enumerate() {
            if index > 0 {
                out.push(Run {
                    piece: Piece::Break,
                    marks: run.marks.clone(),
                });
            }
            if !line.is_empty() {
                out.push(Run::text(line, run.marks.clone()));
            }
        }
    }
    out
}

/// `white-space: normal`: runs of spaces, tabs and newlines become one space, and none start or
/// end a line. No-break spaces stay.
fn collapse_whitespace(runs: &mut [Run]) {
    let mut at_line_start = true;
    for run in runs.iter_mut() {
        match &mut run.piece {
            Piece::Text(text) => {
                let mut out = String::with_capacity(text.len());
                for ch in text.chars() {
                    if matches!(ch, ' ' | '\t' | '\n' | '\r' | '\x0C') {
                        if !at_line_start && !out.ends_with(' ') {
                            out.push(' ');
                        }
                    } else {
                        out.push(ch);
                        at_line_start = false;
                    }
                }
                *text = out;
            }
            Piece::Break => at_line_start = true,
            Piece::Image(_) => at_line_start = false,
        }
    }
    // A space before a break or the end doesn't show.
    let mut at_line_end = true;
    for run in runs.iter_mut().rev() {
        match &mut run.piece {
            Piece::Text(text) => {
                if at_line_end {
                    let trimmed = text.trim_end_matches(' ').len();
                    text.truncate(trimmed);
                }
                if !text.is_empty() {
                    at_line_end = false;
                }
            }
            Piece::Break => at_line_end = true,
            Piece::Image(_) => at_line_end = false,
        }
    }
    // Spaces that end one text run and start the next collapse to one.
    for index in 1..runs.len() {
        let previous_ends_space =
            matches!(&runs[index - 1].piece, Piece::Text(text) if text.ends_with(' '));
        if previous_ends_space && let Piece::Text(text) = &mut runs[index].piece {
            let trimmed = text.trim_start_matches(' ').to_owned();
            *text = trimmed;
        }
    }
}

fn text_of(node: &Handle) -> String {
    let mut out = String::new();
    fn walk(node: &Handle, out: &mut String) {
        match &node.data {
            NodeData::Text { contents } => out.push_str(&contents.borrow()),
            NodeData::Element { name, .. } if &*name.local == "tr" && !out.is_empty() => {
                out.push('\n');
                for child in node.children.borrow().iter() {
                    walk(child, out);
                }
            }
            _ => {
                for child in node.children.borrow().iter() {
                    walk(child, out);
                }
            }
        }
    }
    walk(node, &mut out);
    out.split('\n')
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn outer_html(node: &Handle) -> String {
    let mut out = Vec::new();
    let options = SerializeOpts {
        traversal_scope: TraversalScope::IncludeNode,
        ..SerializeOpts::default()
    };
    serialize(&mut out, &SerializableHandle::from(node.clone()), options)
        .expect("writing to a Vec doesn't fail");
    String::from_utf8(out).expect("the serializer writes UTF-8")
}

/// `html` parsed as a document and written back, which is how an outgoing message wraps each part
/// (`<html><head></head><body>…</body></html>`).
pub fn as_document(html: &str) -> String {
    let dom = parse_document(RcDom::default(), Default::default()).one(html);
    let document = dom.document;
    let root = document
        .children
        .borrow()
        .iter()
        .find(|child| matches!(child.data, NodeData::Element { .. }))
        .cloned();
    root.map_or_else(String::new, |root| outer_html(&root))
}

/// [`as_document`] with its inline images pointed at their Content-IDs, as the quote appended to a
/// sent message is: a `src-cid` image's `src` becomes `cid:{id}`, and it loses Exo's class.
pub(crate) fn as_sent_document(html: &str) -> String {
    let dom = parse_document(RcDom::default(), Default::default()).one(html);
    let mut pending = vec![dom.document.clone()];
    while let Some(node) = pending.pop() {
        if let NodeData::Element { attrs, .. } = &node.data {
            let mut attrs = attrs.borrow_mut();
            for (attr, target) in [
                ("src-cid", "src"),
                ("poster-cid", "poster"),
                ("background-cid", "background"),
            ] {
                let Some(cid) = attrs
                    .iter()
                    .find(|a| &*a.name.local == attr && !a.value.is_empty())
                    .map(|a| a.value.to_string())
                else {
                    continue;
                };
                match attrs.iter_mut().find(|a| &*a.name.local == target) {
                    Some(existing) => existing.value = format!("cid:{cid}").into(),
                    None => attrs.push(html5ever::Attribute {
                        name: html5ever::QualName::new(None, html5ever::ns!(), target.into()),
                        value: format!("cid:{cid}").into(),
                    }),
                }
                if let Some(class) = attrs.iter_mut().find(|a| &*a.name.local == "class") {
                    let kept: Vec<&str> = class
                        .value
                        .split_whitespace()
                        .filter(|name| *name != crate::image::CLASS)
                        .collect();
                    class.value = kept.join(" ").into();
                }
            }
        }
        pending.extend(node.children.borrow().iter().cloned());
    }
    let root = dom
        .document
        .children
        .borrow()
        .iter()
        .find(|child| matches!(child.data, NodeData::Element { .. }))
        .cloned();
    root.map_or_else(String::new, |root| outer_html(&root))
}

/// `html` (a kept table's markup) with each `<img>` handed to `image` to rewrite: what a kept
/// table's drawing loads its images from (`view::table`).
pub(crate) fn map_images(html: &str, mut image: impl FnMut(&mut Element)) -> String {
    let (_document, body) = fragment(html);
    let mut pending = vec![body.clone()];
    while let Some(node) = pending.pop() {
        if let NodeData::Element { name, attrs, .. } = &node.data
            && &*name.local == "img"
        {
            let mut attrs = attrs.borrow_mut();
            let mut element = Element {
                tag: "img".to_owned(),
                attrs: attrs
                    .iter()
                    .map(|attr| (attr.name.local.to_string(), attr.value.to_string()))
                    .collect(),
            };
            image(&mut element);
            *attrs = element
                .attrs
                .into_iter()
                .map(|(name, value)| html5ever::Attribute {
                    name: html5ever::QualName::new(None, html5ever::ns!(), name.into()),
                    value: value.into(),
                })
                .collect();
        }
        pending.extend(node.children.borrow().iter().rev().cloned());
    }
    let mut out = Vec::new();
    let options = SerializeOpts {
        traversal_scope: TraversalScope::ChildrenOnly(None),
        ..SerializeOpts::default()
    };
    serialize(&mut out, &SerializableHandle::from(body), options)
        .expect("writing to a Vec doesn't fail");
    String::from_utf8(out).expect("the serializer writes UTF-8")
}

/// `html` (a kept table) inside the containers of `path`, outermost first, as the editor's DOM
/// has it, for what it inherits from them (a quote's grey): their boxes (margins, padding, a
/// quote's bar) are the editor's own layout's, so they're emptied here.
pub(crate) fn in_containers(path: &[Container], html: &str) -> String {
    const EMPTY: &str =
        "margin:0 !important;padding:0 !important;border:0 !important;background:none !important";
    let mut out = String::new();
    for container in path {
        let mut element = container.element.clone();
        let style = element
            .attr("style")
            .map_or_else(|| EMPTY.to_owned(), |style| format!("{style};{EMPTY}"));
        element.set_attr("style", &style);
        open_tag(&element, &mut out);
    }
    out.push_str(html);
    for container in path.iter().rev() {
        close_tag(&container.element, &mut out);
    }
    out
}

// Writing

/// Writes `doc` as Squire would save it: the draft body.
pub fn write(doc: &Document) -> String {
    let mut out = String::new();
    write_blocks(&doc.blocks, &mut out);
    out
}

/// Writes `blocks` in their containers: a container opens before its first block and closes
/// after its last. Iterative, so editing can nest containers as deep as it likes.
pub(crate) fn write_blocks(blocks: &[Block], out: &mut String) {
    let mut open: Vec<&Container> = Vec::new();
    for block in blocks {
        let shared = open
            .iter()
            .zip(&block.path)
            .take_while(|(open, container)| open.id == container.id)
            .count();
        for container in open.drain(shared..).rev() {
            close_tag(&container.element, out);
        }
        for container in &block.path[shared..] {
            open_tag(&container.element, out);
            open.push(container);
        }
        write_leaf(block, out);
    }
    for container in open.into_iter().rev() {
        close_tag(&container.element, out);
    }
}

fn write_leaf(block: &Block, out: &mut String) {
    let wrapped = !block.element.tag.is_empty();
    if wrapped {
        open_tag(&block.element, out);
    }
    match &block.content {
        Content::Rule => out.push_str("<hr>"),
        Content::Raw { html, .. } => out.push_str(html),
        Content::Inline(runs) if runs.is_empty() => {
            if block.trailing_break {
                out.push_str("<br>");
            }
        }
        Content::Inline(runs) => {
            write_runs(runs, block.in_code(), out);
            if block.trailing_break {
                out.push_str("<br>");
            }
        }
    }
    if wrapped {
        close_tag(&block.element, out);
    }
}

/// Writes inline content, opening and closing elements only where the run stacks differ. In
/// `code` (a `pre`), spaces stay as typed and a line break is a `\n`, as Chromium keeps them.
pub(crate) fn write_runs(runs: &[Run], code: bool, out: &mut String) {
    let spaces = if code {
        vec![false; runs.iter().map(|run| run.piece.len()).sum()]
    } else {
        space_kinds(runs)
    };
    let mut open: Vec<&Element> = Vec::new();
    let mut position = 0;
    for run in runs {
        let shared = open
            .iter()
            .zip(&run.marks)
            .take_while(|(a, b)| **a == *b)
            .count();
        while open.len() > shared {
            close_tag(open.pop().expect("longer than shared"), out);
        }
        for mark in &run.marks[shared..] {
            open_tag(mark, out);
            open.push(mark);
        }
        match &run.piece {
            Piece::Text(text) => {
                for ch in text.chars() {
                    match ch {
                        ' ' if spaces[position] => out.push_str("&nbsp;"),
                        '\u{A0}' => out.push_str("&nbsp;"),
                        '&' => out.push_str("&amp;"),
                        '<' => out.push_str("&lt;"),
                        '>' => out.push_str("&gt;"),
                        ch => out.push(ch),
                    }
                    position += 1;
                }
            }
            Piece::Break => {
                out.push_str(if code { "\n" } else { "<br>" });
                position += 1;
            }
            Piece::Image(image) => {
                open_tag(image, out);
                position += 1;
            }
        }
    }
    while let Some(mark) = open.pop() {
        close_tag(mark, out);
    }
}

/// For each position of `runs`, whether a space there is written as `&nbsp;`: what Chromium's
/// editing leaves for typed spaces. A lone space between words stays a space; longer runs
/// alternate starting with a no-break space; a run at the start or end of a line starts or ends
/// with one.
pub(crate) fn space_kinds(runs: &[Run]) -> Vec<bool> {
    let chars: Vec<Option<char>> = runs
        .iter()
        .flat_map(|run| match &run.piece {
            Piece::Text(text) => text.chars().map(Some).collect::<Vec<_>>(),
            Piece::Break => vec![Some('\n')],
            Piece::Image(_) => vec![None],
        })
        .collect();
    let mut out = vec![false; chars.len()];
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != Some(' ') {
            index += 1;
            continue;
        }
        let start = index;
        while index < chars.len() && chars[index] == Some(' ') {
            index += 1;
        }
        let at_start = start == 0 || chars[start - 1] == Some('\n');
        let at_end = index == chars.len() || chars[index] == Some('\n');
        let len = index - start;
        if len == 1 && !at_start && !at_end {
            continue;
        }
        for (offset, slot) in out[start..index].iter_mut().enumerate() {
            *slot = offset % 2 == 0;
        }
        if at_end {
            out[index - 1] = true;
        }
    }
    out
}

fn open_tag(element: &Element, out: &mut String) {
    out.push('<');
    out.push_str(&element.tag);
    for (name, value) in &element.attrs {
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        for ch in value.chars() {
            match ch {
                '&' => out.push_str("&amp;"),
                '"' => out.push_str("&quot;"),
                '\u{A0}' => out.push_str("&nbsp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                ch => out.push(ch),
            }
        }
        out.push('"');
    }
    out.push('>');
}

fn close_tag(element: &Element, out: &mut String) {
    if matches!(
        element.tag.as_str(),
        "br" | "img" | "hr" | "wbr" | "input" | "col" | "area" | "source"
    ) {
        return;
    }
    out.push_str("</");
    out.push_str(&element.tag);
    out.push('>');
}
