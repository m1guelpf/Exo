//! The selection as HTML, for a reply that quotes it: what goes in the quote. That's the range's
//! `cloneContents()`: the elements between the selection's common ancestor and its ends, partly,
//! with the text selected; narrowed to the new part of the message when it reaches past it,
//! without an empty first or last node, and list items put back in their kind of list.
//!
//! Blitz keeps a selection as byte ranges of its inline roots' text (the blocks' laid-out text,
//! white space collapsed), so the text comes from there, and the inline elements around it from
//! each piece of text's style span. What isn't text in a line (images, inline blocks) is left
//! out.

use std::collections::{HashMap, HashSet};

use blitz_dom::{BaseDocument, Node, NodeId, local_name};

use crate::dom::escape_attribute;

/// A piece of the cloned range.
#[derive(Debug)]
enum Piece {
    /// A copy of the document's element `node`, holding these.
    Element(NodeId, Vec<Self>),
    /// An element the clone adds (the `ol` or `ul` around list items).
    Wrapper(&'static str, Vec<Self>),
    Text(String),
    /// A `<br>`.
    Break,
}

impl Piece {
    /// Its `textContent`.
    fn text(&self) -> String {
        match self {
            Self::Element(_, children) | Self::Wrapper(_, children) => {
                children.iter().map(Self::text).collect()
            }
            Self::Text(text) => text.clone(),
            Self::Break => String::new(),
        }
    }
}

impl super::BodyView {
    /// The selection's bounding box, as its range's `getBoundingClientRect()` has it: `[left,
    /// top, width, height]` in the column's CSS px, the union of the selected text's line
    /// pieces. `None` without a selection.
    pub fn selection_bounds(&self) -> Option<[f32; 4]> {
        self.text_bounds(self.document.get_text_selection_ranges())
    }

    /// The bounding box of the text in `ranges` (inline roots and byte ranges of their text), as
    /// a range's `getBoundingClientRect()` has it, in the column's CSS px.
    pub(super) fn text_bounds(
        &self,
        ranges: impl IntoIterator<Item = (NodeId, usize, usize)>,
    ) -> Option<[f32; 4]> {
        let document: &BaseDocument = &self.document;
        let mut bounds: Option<[f64; 4]> = None;
        for (id, start, end) in ranges {
            let Some(node) = document.get_node(id) else {
                continue;
            };
            let Some(text) = node
                .element_data()
                .and_then(|data| data.inline_layout_data.as_deref())
            else {
                continue;
            };
            let Some(root) = document.get_client_bounding_rect(id) else {
                continue;
            };
            // Line boxes start at the content box, `block_offset` down.
            let layout = node.unrounded_layout();
            let left = root.x + f64::from(layout.padding.left + layout.border.left);
            let top =
                root.y + f64::from(layout.padding.top + layout.border.top + text.block_offset);
            let scale = f64::from(text.layout.scale());
            let anchor =
                parley::Cursor::from_byte_index(&text.layout, start, parley::Affinity::Downstream);
            let focus =
                parley::Cursor::from_byte_index(&text.layout, end, parley::Affinity::Downstream);
            parley::Selection::new(anchor, focus).geometry_with(&text.layout, |rect, line| {
                // Chromium bounds a range by its text's content area, not its line boxes, and
                // leaves out the space hanging past a wrapped line's end.
                let (y0, y1, x1) =
                    text.layout
                        .get(line)
                        .map_or((rect.y0, rect.y1, rect.x1), |line| {
                            let metrics = line.metrics();
                            let end = metrics.offset + metrics.advance - metrics.hanging_advance;
                            (
                                f64::from(metrics.content_block_min_coord),
                                f64::from(metrics.content_block_max_coord),
                                rect.x1.min(f64::from(end)),
                            )
                        });
                let piece = [
                    left + rect.x0 / scale,
                    top + y0 / scale,
                    left + x1 / scale,
                    top + y1 / scale,
                ];
                bounds = Some(match bounds {
                    Some([x0, y0, x1, y1]) => [
                        x0.min(piece[0]),
                        y0.min(piece[1]),
                        x1.max(piece[2]),
                        y1.max(piece[3]),
                    ],
                    None => piece,
                });
            });
        }
        let [x0, y0, x1, y1] = bounds?;
        Some([x0, y0, x1 - x0, y1 - y0].map(|value| value as f32 * self.zoom))
    }

