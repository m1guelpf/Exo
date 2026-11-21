//! The document: Squire's DOM, flattened into a list of leaf blocks.
//!
//! Squire edits plain DOM: leaf blocks (`div`, `li`, …) holding inline content, nested in
//! containers (`ul`/`ol`, `blockquote`, wrapper `div`s). Here each leaf block carries its chain
//! of containers (its *path*, outermost first). Consecutive blocks whose paths share a container
//! id are in the same element, so `<ul><li>a</li><ul><li>b</li></ul></ul>` is two blocks with
//! paths `[ul#1]` and `[ul#1, ul#2]`. Inline formatting is a stack of elements per run (outermost
//! first), which keeps imported nesting (`<span dir="ltr">&lt;<a>…</a>&gt;</span>`) exact.

use std::ops::Range;

/// An HTML element: its tag and its attributes in source order.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Element {
    /// The lowercase tag name.
    pub tag: String,
    /// `(name, value)` pairs, in order.
    pub attrs: Vec<(String, String)>,
}

impl Element {
    /// An element with no attributes.
    pub fn new(tag: &str) -> Self {
        Self {
            tag: tag.to_owned(),
            attrs: Vec::new(),
        }
    }

    /// An element with `attrs`.
    pub fn with(tag: &str, attrs: &[(&str, &str)]) -> Self {
        Self {
            tag: tag.to_owned(),
            attrs: attrs
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    /// The value of attribute `name`.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Sets attribute `name`, keeping its place if it's already there.
    pub fn set_attr(&mut self, name: &str, value: &str) {
        match self.attrs.iter_mut().find(|(key, _)| key == name) {
            Some((_, existing)) => value.clone_into(existing),
            None => self.attrs.push((name.to_owned(), value.to_owned())),
        }
    }

    /// Whether this is a `tag` element.
    pub fn is(&self, tag: &str) -> bool {
        self.tag == tag
    }

    /// What kind of container this element is, when it wraps blocks.
    pub fn container_kind(&self) -> ContainerKind {
        match self.tag.as_str() {
            "ul" => ContainerKind::List { ordered: false },
            "ol" => ContainerKind::List { ordered: true },
            // `blockquote[style*="border:none"]` is an indent: the body CSS gives it no bar.
            "blockquote"
                if self
                    .attr("style")
                    .is_some_and(|style| style.contains("border:none")) =>
            {
                ContainerKind::Indent
            }
            "blockquote" => ContainerKind::Quote,
            "pre" => ContainerKind::Code,
            _ => ContainerKind::Other,
        }
    }
}

/// How a container element lays out its blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerKind {
    /// `ul` or `ol`.
    List {
        /// `ol`.
        ordered: bool,
    },
    /// A quote: a `blockquote` with the gray bar.
    Quote,
    /// A 40px indent: `blockquote[style*="border:none"]`.
    Indent,
    /// A `pre` code block.
    Code,
    /// Any other wrapper (`div`, `div.gmail_quote`, …), which adds no styling.
    Other,
}

/// Identifies one container element, so adjacent blocks can tell "same quote" from "two quotes".
pub type ContainerId = u32;

/// One container element on a block's path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Container {
    /// Which element this is; equal ids mean the same element.
    pub id: ContainerId,
    /// The element.
    pub element: Element,
}

/// The atom a run holds.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Piece {
    /// Text, never empty.
    Text(String),
    /// A soft line break (`<br>`).
    Break,
    /// An inline image (`<img>`, with its attributes).
    Image(Element),
}

impl Piece {
    /// Its length in positions: characters, or 1 for an atom.
    pub fn len(&self) -> usize {
        match self {
            Self::Text(text) => text.chars().count(),
            Self::Break | Self::Image(_) => 1,
        }
    }

    /// Whether it holds nothing.
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Text(text) if text.is_empty())
    }
}

/// A piece of inline content and the inline elements around it, outermost first.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Run {
    /// The content.
    pub piece: Piece,
    /// The inline elements it sits in (`b`, `a`, `span.colour`, …), outermost first.
    pub marks: Vec<Element>,
}

