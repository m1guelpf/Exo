//! A message's text as quote and signature detection sees it: a whitespace-free "clean" string
//! for matching, a "formatted" string with line breaks where blocks end, and struts tying offsets
//! back to DOM nodes.
//!
//! Offsets are byte offsets into the clean string.

use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use crate::{
    dom::{self, NodeKey, NodeRef, Style},
    pixels,
};

/// Stands for an image, table cell or other replaced element in both strings.
pub(crate) const REPLACED: char = '⌧';

/// Line breaks a block element guarantees around itself.
fn block_breaks(tag: &str) -> u32 {
    match tag {
        "blockquote" | "dl" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "hr" | "ol" | "p"
        | "pre" | "tr" | "ul" => 2,
        "dd" | "div" | "dt" | "li" | "listing" => 1,
        _ => 0,
    }
}

/// Elements whose text doesn't count.
fn is_invisible_tag(tag: &str) -> bool {
    matches!(tag, "style" | "script" | "title")
}

/// Elements that count as content even when empty.
fn is_replaced(tag: &str) -> bool {
    matches!(
        tag,
        "map"
            | "area"
            | "col"
            | "td"
            | "button"
            | "input"
            | "meter"
            | "progress"
            | "select"
            | "output"
            | "option"
            | "img"
            | "audio"
            | "video"
            | "track"
            | "source"
            | "svg"
            | "math"
    )
}

/// JavaScript's `\s` (also what its `trim` strips): Unicode white space but U+0085, and U+FEFF.
pub(crate) fn is_space(c: char) -> bool {
    c == '\u{feff}' || (c.is_whitespace() && c != '\u{85}')
}

/// Removes all whitespace.
pub(crate) fn clean(text: &str) -> String {
    text.chars().filter(|c| !is_space(*c)).collect()
}

/// A span of the clean string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Range {
    pub(crate) start: usize,
    pub(crate) end: usize,
}

#[derive(Clone)]
struct Strut {
    formatted_start: usize,
    clean_start: usize,
    node: NodeRef,
    depth: usize,
}

/// The text model of one DOM subtree.
pub(crate) struct Content {
    root: NodeRef,
    clean: String,
    formatted: String,
    /// In document order.
    struts: Vec<Strut>,
    /// The first strut of each node that has one, kept in step with `struts`.
    strut_of: HashMap<NodeKey, usize>,
}

impl Content {
    /// Reads the text under `root`.
    pub(crate) fn new(root: &NodeRef) -> Self {
        let mut content = Self {
            root: root.clone(),
            clean: String::new(),
            formatted: String::new(),
            struts: Vec::new(),
            strut_of: HashMap::new(),
        };
        let mut state = ParseState {
            newlines: 2,
            ended_with_space: false,
        };
        content.read(root, 0, &mut state);
        content.struts.push(Strut {
            formatted_start: content.formatted.len(),
            clean_start: content.clean.len(),
            node: root.clone(),
            depth: 0,
        });
        for (index, strut) in content.struts.iter().enumerate() {
            content
                .strut_of
                .entry(dom::key(&strut.node))
                .or_insert(index);
        }
        content
    }