    /// The selection as HTML (what a reply's quote of it holds), or `None` without one.
    pub fn selected_html(&self) -> Option<String> {
        let document: &BaseDocument = &self.document;
        let ranges = document.get_text_selection_ranges();
        let (first, ..) = *ranges.first()?;
        let selected: HashMap<NodeId, (usize, usize)> = ranges
            .iter()
            .map(|&(node, start, end)| (node, (start, end)))
            .collect();
        let parent = |id: NodeId| document.get_node(id).and_then(|node| node.parent);
        let ancestors_of = |id: NodeId| std::iter::successors(parent(id), move |&id| parent(id));
        let ancestors: HashSet<NodeId> = selected.keys().flat_map(|&id| ancestors_of(id)).collect();
        // The deepest node holding every end of the selection (their common ancestor). An
        // end at the very end of a block has no text selected there, but still counts.
        let ends: Vec<NodeId> = selected
            .keys()
            .copied()
            .chain(
                self.selection_ends
                    .into_iter()
                    .flat_map(|(anchor, focus)| [anchor, focus]),
            )
            .filter(|&id| document.get_node(id).is_some())
            .collect();
        let common = std::iter::once(first)
            .chain(ancestors_of(first))
            .find(|&candidate| {
                ends.iter().all(|&id| {
                    id == candidate || ancestors_of(id).any(|ancestor| ancestor == candidate)
                })
            })?;
        let clone = Clone {
            document,
            selected: &selected,
            ancestors: &ancestors,
        };
        let mut pieces = match selected.get(&common) {
            Some(&range) => {
                // Within one block, the clone starts below whatever element holds all of it.
                let mut inline = clone.inline(common, range);
                while let [Piece::Element(_, children)] = &mut inline[..] {
                    inline = std::mem::take(children);
                }
                inline
            }
            None => clone.children(common),
        };
        if let Some(unquoted) = find_unquoted(document, &mut pieces) {
            pieces = unquoted;
        }
        if pieces.first().is_some_and(|piece| piece.text().is_empty()) {
            pieces.remove(0);
        }
        if pieces.last().is_some_and(|piece| piece.text().is_empty()) {
            pieces.pop();
        }
        let all_items = !pieces.is_empty() && pieces.iter().all(|piece| matches!(piece, Piece::Element(id, _) if is(document, *id, &local_name!("li"))));
        if all_items {
            let ordered = std::iter::once(common)
                .chain(ancestors_of(common))
                .find_map(|id| {
                    if is(document, id, &local_name!("ol")) {
                        Some(true)
                    } else {
                        is(document, id, &local_name!("ul")).then_some(false)
                    }
                });
            pieces = vec![Piece::Wrapper(
                if ordered == Some(true) { "ol" } else { "ul" },
                pieces,
            )];
        }
        let mut html = String::new();
        for piece in &pieces {
            write(document, piece, &mut html);
        }
        Some(html)
    }
}

/// What the clone is made from: the selection's ranges and every element above them.
struct Clone<'a> {
    document: &'a BaseDocument,
    selected: &'a HashMap<NodeId, (usize, usize)>,
    ancestors: &'a HashSet<NodeId>,
}