impl Run {
    /// Text in `marks`.
    pub fn text(text: impl Into<String>, marks: Vec<Element>) -> Self {
        Self {
            piece: Piece::Text(text.into()),
            marks,
        }
    }
}

/// What a leaf block holds.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Content {
    /// Inline content; empty for an empty line (`<div><br></div>`).
    Inline(Vec<Run>),
    /// A horizontal rule (`<div><hr></div>`).
    Rule,
    /// Markup the editor doesn't model (a pasted or signature table), kept verbatim and edited
    /// as one atom. `text` is what it shows.
    Raw {
        /// The element's outer HTML.
        html: String,
        /// Its text content, one line per row.
        text: String,
    },
}

/// A leaf block: one `div`, `li`, `p`, `h1`… with inline content, and the containers around it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// The containers around it, outermost first.
    pub path: Vec<Container>,
    /// The block element itself. Its tag is empty for a bare `<hr>` or [`Content::Raw`], which
    /// have no wrapper.
    pub element: Element,
    /// What it holds.
    pub content: Content,
    /// Whether the markup ends in a `<br>`: the placeholder of an empty line, or the one Squire
    /// leaves after text (where it draws nothing). Kept so drafts round-trip byte for byte.
    pub trailing_break: bool,
}

impl Block {
    /// An empty line (`<div><br></div>`) in `path`.
    pub fn empty(path: Vec<Container>) -> Self {
        Self {
            trailing_break: true,
            ..Self::div(path, Vec::new())
        }
    }

    /// A `div` in `path` holding `runs`.
    pub fn div(path: Vec<Container>, runs: Vec<Run>) -> Self {
        Self {
            path,
            element: Element::new("div"),
            content: Content::Inline(runs),
            trailing_break: false,
        }
    }

    /// Its inline runs (none for a rule or raw block).
    pub fn runs(&self) -> &[Run] {
        match &self.content {
            Content::Inline(runs) => runs,
            _ => &[],
        }
    }

    /// Its runs, to edit. A rule or raw block becomes the empty line left where it was deleted.
    pub(crate) fn runs_mut(&mut self) -> &mut Vec<Run> {
        if !matches!(self.content, Content::Inline(_)) {
            self.content = Content::Inline(Vec::new());
            self.trailing_break = true;
            if self.element.tag.is_empty() {
                self.element = Element::new("div");
            }
        }
        match &mut self.content {
            Content::Inline(runs) => runs,
            _ => unreachable!("set to inline above"),
        }
    }

    /// Whether it holds inline content (not a rule or raw markup).
    pub fn is_inline(&self) -> bool {
        matches!(self.content, Content::Inline(_))
    }

    /// Its length in positions. A rule or raw block is one atom.
    pub fn len(&self) -> usize {
        match &self.content {
            Content::Inline(runs) => runs.iter().map(|run| run.piece.len()).sum(),
            Content::Rule | Content::Raw { .. } => 1,
        }
    }

    /// Whether it's an empty line.
    pub fn is_empty(&self) -> bool {
        matches!(&self.content, Content::Inline(runs) if runs.is_empty())
    }

    /// Its text: breaks are `\n`, images and atoms U+FFFC.
    pub fn text(&self) -> String {
        match &self.content {
            Content::Inline(runs) => runs
                .iter()
                .map(|run| match &run.piece {
                    Piece::Text(text) => text.as_str(),
                    Piece::Break => "\n",
                    Piece::Image(_) => "\u{FFFC}",
                })
                .collect(),
            Content::Rule | Content::Raw { .. } => "\u{FFFC}".to_owned(),
        }
    }

    /// Gives it the placeholder `<br>` a browser needs to show its last line: an empty line,
    /// or one after a soft break, has no height without it.
    pub(crate) fn fix_placeholder(&mut self) {
        if let Content::Inline(runs) = &self.content
            && runs.last().is_none_or(|run| run.piece == Piece::Break)
        {
            self.trailing_break = true;
        }
    }

    /// Whether it's a list item.
    pub fn is_list_item(&self) -> bool {
        self.element.is("li")
    }

