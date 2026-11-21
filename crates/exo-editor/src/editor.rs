//! The editor's state and its commands, with Squire's semantics. Everything here works without a
//! UI; the widget ([`crate::view`]) turns input into these calls.

mod drop;
mod images;
mod quote;

use std::{collections::HashSet, ops::Range};

pub use images::{Fit, Upload};

use crate::{
    blocks::{INDENT_STYLE, QUOTE_STYLE},
    grapheme, html,
    model::{Block, ContainerKind, Content, Document, Element, Piece, Pos, Run, Selection},
    trigger::{self, Trigger},
    undo::{UndoMark, UndoStack},
};

/// An inline format with a key of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    /// `<b>` (⌘B).
    Bold,
    /// `<i>` (⌘I).
    Italic,
    /// `<u>` (⌘U).
    Underline,
    /// `<s>` (⌘⇧X).
    Strikethrough,
}

impl Format {
    /// The element Squire wraps text in.
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Bold => "b",
            Self::Italic => "i",
            Self::Underline => "u",
            Self::Strikethrough => "s",
        }
    }
}

/// A swatch of the colour subformatter, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    /// `No color`: dark gray that renders as the theme's text colour.
    None,
    /// `Lava Red`.
    LavaRed,
    /// `Irascible Orange`.
    IrascibleOrange,
    /// `Seafoam Green`.
    SeafoamGreen,
    /// `Conscientious Blue`.
    ConscientiousBlue,
    /// `Pretty In Pink`.
    PrettyInPink,
}

impl Color {
    /// Every swatch, in the picker's order.
    pub const ALL: [Self; 6] = [
        Self::None,
        Self::LavaRed,
        Self::IrascibleOrange,
        Self::SeafoamGreen,
        Self::ConscientiousBlue,
        Self::PrettyInPink,
    ];

    /// The swatch's name, its tooltip.
    pub const fn name(self) -> &'static str {
        match self {
            Self::None => "No color",
            Self::LavaRed => "Lava Red",
            Self::IrascibleOrange => "Irascible Orange",
            Self::SeafoamGreen => "Seafoam Green",
            Self::ConscientiousBlue => "Conscientious Blue",
            Self::PrettyInPink => "Pretty In Pink",
        }
    }

    /// The CSS colour.
    pub const fn rgb(self) -> [u8; 3] {
        match self {
            Self::None => [34, 34, 34],
            Self::LavaRed => [222, 84, 84],
            Self::IrascibleOrange => [233, 150, 56],
            Self::SeafoamGreen => [92, 186, 135],
            Self::ConscientiousBlue => [74, 143, 225],
            Self::PrettyInPink => [201, 118, 209],
        }
    }

    /// The `span.colour` it applies, as outgoing HTML has it.
    pub fn element(self) -> Element {
        let [r, g, b] = self.rgb();
        let important = if self == Self::None {
            ""
        } else {
            " !important"
        };
        Element::with(
            "span",
            &[
                ("class", "colour"),
                ("style", &format!("color:rgb({r}, {g}, {b}){important}")),
            ],
        )
    }

    /// The swatch a `span.colour` element is, if it's one of the palette's.
    pub fn of(element: &Element) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|color| color.element() == *element)
    }
}

/// The links autolinking makes in one text node: [`exo_mail::match_link`], then again on what
/// follows each link.
fn autolinks(text: &str) -> Vec<(Range<usize>, String)> {
    let mut found = Vec::new();
    let mut start = 0;
    while let Some((range, href)) = exo_mail::match_link(&text[start..]) {
        if range.is_empty() {
            break;
        }
        found.push((start + range.start..start + range.end, href));
        start += range.end;
    }
    found
}

/// Whether autolinking leaves `text` alone outside a plain-text paste: a text node that is only an
/// email address.
fn is_address(text: &str) -> bool {
    matches!(exo_mail::match_link(text), Some((range, href)) if range.len() == text.len() && href.starts_with("mailto:"))
}

/// `run` with the links `finder` finds in its text (`(byte range, href)`, in order) wrapped in
/// `link(href)`, innermost (autolinking one text node). A run already in a link stays as it is.
pub(crate) fn link_run(
    run: Run,
    finder: impl Fn(&str) -> Vec<(Range<usize>, String)>,
    link: impl Fn(&str) -> Element,
) -> Vec<Run> {
    let Piece::Text(text) = &run.piece else {
        return vec![run];
    };
    if run.marks.iter().any(|mark| mark.is("a")) {
        return vec![run];
    }
    let mut out = Vec::new();
    let mut last = 0;
    for (range, href) in finder(text) {
        if range.start > last {
            out.push(Run::text(&text[last..range.start], run.marks.clone()));
        }
        let mut marks = run.marks.clone();
        marks.push(link(&href));
        out.push(Run::text(&text[range.clone()], marks));
        last = range.end;
    }
    if last < text.len() {
        out.push(Run::text(&text[last..], run.marks.clone()));
    }
    out
}

/// The link autolinking makes, with `target="_blank"`.
fn autolink_element(href: &str) -> Element {
    Element::with("a", &[("href", href), ("target", "_blank")])
}

/// Options that change what typing does, and where the editor is.
#[derive(Clone, Copy, Debug, Default)]
pub struct Settings {
    /// `convertHyphenToBulletList`: `-` and Space at the start of a line starts a bulleted list.
    pub hyphen_starts_list: bool,
    /// `skipDateDetection`: dates aren't detected (a snippet's body, an event's About).
    pub skip_dates: bool,
    /// The draft is popped out, so its date tooltip keeps 7.5px from the window's left rather than
    /// 22.5px.
    pub popped_out: bool,
    /// Another draft is popped out, so this one's date tooltip keeps 7.5px from the window's right
    /// rather than 22.5px.
    pub another_popped_out: bool,
}

/// What the link button and ⌘K do with the current selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkAction {
    /// Nothing selected and not in a link: ⌘K isn't the editor's (it opens the palette).
    None,
    /// The selection is inside one link: remove it.
    Remove,
    /// Open the link input, prefilled with this (empty, or the `href` of a link the selection
    /// extends).
    Edit(String),
}

/// The editor: a document, a selection, Squire's undo stack, and the editing commands.
pub struct Editor {
    pub(crate) doc: Document,
    pub(crate) selection: Selection,
    pub(crate) undo: UndoStack,
    /// Marks the next typed text takes instead of its neighbour's (⌘B on a caret).
    pending: Option<Vec<Element>>,
    /// The trigger text of a list just made from markdown (`* `), restored by Backspace.
    auto_list: Option<String>,
    /// Where the caret was after the last Unicode replacement: Backspace there undoes it.
    replacement: Option<Pos>,
    /// The inline trigger typed into, if it's still open.
    pub(crate) trigger: Option<Trigger>,
    trigger_dismissed: bool,
    /// A trigger character was typed and nothing has been picked since: a `+` opens its contacts
    /// once a name follows it.
    trigger_armed: bool,
    /// Ghost text shown after the caret (a prediction), accepted with Tab or →.
    pub(crate) ghost: Option<String>,
    pub(crate) settings: Settings,
    /// The image the resizer is on ([`Self::focused_image`]).
    image_focus: Option<Pos>,
    /// Content-IDs of images put in that get Best Fit's width once their pixels come.
    fit_on_load: HashSet<String>,
    /// `data:` images pasted, for the app to keep ([`Self::take_uploads`]).
    uploads: Vec<Upload>,
    /// Bumped on every change, so views know to lay out again.
    pub(crate) revision: u64,
    pub(crate) view: crate::view::ViewState,
    pub(crate) spelling: crate::spelling::State,
    pub(crate) autocorrect: crate::autocorrect::State,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    /// An empty body: one line. Its `div` carries the empty `class` outgoing HTML has on a draft's
    /// first line.
    pub fn new() -> Self {
        let mut first = Block::empty(Vec::new());
        first.element = Element::with("div", &[("class", "")]);
        Self::with_document(Document::new(vec![first]))
    }

