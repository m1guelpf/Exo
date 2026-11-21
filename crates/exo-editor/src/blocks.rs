//! Block structure edits with Squire's semantics: splitting and merging blocks, and the block
//! operations (lists, quotes, indents).
//!
//! Squire runs a block operation by extracting the selected blocks *up to the root*, splitting
//! every container around them, transforming that fragment, putting it back and merging it with
//! alike neighbouring containers. [`Document::modify_blocks`] does the same on paths:
//! [`isolate`](Document::isolate) gives the selected blocks' containers fresh ids, the operation
//! edits their paths, and [`merge_edge`](Document::merge_edge) re-joins alike containers at both
//! ends. That is why two quoted lines become one quote, and why ⌘] in a quote wraps the *outside*
//! of it.

use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

use crate::model::{
    Block, Container, ContainerId, ContainerKind, Content, Document, Element, Piece, Pos,
};

/// The inline style a quote's `blockquote` gets.
pub(crate) const QUOTE_STYLE: &str = "background-image: -webkit-linear-gradient(left, rgb(237, 237, 237) 75%, rgb(237, 237, 237) 75%);background-size: 1px 1px;background-position: left;background-repeat: repeat-y;padding: 0 0 0 20px;";
/// The inline style of a plain indent.
pub(crate) const INDENT_STYLE: &str = "margin:0 0 0 40px;border:none;padding:0px";
/// List marker styles per nesting level.
const ORDERED_STYLES: [&str; 3] = ["decimal", "lower-alpha", "lower-roman"];
const BULLET_STYLES: [&str; 3] = ["disc", "circle", "square"];

impl Document {
    /// Splits the block at `at` (Enter). The new block is a copy of the block element (a `div` or
    /// `li` keeps its attributes; any other tag becomes a plain `div`) in the same containers.
    /// Returns the start of the new block.
    pub(crate) fn split(&mut self, at: Pos) -> Pos {
        let block = &mut self.blocks[at.block];
        let rest = block.split_off(at.offset);
        let element = if block.element.is("div") || block.element.is("li") {
            block.element.clone()
        } else {
            Element::new("div")
        };
        let trailing = block.trailing_break;
        // The first half gets a `<br>` unless it already has one.
        block.trailing_break = !has_break(block);
        let new = Block {
            path: block.path.clone(),
            element,
            trailing_break: trailing || rest.is_empty(),
            content: Content::Inline(rest),
        };
        self.blocks.insert(at.block + 1, new);
        Pos::new(at.block + 1, 0)
    }

    /// Appends block `index` to the block before it and returns where they meet. The earlier block
    /// keeps its element; its `<br>` fixer goes. An atom (a rule, a table) can't hold text, so it
    /// goes and the text takes its place.
    pub(crate) fn merge_with_previous(&mut self, index: usize) -> Pos {
        let next = self.blocks.remove(index);
        let previous = &mut self.blocks[index - 1];
        previous.runs_mut();
        let offset = previous.len();
        let runs = match next.content {
            Content::Inline(runs) => runs,
            _ => Vec::new(),
        };
        previous.insert(offset, runs);
        previous.trailing_break = next.trailing_break;
        if index < self.blocks.len() {
            self.merge_edge(index);
        }
        Pos::new(index - 1, offset)
    }

    /// Deletes everything between `start` and `end`: the rest of the end block joins the start
    /// block. Returns where the range starts.
    pub(crate) fn delete(&mut self, start: Pos, end: Pos) -> Pos {
        let (start, end) = self.trim_atoms(start, end);
        if start.block == end.block {
            self.blocks[start.block].remove(start.offset..end.offset);
            return start;
        }
        let tail = {
            let last = &mut self.blocks[end.block];
            let len = last.len();
            last.remove(end.offset..len)
        };
        let trailing = self.blocks[end.block].trailing_break;
        self.blocks.drain(start.block + 1..=end.block);
        let first = &mut self.blocks[start.block];
        let len = first.len();
        first.remove(start.offset..len);
        first.insert(start.offset, tail);
        first.trailing_break = trailing;
        if start.block + 1 < self.blocks.len() {
            self.merge_edge(start.block + 1);
        }
        start
    }