impl Clone<'_> {
    /// The clone of `node`'s children, in layout order (an anonymous block stands for the
    /// inline children it wraps).
    fn children(&self, node: NodeId) -> Vec<Piece> {
        let Some(node) = self.document.get_node(node) else {
            return Vec::new();
        };
        let mut pieces = Vec::new();
        for child in super::in_layout_order(node) {
            let Some(child_node) = self.document.get_node(child) else {
                continue;
            };
            let inner = match self.selected.get(&child) {
                Some(&range) => self.inline(child, range),
                None if self.ancestors.contains(&child) => self.children(child),
                None => continue,
            };
            if child_node.is_anonymous() {
                pieces.extend(inner);
            } else {
                pieces.push(Piece::Element(child, inner));
            }
        }
        pieces
    }

    /// Inline root `root`'s text in `range`, in the inline elements each piece of it is in.
    fn inline(&self, root: NodeId, (start, end): (usize, usize)) -> Vec<Piece> {
        let Some(node) = self.document.get_node(root) else {
            return Vec::new();
        };
        let Some(text) = node
            .element_data()
            .and_then(|data| data.inline_layout_data.as_deref())
        else {
            return Vec::new();
        };
        // An anonymous block's text is its parent's inline children.
        let top = if node.is_anonymous() {
            node.parent
        } else {
            Some(root)
        };
        let mut pieces: Vec<Piece> = Vec::new();
        let mut open: Vec<NodeId> = Vec::new();
        for line in text.layout.lines() {
            for run in line.runs() {
                for cluster in run.clusters() {
                    let range = cluster.text_range();
                    if range.start < start || range.end > end || text.is_generated_char(range.start)
                    {
                        continue;
                    }
                    let path = self.inline_path(cluster.style().brush.id, top);
                    let common = open.iter().zip(&path).take_while(|(a, b)| a == b).count();
                    open.truncate(common);
                    for &element in &path[common..] {
                        push(&mut pieces, open.len(), Piece::Element(element, Vec::new()));
                        open.push(element);
                    }
                    let content = &text.text[range];
                    if content == "\n" {
                        push(&mut pieces, open.len(), Piece::Break);
                    } else {
                        append_text(&mut pieces, open.len(), content);
                    }
                }
            }
        }
        pieces
    }

    /// The elements from just inside `top` down to `element`, outermost first.
    fn inline_path(&self, element: NodeId, top: Option<NodeId>) -> Vec<NodeId> {
        let mut path: Vec<NodeId> = std::iter::successors(Some(element), |&id| {
            self.document.get_node(id).and_then(|node| node.parent)
        })
        .take_while(|&id| Some(id) != top)
        .filter(|&id| self.document.get_node(id).is_some_and(Node::is_element))
        .collect();
        path.reverse();
        path
    }
}

/// The open element `depth` deep in `pieces` (each the last child of the one before).
fn innermost(pieces: &mut Vec<Piece>, depth: usize) -> &mut Vec<Piece> {
    if depth == 0 || !matches!(pieces.last(), Some(Piece::Element(..))) {
        return pieces;
    }
    match pieces.last_mut() {
        Some(Piece::Element(_, inner)) => innermost(inner, depth - 1),
        _ => unreachable!("checked above"),
    }
}

fn push(pieces: &mut Vec<Piece>, depth: usize, piece: Piece) {
    innermost(pieces, depth).push(piece);
}

fn append_text(pieces: &mut Vec<Piece>, depth: usize, text: &str) {
    let children = innermost(pieces, depth);
    match children.last_mut() {
        Some(Piece::Text(last)) => last.push_str(text),
        _ => children.push(Piece::Text(text.to_owned())),
    }
}

/// The children of the first `.unquoted-content` in `pieces`, in document order.
fn find_unquoted(document: &BaseDocument, pieces: &mut Vec<Piece>) -> Option<Vec<Piece>> {
    for piece in pieces {
        if let Piece::Element(id, children) = piece {
            let unquoted = document
                .get_node(*id)
                .and_then(|node| node.element_data())
                .and_then(|data| data.attr(local_name!("class")))
                .is_some_and(|class| {
                    class
                        .split_ascii_whitespace()
                        .any(|name| name == "unquoted-content")
                });
            if unquoted {
                return Some(std::mem::take(children));
            }
            if let Some(found) = find_unquoted(document, children) {
                return Some(found);
            }
        }
    }
    None
}