    /// An editor on `document`, the caret at its start.
    fn with_document(document: Document) -> Self {
        let mut editor = Self {
            doc: document,
            selection: Selection::default(),
            undo: UndoStack::default(),
            pending: None,
            auto_list: None,
            replacement: None,
            trigger: None,
            trigger_dismissed: false,
            trigger_armed: false,
            ghost: None,
            settings: Settings::default(),
            image_focus: None,
            fit_on_load: HashSet::new(),
            uploads: Vec::new(),
            revision: 0,
            view: crate::view::ViewState::default(),
            spelling: crate::spelling::State::default(),
            autocorrect: crate::autocorrect::State::default(),
        };
        editor.undo.reset(&editor.doc, editor.selection);
        editor
    }

    /// An editor on a draft body.
    pub fn from_html(body: &str) -> Self {
        Self::with_document(load(body))
    }

    /// Replaces the body and clears undo.
    pub fn set_html(&mut self, body: &str) {
        self.replace_document(load(body));
    }

    /// Replaces the body with `body`, one undo step. The caret keeps its line counted from the
    /// end, as an edit at the top of the body leaves it.
    pub fn modify_html(&mut self, body: &str) {
        self.undo.save(&self.doc, self.selection);
        let from_end = self.doc.blocks.len() - 1 - self.caret().block;
        let offset = self.caret().offset;
        self.doc = load(body);
        let block = (self.doc.blocks.len() - 1).saturating_sub(from_end);
        self.collapse_to(self.snap(Pos::new(block, offset)));
        self.trigger = None;
        self.changed();
    }

    fn replace_document(&mut self, doc: Document) {
        self.doc = doc;
        self.selection = Selection::default();
        self.undo.reset(&self.doc, self.selection);
        self.touch();
    }

    /// The body as HTML: the draft body that's saved.
    pub fn html(&self) -> String {
        html::write(&self.doc)
    }

    /// The document.
    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// The selection.
    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// Moves the selection (clamped to the document).
    pub fn set_selection(&mut self, selection: Selection) {
        let selection = Selection::new(self.snap(selection.anchor), self.snap(selection.focus));
        if selection != self.selection {
            self.selection = selection;
            self.image_focus = None;
            self.pending = None;
            self.replacement = None;
            self.ghost = None;
            self.spelling.typing = false;
            self.update_trigger(false);
            self.view.caret_moved();
        }
    }

    /// `pos` inside the document, and not inside a grapheme.
    fn snap(&self, pos: Pos) -> Pos {
        let pos = self.doc.clamp(pos);
        Pos::new(
            pos.block,
            grapheme::snap(&self.doc.blocks[pos.block].text(), pos.offset),
        )
    }

    /// Selects everything (⌘A).
    pub fn select_all(&mut self) {
        self.set_selection(Selection::new(Pos::default(), self.doc.end()));
    }

    /// Sets the typing options.
    pub fn set_settings(&mut self, settings: Settings) {
        if settings.skip_dates != self.settings.skip_dates {
            // Dates change how lines wrap.
            self.view.forget_layout();
        }
        self.settings = settings;
    }

    /// Whether the body is visually empty (shows the placeholder).
    pub fn is_empty(&self) -> bool {
        self.doc.is_empty()
    }

    /// The selected text, blocks separated by newlines.
    pub fn selected_text(&self) -> String {
        self.doc.text(self.selection.start(), self.selection.end())
    }

    fn touch(&mut self) {
        self.revision += 1;
        self.spelling.typing = false;
        self.view.caret_moved();
    }

    /// The document changed. Lines then get the `<br>`s that keep the sent mail showing them as the
    /// editor does ([`Document::fix_lines`]).
    pub(crate) fn changed(&mut self) {
        self.doc.fix_lines();
        self.undo.changed();
        self.ghost = None;
        self.touch();
    }

    fn caret(&self) -> Pos {
        self.selection.focus
    }

    fn collapse_to(&mut self, pos: Pos) {
        self.selection = Selection::caret(pos);
    }

    /// The caret, where text can go: on an atom (a rule, a table), which can't hold any, a new
    /// line after it (before it, from its start) takes the caret.
    fn caret_for_text(&mut self) -> Pos {
        let caret = self.caret();
        let block = &self.doc.blocks[caret.block];
        if block.is_inline() {
            return caret;
        }
        let at = Pos::new(caret.block + usize::from(caret.offset > 0), 0);
        self.doc
            .blocks
            .insert(at.block, Block::empty(block.path.clone()));
        self.collapse_to(at);
        at
    }

    /// Deletes the selection if there is one; returns where the caret is.
    fn delete_selection(&mut self) -> Pos {
        let (start, end) = (self.selection.start(), self.selection.end());
        if start == end {
            return start;
        }
        let pos = self.doc.delete(start, end);
        self.doc.ensure_bottom_line();
        self.collapse_to(pos);
        pos
    }

    /// The marks text typed at the caret gets.
    fn typing_marks(&self) -> Vec<Element> {
        if let Some(pending) = &self.pending {
            return pending.clone();
        }
        let caret = self.caret();
        self.doc.blocks[caret.block].marks_at(caret.offset)
    }

    // Typing