    fn read(&mut self, node: &NodeRef, depth: usize, state: &mut ParseState) {
        if let Some(text) = dom::text(node) {
            let collapsed = collapse_spaces(&text);
            if !collapsed.trim_matches(is_space).is_empty() {
                state.newlines = 0;
                let last_is_space = self.formatted.chars().last().is_some_and(is_space);
                if !self.formatted.is_empty()
                    && !last_is_space
                    && (collapsed.starts_with(is_space) || state.ended_with_space)
                {
                    self.formatted.push(' ');
                }
                state.ended_with_space = collapsed.ends_with(is_space);
                self.struts.push(Strut {
                    formatted_start: self.formatted.len(),
                    clean_start: self.clean.len(),
                    node: node.clone(),
                    depth,
                });
                self.formatted.push_str(collapsed.trim_matches(is_space));
                self.clean.push_str(&clean(&collapsed));
            } else if !collapsed.is_empty() {
                state.ended_with_space = true;
                if text.contains(' ') {
                    state.newlines = 0;
                }
            }
            return;
        }
        let Some(tag) = dom::tag(node).map(str::to_ascii_lowercase) else {
            return;
        };
        if is_invisible_tag(&tag) || is_hidden(node) {
            return;
        }
        if tag == "br" {
            self.formatted.push('\n');
            state.newlines += 1;
            return;
        }
        if is_replaced(&tag) && !is_empty_image(node) {
            self.struts.push(Strut {
                formatted_start: self.formatted.len(),
                clean_start: self.clean.len(),
                node: node.clone(),
                depth,
            });
            self.formatted.push(REPLACED);
            self.clean.push(REPLACED);
            if tag != "td" {
                state.newlines = 0;
            }
        }
        if block_breaks(&tag) > 0 && state.newlines == 0 {
            self.formatted.push('\n');
            state.newlines += 1;
        }
        if is_button_like_link(node) {
            self.formatted.push('\n');
            state.newlines += 1;
        }
        while is_quote(node) && state.newlines < 2 {
            self.formatted.push('\n');
            state.newlines += 1;
        }
        for child in dom::children(node) {
            self.read(&child, depth + 1, state);
        }
        while block_breaks(&tag) > state.newlines {
            self.formatted.push('\n');
            state.newlines += 1;
        }
    }

    /// The text with block breaks, trimmed.
    pub(crate) fn to_formatted_string(&self) -> String {
        undo_replacement(self.formatted.trim_matches(is_space))
    }

    /// The formatted text of a range (or all of it), trimmed, placeholders kept.
    pub(crate) fn search_string(&mut self, range: Option<Range>) -> String {
        match range {
            Some(range) => {
                let (start, end) = self.formatted_range(range.start, range.end);
                self.formatted[start..end].trim_matches(is_space).to_owned()
            }
            None => self.formatted.trim_matches(is_space).to_owned(),
        }
    }

    /// The text without whitespace.
    #[cfg(test)]
    pub(crate) fn clean_string(&self) -> &str {
        &self.clean
    }

    /// Whether two subtrees read the same, ignoring whitespace.
    pub(crate) fn equivalent_to(&self, other: &Content) -> bool {
        self.clean == other.clean
    }

    /// The clean string's length.
    pub(crate) fn len(&self) -> usize {
        self.clean.len()
    }

    /// The first occurrence of `text` (whitespace ignored).
    pub(crate) fn first_range_containing(&self, text: &str) -> Option<Range> {
        let needle = clean(text);
        self.clean.find(&needle).map(|start| Range {
            start,
            end: start + needle.len(),
        })
    }

    /// The last occurrence of `text` starting before `before` (whitespace ignored), allowing
    /// `<attachment name>` and its placeholder to stand for each other.
    pub(crate) fn last_range_containing(
        &self,
        text: &str,
        before: Option<Range>,
        equivalences: &[(String, String)],
    ) -> Option<Range> {
        let needle = clean(text);
        let limit = before.map_or(self.clean.len(), |range| range.start);
        // The last match that starts at `limit` at the latest.
        let search_end = (limit + needle.len()).min(self.clean.len());
        if let Some(start) =
            self.clean[..floor_char_boundary(&self.clean, search_end)].rfind(&needle)
        {
            return Some(Range {
                start,
                end: start + needle.len(),
            });
        }
        self.last_range_with_equivalences(&needle, limit, equivalences)
    }

    fn last_range_with_equivalences(
        &self,
        needle: &str,
        limit: usize,
        equivalences: &[(String, String)],
    ) -> Option<Range> {
        if equivalences.is_empty() {
            return None;
        }
        let pairs: Vec<(String, String)> = equivalences
            .iter()
            .map(|(a, b)| (clean(a), clean(b)))
            .collect();
        let tokens: Vec<&str> = pairs
            .iter()
            .flat_map(|(a, b)| [a.as_str(), b.as_str()])
            .collect();
        let haystack = split_keeping(&self.clean, &tokens);
        let pattern = split_keeping(needle, &tokens);
        let equivalent = |a: &str, b: &str| {
            a == b
                || pairs
                    .iter()
                    .any(|(x, y)| (a == x && b == y) || (a == y && b == x))
                || {
                    let canonical = |s: &str| {
                        pairs
                            .iter()
                            .find(|(x, y)| s == x || s == y)
                            .map(|(x, _)| x.clone())
                    };
                    canonical(a).is_some() && canonical(a) == canonical(b)
                }
        };
        let mut index = 0;
        while index < haystack.len() {
            while index < haystack.len() && !haystack[index].ends_with(pattern[0]) {
                index += 1;
            }
            if index == haystack.len() {
                break;
            }
            let mut matched = 1;
            while matched < pattern.len()
                && haystack
                    .get(index + matched)
                    .is_some_and(|piece| equivalent(piece, pattern[matched]))
            {
                matched += 1;
            }
            if matched == pattern.len() {
                let before: usize = haystack[..index].iter().map(|p| p.len()).sum();
                let start = before + haystack[index].rfind(pattern[0]).unwrap_or(0);
                let mut end = start + pattern[0].len();
                for piece in &haystack[index + 1..index + matched] {
                    end += piece.len();
                }
                return (end <= limit).then_some(Range { start, end });
            }
            index += 1;
        }
        None
    }

