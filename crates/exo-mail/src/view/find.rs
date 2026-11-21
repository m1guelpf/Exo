//! Find in page in a body: its laid-out text searched block by block, as Chromium's find-in-page
//! searches the DOM (a match doesn't cross from one block to the next; text that isn't laid out,
//! such as a collapsed quote, isn't found), and the matches highlighted as Chromium highlights
//! them: yellow, the current one orange, the text in them black.

use std::ops::Range;

use blitz_dom::{NodeId, TextHighlight, util::Color};

/// Chromium's `PlatformTextSearchHighlightColor`, inactive and active, and its
/// `PlatformTextSearchColor` (as exo-ui's find highlights the text Exo paints).
const MATCH: Color = Color::from_rgb8(255, 255, 0);
const CURRENT: Color = Color::from_rgb8(255, 150, 50);
const MATCH_TEXT: Color = Color::from_rgb8(0, 0, 0);

impl super::BodyView {
    /// Finds what `matcher` matches (byte ranges of a block's text, which it's handed) in the
    /// laid-out body, in document order, and highlights it. Returns each match's box (`[left,
    /// top, width, height]` in the column's CSS px). Find again after laying out again: the
    /// matches are of this layout.
    pub fn find(&mut self, mut matcher: impl FnMut(&str) -> Vec<Range<usize>>) -> Vec<[f32; 4]> {
        self.found.clear();
        let mut stack = vec![self.root];
        while let Some(id) = stack.pop() {
            let Some(node) = self.document.get_node(id) else {
                continue;
            };
            if let Some(text) = node
                .element_data()
                .and_then(|data| data.inline_layout_data.as_deref())
            {
                // Without what layout put in the text (a `<wbr>`'s break, an inline box), which
                // Chromium's find doesn't see; `at` takes each searched byte back to the text's.
                let mut searched = String::with_capacity(text.text.len());
                let mut at = Vec::with_capacity(text.text.len());
                for (index, ch) in text
                    .text
                    .char_indices()
                    .filter(|&(index, _)| !text.is_generated_char(index))
                {
                    searched.push(ch);
                    at.extend(index..index + ch.len_utf8());
                }
                let found = matcher(&searched)
                    .into_iter()
                    .filter(|range| !range.is_empty());
                self.found
                    .extend(found.map(|range| (id, at[range.start]..at[range.end - 1] + 1)));
            }
            // In layout order, which has the anonymous blocks and leaves out what isn't laid out.
            let children: Vec<NodeId> = node
                .layout_children
                .borrow()
                .as_ref()
                .map_or_else(|| node.children.to_vec(), |children| children.to_vec());
            stack.extend(children.into_iter().rev());
        }
        self.current_match = None;
        self.highlight();
        self.found
            .iter()
            .map(|(node, range)| {
                self.text_bounds([(*node, range.start, range.end)])
                    .unwrap_or_default()
            })
            .collect()
    }

    /// Makes match `index` of the last [`Self::find`] the current one (or none).
    pub fn set_current_match(&mut self, index: Option<usize>) {
        self.current_match = index.filter(|index| *index < self.found.len());
        self.highlight();
    }

    /// Selects match `index` of the last [`Self::find`], as Chromium leaves the current match
    /// selected when find stops (`kKeepSelection`).
    pub fn select_match(&mut self, index: usize) {
        if let Some((node, range)) = self.found.get(index).cloned() {
            self.selected_unit = None;
            self.select((node, range.start), (node, range.end));
        }
    }

    /// Takes the highlights away.
    pub fn clear_find(&mut self) {
        self.found.clear();
        self.current_match = None;
        self.document.set_text_highlights(Vec::new());
    }

    fn highlight(&mut self) {
        let highlights = self
            .found
            .iter()
            .enumerate()
            .map(|(index, (node, range))| TextHighlight {
                node: *node,
                range: range.clone(),
                background: if self.current_match == Some(index) {
                    CURRENT
                } else {
                    MATCH
                },
                color: Some(MATCH_TEXT),
            })
            .collect();
        self.document.set_text_highlights(highlights);
    }
}

