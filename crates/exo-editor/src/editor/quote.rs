//! A reply that quotes the selection of the message it answers: the selection goes in a quote
//! below the caret's line, the caret two lines under it.

use super::{Editor, fix_cursors};
use crate::{
    blocks::QUOTE_STYLE,
    html,
    model::{Block, Document, Pos},
};

/// What a block's top-level element is: its outermost container, or the block itself.
#[derive(PartialEq, Eq)]
enum Top {
    Container(crate::ContainerId),
    Block(usize),
}

fn top(blocks: &[Block], index: usize) -> Top {
    blocks[index]
        .path
        .first()
        .map_or(Top::Block(index), |container| Top::Container(container.id))
}

/// The blocks of the top-level element block `index` is in.
fn group(blocks: &[Block], index: usize) -> std::ops::Range<usize> {
    let key = top(blocks, index);
    let start = (0..index)
        .rev()
        .take_while(|&at| top(blocks, at) == key)
        .last()
        .unwrap_or(index);
    let end = (index + 1..blocks.len())
        .take_while(|&at| top(blocks, at) == key)
        .last()
        .map_or(index + 1, |at| at + 1);
    start..end
}

/// The text of `blocks` (`textContent`).
fn text(blocks: &[Block]) -> String {
    blocks
        .iter()
        .map(|block| block.text().replace('\u{FFFC}', ""))
        .collect()
}

impl Editor {
    /// Quotes `selection` (a message's selected part as HTML, the clone
    /// [`exo_mail::BodyView::selected_html`] makes): in a gray-barred quote unless it's all quotes
    /// already, then an empty line, and the caret on the line after. An empty body becomes just
    /// that; otherwise it goes after the caret's top-level element, an empty line first if that
    /// has text, or in place of the empty element before it if neither has. One undo step.
    pub fn add_quote(&mut self, selection: &str) {
        let quote = if html::is_blockquoted(selection) {
            format!("<div>{selection}</div>")
        } else {
            format!("<div><blockquote style=\"{QUOTE_STYLE}\">{selection}</blockquote></div>")
        };
        self.undo.save(&self.doc, self.selection);
        if text(&self.doc.blocks).trim().is_empty() {
            let mut doc = html::parse(&exo_mail::sanitize_fragment(&format!(
                "{quote}<div><br></div><div><br></div>"
            )));
            fix_cursors(&mut doc);
            self.doc = Document::new(doc.blocks);
            let caret = Pos::new(self.doc.blocks.len() - 1, 0);
            self.collapse_to(caret);
            return self.changed();
        }
        let mut at = group(&self.doc.blocks, self.selection.end().block);
        let has_text = !text(&self.doc.blocks[at.clone()]).trim().is_empty();
        if !has_text && at.start > 0 {
            let before = group(&self.doc.blocks, at.start - 1);
            if text(&self.doc.blocks[before.clone()]).is_empty() {
                self.doc.blocks.drain(before.clone());
                at = at.start - before.len()..at.end - before.len();
            }
        }
        let markup = format!(
            "{}{quote}<div><br></div><div><br></div>",
            if has_text { "<div><br></div>" } else { "" }
        );
        let mut fragment = html::parse(&exo_mail::sanitize_fragment(&markup));
        fix_cursors(&mut fragment);
        let mut ids = std::collections::HashMap::new();
        let blocks: Vec<Block> = fragment
            .blocks
            .into_iter()
            .map(|mut block| {
                for container in &mut block.path {
                    container.id = *ids
                        .entry(container.id)
                        .or_insert_with(|| self.doc.fresh_id());
                }
                block
            })
            .collect();
        let caret = Pos::new(at.end + blocks.len() - 1, 0);
        self.doc.blocks.splice(at.end..at.end, blocks);
        self.collapse_to(caret);
        self.changed();
    }
}

#[cfg(test)]
mod tests {
    use crate::{Editor, Pos, Selection};

    const QUOTE: &str = "<div><blockquote style=\"background-image: -webkit-linear-gradient(left, rgb(237, 237, 237) 75%, rgb(237, 237, 237) 75%);background-size: 1px 1px;background-position: left;background-repeat: repeat-y;padding: 0 0 0 20px;\">";

    #[test]
    fn an_empty_reply_becomes_the_quote_and_two_lines() {
        let mut editor = Editor::new();
        editor.add_quote("Can you send the <b>deck</b>?");
        assert_eq!(
            editor.html(),
            format!(
                "{QUOTE}Can you send the <b>deck</b>?</blockquote></div><div><br></div><div><br></div>"
            )
        );
        assert_eq!(
            editor.selection(),
            Selection::caret(Pos::new(2, 0)),
            "the caret on the last line"
        );
        editor.undo();
        assert!(editor.is_empty(), "one undo takes it out");
    }

    #[test]
    fn a_quote_already_quoted_isnt_quoted_again() {
        let mut editor = Editor::new();
        editor.add_quote("<blockquote>Earlier</blockquote>");
        assert!(
            editor.html().starts_with("<div><blockquote>Earlier"),
            "{}",
            editor.html()
        );
    }

    #[test]
    fn it_goes_after_the_caret_line_with_an_empty_line_between() {
        let mut editor = Editor::from_html("<div>Thanks!</div><div>Maya</div>");
        editor.set_selection(Selection::caret(Pos::new(0, 3)));
        editor.add_quote("the numbers");
        assert_eq!(
            editor.html(),
            format!(
                "<div>Thanks!<br></div><div><br></div>{QUOTE}the numbers</blockquote></div><div><br></div><div><br></div><div>Maya<br></div>"
            )
        );
        assert_eq!(editor.selection(), Selection::caret(Pos::new(4, 0)));
    }

    #[test]
    fn on_an_empty_line_it_takes_the_empty_line_before() {
        let mut editor = Editor::from_html("<div>Thanks!</div><div><br></div><div><br></div>");
        editor.set_selection(Selection::caret(Pos::new(2, 0)));
        editor.add_quote("the numbers");
        assert_eq!(
            editor.html(),
            format!(
                "<div>Thanks!<br></div><div><br></div>{QUOTE}the numbers</blockquote></div><div><br></div><div><br></div>"
            )
        );
    }
}