    /// `range` grown backwards by `text`'s clean length.
    pub(crate) fn extend_range_backwards(&self, range: Range, text: &str) -> Range {
        let length = clean(text).len();
        Range {
            start: range.start.saturating_sub(length),
            end: range.end,
        }
    }

    /// `range` grown forwards by `text`'s clean length.
    pub(crate) fn extend_range_forwards(&self, range: Range, text: &str) -> Range {
        Range {
            start: range.start,
            end: range.end + clean(text).len(),
        }
    }

    /// The paragraph that ends at clean offset `at`.
    pub(crate) fn paragraph_before(&mut self, at: usize) -> String {
        let index = self.strut_index(at);
        let mut end = self.struts[index].formatted_start;
        while end > 0 && self.formatted[..end].ends_with(is_space) {
            end = floor_char_boundary(&self.formatted, end - 1);
        }
        if end > 0 {
            let start = last_index_of(&self.formatted, "\n\n", end - 1)
                .into_iter()
                .chain(last_index_of(&self.formatted, "\n--\n", end - 1))
                .max();
            if let Some(start) = start {
                return self.formatted[start..end].to_owned();
            }
        }
        self.formatted[..end].to_owned()
    }

    /// The paragraph that starts at clean offset `at`.
    pub(crate) fn paragraph_after(&mut self, at: usize) -> String {
        let index = self.strut_index(at);
        let mut start = self.struts[index].formatted_start;
        while start < self.formatted.len() && self.formatted[start..].starts_with(is_space) {
            start += self.formatted[start..]
                .chars()
                .next()
                .map_or(1, char::len_utf8);
        }
        match self.formatted[start..].find("\n\n") {
            Some(end) => self.formatted[start..start + end].to_owned(),
            None => self.formatted[start..].to_owned(),
        }
    }

    /// The sentence around `node`, up to `size` characters.
    pub(crate) fn context_around(&self, node: &NodeRef, size: usize) -> Option<String> {
        let (index, _) = self.struts_within(node)?;
        let half = size / 2;
        let at = self.struts[index].formatted_start;
        let text = &self.formatted;
        // Only the window around `at` can matter, so only it is searched.
        let from = floor_char_boundary(text, at.saturating_sub(half));
        let after_last = |needle: &str| {
            last_index_of(&text[from..], needle, at - from).map_or(0, |i| from + i + 1)
        };
        let start = after_last("\n")
            .max(after_last("."))
            .max(at.saturating_sub(half));
        // The next line or sentence end never ends it sooner, so neither is looked for.
        let mut end = floor_char_boundary(text, at + half);
        if let Some(paragraph) = text[at..floor_char_boundary(text, end + 1)]
            .find("\n\n")
            .map(|i| at + i)
            && paragraph > at
            && paragraph < end
        {
            end = paragraph;
        }
        let start = floor_char_boundary(text, start);
        (start <= end).then(|| text[start..end].trim_matches(is_space).to_owned())
    }

    /// The first and last struts of `node` and its descendants, which are consecutive.
    fn struts_within(&self, node: &NodeRef) -> Option<(usize, usize)> {
        let mut indices = std::iter::once(node.clone())
            .chain(dom::descendants(node))
            .filter_map(|n| self.strut_of.get(&dom::key(&n)).copied());
        let first = indices.next()?;
        Some((first, indices.last().unwrap_or(first)))
    }

