//! Spelling: the body's misspelled words, marked as Chromium marks them on a Mac, by a spellchecker
//! the app hands the editor ([`Editor::set_spelling`]: macOS's in Exo, a word list in tests), and
//! what the right-click menu does with one.
//!
//! As Chromium's idle-time checker does, the editor checks only while it has the keyboard, a few
//! milliseconds' worth of lines a frame (a long quote revealed is checked over a few frames), and
//! a word isn't marked while it's being typed: the misspelling the caret is in after typing waits
//! until the caret leaves it (a space, a click, an arrow). Marks stay when the editor loses the
//! keyboard, as a `contenteditable`'s do. With autocorrect on, none are marked: the body's
//! spellcheck is off for it, and autocorrect marks words itself ([`crate::autocorrect`]).

use std::{
    collections::HashMap,
    ops::Range,
    time::{Duration, Instant},
};

use crate::{
    editor::Editor,
    model::{Content, Pos},
};

/// How long a frame spends checking lines it hasn't checked yet.
const BUDGET: Duration = Duration::from_millis(6);

/// A spellchecker.
pub trait Spelling {
    /// The misspelled words in `text`, as character ranges, in order.
    fn check(&mut self, text: &str) -> Vec<Range<usize>>;
    /// Corrections for the word at `word` (a character range) in `text`, the likeliest first.
    fn guesses(&mut self, text: &str, word: Range<usize>) -> Vec<String>;
    /// Adds `word` to the user's dictionary (`Add to Dictionary`).
    fn learn(&mut self, word: &str);
    /// Stops marking `word` in this body (`Ignore Spelling`).
    fn ignore(&mut self, word: &str);
    /// What autocorrect replaces `word` with whether it's misspelled or not: the user's text
    /// replacement for it, if any.
    fn replacement(&mut self, _word: &str) -> Option<String> {
        None
    }
    /// What autocorrect replaces the misspelled word at `word` (a character range) in `text`
    /// with, if anything.
    fn correction(&mut self, _text: &str, _word: Range<usize>) -> Option<String> {
        None
    }
    /// Words the partial word at `word` (a character range) in `text` may be the start of, the
    /// likeliest first (autocomplete).
    fn completions(&mut self, _text: &str, _word: Range<usize>) -> Vec<String> {
        Vec::new()
    }
}

/// A misspelled word, where it is, and its corrections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Misspelling {
    /// Its block.
    pub block: usize,
    /// Its characters in the block.
    pub range: Range<usize>,
    /// The word as written.
    pub word: String,
    /// Corrections, the likeliest first.
    pub guesses: Vec<String>,
}

/// The editor's spelling: its spellchecker, and the marks of the document as last checked.
#[derive(Default)]
pub(crate) struct State {
    pub(crate) engine: Option<Box<dyn Spelling>>,
    /// Each block's misspellings, by the block's text: an edit checks only the blocks it changed.
    checked: HashMap<String, Vec<Range<usize>>>,
    /// The document revision `marks` are for, and whether it was checked then.
    revision: Option<(u64, bool)>,
    /// Each block's misspellings.
    marks: Vec<Vec<Range<usize>>>,
    /// The last change typed at the caret: the misspelling it's in isn't marked yet.
    pub(crate) typing: bool,
}

impl Editor {
    /// Checks the body's spelling with `engine` from now on.
    pub fn set_spelling(&mut self, engine: Box<dyn Spelling>) {
        self.spelling = State {
            engine: Some(engine),
            ..State::default()
        };
    }

    /// Whether the body has a spellchecker ([`Self::set_spelling`]).
    pub fn has_spelling(&self) -> bool {
        self.spelling.engine.is_some()
    }