    /// The innermost list it's in, and that list's depth in its path.
    pub fn list(&self) -> Option<(usize, &Container)> {
        self.path.iter().enumerate().rev().find(|(_, container)| {
            matches!(
                container.element.container_kind(),
                ContainerKind::List { .. }
            )
        })
    }

    /// How many lists it's nested in.
    pub fn list_depth(&self) -> usize {
        self.path
            .iter()
            .filter(|container| {
                matches!(
                    container.element.container_kind(),
                    ContainerKind::List { .. }
                )
            })
            .count()
    }

    /// Whether it's inside a `blockquote` of any kind (quote, indent, `gmail_quote`).
    pub fn in_blockquote(&self) -> bool {
        self.path
            .iter()
            .any(|container| container.element.is("blockquote"))
    }

    /// Whether it's code: a `pre`, or inside one, where white space shows as typed and a line
    /// break is a `\n`.
    pub fn in_code(&self) -> bool {
        self.element.is("pre")
            || self
                .path
                .iter()
                .any(|container| container.element.is("pre"))
    }

    /// Whether it's inside a quote (a `blockquote` that isn't an indent).
    pub fn in_quote(&self) -> bool {
        self.path
            .iter()
            .any(|container| container.element.container_kind() == ContainerKind::Quote)
    }

    /// The marks text typed at `offset` takes: those of the character before it, or at the
    /// start of the block those of the character after it (where the browser puts the caret).
    pub fn marks_at(&self, offset: usize) -> Vec<Element> {
        let runs = self.runs();
        let mut start = 0;
        for (index, run) in runs.iter().enumerate() {
            let end = start + run.piece.len();
            if offset > start && offset <= end || offset == 0 && index == 0 {
                return run.marks.clone();
            }
            start = end;
        }
        Vec::new()
    }

    /// The runs covering `range`, cut to it.
    pub fn slice(&self, range: Range<usize>) -> Vec<Run> {
        let mut out = Vec::new();
        let mut start = 0;
        for run in self.runs() {
            let len = run.piece.len();
            let (from, to) = (range.start.max(start), range.end.min(start + len));
            if from < to {
                out.push(cut(run, from - start..to - start));
            }
            start += len;
        }
        out
    }

    /// Removes `range` and returns what was there.
    pub(crate) fn remove(&mut self, range: Range<usize>) -> Vec<Run> {
        if range.is_empty() {
            return Vec::new();
        }
        let removed = self.slice(range.clone());
        let len = self.len();
        let mut kept = self.slice(0..range.start);
        kept.extend(self.slice(range.end..len));
        *self.runs_mut() = kept;
        normalize(self.runs_mut());
        removed
    }

    /// Inserts `runs` at `offset`.
    pub(crate) fn insert(&mut self, offset: usize, runs: Vec<Run>) {
        if runs.is_empty() {
            return;
        }
        let len = self.len();
        let mut out = self.slice(0..offset);
        out.extend(runs);
        out.extend(self.slice(offset..len));
        *self.runs_mut() = out;
        normalize(self.runs_mut());
    }

    /// Cuts the block at `offset`, returning the runs after it.
    pub(crate) fn split_off(&mut self, offset: usize) -> Vec<Run> {
        let len = self.len();
        self.remove(offset..len)
    }
}

/// The part of `run` at `range` (in its own positions).
fn cut(run: &Run, range: Range<usize>) -> Run {
    let piece = match &run.piece {
        Piece::Text(text) => {
            Piece::Text(text.chars().skip(range.start).take(range.len()).collect())
        }
        atom => atom.clone(),
    };
    Run {
        piece,
        marks: run.marks.clone(),
    }
}

/// Drops empty text and merges neighbouring text with the same marks.
pub(crate) fn normalize(runs: &mut Vec<Run>) {
    let mut out: Vec<Run> = Vec::with_capacity(runs.len());
    for run in runs.drain(..) {
        if run.piece.is_empty() {
            continue;
        }
        if let (
            Some(Run {
                piece: Piece::Text(previous),
                marks,
            }),
            Piece::Text(text),
        ) = (out.last_mut(), &run.piece)
            && *marks == run.marks
        {
            previous.push_str(text);
            continue;
        }
        out.push(run);
    }
    *runs = out;
}