    /// The ranges covered by the elements `matches` accepts.
    pub(crate) fn ranges_matching(&self, matches: impl Fn(&NodeRef) -> bool) -> Vec<Range> {
        dom::select(&self.root, matches)
            .iter()
            .filter_map(|element| {
                let (first, last) = self.struts_within(element)?;
                let end = self
                    .struts
                    .get(last + 1)
                    .map_or(self.clean.len(), |strut| strut.clean_start);
                Some(Range {
                    start: self.struts[first].clean_start,
                    end,
                })
            })
            .collect()
    }

    /// The formatted text between two clean offsets, placeholders removed.
    pub(crate) fn formatted_slice(&mut self, start: usize, end: usize) -> String {
        let (start, end) = self.formatted_range(start, end);
        undo_replacement(&self.formatted[start..end])
    }

    fn formatted_range(&mut self, start: usize, end: usize) -> (usize, usize) {
        let first = self.strut_index(start);
        let last = self.strut_index(end);
        let mut from = self.struts[first].formatted_start;
        let mut to = self.struts[last].formatted_start;
        while from < to && self.formatted[from..].starts_with(is_space) {
            from += self.formatted[from..]
                .chars()
                .next()
                .map_or(1, char::len_utf8);
        }
        while to > from && self.formatted[..to].ends_with(is_space) {
            to = floor_char_boundary(&self.formatted, to - 1);
        }
        (from, to)
    }

    /// The top-level nodes a range covers, without changing the tree beyond splitting text
    /// at its ends.
    pub(crate) fn nodes_for_range(&mut self, range: Range) -> Vec<NodeRef> {
        let first = self.strut_index(range.start);
        let last = self.strut_index(range.end);
        if first == last {
            return Vec::new();
        }
        let start_marker = dom::element("span", &[]);
        let end_marker = dom::element("span", &[]);
        let mut start_depth = if first == 0 {
            insert_first(&self.root, &start_marker);
            1
        } else {
            let previous = &self.struts[first - 1];
            dom::insert_after(&previous.node, &start_marker);
            previous.depth
        };
        let mut end_depth = if Rc::ptr_eq(&self.struts[last].node, &self.root) {
            dom::append(&self.root, &end_marker);
            1
        } else {
            let strut = &self.struts[last];
            dom::insert_before(&strut.node, &end_marker);
            strut.depth
        };

        let mut cursor = start_marker.clone();
        let mut end_cursor = end_marker.clone();
        let mut leading = Vec::new();
        while start_depth > end_depth {
            leading.extend(dom::following_siblings(&cursor));
            let Some(parent) = dom::parent(&cursor) else {
                break;
            };
            cursor = parent;
            start_depth -= 1;
        }
        // Sibling runs before the end, innermost first.
        let mut trailing = Vec::new();
        while end_depth > start_depth {
            trailing.push(dom::preceding_siblings(&end_cursor));
            let Some(parent) = dom::parent(&end_cursor) else {
                break;
            };
            end_cursor = parent;
            end_depth -= 1;
        }
        let mut middle = Vec::new();
        let mut middle_trailing = Vec::new();
        while !Rc::ptr_eq(&cursor, &end_cursor) {
            let (Some(parent), Some(end_parent)) = (dom::parent(&cursor), dom::parent(&end_cursor))
            else {
                break;
            };
            if Rc::ptr_eq(&parent, &end_parent) {
                middle.extend(
                    dom::following_siblings(&cursor)
                        .into_iter()
                        .take_while(|n| !Rc::ptr_eq(n, &end_cursor)),
                );
                break;
            }
            middle.extend(dom::following_siblings(&cursor));
            middle_trailing.push(dom::preceding_siblings(&end_cursor));
            cursor = parent;
            end_cursor = end_parent;
        }
        let markers = [start_marker, end_marker];
        dom::detach_all(&markers);
        leading
            .into_iter()
            .chain(middle)
            .chain(middle_trailing.into_iter().rev().flatten())
            .chain(trailing.into_iter().rev().flatten())
            .filter(|node| !markers.iter().any(|m| Rc::ptr_eq(m, node)))
            .collect()
    }