    /// `start..end` without the atoms (rules, tables) it only touches: a range that starts
    /// after one or ends before one doesn't take it.
    fn trim_atoms(&self, start: Pos, end: Pos) -> (Pos, Pos) {
        let atom = |pos: Pos| !self.blocks[pos.block].is_inline();
        let start = if atom(start) && start.offset > 0 && start.block < end.block {
            Pos::new(start.block + 1, 0)
        } else {
            start
        };
        let end = if atom(end) && end.offset == 0 && end.block > start.block {
            Pos::new(end.block - 1, self.blocks[end.block - 1].len())
        } else {
            end
        };
        (start, end)
    }

    /// The blocks between `start` and `end`, cut to them: what ⌘C copies.
    pub(crate) fn fragment(&self, start: Pos, end: Pos) -> Vec<Block> {
        let (start, end) = self.trim_atoms(start, end);
        let mut blocks = self.blocks[start.block..=end.block].to_vec();
        let last = blocks.len() - 1;
        let len = blocks[last].len();
        blocks[last].remove(end.offset..len);
        blocks[0].remove(0..start.offset);
        blocks
    }

    /// Inserts `text` in `marks` at `at` and returns the position after it. An empty line's
    /// placeholder `<br>` goes once it has text, as the browser drops it.
    pub(crate) fn insert_text(&mut self, at: Pos, text: &str, marks: Vec<Element>) -> Pos {
        let block = &mut self.blocks[at.block];
        if block.is_empty() {
            block.trailing_break = false;
        }
        block.insert(at.offset, vec![crate::model::Run::text(text, marks)]);
        Pos::new(at.block, at.offset + text.chars().count())
    }

    /// Runs `operation` on blocks `range` the way Squire runs a block operation (see the module
    /// docs).
    pub(crate) fn modify_blocks(
        &mut self,
        range: Range<usize>,
        operation: impl FnOnce(&mut Self, Range<usize>),
    ) {
        self.isolate(range.clone());
        operation(self, range.clone());
        self.split_broken_containers();
        self.merge_edge(range.end);
        self.merge_edge(range.start);
    }

    /// Splits every container that `range` shares with blocks outside it, as extracting the
    /// range up to the root does: the blocks before keep their containers, the range and the
    /// blocks after get copies.
    fn isolate(&mut self, range: Range<usize>) {
        self.split_containers_at(range.end);
        self.split_containers_at(range.start);
    }

    /// Gives the blocks from `index` on their own copies of the containers they share with the
    /// blocks before it.
    pub(crate) fn split_containers_at(&mut self, index: usize) {
        if index == 0 || index >= self.blocks.len() {
            return;
        }
        let before: HashSet<ContainerId> = self.blocks[..index]
            .iter()
            .flat_map(|block| block.path.iter().map(|c| c.id))
            .collect();
        let shared: Vec<ContainerId> = self.blocks[index..]
            .iter()
            .flat_map(|block| block.path.iter().map(|c| c.id))
            .filter(|id| before.contains(id))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let fresh: HashMap<ContainerId, ContainerId> =
            shared.into_iter().map(|id| (id, self.fresh_id())).collect();
        for block in &mut self.blocks[index..] {
            for container in &mut block.path {
                if let Some(id) = fresh.get(&container.id) {
                    container.id = *id;
                }
            }
        }
    }