#[cfg(test)]
mod tests {
    use crate::{BodyView, Fonts, Message, Region, ViewOptions, render};

    fn view(html: &str) -> BodyView {
        let message = Message {
            html: html.into(),
            from_email: "ann@example.com".into(),
            ..Message::default()
        };
        let mut view = BodyView::new(
            &render(&message, None, &[]),
            ViewOptions::default(),
            &Fonts::load(),
            None,
        );
        view.layout(600.0, 2.0);
        view
    }

    fn word(text: &str, word: &str) -> Vec<std::ops::Range<usize>> {
        text.match_indices(word)
            .map(|(at, _)| at..at + word.len())
            .collect()
    }

    #[test]
    fn matches_come_in_document_order_with_their_boxes() {
        let mut view = view("<p>One agenda</p><div>Two <b>agen</b>da and an agenda</div>");
        let boxes = view.find(|text| word(text, "agenda"));
        assert_eq!(
            boxes.len(),
            3,
            "a match within a block crosses its inline elements: {boxes:?}"
        );
        assert!(
            boxes[0][1] < boxes[1][1] && boxes[1][0] < boxes[2][0],
            "in order: {boxes:?}"
        );
        assert!(
            view.find(|text| word(text, "agendaTwo")).is_empty(),
            "nor does one cross blocks"
        );
        assert!(
            boxes
                .iter()
                .all(|[_, _, width, height]| *width > 0.0 && *height > 10.0 && *height < 30.0),
            "{boxes:?}"
        );
    }

    #[test]
    fn what_hides_behind_the_ellipsis_isnt_searched() {
        // A trailing signature, which the pipeline hides as it hides a quote.
        let html = r#"<div dir="ltr">Sounds good, agenda attached.<br clear="all"><br><div><div dir="ltr" class="gmail_signature">Ann Lee<div>Agenda keeper, Brightmoor</div></div></div></div>"#;
        let message = Message {
            html: html.into(),
            from_name: "Ann Lee".into(),
            from_email: "ann@brightmoor.co".into(),
            ..Message::default()
        };
        let rendered = render(&message, None, &[]);
        assert!(rendered.has_quoted_content);
        for (show_quoted, found) in [(false, 1), (true, 2)] {
            let mut view = BodyView::new(
                &rendered,
                ViewOptions {
                    show_quoted,
                    ..ViewOptions::default()
                },
                &Fonts::load(),
                None,
            );
            view.layout(600.0, 2.0);
            assert_eq!(
                view.find(|text| word(&text.to_lowercase(), "agenda")).len(),
                found,
                "shown: {show_quoted}"
            );
        }
    }

    #[test]
    fn highlights_paint_behind_the_text() {
        let mut view = view("<p>agenda</p>");
        let height = view.height();
        let region = |view: &mut BodyView| {
            view.paint(
                Region {
                    top: 0.0,
                    height,
                    left: 0.0,
                    width: 600.0,
                    phase: [0.0; 2],
                },
                [255; 4],
            )
        };
        let plain = region(&mut view);
        let [left, top, width, _] = view.find(|text| word(text, "agenda"))[0];
        let found = region(&mut view);
        // A pixel inside the match's box but clear of the glyphs: yellow; the current match's, orange.
        let pixel = |image: &crate::Image, x: f32, y: f32| {
            let at = ((y * 2.0) as usize * image.width as usize + (x * 2.0) as usize) * 4;
            image.rgba[at..at + 3].to_vec()
        };
        let (x, y) = (left + 0.5, top + 1.0);
        assert_eq!(pixel(&plain, x, y), vec![255, 255, 255]);
        assert_eq!(pixel(&found, x, y), vec![255, 255, 0]);
        view.set_current_match(Some(0));
        assert_eq!(pixel(&region(&mut view), x, y), vec![255, 150, 50]);
        view.clear_find();
        assert_eq!(
            pixel(&region(&mut view), x + width / 2.0, y),
            pixel(&plain, x + width / 2.0, y)
        );
    }
}