    /// Drops each node (and its empty following siblings) that holds nothing visible, and
    /// returns the ones still in the tree.
    pub(crate) fn remove_empty_nodes(nodes: Vec<NodeRef>) -> Vec<NodeRef> {
        let starts: HashSet<NodeKey> = nodes.iter().map(dom::key).collect();
        let mut empty = Vec::new();
        for parent in dom::parents_of(&nodes) {
            let mut removing = false;
            for child in dom::children(&parent) {
                removing |= starts.contains(&dom::key(&child));
                removing = removing && recursively_empty(&child, Direction::Forward, true);
                if removing {
                    empty.push(child);
                }
            }
        }
        dom::detach_all(&empty);
        nodes
            .into_iter()
            .filter(|node| dom::parent(node).is_some())
            .collect()
    }

    /// The strut at clean offset `at`, splitting a text node when `at` falls inside one.
    fn strut_index(&mut self, at: usize) -> usize {
        let at = at.clamp(
            self.struts[0].clean_start,
            self.struts.last().map_or(0, |s| s.clean_start),
        );
        match self
            .struts
            .binary_search_by(|strut| strut.clean_start.cmp(&at))
        {
            Ok(mut index) => {
                // Several struts can start at the same offset; take the first.
                while index > 0 && self.struts[index - 1].clean_start == at {
                    index -= 1;
                }
                index
            }
            Err(insert) => {
                let index = insert - 1;
                let strut = self.struts[index].clone();
                let Some(text) = dom::text(&strut.node) else {
                    return index;
                };
                let wanted = at - strut.clean_start;
                let (mut split, mut counted) = (0, 0);
                for (i, c) in text.char_indices() {
                    if counted >= wanted {
                        split = i;
                        break;
                    }
                    if !is_space(c) {
                        counted += c.len_utf8();
                    }
                    split = i + c.len_utf8();
                }
                while text[split..].starts_with(is_space) {
                    split += text[split..].chars().next().map_or(1, char::len_utf8);
                }
                let head = &text[..split];
                let tail = dom::text_node(&text[split..]);
                dom::set_text(&strut.node, head);
                dom::insert_after(&strut.node, &tail);
                // Where the tail's text starts in the node's (collapsed, trimmed) text. Only the
                // head's start is trimmed: trimming its end too would land a byte early,
                // mid-character after a second split.
                let formatted = collapse_spaces(head).trim_start_matches(is_space).len();
                for later in self.strut_of.values_mut().filter(|i| **i > index) {
                    *later += 1;
                }
                self.strut_of.insert(dom::key(&tail), index + 1);
                self.struts.insert(
                    index + 1,
                    Strut {
                        formatted_start: strut.formatted_start + formatted,
                        clean_start: at,
                        node: tail,
                        depth: strut.depth,
                    },
                );
                index + 1
            }
        }
    }
}

/// Removes empty leading and trailing content under `root` (`DOMContent.trim`).
pub(crate) fn trim(root: &NodeRef) {
    recursively_empty(root, Direction::Forward, false);
    recursively_empty(root, Direction::Backward, false);
}

struct ParseState {
    /// Line breaks written since the last text.
    newlines: u32,
    /// Whether the last text node ended in whitespace.
    ended_with_space: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    Forward,
    Backward,
}

/// Whether `node` shows nothing, removing its empty children in `direction` until it finds content.
/// With `strict`, invisible elements and spacer images count as empty too.
pub(crate) fn recursively_empty(node: &NodeRef, direction: Direction, strict: bool) -> bool {
    if let Some(text) = dom::text(node) {
        return text.chars().all(is_space);
    }
    let Some(tag) = dom::tag(node).map(str::to_ascii_lowercase) else {
        // The document node, comments.
        return children_empty(node, direction, strict);
    };
    if is_invisible_tag(&tag) {
        return strict;
    }
    if is_replaced(&tag) {
        return strict && is_empty_image(node);
    }
    children_empty(node, direction, strict)
}

fn children_empty(node: &NodeRef, direction: Direction, strict: bool) -> bool {
    let mut children = dom::children(node);
    if direction == Direction::Backward {
        children.reverse();
    }
    let count = children.len();
    let empty: Vec<NodeRef> = children
        .into_iter()
        .take_while(|child| recursively_empty(child, direction, strict))
        .collect();
    dom::detach_all(&empty);
    empty.len() == count
}