    /// Gives each element a container id of its own again after an operation moved blocks out
    /// of the middle of one: blocks share a container only while it runs unbroken from one
    /// block to the next, as an element does.
    fn split_broken_containers(&mut self) {
        let mut seen = HashSet::new();
        // The block before's containers, `(id before, id now)`.
        let mut previous: Vec<(ContainerId, ContainerId)> = Vec::new();
        for index in 0..self.blocks.len() {
            let mut ids = Vec::with_capacity(self.blocks[index].path.len());
            let mut continues = true;
            for depth in 0..self.blocks[index].path.len() {
                let old = self.blocks[index].path[depth].id;
                continues &= previous.get(depth).is_some_and(|(id, _)| *id == old);
                let new = if continues {
                    previous[depth].1
                } else if seen.insert(old) {
                    old
                } else {
                    self.fresh_id()
                };
                self.blocks[index].path[depth].id = new;
                ids.push((old, new));
            }
            previous = ids;
        }
    }

    /// Joins alike containers that meet between blocks `index - 1` and `index`, outermost
    /// first.
    pub(crate) fn merge_edge(&mut self, index: usize) {
        if index == 0 || index >= self.blocks.len() {
            return;
        }
        let mut depth = 0;
        loop {
            let (Some(previous), Some(next)) = (
                self.blocks[index - 1].path.get(depth),
                self.blocks[index].path.get(depth),
            ) else {
                return;
            };
            if previous.id != next.id {
                if previous.element != next.element {
                    return;
                }
                let (from, to) = (next.id, previous.id);
                for block in &mut self.blocks[index..] {
                    match block.path.get_mut(depth) {
                        Some(container) if container.id == from => container.id = to,
                        _ => break,
                    }
                }
            }
            depth += 1;
        }
    }

    /// Gives lines the `<br>`s a browser needs to show them as the editor does: a block whose
    /// last line is empty gets its placeholder (as the browser adds one), or the line has no
    /// height; and loose text (right inside a container) followed by more loose text gets one,
    /// or the two run together into one line.
    pub(crate) fn fix_lines(&mut self) {
        let loose = |block: &Block| block.element.tag.is_empty() && block.is_inline();
        // Neighbours in the same innermost container at the same depth share their whole path.
        let innermost = |block: &Block| {
            (
                block.path.len(),
                block.path.last().map(|container| container.id),
            )
        };
        for index in 0..self.blocks.len() {
            if self.blocks.get(index + 1).is_some_and(|next| {
                loose(next)
                    && loose(&self.blocks[index])
                    && innermost(next) == innermost(&self.blocks[index])
            }) {
                self.blocks[index].trailing_break = true;
            }
            self.blocks[index].fix_placeholder();
        }
    }

    /// Keeps an editable line at the end: if the last block is inside a container, or isn't a plain
    /// line, an empty `div` follows it.
    pub(crate) fn ensure_bottom_line(&mut self) {
        let last = self.blocks.last().expect("a document has a block");
        if !last.path.is_empty() || last.element.tag.is_empty() || !last.is_inline() {
            self.blocks.push(Block::empty(Vec::new()));
        }
    }

    /// The deepest container blocks `range` all share: the selection's common ancestor.
    pub(crate) fn common_path(&self, range: Range<usize>) -> Vec<Container> {
        let first = &self.blocks[range.start].path;
        let mut shared = first.len();
        for block in &self.blocks[range] {
            shared = shared.min(
                first
                    .iter()
                    .zip(&block.path)
                    .take_while(|(a, b)| a.id == b.id)
                    .count(),
            );
        }
        first[..shared].to_vec()
    }