/// A place between two positions of a block: `offset` positions from its start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    /// The block's index.
    pub block: usize,
    /// Positions from the block's start.
    pub offset: usize,
}

impl Pos {
    /// `offset` into block `block`.
    pub const fn new(block: usize, offset: usize) -> Self {
        Self { block, offset }
    }
}

/// The selection: where it started (`anchor`) and where it ends now (`focus`, the caret).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Selection {
    /// Where it started.
    pub anchor: Pos,
    /// Where it is now.
    pub focus: Pos,
}

impl Selection {
    /// A caret at `pos`.
    pub const fn caret(pos: Pos) -> Self {
        Self {
            anchor: pos,
            focus: pos,
        }
    }

    /// From `anchor` to `focus`.
    pub const fn new(anchor: Pos, focus: Pos) -> Self {
        Self { anchor, focus }
    }

    /// Whether it's a caret.
    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.focus
    }

    /// Its start.
    pub fn start(&self) -> Pos {
        self.anchor.min(self.focus)
    }

    /// Its end.
    pub fn end(&self) -> Pos {
        self.anchor.max(self.focus)
    }
}

/// A draft body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    /// Its leaf blocks, in order. Never empty.
    pub blocks: Vec<Block>,
    /// The id [`Self::fresh_id`] hands out next.
    pub(crate) next_id: ContainerId,
}

impl Default for Document {
    fn default() -> Self {
        Self::new(vec![Block::empty(Vec::new())])
    }
}

impl Document {
    /// A document of `blocks` (an empty line if there are none).
    pub fn new(mut blocks: Vec<Block>) -> Self {
        if blocks.is_empty() {
            blocks.push(Block::empty(Vec::new()));
        }
        let next_id = blocks
            .iter()
            .flat_map(|block| &block.path)
            .map(|container| container.id + 1)
            .max()
            .unwrap_or(1);
        Self { blocks, next_id }
    }

    /// A container id no block uses yet.
    pub(crate) fn fresh_id(&mut self) -> ContainerId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// A new container around `element`.
    pub(crate) fn container(&mut self, element: Element) -> Container {
        Container {
            id: self.fresh_id(),
            element,
        }
    }

    /// The position after everything.
    pub fn end(&self) -> Pos {
        let last = self.blocks.len() - 1;
        Pos::new(last, self.blocks[last].len())
    }

    /// `pos`, moved inside the document.
    pub fn clamp(&self, pos: Pos) -> Pos {
        let block = pos.block.min(self.blocks.len() - 1);
        Pos::new(block, pos.offset.min(self.blocks[block].len()))
    }

    /// The text between `start` and `end`, blocks separated by `\n` (as Chromium gives a
    /// selection's text, for simple content).
    pub fn text(&self, start: Pos, end: Pos) -> String {
        let mut out = String::new();
        for (index, block) in self
            .blocks
            .iter()
            .enumerate()
            .take(end.block + 1)
            .skip(start.block)
        {
            let from = if index == start.block {
                start.offset
            } else {
                0
            };
            let to = if index == end.block {
                end.offset
            } else {
                block.len()
            };
            if index > start.block {
                out.push('\n');
            }
            out.extend(
                block
                    .text()
                    .chars()
                    .skip(from)
                    .take(to.saturating_sub(from)),
            );
        }
        out.replace('\u{FFFC}', "")
    }

    /// Whether the body is visually empty (no text, no images, lists, links or quotes).
    pub fn is_empty(&self) -> bool {
        self.blocks.iter().all(|block| {
            block.path.is_empty()
                && block.is_inline()
                && block.runs().iter().all(|run| match &run.piece {
                    Piece::Text(text) => {
                        text.trim().is_empty() && !run.marks.iter().any(|mark| mark.is("a"))
                    }
                    Piece::Break => true,
                    Piece::Image(_) => false,
                })
        })
    }
}