fn is(document: &BaseDocument, id: NodeId, name: &blitz_dom::LocalName) -> bool {
    document
        .get_node(id)
        .and_then(|node| node.element_data())
        .is_some_and(|data| data.name.local == *name)
}

/// Writes `piece` as `innerHTML` does.
fn write(document: &BaseDocument, piece: &Piece, out: &mut String) {
    match piece {
        Piece::Text(text) => {
            for ch in text.chars() {
                match ch {
                    '&' => out.push_str("&amp;"),
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    '\u{a0}' => out.push_str("&nbsp;"),
                    ch => out.push(ch),
                }
            }
        }
        Piece::Break => out.push_str("<br>"),
        Piece::Wrapper(tag, children) => {
            out.push_str(&format!("<{tag}>"));
            children
                .iter()
                .for_each(|child| write(document, child, out));
            out.push_str(&format!("</{tag}>"));
        }
        Piece::Element(id, children) => {
            let Some(data) = document.get_node(*id).and_then(|node| node.element_data()) else {
                return;
            };
            let tag = &data.name.local;
            out.push('<');
            out.push_str(tag);
            for attribute in data.attrs() {
                out.push_str(&format!(
                    " {}=\"{}\"",
                    attribute.name.local,
                    escape_attribute(&attribute.value).replace('\u{a0}', "&nbsp;")
                ));
            }
            out.push('>');
            children
                .iter()
                .for_each(|child| write(document, child, out));
            out.push_str(&format!("</{tag}>"));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{BodyView, Fonts, Granularity, Message, ViewOptions, render};

    /// A view of `html` laid out 600px wide, with the text from `from` (x, y) to `to` selected.
    fn selected(html: &str, from: (f32, f32), to: (f32, f32)) -> Option<String> {
        let fonts = Fonts::load();
        let message = Message {
            html: html.into(),
            from_email: "ann@example.com".into(),
            ..Message::default()
        };
        let mut view = BodyView::new(
            &render(&message, None, &[]),
            ViewOptions::default(),
            &fonts,
            None,
        );
        view.layout(600.0, 2.0);
        view.select_from(from.0, from.1, Granularity::Character);
        view.select_to(to.0, to.1);
        view.selected_html()
    }

    #[test]
    fn nothing_selected_is_nothing_to_quote() {
        assert_eq!(selected("<p>Hello</p>", (1.0, 5.0), (1.0, 5.0)), None);
    }

    #[test]
    fn a_selection_in_one_paragraph_keeps_its_inline_elements() {
        let html = selected(
            "<div>Hello <b>bold</b> world, and more text after it</div>",
            (0.0, 8.0),
            (590.0, 8.0),
        )
        .expect("a selection");
        // The pipeline's color classes come along.
        assert_eq!(
            html,
            "Hello <b class=\"exo-color-black exo-color\">bold</b> world, and more text after it"
        );
    }

    #[test]
    fn a_selection_across_blocks_keeps_the_blocks() {
        let html = selected(
            "<div>First line</div><div>Second line</div>",
            (0.0, 8.0),
            (590.0, 30.0),
        )
        .expect("a selection");
        assert_eq!(
            html,
            "<div class=\"exo-color-black exo-color\">First line</div><div class=\"exo-color-black exo-color\">Second line</div>"
        );
    }

    #[test]
    fn list_items_go_back_in_their_kind_of_list() {
        let html = selected(
            "<ol><li>One</li><li>Two</li></ol>",
            (20.0, 8.0),
            (590.0, 30.0),
        )
        .expect("a selection");
        assert_eq!(
            html,
            "<ol><li class=\"exo-color-black exo-color\">e</li><li class=\"exo-color-black exo-color\">Two</li></ol>"
        );
        // From the end of the first item, as Chromium's clone has it once its empty first item
        // is trimmed.
        let html = selected(
            "<ol><li>One</li><li>Two</li></ol>",
            (45.0, 8.0),
            (590.0, 30.0),
        )
        .expect("a selection");
        assert_eq!(
            html,
            "<ol><li class=\"exo-color-black exo-color\">Two</li></ol>"
        );
    }
}