    /// Turns the blocks into items of a `ul` (or `ol`), renaming lists of the other
    /// kind. `value` is the first item's `value` attribute (from a `1.` trigger).
    pub(crate) fn make_list(&mut self, range: Range<usize>, ordered: bool, value: Option<&str>) {
        let tag = if ordered { "ol" } else { "ul" };
        for index in range {
            if self.blocks[index].is_list_item() {
                let Some((depth, list)) = self.blocks[index]
                    .list()
                    .map(|(depth, list)| (depth, list.id))
                else {
                    continue;
                };
                if !self.blocks[index].path[depth].element.is(tag) {
                    let renamed = Element::new(tag);
                    for block in &mut self.blocks {
                        if let Some(container) =
                            block.path.iter_mut().find(|container| container.id == list)
                        {
                            container.element = renamed.clone();
                        }
                    }
                }
                continue;
            }
            // An earlier sibling list of this kind takes the item; otherwise a new list starts.
            let depth = self.blocks[index].path.len();
            let sibling = index
                .checked_sub(1)
                .map(|previous| &self.blocks[previous])
                .filter(|previous| {
                    previous.path.len() > depth
                        && previous.path[..depth]
                            .iter()
                            .zip(&self.blocks[index].path)
                            .all(|(a, b)| a.id == b.id)
                        && previous.path[depth].element.is(tag)
                })
                .map(|previous| previous.path[depth].clone());
            let list = sibling.unwrap_or_else(|| self.container(Element::new(tag)));
            let block = &mut self.blocks[index];
            block.path.push(list);
            block.element = match value {
                Some(value) => Element::with("li", &[("value", value)]),
                None => Element::new("li"),
            };
        }
    }

    /// Every list around the blocks goes, and their items become `div`s.
    pub(crate) fn remove_list(&mut self, range: Range<usize>) {
        for block in &mut self.blocks[range] {
            block.path.retain(|container| {
                !matches!(
                    container.element.container_kind(),
                    ContainerKind::List { .. }
                )
            });
            if block.is_list_item() {
                block.element = Element::new("div");
            }
        }
    }

    /// Nests each item in a list of its list's kind: the one the item before it was just nested in,
    /// or a new one.
    pub(crate) fn increase_list_level(&mut self, range: Range<usize>) {
        for index in range {
            let Some((depth, list)) = self.blocks[index]
                .list()
                .map(|(depth, list)| (depth, list.clone()))
            else {
                continue;
            };
            if !self.blocks[index].is_list_item() {
                continue;
            }
            let previous = index
                .checked_sub(1)
                .map(|previous| &self.blocks[previous].path);
            let inner = match previous.and_then(|path| {
                path.get(depth + 1)
                    .filter(|_| path.get(depth).is_some_and(|c| c.id == list.id))
            }) {
                Some(inner) if inner.element.is(&list.element.tag) => inner.clone(),
                _ => self.container(Element::new(&list.element.tag)),
            };
            let block = &mut self.blocks[index];
            block.path.insert(depth + 1, inner);
            restyle_list_item(block);
        }
    }

    /// Moves each item out of its list, into the parent list or, at the top, out of lists
    /// altogether as a `div`.
    pub(crate) fn decrease_list_level(&mut self, range: Range<usize>) {
        for block in &mut self.blocks[range] {
            let Some((depth, _)) = block.list().filter(|_| block.is_list_item()) else {
                continue;
            };
            block.path.remove(depth);
            if block.list().is_some_and(|(parent, _)| parent + 1 == depth) {
                restyle_list_item(block);
            } else {
                block.element = Element::new("div");
            }
        }
    }

    /// Wraps the blocks in a new `blockquote` with `style`, outermost.
    pub(crate) fn wrap_blockquote(&mut self, range: Range<usize>, style: &str) {
        let quote = self.container(Element::with("blockquote", &[("style", style)]));
        for block in &mut self.blocks[range] {
            block.path.insert(0, quote.clone());
        }
    }

    /// Removes each block's outermost `blockquote`.
    pub(crate) fn unwrap_outer_blockquote(&mut self, range: Range<usize>) {
        for block in &mut self.blocks[range] {
            if let Some(depth) = block
                .path
                .iter()
                .position(|container| container.element.is("blockquote"))
            {
                block.path.remove(depth);
            }
        }
    }

    /// Removes every `blockquote` around the blocks.
    pub(crate) fn unwrap_all_blockquotes(&mut self, range: Range<usize>) {
        for block in &mut self.blocks[range] {
            block
                .path
                .retain(|container| !container.element.is("blockquote"));
        }
    }