/// Whether the element is hidden by its inline style or attributes.
pub(crate) fn is_hidden(node: &NodeRef) -> bool {
    let style = Style::of(node);
    style.get("display") == "none"
        || style.get("visibility") == "hidden"
        || dom::has_attr(node, "hidden")
        || dom::attr(node, "aria-hidden").is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
}

/// Spacer GIFs, hidden images and tracking pixels.
pub(crate) fn is_empty_image(node: &NodeRef) -> bool {
    if !dom::is(node, "img") {
        return false;
    }
    if Style::of(node).get("display") == "none" {
        return true;
    }
    static SPACER: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"https?(:|%3A)(/|%2F){2}.+(/|%2F)spacer?\.gif.*").expect("valid regex")
    });
    dom::attr(node, "src")
        .filter(|src| !src.is_empty())
        .is_some_and(|src| SPACER.is_match(&src) || pixels::is_read_status_pixel(node))
}

/// Links that look like buttons.
fn is_button_like_link(node: &NodeRef) -> bool {
    dom::is(node, "a")
        && (dom::attr(node, "style").is_some_and(|style| style.contains("background"))
            || dom::select_first(node, |n| {
                dom::is(n, "img") || dom::attr(n, "style").is_some_and(|s| s.contains("background"))
            })
            .is_some())
}

fn is_quote(node: &NodeRef) -> bool {
    dom::is(node, "blockquote") || dom::has_class(node, "gmail_quote")
}

fn insert_first(parent: &NodeRef, node: &NodeRef) {
    match dom::children(parent).first() {
        Some(first) => dom::insert_before(first, node),
        None => dom::append(parent, node),
    }
}

fn collapse_spaces(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for c in text.chars() {
        if is_space(c) {
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

fn undo_replacement(text: &str) -> String {
    text.replace(REPLACED, "")
}

/// Splits `text` around every occurrence of `tokens`, keeping the tokens as pieces (a JS
/// `split` on a capturing alternation).
fn split_keeping<'a>(text: &'a str, tokens: &[&str]) -> Vec<&'a str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < text.len() {
        if let Some(token) = tokens
            .iter()
            .find(|t| !t.is_empty() && text[i..].starts_with(**t))
        {
            pieces.push(&text[start..i]);
            pieces.push(&text[i..i + token.len()]);
            i += token.len();
            start = i;
        } else {
            i += text[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    pieces.push(&text[start..]);
    pieces
}

/// The last match of `needle` starting at or before `from`.
fn last_index_of(text: &str, needle: &str, from: usize) -> Option<usize> {
    let end = floor_char_boundary(text, from.saturating_add(needle.len()));
    text[..end].rfind(needle)
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content(html: &str) -> Content {
        let document = dom::parse(html);
        Content::new(&dom::body(&document).unwrap())
    }

    #[test]
    fn reads_blocks_as_lines() {
        let text = content(
            "<div>Hi Ann,</div><div><br></div><p>See <b>this</b>.</p><blockquote>quoted</blockquote>",
        );
        assert_eq!(text.to_formatted_string(), "Hi Ann,\n\nSee this.\n\nquoted");
        assert_eq!(text.clean_string(), "HiAnn,Seethis.quoted");
    }

    #[test]
    fn finds_ranges_and_paragraphs() {
        let mut text =
            content("<div>new</div><div>On Mon wrote:</div><blockquote>old text</blockquote>");
        let range = text.last_range_containing("old text", None, &[]).unwrap();
        assert_eq!(&text.clean_string()[range.start..range.end], "oldtext");
        assert_eq!(
            text.paragraph_before(range.start).trim(),
            "new\nOn Mon wrote:"
        );
    }

    #[test]
    fn splits_text_nodes_repeatedly() {
        let mut text = content("<div>ab cd\u{fffd}ef</div>");
        assert_eq!(text.paragraph_after(2), "cd\u{fffd}ef\n");
        assert_eq!(text.paragraph_after(7), "ef\n");
    }

    #[test]
    fn trims_empty_edges() {
        let document = dom::parse("<div><br></div> <p>text</p><div><br></div>\n");
        let body = dom::body(&document).unwrap();
        trim(&body);
        assert_eq!(dom::inner_html(&body), "<p>text</p>");
    }
}