    /// Brings the marks up to date with the document: blocks not checked yet are checked when
    /// `check` (the editor has the keyboard), as many as [`BUDGET`] allows; the others keep what
    /// they had. Returns whether some are left for next frame.
    pub(crate) fn update_spelling(&mut self, check: bool) -> bool {
        // Autocorrect marks words itself: the body's `spellcheck` is off with it on.
        if self.autocorrect.on {
            return false;
        }
        let Some(engine) = self.spelling.engine.as_mut() else {
            return false;
        };
        if self
            .spelling
            .revision
            .is_some_and(|(revision, checked)| revision == self.revision && (checked || !check))
        {
            return false;
        }
        let started = Instant::now();
        let mut left = false;
        let mut checked = HashMap::new();
        let mut marks = Vec::with_capacity(self.doc.blocks.len());
        for block in &self.doc.blocks {
            if !matches!(block.content, Content::Inline(_)) {
                marks.push(Vec::new());
                continue;
            }
            let text = block.text();
            let found = match self
                .spelling
                .checked
                .remove(&text)
                .or_else(|| checked.get(&text).cloned())
            {
                Some(found) => found,
                None if check && started.elapsed() < BUDGET => engine.check(&text),
                None => {
                    left |= check;
                    marks.push(Vec::new());
                    continue;
                }
            };
            marks.push(found.clone());
            checked.insert(text, found);
        }
        self.spelling.checked = checked;
        self.spelling.marks = marks;
        self.spelling.revision = Some((self.revision, check && !left));
        left
    }

    /// The misspellings marked now: each one's block and characters.
    pub fn marked_misspellings(&self) -> Vec<(usize, Range<usize>)> {
        (0..self.spelling.marks.len())
            .flat_map(|index| {
                self.misspellings(index)
                    .map(move |range| (index, range.clone()))
            })
            .collect()
    }

    /// Block `index`'s marked misspellings: all but the one being typed.
    pub(crate) fn misspellings(&self, index: usize) -> impl Iterator<Item = &Range<usize>> {
        let caret = self.selection.focus;
        let typing = self.spelling.typing && self.selection.is_collapsed() && caret.block == index;
        let marks = if self.autocorrect.on {
            None
        } else {
            self.spelling.marks.get(index)
        };
        marks.into_iter().flatten().filter(move |range| {
            !(typing && range.start <= caret.offset && caret.offset <= range.end)
        })
    }

    /// The marked misspelling at `pos` (in it or at its end), with its corrections.
    pub fn misspelling_at(&mut self, pos: Pos) -> Option<Misspelling> {
        let range = self
            .misspellings(pos.block)
            .find(|range| range.start <= pos.offset && pos.offset <= range.end)?
            .clone();
        let text = self.doc.blocks.get(pos.block)?.text();
        let word: String = text.chars().skip(range.start).take(range.len()).collect();
        let guesses = self.spelling.engine.as_mut()?.guesses(&text, range.clone());
        Some(Misspelling {
            block: pos.block,
            range,
            word,
            guesses,
        })
    }

    /// Replaces `misspelling` with `correction`, one undo step, the caret after it. Does nothing if
    /// the word has changed since.
    pub fn correct(&mut self, misspelling: &Misspelling, correction: &str) {
        let Some(block) = self.doc.blocks.get(misspelling.block) else {
            return;
        };
        let word: String = block
            .text()
            .chars()
            .skip(misspelling.range.start)
            .take(misspelling.range.len())
            .collect();
        if word != misspelling.word {
            return;
        }
        self.replace(
            Pos::new(misspelling.block, misspelling.range.start),
            Pos::new(misspelling.block, misspelling.range.end),
            correction,
        );
    }

    /// Adds `word` to the user's dictionary (`Add to Dictionary`): it's marked nowhere from now on.
    pub fn learn_spelling(&mut self, word: &str) {
        if let Some(engine) = self.spelling.engine.as_mut() {
            engine.learn(word);
            self.forget_checks();
        }
    }

    /// Stops marking `word` in this body (`Ignore Spelling`).
    pub fn ignore_spelling(&mut self, word: &str) {
        if let Some(engine) = self.spelling.engine.as_mut() {
            engine.ignore(word);
            self.forget_checks();
        }
    }

    /// Checks every block again.
    fn forget_checks(&mut self) {
        self.spelling.checked.clear();
        self.spelling.revision = None;
        self.update_spelling(true);
    }
}