    /// Enter inside any `blockquote`: the quote is split at `at` and a `div` holding three empty
    /// lines goes between the halves, at the top level; the caret lands on the middle line. Returns
    /// the caret.
    pub(crate) fn break_out_of_quote(&mut self, at: Pos) -> Pos {
        let index = at.block;
        let rest = self.blocks[index].split_off(at.offset);
        let mut after = self.blocks[index].clone();
        after.content = Content::Inline(rest);
        let before = &mut self.blocks[index];
        before.trailing_break = !before.is_empty() && !has_break(before);
        let wrapper = self.container(Element::new("div"));
        let mut replacement: Vec<Block> = Vec::new();
        if !self.blocks[index].is_empty() {
            replacement.push(self.blocks[index].clone());
        }
        let caret = index + replacement.len() + 1;
        replacement.extend((0..3).map(|_| Block::empty(vec![wrapper.clone()])));
        let rejoin = index + replacement.len();
        if !after.is_empty() {
            replacement.push(after);
        }
        self.blocks.splice(index..=index, replacement);
        // What follows the caret is extracted from the quote too.
        self.split_containers_at(rejoin);
        self.ensure_bottom_line();
        Pos::new(caret, 0)
    }

    /// A code block, with a 20px `margin-left`: each block becomes a plain `div` (keeping its
    /// `dir`) in a new innermost `pre`, which joins a `pre` just before it
    /// ([`Document::modify_blocks`] merges the two).
    pub(crate) fn make_code(&mut self, range: Range<usize>) {
        let pre = self.container(Element::with("pre", &[("style", "margin-left: 20px;")]));
        for block in &mut self.blocks[range] {
            let dir = block.element.attr("dir").map(str::to_owned);
            block.element = Element::new("div");
            if let Some(dir) = dir {
                block.element.set_attr("dir", &dir);
            }
            block.path.push(pre.clone());
        }
    }

    /// Squire's Enter on an empty line of a code block: the line goes, and a plain line at the
    /// top level takes its place, splitting the code (and whatever holds it) around it when
    /// there's more below. Returns the new line's start.
    pub(crate) fn leave_code(&mut self, index: usize, line: Range<usize>) -> Pos {
        let block = &mut self.blocks[index];
        block.remove(line.clone());
        let rest = block.split_off(line.start);
        let mut after = block.clone();
        after.content = Content::Inline(rest);
        block.trailing_break = true;
        let mut replacement = Vec::with_capacity(3);
        if !block.is_empty() {
            replacement.push(block.clone());
        }
        let caret = index + replacement.len();
        replacement.push(Block::empty(Vec::new()));
        if !after.is_empty() {
            replacement.push(after);
        }
        self.blocks.splice(index..=index, replacement);
        self.split_containers_at(caret + 1);
        Pos::new(caret, 0)
    }

    /// Drops the `value` of every item but the first in the list holding block `index`.
    pub(crate) fn renumber(&mut self, index: usize) {
        let Some((_, list)) = self
            .blocks
            .get(index)
            .and_then(|block| block.list())
            .map(|(depth, list)| (depth, list.id))
        else {
            return;
        };
        let mut first = true;
        for block in &mut self.blocks {
            if block.list().is_some_and(|(_, inner)| inner.id == list) {
                if !first {
                    block.element.attrs.retain(|(name, _)| name != "value");
                }
                first = false;
            }
        }
    }
}

fn has_break(block: &Block) -> bool {
    block.runs().iter().any(|run| run.piece == Piece::Break)
}

/// Sets an item's `aria-level` and marker style for its depth.
fn restyle_list_item(block: &mut Block) {
    let level = block.list_depth();
    let ordered = matches!(
        block.list().map(|(_, list)| list.element.container_kind()),
        Some(ContainerKind::List { ordered: true })
    );
    let styles = if ordered {
        ORDERED_STYLES
    } else {
        BULLET_STYLES
    };
    let style = styles[(level.max(1) - 1) % styles.len()];
    block.element.set_attr("aria-level", &level.to_string());
    block
        .element
        .set_attr("style", &format!("list-style-type:{style}"));
}