    /// Types `text` at the caret, as keystrokes: newlines are Enter, spaces (and the whitespace a
    /// browser shows as one) Space, everything else a character. A paste of plain text goes
    /// through [`Self::paste_text`].
    pub fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            match ch {
                '\n' => self.enter(false),
                ch if is_collapsible(ch) => self.space(),
                ch if is_invisible(ch) => {}
                ch => self.type_char(ch),
            }
        }
    }

    fn type_char(&mut self, ch: char) {
        if !self.selection.is_collapsed() {
            self.undo.save(&self.doc, self.selection);
            self.delete_selection();
        }
        let marks = self.typing_marks();
        self.insert_at_caret(&ch.to_string(), marks);
        self.replace_unicode(ch);
        self.update_trigger(trigger::is_trigger_char(ch));
        self.spelling.typing = true;
        self.typed_for_assistance(ch);
    }

    fn insert_at_caret(&mut self, text: &str, marks: Vec<Element>) {
        let caret = self.caret_for_text();
        let end = self.doc.insert_text(caret, text, marks.clone());
        self.pending = self.pending.take().map(|_| marks);
        self.collapse_to(end);
        self.changed();
    }

    /// Space, with the markdown list and quote triggers.
    pub fn space(&mut self) {
        if self.selection.is_collapsed() && self.markdown_trigger() {
            return;
        }
        self.auto_list = None;
        self.undo.record(&self.doc, self.selection, false);
        self.autolink();
        if !self.selection.is_collapsed() {
            self.delete_selection();
        }
        let caret = self.caret();
        let mut marks = self.typing_marks();
        // At the end of a link, the space goes after it.
        let block = &self.doc.blocks[caret.block];
        let next = block
            .slice(caret.offset..caret.offset + 1)
            .pop()
            .map(|run| run.marks)
            .unwrap_or_default();
        if self.pending.is_none() {
            marks.retain(|mark| !mark.is("a") || next.contains(mark));
        }
        self.insert_at_caret(" ", marks);
        self.finish_word(caret);
        self.update_trigger(false);
        self.complete();
    }

    /// Enter, or Shift+Enter: a new line, and Enter finishes the word before it for autocorrect.
    pub fn enter(&mut self, shift: bool) {
        let before = self.selection.start();
        self.break_line(shift);
        if !shift && self.caret().block > before.block {
            self.finish_word(before);
        }
    }

    /// Enter, or Shift+Enter with `shift`: a rule from dashes, a code block from a fence, the
    /// markdown lists, and links typed before it.
    fn break_line(&mut self, shift: bool) {
        self.auto_list = None;
        let caret_block = &self.doc.blocks[self.selection.start().block];
        if shift && caret_block.list().is_some() {
            // A soft break inside a list item.
            self.undo.record(&self.doc, self.selection, false);
            self.delete_selection();
            let caret = self.caret_for_text();
            let marks = self.typing_marks();
            self.doc.blocks[caret.block].insert(
                caret.offset,
                vec![Run {
                    piece: Piece::Break,
                    marks,
                }],
            );
            self.collapse_to(Pos::new(caret.block, caret.offset + 1));
            return self.changed();
        }
        self.undo.record(&self.doc, self.selection, false);
        if self.rule_from_dashes() || self.code_from_fence() {
            return;
        }
        self.autolink();
        let pending = self.pending.clone();
        let caret = self.delete_selection();
        let block = &self.doc.blocks[caret.block];
        if !block.is_inline() {
            // Enter on an atom opens a line beside it.
            self.caret_for_text();
            return self.changed();
        }
        if block.in_code() {
            return self.code_enter(caret, shift);
        }
        if block.in_blockquote() {
            let pos = self.doc.break_out_of_quote(caret);
            self.collapse_to(pos);
            return self.changed();
        }
        if block.element.tag.is_empty() {
            // Loose inline content (no block element): the browser inserts a `<br>`.
            let marks = block.marks_at(caret.offset);
            self.doc.blocks[caret.block].insert(
                caret.offset,
                vec![Run {
                    piece: Piece::Break,
                    marks,
                }],
            );
            self.collapse_to(Pos::new(caret.block, caret.offset + 1));
            return self.changed();
        }
        if block.is_empty() && block.list().is_some() {
            // An empty item leaves its list (outdents); the line Squire keeps after the list
            // stays after it.
            self.doc.ensure_bottom_line();
            self.doc
                .modify_blocks(caret.block..caret.block + 1, Document::decrease_list_level);
            self.collapse_to(Pos::new(caret.block, 0));
            self.doc.renumber(caret.block);
            return self.changed();
        }
        let marks_before = self.doc.blocks[caret.block].marks_at(caret.offset);
        let pos = self.doc.split(caret);
        self.collapse_to(pos);
        self.doc.renumber(pos.block);
        // The new line keeps typing in the formatting the caret was in.
        self.pending = pending.or_else(|| {
            (!marks_before.is_empty() && self.doc.blocks[pos.block].is_empty())
                .then_some(marks_before)
        });
        self.changed();
    }

    /// On a line of three or more dashes (an em dash counts as two), outside lists, Enter puts a
    /// rule before the line and empties it, before anything else it would do (in a quote too).
    fn rule_from_dashes(&mut self) -> bool {
        let caret = self.caret();
        let block = &self.doc.blocks[caret.block];
        if !self.selection.is_collapsed() || block.list().is_some() || !block.is_inline() {
            return false;
        }
        let text: String = block
            .text()
            .chars()
            .filter(|ch| *ch != '\u{200B}')
            .collect();
        let dashes = text.chars().filter(|ch| *ch == '-').count();
        let em = text.chars().filter(|ch| *ch == '—').count();
        let weight = text.chars().count() + em;
        if weight < 3 || weight != dashes + 2 * em {
            return false;
        }
        let rule = Block {
            path: block.path.clone(),
            element: Element::new("div"),
            content: Content::Rule,
            trailing_break: false,
        };
        self.doc
            .delete(Pos::new(caret.block, 0), Pos::new(caret.block, block.len()));
        self.doc.blocks.insert(caret.block, rule);
        self.collapse_to(Pos::new(caret.block + 1, 0));
        self.pending = None;
        self.changed();
        true
    }

    /// On a line of just ```` ``` ```` or `~~~~`, outside lists, Enter makes the line a code
    /// block, empty, the caret in it. The fence's line is emptied: a space left where the fence
    /// was, with the caret before it, would keep a second Enter at the end of the code from
    /// leaving the block, and on a later line make it wrap the line above instead.
    fn code_from_fence(&mut self) -> bool {
        let caret = self.caret();
        let block = &self.doc.blocks[caret.block];
        let text: String = block
            .text()
            .chars()
            .filter(|ch| *ch != '\u{200B}')
            .collect();
        if !self.selection.is_collapsed()
            || caret.offset == 0
            || block.list().is_some()
            || block.in_code()
            || !matches!(text.as_str(), "```" | "~~~~")
        {
            return false;
        }
        let len = block.len();
        self.doc
            .delete(Pos::new(caret.block, 0), Pos::new(caret.block, len));
        self.doc.blocks[caret.block].trailing_break = true;
        self.doc
            .modify_blocks(caret.block..caret.block + 1, Document::make_code);
        self.doc.ensure_bottom_line();
        self.collapse_to(Pos::new(caret.block, 0));
        self.pending = None;
        self.changed();
        true
    }

    /// Enter in a code block (Squire's `enter` in a `PRE`): a line break (`\n`), or, on an
    /// empty line without Shift, out of the block onto a plain line.
    fn code_enter(&mut self, caret: Pos, shift: bool) {
        let block = &self.doc.blocks[caret.block];
        let is_break = |offset: usize| {
            block
                .slice(offset..offset + 1)
                .first()
                .is_some_and(|run| run.piece == Piece::Break)
        };
        let after_break = caret.offset > 0 && is_break(caret.offset - 1);
        let before_break = is_break(caret.offset);
        if !shift
            && (caret.offset == 0 || after_break)
            && (caret.offset == block.len() || before_break)
        {
            let line =
                caret.offset - usize::from(after_break)..caret.offset + usize::from(before_break);
            let pos = self.doc.leave_code(caret.block, line);
            self.collapse_to(pos);
            return self.changed();
        }
        let marks = self.typing_marks();
        let block = &mut self.doc.blocks[caret.block];
        block.insert(
            caret.offset,
            vec![Run {
                piece: Piece::Break,
                marks,
            }],
        );
        block.fix_placeholder();
        self.collapse_to(Pos::new(caret.block, caret.offset + 1));
        self.changed();
    }

    /// Backspace, with list outdenting and the Unicode-replacement undo.
    pub fn backspace(&mut self) {
        if self
            .replacement
            .take()
            .is_some_and(|at| self.selection.is_collapsed() && at == self.caret())
        {
            self.undo();
            return;
        }
        let auto_list = self.auto_list.take();
        self.undo.save(&self.doc, self.selection);
        if !self.selection.is_collapsed() {
            self.delete_selection();
            return self.changed();
        }
        let caret = self.caret();
        if caret.offset > 0 {
            let block = &self.doc.blocks[caret.block];
            let start = grapheme::backspace(&block.text(), caret.offset);
            let removed = block.slice(start..caret.offset);
            self.doc.delete(Pos::new(caret.block, start), caret);
            // Squire keeps a one-character format alive for the next keystroke.
            if let [run] = removed.as_slice()
                && !run.marks.is_empty()
                && self.doc.blocks[caret.block].marks_at(start) != run.marks
            {
                self.pending = Some(run.marks.clone());
            }
            self.collapse_to(Pos::new(caret.block, start));
            self.changed();
            self.spelling.typing = true;
            return self.update_trigger(false);
        }
        let block = &self.doc.blocks[caret.block];
        // At the start of a list's first item, outdent (restoring a markdown trigger just used).
        if block.list().is_some() && !self.is_later_item(caret.block) {
            let in_ordered =
                block.path.len() >= 2 && block.path[block.path.len() - 2].element.is("ol");
            self.doc
                .modify_blocks(caret.block..caret.block + 1, Document::decrease_list_level);
            if let Some(trigger) = auto_list.filter(|_| !in_ordered) {
                self.doc
                    .insert_text(Pos::new(caret.block, 0), &trigger, Vec::new());
            }
            self.collapse_to(Pos::new(caret.block, 0));
            return self.changed();
        }
        if caret.block > 0 {
            // A rule before the line goes with it.
            let pos = self.doc.merge_with_previous(caret.block);
            self.collapse_to(pos);
            return self.changed();
        }
        // At the very start: leave the list or quote.
        if block.list().is_some() {
            self.doc.modify_blocks(0..1, Document::decrease_list_level);
        } else if block.in_blockquote() {
            self.doc
                .modify_blocks(0..1, Document::unwrap_outer_blockquote);
        } else {
            return;
        }
        self.collapse_to(Pos::new(0, 0));
        self.changed();
    }

    /// Whether block `index` is a list item with an earlier item in the same list.
    fn is_later_item(&self, index: usize) -> bool {
        let Some((depth, list)) = self.doc.blocks[index].list() else {
            return false;
        };
        index > 0
            && self.doc.blocks[index - 1]
                .path
                .get(depth)
                .is_some_and(|previous| previous.id == list.id)
    }

    /// Forward delete (Squire's `delete`).
    pub fn delete_forward(&mut self) {
        self.undo.save(&self.doc, self.selection);
        if !self.selection.is_collapsed() {
            self.delete_selection();
            return self.changed();
        }
        let caret = self.caret();
        let block = &self.doc.blocks[caret.block];
        if caret.offset < block.len() {
            let end = grapheme::next(&block.text(), caret.offset);
            self.doc.delete(caret, Pos::new(caret.block, end));
            self.changed();
            self.spelling.typing = true;
            return;
        }
        if caret.block + 1 < self.doc.blocks.len() {
            let pos = self.doc.merge_with_previous(caret.block + 1);
            self.collapse_to(pos);
            self.changed();
        }
    }

    /// Deletes the selection after it was copied (⌘X), one undo step.
    pub(crate) fn cut(&mut self) {
        self.undo.save(&self.doc, self.selection);
        self.delete_selection();
        self.changed();
    }

    /// Deletes from `start` to the caret (⌥⌫ to a word start, ⌘⌫ to a line start), one undo step.
    pub fn delete_back_to(&mut self, start: Pos) {
        self.undo.save(&self.doc, self.selection);
        let pos = self.doc.delete(start.min(self.caret()), self.caret());
        self.collapse_to(pos);
        self.changed();
    }

    /// Deletes from the caret to `end` (⌥⌦), one undo step.
    pub fn delete_forward_to(&mut self, end: Pos) {
        self.undo.save(&self.doc, self.selection);
        let pos = self.doc.delete(self.caret(), end.max(self.caret()));
        self.collapse_to(pos);
        self.changed();
    }

    /// The markdown triggers: `*`, `1.` or `>` alone before the caret, then Space.
    fn markdown_trigger(&mut self) -> bool {
        let caret = self.caret();
        let block = &self.doc.blocks[caret.block];
        if !block.is_inline() || block.element.tag.is_empty() {
            return false;
        }
        let prefix: String = block
            .text()
            .chars()
            .take(caret.offset)
            .filter(|ch| *ch != '\u{200B}')
            .collect();
        let number = prefix.strip_suffix('.').filter(|digits| {
            (1..=4).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit())
        });
        let bullet = prefix == "*" || prefix == "-" && self.settings.hyphen_starts_list;
        let ordered = number.is_some_and(|digits| {
            let number = digits.parse::<u32>().ok();
            number == Some(1) && !block.is_list_item()
                || number.is_some_and(|number| self.continues_list(caret.block, number))
        });
        if bullet || ordered {
            self.undo.save(&self.doc, self.selection);
            self.auto_list = Some(format!("{prefix} "));
            self.doc.delete(Pos::new(caret.block, 0), caret);
            let value = number.map(|digits| digits.trim_start_matches('0').to_owned());
            self.doc
                .modify_blocks(caret.block..caret.block + 1, |doc, range| {
                    doc.make_list(range, !bullet, value.as_deref())
                });
            self.doc.ensure_bottom_line();
            self.collapse_to(Pos::new(caret.block, 0));
            self.doc.renumber(caret.block);
            self.changed();
            return true;
        }
        if prefix == ">" {
            self.undo.save(&self.doc, self.selection);
            let block = &mut self.doc.blocks[caret.block];
            block.remove(0..caret.offset);
            if block.is_empty() {
                self.doc
                    .insert_text(Pos::new(caret.block, 0), "\u{200B}", Vec::new());
            }
            self.doc
                .modify_blocks(caret.block..caret.block + 1, |doc, range| {
                    doc.wrap_blockquote(range, QUOTE_STYLE)
                });
            self.doc.ensure_bottom_line();
            let len = self.doc.blocks[caret.block].len();
            self.collapse_to(Pos::new(caret.block, len));
            self.changed();
            return true;
        }
        false
    }

    /// Whether `number.` starting block `index` continues a numbered list cut short before it:
    /// the nearest `ol` among the block's earlier siblings, whose first number plus its items is
    /// `number`.
    fn continues_list(&self, index: usize, number: u32) -> bool {
        let path = &self.doc.blocks[index].path;
        let depth = path.len();
        let in_parent = |block: &Block| {
            block.path.len() >= depth && block.path.iter().zip(path).all(|(a, b)| a.id == b.id)
        };
        let Some(list) = self.doc.blocks[..index]
            .iter()
            .rev()
            .take_while(|block| in_parent(block))
            .find_map(|block| {
                block
                    .path
                    .get(depth)
                    .filter(|container| container.element.is("ol"))
            })
        else {
            return false;
        };
        let items: Vec<&Block> = self
            .doc
            .blocks
            .iter()
            .filter(|block| {
                block.path.len() == depth + 1
                    && block.path[depth].id == list.id
                    && block.is_list_item()
            })
            .collect();
        let parse = |value: Option<&str>| value.and_then(|value| value.trim().parse::<u32>().ok());
        let start = parse(list.element.attr("start")).unwrap_or(1);
        let first = parse(items.first().and_then(|item| item.element.attr("value"))).unwrap_or(0);
        number == start.max(first) + items.len() as u32
    }

    /// Unicode replacements: `->` becomes `→` and so on, as the last character is typed.
    fn replace_unicode(&mut self, typed: char) {
        let caret = self.caret();
        let before: Vec<char> = self.doc.blocks[caret.block]
            .text()
            .chars()
            .take(caret.offset)
            .filter(|ch| *ch != '\u{200B}')
            .collect();
        let Some((len, replacement)) = unicode_replacement(typed, &before) else {
            return;
        };
        self.undo.save(&self.doc, self.selection);
        let start = caret.offset - len;
        let block = &mut self.doc.blocks[caret.block];
        let marks = block.marks_at(start + 1);
        block.remove(start..caret.offset);
        block.insert(start, vec![Run::text(replacement, marks)]);
        let at = Pos::new(caret.block, start + replacement.chars().count());
        self.collapse_to(at);
        self.replacement = Some(at);
        self.changed();
    }

    /// Links URLs in the run before the caret, on Space and Enter (a text that is only an email
    /// address is left alone).
    fn autolink(&mut self) {
        let caret = self.caret();
        let block = &mut self.doc.blocks[caret.block];
        // The text node the caret is in: the run that ends at (or holds) the caret.
        let mut end = 0;
        let Some(index) = block.runs().iter().position(|run| {
            let start = end;
            end += run.piece.len();
            caret.offset > start && caret.offset <= end
        }) else {
            return;
        };
        if matches!(&block.runs()[index].piece, Piece::Text(text) if is_address(text)) {
            return;
        }
        let runs = block.runs_mut();
        let linked = link_run(runs.remove(index), autolinks, autolink_element);
        runs.splice(index..index, linked);
        crate::model::normalize(runs);
    }

    // Formats

    /// Whether the selection (or the caret) is all in `tag`.
    pub fn has_format(&self, tag: &str) -> bool {
        if self.selection.is_collapsed() {
            return self.typing_marks().iter().any(|mark| mark.is(tag));
        }
        let mut seen = false;
        for (index, range) in self.selected_ranges() {
            for run in self.doc.blocks[index].slice(range) {
                if let Piece::Text(_) = run.piece {
                    if !run.marks.iter().any(|mark| mark.is(tag)) {
                        return false;
                    }
                    seen = true;
                }
            }
        }
        seen
    }

    /// Toggles `format` on the selection, or for what's typed next at a caret (⌘B, ⌘I, ⌘U,
    /// ⌘⇧X).
    pub fn toggle_format(&mut self, format: Format) {
        let tag = format.tag();
        self.undo.save(&self.doc, self.selection);
        if self.has_format(tag) {
            self.format(None, |mark| mark.is(tag));
        } else {
            self.format(Some(Element::new(tag)), |_| false);
        }
    }

    /// Squire's `changeFormat`, without its undo step: on the selection, drops the marks
    /// `remove` matches and wraps what isn't in `add` yet in it (innermost); at a caret, does
    /// that to what's typed next.
    fn format(&mut self, add: Option<Element>, remove: impl Fn(&Element) -> bool) {
        let restyle = |marks: &mut Vec<Element>| {
            marks.retain(|mark| !remove(mark));
            if let Some(element) = &add
                && !marks.contains(element)
            {
                marks.push(element.clone());
            }
        };
        if self.selection.is_collapsed() {
            let mut marks = self.typing_marks();
            restyle(&mut marks);
            self.pending = Some(marks);
            return self.touch();
        }
        for (index, range) in self.selected_ranges() {
            let block = &mut self.doc.blocks[index];
            let mut runs = block.slice(range.clone());
            for run in &mut runs {
                restyle(&mut run.marks);
            }
            block.remove(range.clone());
            block.insert(range.start, runs);
        }
        self.changed();
    }

    /// The selected part of each block, `(block, offsets)`.
    fn selected_ranges(&self) -> Vec<(usize, Range<usize>)> {
        let (start, end) = (self.selection.start(), self.selection.end());
        (start.block..=end.block)
            .filter(|index| self.doc.blocks[*index].is_inline())
            .map(|index| {
                let from = if index == start.block {
                    start.offset
                } else {
                    0
                };
                let to = if index == end.block {
                    end.offset
                } else {
                    self.doc.blocks[index].len()
                };
                (index, from..to)
            })
            .filter(|(_, range)| !range.is_empty())
            .collect()
    }

    /// The swatch the selection (or caret) is coloured with: what the colour picker starts on.
    pub fn color(&self) -> Color {
        let marks = if self.selection.is_collapsed() {
            self.typing_marks()
        } else {
            let start = self.selection.start();
            self.doc.blocks[start.block].marks_at(start.offset + 1)
        };
        marks
            .iter()
            .rev()
            .find_map(Color::of)
            .unwrap_or(Color::None)
    }

    /// Colours the selection, or what's typed next at a caret (the colour subformatter).
    pub fn set_color(&mut self, color: Color) {
        self.undo.save(&self.doc, self.selection);
        self.apply_color(color);
    }

    /// Colours the selection without an undo step of its own: the picker recorded one when it
    /// opened, and moving through swatches applies each live.
    pub(crate) fn apply_color(&mut self, color: Color) {
        self.format(Some(color.element()), |mark| {
            mark.is("span") && mark.attr("class") == Some("colour")
        });
    }

    // Links

    /// The link the caret is in, if any (for the link popover): the caret is in a
    /// link when the text it types into is.
    pub fn link_at_caret(&self) -> Option<String> {
        if !self.selection.is_collapsed() {
            return None;
        }
        let caret = self.caret();
        let marks = self.doc.blocks[caret.block].marks_at(caret.offset);
        marks
            .iter()
            .find(|mark| mark.is("a"))
            .and_then(|a| a.attr("href").map(str::to_owned))
    }

    /// What ⌘K or the link button does now.
    pub fn link_action(&self) -> LinkAction {
        if self.selection.is_collapsed() {
            return if self.link_at_caret().is_some() {
                LinkAction::Remove
            } else {
                LinkAction::None
            };
        }
        let mut hrefs = Vec::new();
        let mut unlinked = false;
        for (index, range) in self.selected_ranges() {
            for run in self.doc.blocks[index].slice(range) {
                match run
                    .marks
                    .iter()
                    .find(|mark| mark.is("a"))
                    .and_then(|a| a.attr("href"))
                {
                    Some(href) => hrefs.push(href.to_owned()),
                    None => unlinked = true,
                }
            }
        }
        match hrefs.first() {
            Some(_) if !unlinked && hrefs.windows(2).all(|pair| pair[0] == pair[1]) => {
                LinkAction::Remove
            }
            Some(href) => LinkAction::Edit(href.clone()),
            None => LinkAction::Edit(String::new()),
        }
    }

    /// Saves the link input: empty removes the link, `chrome://` is ignored, a missing scheme
    /// becomes `http://`; then the selection links there with `target="_blank"`. A caret gets the
    /// URL (without its scheme) as the link's text. A `data:` URL, which the sanitizer would strip
    /// from the draft, is ignored too.
    pub fn set_link(&mut self, input: &str) {
        let url = input.trim();
        if url.is_empty() {
            return self.remove_link();
        }
        if url.starts_with("chrome://") || url.starts_with("chrome-extension://") {
            return;
        }
        const SCHEMES: [&str; 13] = [
            "about:", "data:", "file:///", "ftp://", "http://", "https://", "irc://", "mailto:",
            "ssh://", "tel:", "ws://", "wss://", "chrome:",
        ];
        let href = if SCHEMES.iter().any(|scheme| url.starts_with(scheme)) {
            url.to_owned()
        } else {
            format!("http://{url}")
        };
        // The one scheme on the list DOMPurify takes off a link.
        if href.starts_with("data:") {
            return;
        }
        self.undo.save(&self.doc, self.selection);
        if self.selection.is_collapsed() {
            let text = plain(
                href.split_once("//")
                    .map_or(href.as_str(), |(_, rest)| rest),
            );
            let start = self.caret_for_text();
            let marks = self.typing_marks();
            let end = self.doc.insert_text(start, &text, marks);
            self.selection = Selection::new(start, end);
        }
        self.format(
            Some(Element::with("a", &[("href", &href), ("target", "_blank")])),
            |mark| mark.is("a"),
        );
    }

    /// The whole link the caret is in: its block, its range there and its element.
    pub(crate) fn link_around_caret(&self) -> Option<(usize, Range<usize>, Element)> {
        if !self.selection.is_collapsed() {
            return None;
        }
        let caret = self.caret();
        let block = &self.doc.blocks[caret.block];
        let link = block
            .marks_at(caret.offset)
            .into_iter()
            .find(|mark| mark.is("a") && mark.attr("href").is_some())?;
        let linked = |offset: usize| {
            block
                .slice(offset..offset + 1)
                .first()
                .is_some_and(|run| run.marks.contains(&link))
        };
        let mut start = caret.offset;
        while start > 0 && linked(start - 1) {
            start -= 1;
        }
        let mut end = caret.offset;
        while end < block.len() && linked(end) {
            end += 1;
        }
        Some((caret.block, start..end, link))
    }

    /// Removes links from the selection, or the whole link the caret is in.
    pub fn remove_link(&mut self) {
        if self.selection.is_collapsed() {
            let Some((block, range, _)) = self.link_around_caret() else {
                return;
            };
            self.undo.save(&self.doc, self.selection);
            let saved = self.selection;
            self.selection =
                Selection::new(Pos::new(block, range.start), Pos::new(block, range.end));
            self.format(None, |mark| mark.is("a"));
            self.selection = saved;
            return;
        }
        self.undo.save(&self.doc, self.selection);
        self.format(None, |mark| mark.is("a"));
    }

    // Blocks

    /// The selected blocks, with the Tab rule: a selection ending at the very start of a later
    /// block doesn't take that block.
    fn selected_blocks(&self, pull_back: bool) -> Range<usize> {
        let (start, end) = (self.selection.start(), self.selection.end());
        let last = if pull_back && end.offset == 0 && end.block > start.block {
            end.block - 1
        } else {
            end.block
        };
        start.block..last + 1
    }

    /// Runs a block operation on the selection, which stays where it is (block operations move
    /// blocks between containers, never add or remove any). Quote operations then keep a line
    /// after the last block; list operations don't.
    fn modify(
        &mut self,
        pull_back: bool,
        bottom_line: bool,
        operation: impl FnOnce(&mut Document, Range<usize>),
    ) {
        self.undo.save(&self.doc, self.selection);
        self.doc
            .modify_blocks(self.selected_blocks(pull_back), operation);
        if bottom_line {
            self.doc.ensure_bottom_line();
        }
        self.doc.renumber(self.selection.focus.block);
        self.changed();
    }

    /// Whether the selection is in a list: where Tab and Shift+Tab indent.
    pub fn in_list(&self) -> bool {
        let range = self.selected_blocks(false);
        self.doc
            .common_path(range.clone())
            .iter()
            .any(|c| matches!(c.element.container_kind(), ContainerKind::List { .. }))
            || (range.len() == 1 && self.doc.blocks[range.start].list().is_some())
    }

    fn in_quote(&self) -> bool {
        let range = self.selected_blocks(false);
        self.doc
            .common_path(range)
            .iter()
            .any(|c| c.element.container_kind() == ContainerKind::Quote)
    }

    fn in_any_blockquote(&self) -> bool {
        let range = self.selected_blocks(false);
        self.doc
            .common_path(range)
            .iter()
            .any(|c| c.element.is("blockquote"))
    }

    /// Toggles a numbered (`ordered`) or bulleted list (⌘⇧7, ⌘⇧8).
    pub fn toggle_list(&mut self, ordered: bool) {
        let tag = if ordered { "ol" } else { "ul" };
        let range = self.selected_blocks(false);
        let common = self.doc.common_path(range.clone());
        let inside = common.iter().any(|c| c.element.is(tag))
            || (range.len() == 1
                && self.doc.blocks[range.start]
                    .path
                    .iter()
                    .any(|c| c.element.is(tag)));
        // Squire drops the `style` of the selection's common ancestor first.
        if range.len() == 1 {
            self.doc.blocks[range.start]
                .element
                .attrs
                .retain(|(name, _)| name != "style");
        } else if let Some(ancestor) = common.last().map(|c| c.id) {
            for block in &mut self.doc.blocks {
                if let Some(container) = block.path.iter_mut().find(|c| c.id == ancestor) {
                    container.element.attrs.retain(|(name, _)| name != "style");
                }
            }
        }
        self.modify(false, false, |doc, range| {
            if inside {
                doc.remove_list(range)
            } else {
                doc.make_list(range, ordered, None)
            }
        });
    }

    /// Toggles a quote (⌘⇧9): inside any blockquote, removes them all; otherwise wraps the
    /// selection in one.
    pub fn toggle_quote(&mut self) {
        let inside = self.in_any_blockquote();
        self.modify(false, true, |doc, range| {
            if inside {
                doc.unwrap_all_blockquotes(range)
            } else {
                doc.wrap_blockquote(range, QUOTE_STYLE)
            }
        });
    }

    /// ⌘]: nests list items, adds a quote level inside a quote, and indents anything else by 40px.
    pub fn increase_level(&mut self) {
        if self.in_list() {
            return self.modify(false, false, Document::increase_list_level);
        }
        let style = if self.in_quote() {
            QUOTE_STYLE
        } else {
            INDENT_STYLE
        };
        self.modify(false, true, |doc, range| doc.wrap_blockquote(range, style));
    }

    /// ⌘[: outdents list items, or removes the outermost quote or indent.
    pub fn decrease_level(&mut self) {
        if self.in_list() {
            return self.modify(false, false, Document::decrease_list_level);
        }
        self.modify(false, true, Document::unwrap_outer_blockquote);
    }

    /// Tab in a list: nests the selected items. Returns whether the selection was in
    /// a list (outside one, Tab isn't the editor's).
    pub fn indent_list(&mut self) -> bool {
        if !self.in_list() {
            return false;
        }
        self.modify(true, false, Document::increase_list_level);
        true
    }

    /// Shift+Tab in a list: outdents the selected items. Returns whether it applied.
    pub fn outdent_list(&mut self) -> bool {
        if !self.in_list() {
            return false;
        }
        self.modify(true, false, Document::decrease_list_level);
        true
    }

    // Undo

    /// Undo (⌘Z). Returns the marks of the change undone ([`Self::mark_undo`]), so the caller
    /// can revert what it tied to it.
    pub fn undo(&mut self) -> Vec<UndoMark> {
        match self.undo.undo(&self.doc, self.selection) {
            Some((doc, selection, marks)) => {
                self.restore(doc, selection);
                marks
            }
            None => Vec::new(),
        }
    }

    /// Redo (⌘⇧Z, ⌘Y). Returns the marks of the change redone.
    pub fn redo(&mut self) -> Vec<UndoMark> {
        match self.undo.redo() {
            Some((doc, selection, marks)) => {
                self.restore(doc, selection);
                marks
            }
            None => Vec::new(),
        }
    }

    /// Collapses the selection to its focus.
    pub(crate) fn collapse_selection(&mut self) {
        self.set_selection(Selection::caret(self.selection.focus));
    }

    /// Puts back an earlier state: an undo step, or the document as it was when the colour
    /// picker opened (Esc).
    pub(crate) fn restore(&mut self, doc: Document, selection: Selection) {
        self.doc = doc;
        self.selection = selection;
        self.image_focus = None;
        self.pending = None;
        self.replacement = None;
        self.trigger = None;
        self.ghost = None;
        self.touch();
    }

    /// Whether ⌘Z would do anything.
    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    /// Whether redo would do anything.
    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }

    /// Ties `mark` to the latest change, so [`Self::undo`] and [`Self::redo`] hand it back (a
    /// recipient change made from the body).
    pub fn mark_undo(&mut self, mark: UndoMark) {
        self.undo.mark(mark);
    }

    /// Records an undo step now, as Squire does before inserting an emoji, mention or snippet.
    pub fn save_undo(&mut self) {
        self.undo.save(&self.doc, self.selection);
    }

    // Paste and insertion

    /// Pastes HTML at the selection, through the paste cleanup. Its `data:` images become the
    /// draft's inline attachments ([`Self::take_uploads`]).
    pub fn paste_html(&mut self, markup: &str) {
        let fragment = self.pasted_html(markup);
        self.insert_fragment(fragment, false);
    }

    /// `markup` cleaned as a paste is, its `data:` images taken as the draft's.
    pub(crate) fn pasted_html(&mut self, markup: &str) -> Document {
        let mut fragment = html::parse_paste(&exo_mail::sanitize_fragment(
            &exo_mail::prepare_paste(markup),
        ));
        self.take_pasted_images(&mut fragment);
        fragment
    }

    /// Pastes plain text: each line a block, less one trailing newline. A single URL pasted over
    /// a selection links it instead.
    pub fn paste_text(&mut self, text: &str) {
        let trimmed = text.trim();
        if !self.selection.is_collapsed()
            && !trimmed.is_empty()
            && !trimmed.contains(char::is_whitespace)
            && (trimmed.starts_with("http://")
                || trimmed.starts_with("https://")
                || trimmed.starts_with("www."))
        {
            return self.set_link(trimmed);
        }
        self.insert_fragment(plain_fragment(text), true);
    }

    /// Inserts a document fragment at the selection: its URLs are linked (a text that is only an
    /// email address too if `emails`, as for plain text), its blocks get their trailing `<br>`, its
    /// first block's content joins the caret's line, and its last block's the rest of that line.
    pub(crate) fn insert_fragment(&mut self, mut fragment: Document, emails: bool) {
        for block in &mut fragment.blocks {
            if let Content::Inline(runs) = &mut block.content {
                *runs = std::mem::take(runs)
                    .into_iter()
                    .flat_map(|run| {
                        let address =
                            !emails && matches!(&run.piece, Piece::Text(text) if is_address(text));
                        if address {
                            vec![run]
                        } else {
                            link_run(run, autolinks, autolink_element)
                        }
                    })
                    .collect();
            }
        }
        fix_cursors(&mut fragment);
        self.undo.save(&self.doc, self.selection);
        self.delete_selection();
        self.place_fragment(fragment);
    }

    /// Puts `fragment`'s blocks in at the caret, the caret after them.
    fn place_fragment(&mut self, fragment: Document) {
        let caret = self.caret_for_text();
        let mut blocks = fragment.blocks;
        let marks = self.typing_marks();
        let inline_only = blocks.len() == 1 && blocks[0].path.is_empty() && blocks[0].is_inline();
        if inline_only {
            let mut runs = blocks.remove(0).runs().to_vec();
            for run in &mut runs {
                // A pasted link inside a link replaces it: HTML can't nest them.
                let linked = run.marks.iter().any(|mark| mark.is("a"));
                run.marks.splice(
                    0..0,
                    marks
                        .iter()
                        .filter(|mark| !(linked && mark.is("a")))
                        .cloned(),
                );
            }
            let len: usize = runs.iter().map(|run| run.piece.len()).sum();
            self.doc.blocks[caret.block].insert(caret.offset, runs);
            self.collapse_to(Pos::new(caret.block, caret.offset + len));
            return self.changed();
        }
        // Split the line and put the fragment's blocks between the halves, in its containers.
        let after = self.doc.split(caret);
        let base_path = self.doc.blocks[caret.block].path.clone();
        let mut ids = std::collections::HashMap::new();
        let inserted: Vec<Block> = blocks
            .into_iter()
            .map(|mut block| {
                for container in &mut block.path {
                    container.id = *ids
                        .entry(container.id)
                        .or_insert_with(|| self.doc.fresh_id());
                }
                block.path.splice(0..0, base_path.iter().cloned());
                block
            })
            .collect();
        let count = inserted.len();
        self.doc.blocks.splice(after.block..after.block, inserted);
        let last = after.block + count - 1;
        // The first and last fragment blocks join the line's halves when they're plain lines.
        let end = Pos::new(last, self.doc.blocks[last].len());
        let mut caret_at = end;
        if self.doc.blocks[last + 1].path == self.doc.blocks[last].path
            && self.doc.blocks[last].is_inline()
        {
            caret_at = self.doc.merge_with_previous(last + 1);
        }
        if self.doc.blocks[after.block].path == self.doc.blocks[caret.block].path
            && self.doc.blocks[after.block].is_inline()
        {
            let joined = self.doc.merge_with_previous(after.block);
            if caret_at.block == after.block {
                caret_at = Pos::new(joined.block, joined.offset + caret_at.offset);
            } else {
                caret_at.block -= 1;
            }
        }
        if self.doc.blocks[caret.block].is_empty()
            && caret.block + 1 < self.doc.blocks.len()
            && self.doc.blocks[caret.block].path == base_path
            && caret_at.block > caret.block
        {
            self.doc.blocks.remove(caret.block);
            caret_at.block -= 1;
        }
        self.doc.ensure_bottom_line();
        self.collapse_to(self.doc.clamp(caret_at));
        self.changed();
    }

    /// Replaces `start..end` with `text` in one undo step: how a caller resolves an inline
    /// trigger (an emoji for `:smile`, a name for `+ali`).
    pub fn replace(&mut self, start: Pos, end: Pos, text: &str) {
        let (start, end) = (
            self.doc.clamp(start.min(end)),
            self.doc.clamp(start.max(end)),
        );
        self.undo.save(&self.doc, self.selection);
        let marks = self.doc.blocks[start.block].marks_at(start.offset);
        let pos = self.doc.delete(start, end);
        self.collapse_to(pos);
        let pos = self.caret_for_text();
        let end = self.doc.insert_text(pos, &plain(text), marks);
        self.collapse_to(end);
        self.trigger = None;
        self.trigger_armed = false;
        self.changed();
    }

    /// Inserts an already-built HTML fragment at the caret (a snippet's body), one undo step.
    pub fn insert_html(&mut self, markup: &str) {
        let fragment = html::parse(&exo_mail::sanitize_fragment(markup));
        self.insert_fragment(fragment, false);
    }

    /// Opens the quote and signature into the body ("…"): the body is rebuilt around them
    /// ([`outgoing::inlined`](crate::outgoing::inlined)) and undo starts over. `signature` is the
    /// signature block ([`crate::outgoing::signature`]); `quote` the reply or forward quote, `None`
    /// for new mail.
    ///
    /// This doesn't sanitize, so `target="_blank"` stays on the quote's links: both parts must
    /// already be sanitized, as for [`outgoing::html`](crate::outgoing::html).
    pub fn reveal_quoted(&mut self, signature: Option<&str>, quote: Option<&str>) {
        let mut doc = html::parse(&crate::outgoing::inlined(&self.html(), signature, quote));
        fix_cursors(&mut doc);
        // The empty classes outgoing HTML has on what shows text.
        crate::outgoing::add_empty_classes(&mut doc.blocks);
        self.replace_document(doc);
    }

    /// Swaps the signature, for a From changed after "…" was opened: the signature block draft
    /// `draft_id` put in the body becomes `new`, unless its text no longer reads as `old` (it was
    /// edited), when it's left alone. Both are [`crate::outgoing::signature`] blocks. Undo starts
    /// over. Returns whether it was swapped.
    pub fn switch_signature(&mut self, draft_id: &str, old: &str, new: &str) -> bool {
        let body = self.html();
        let Some(range) = signature_range(&body, draft_id) else {
            return false;
        };
        // Their text without whitespace, which is what's compared.
        let squashed = |markup: &str| -> String {
            html::parse(markup)
                .blocks
                .iter()
                .flat_map(|block| block.text().chars().collect::<Vec<_>>())
                .filter(|ch| !ch.is_whitespace())
                .collect()
        };
        if squashed(&body[range.clone()]) != squashed(old) {
            return false;
        }
        let mut swapped = body;
        swapped.replace_range(range, new);
        let mut doc = html::parse(&swapped);
        fix_cursors(&mut doc);
        crate::outgoing::add_empty_classes(&mut doc.blocks);
        self.replace_document(doc);
        true
    }

    // Inline triggers and ghost text

    /// The inline trigger being typed (`:smi`, `;intro`, `+ali`), for the caller to resolve.
    pub fn trigger(&self) -> Option<&Trigger> {
        self.trigger.as_ref()
    }

    /// The name of the `+`, `@` or `-` mention the caret is at the end of, at its line's end or
    /// before a space.
    pub fn mention_before_caret(&self) -> Option<String> {
        if !self.selection.is_collapsed() {
            return None;
        }
        let caret = self.caret();
        let text: Vec<char> = self.doc.blocks[caret.block].text().chars().collect();
        if text.get(caret.offset).is_some_and(|ch| !ch.is_whitespace()) {
            return None;
        }
        let before: String = text[..caret.offset.min(text.len())].iter().collect();
        match trigger::find(&before)? {
            (trigger::TriggerKind::Contact(_), _, name) if !name.is_empty() => Some(name),
            _ => None,
        }
    }

    /// Lets the next edit open the trigger before the caret as if its character had just been
    /// typed: Backspace into a mention opens its contacts again.
    pub fn arm_trigger(&mut self) {
        self.trigger_dismissed = false;
        self.trigger_armed = true;
    }

    /// Closes the trigger's dropdown until another trigger character is typed (Esc).
    pub fn dismiss_trigger(&mut self) {
        self.trigger = None;
        self.trigger_dismissed = true;
        self.trigger_armed = false;
    }

    /// Re-reads the trigger before the caret. `typed` says a trigger character was just typed,
    /// which is what opens one (a caret merely landing on `:smile` doesn't).
    fn update_trigger(&mut self, typed: bool) {
        if typed {
            self.trigger_dismissed = false;
            self.trigger_armed = true;
        }
        if self.trigger_dismissed || !self.selection.is_collapsed() {
            self.trigger = None;
            return;
        }
        let caret = self.caret();
        let block = &self.doc.blocks[caret.block];
        let before: String = block.text().chars().take(caret.offset).collect();
        let found = trigger::find(&before);
        self.trigger = match (found, &self.trigger) {
            (Some((kind, start, query)), _)
                if typed
                    || self.trigger_armed
                    || self.trigger.as_ref().is_some_and(|open| {
                        open.start.block == caret.block && open.start.offset == start
                    }) =>
            {
                Some(Trigger {
                    kind,
                    query,
                    start: Pos::new(caret.block, start),
                    end: caret,
                    anchor: egui::Rect::NOTHING,
                })
            }
            _ => None,
        };
    }

    /// Shows `text` as ghost text after the caret (a prediction), or hides it.
    pub fn set_ghost_text(&mut self, text: Option<String>) {
        self.ghost = text
            .map(|text| plain(&text))
            .filter(|text| !text.is_empty() && self.selection.is_collapsed());
    }

    /// The ghost text showing after the caret.
    pub fn ghost_text(&self) -> Option<&str> {
        self.ghost.as_deref()
    }

    /// Accepts the ghost text (Tab or →): it's inserted as plain text, one undo step. Returns
    /// whether there was any.
    pub fn accept_ghost_text(&mut self) -> bool {
        let Some(text) = self.ghost.take() else {
            return false;
        };
        self.took_completion();
        self.undo.save(&self.doc, self.selection);
        let marks = self.typing_marks();
        self.insert_at_caret(&text, marks);
        true
    }
}

/// A draft body as the editor holds it: sanitized (`exo_mail::sanitize_fragment`), read, and
/// given its trailing `<br>`s.
fn load(body: &str) -> Document {
    let mut doc = html::parse(&exo_mail::sanitize_fragment(body));
    fix_cursors(&mut doc);
    doc
}

/// Where the `<div data-signature-draft-id="{draft_id}">` element is in `body` (the editor's
/// own serialization, so well formed), from its `<div` to its `</div>`.
fn signature_range(body: &str, draft_id: &str) -> Option<Range<usize>> {
    let attribute = body.find(&format!(" data-signature-draft-id=\"{draft_id}\""))?;
    let start = body[..attribute].rfind("<div")?;
    let mut depth = 0usize;
    let mut at = start;
    while at < body.len() {
        let rest = &body[at..];
        if rest.starts_with("<div") {
            depth += 1;
            at += 4;
        } else if rest.starts_with("</div>") {
            depth -= 1;
            at += 6;
            if depth == 0 {
                return Some(start..at);
            }
        } else {
            at += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    None
}

/// After loading, a block element without a `<br>` gets one at its end.
fn fix_cursors(doc: &mut Document) {
    for block in &mut doc.blocks {
        if !block.element.tag.is_empty()
            && block.is_inline()
            && !block.runs().iter().any(|run| run.piece == Piece::Break)
        {
            block.trailing_break = true;
        }
    }
}

/// Plain text as a paste makes it: each line a block, less one trailing newline.
pub(crate) fn plain_fragment(text: &str) -> Document {
    let text = text.replace("\r\n", "\n");
    let text = text.strip_suffix('\n').unwrap_or(&text);
    let blocks = text
        .split('\n')
        .map(|line| {
            let line = plain(line);
            Block::div(
                Vec::new(),
                if line.is_empty() {
                    Vec::new()
                } else {
                    vec![Run::text(line, Vec::new())]
                },
            )
        })
        .collect();
    Document::new(blocks)
}

/// The Unicode replacement, shared with the subject field: after `typed` was typed, the replacement
/// for the end of `before` (the text up to the caret, `typed` included) and how many characters it
/// replaces: `->` becomes `→` and so on.
pub fn unicode_replacement(typed: char, before: &[char]) -> Option<(usize, &'static str)> {
    if !matches!(typed, '>' | '-' | '—' | '=' | '~' | '<' | '.') {
        return None;
    }
    // Each rule's alternatives must follow the start, a word character or whitespace.
    let boundary = |at: usize| {
        at == 0
            || before[at - 1].is_ascii_alphanumeric()
            || before[at - 1] == '_'
            || before[at - 1].is_whitespace()
    };
    UNICODE_REPLACEMENTS
        .iter()
        .find_map(|(patterns, replacement)| {
            patterns.iter().find_map(|pattern| {
                let pattern: Vec<char> = pattern.chars().collect();
                (before.ends_with(&pattern) && boundary(before.len() - pattern.len()))
                    .then_some((pattern.len(), *replacement))
            })
        })
}

/// The Unicode replacements: the first rule that matches wins; a rule's longer alternatives are
/// tried first, as a regex's leftmost match does.
const UNICODE_REPLACEMENTS: [(&[&str], &str); 11] = [
    (&["-->", "->", "—>"], "→"),
    (&["==>", "=>"], "⇒"),
    (&["--"], "—"),
    (&["<="], "⇐"),
    (&["<-", "<—"], "←"),
    (&["<~"], "↜"),
    (&["~~>", "~>"], "↝"),
    (&[">="], "≥"),
    (&["=<"], "≤"),
    (&["<->", "←>"], "⭤"),
    (&["..."], "…"),
];

/// `text` as a run can hold it: whitespace a browser shows as a space is one, and what it
/// shows as nothing goes. Text enters the model already as it shows, so what's written reads
/// back the same.
fn plain(text: &str) -> String {
    text.chars()
        .filter(|ch| !is_invisible(*ch))
        .map(|ch| if is_collapsible(ch) { ' ' } else { ch })
        .collect()
}

/// HTML whitespace other than a no-break space: what CSS shows as (at most) one space.
fn is_collapsible(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r' | '\x0C')
}

/// Control characters (other than whitespace) and U+FEFF, Squire's caret placeholder, which the
/// saved HTML drops.
fn is_invisible(ch: char) -> bool {
    (ch.is_control() && !is_collapsible(ch)) || ch == '\u{FEFF}'
}
