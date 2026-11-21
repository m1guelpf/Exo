//! Autocorrect and autocomplete: typing assistance from the body's spellchecker
//! ([`crate::Spelling`], macOS's).
//!
//! With autocorrect on, a word is checked as it's finished (Space, Enter, `-`, or punctuation once
//! typing pauses): a text replacement or a misspelling's correction replaces it (one undo step),
//! marked with grey dots while the caret stays near it; a misspelling with no correction is marked
//! with red dots. The caret resting on a marked word shows the suggestion pill: the original word
//! to undo a correction, or up to three corrections and `Learn "…"`; resting at the end of a
//! misspelled word being typed, its one correction. The body's own spellcheck is off meanwhile.
//!
//! Autocomplete shows what's likely next at the end of a line as ghost text; Tab or → takes it.
//! First the editor's own predictions: a draft's people's names, and on its first line a greeting
//! to them (`Hi Priya`); else the rest of the word being typed, from the spellchecker's
//! completions.
//!
//! The marks aren't in the document: they're kept by block and characters, and follow edits by
//! comparing the blocks' text before and after, as long as the word they mark still reads the
//! same, whole.

use std::{collections::HashSet, ops::Range};

use egui::Key;

use crate::{
    editor::Editor,
    model::{Pos, Selection},
};

/// How long the caret rests before a suggestion shows, and typing pauses after punctuation
/// before the word before it is finished.
const DEBOUNCE: f64 = 0.3;
/// The punctuation that finishes the word before it once typing pauses.
const PUNCTUATION: [char; 7] = ['.', ',', '!', '?', '"', ':', ';'];
/// How many characters past a corrected word the caret can go before its dots go for good.
const NEAR: usize = 3;
/// At most this many corrections in the pill (`slice(0, 3)`).
const SUGGESTIONS: usize = 3;
/// Autocomplete waits for this many letters of a word.
const COMPLETE_AFTER: usize = 3;
/// How long a mark made by re-checking an edited word stays hidden after typing pauses.
const REVEAL: f64 = 0.6;

/// The editor's typing assistance.
#[derive(Default)]
pub(crate) struct State {
    /// Autocorrect is on (`typingAssistance/enabled`).
    pub(crate) on: bool,
    /// Autocomplete is on (`typingAssistance/autocompleteEnabled`).
    complete: bool,
    annotations: Vec<Annotation>,
    /// The blocks' text the marks were last placed against, and the document's revision then.
    texts: Vec<String>,
    revision: Option<u64>,
    /// Words whose correction was undone, lower-cased: marked, never corrected again in this
    /// body.
    undone: HashSet<String>,
    /// Words people are named with, lower-cased, never corrected or marked (the people the draft
    /// may be to).
    names: HashSet<String>,
    /// Punctuation was typed at this revision and caret: once typing pauses, the word before it
    /// is finished.
    punctuation: Option<(u64, Pos)>,
    /// The blocks whose marks the edit last followed dropped one (it read otherwise).
    dropped: Vec<usize>,
    /// Capitalisations the user undid by editing them, lower-cased: never made again in this body.
    rejected_capitals: HashSet<String>,
    /// A snippet was just inserted: no word is finished until a key other than Space or Enter.
    after_snippet: bool,
    pill: Option<Pill>,
    /// The revision and selection the pill was last looked for at.
    looked: Option<(u64, Selection)>,
    /// The marked word whose pill was dismissed, which doesn't offer it again until the caret
    /// leaves.
    skip: Option<(usize, Range<usize>)>,
    /// A word learned from the pill, for the app's hint.
    pub(crate) learned: Option<String>,
    /// The ghost text as it was first shown: typing it takes it a letter at a time, Backspace gives
    /// them back.
    pub(crate) completion: Option<Completion>,
    /// The people the draft is from and to, for names and greetings.
    people: People,
    /// The languages whose greetings are offered.
    languages: Vec<String>,
    /// A greeting was the last thing taken, and only spaces and Backspace typed since: no name is
    /// offered then.
    greeted: bool,
}

/// Ghost text as it was first shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Completion {
    /// Where in the caret's block `full` starts.
    start: usize,
    /// The text from `start` the prediction completes it to.
    full: String,
    /// It's a greeting.
    greeting: bool,
}

/// Someone a draft is from or to, by name: first, last, and as given.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Named {
    /// What they're called.
    pub first: String,
    /// The rest of their name, or all of it if there's no more (`split(first)[1] || name`).
    pub last: String,
    /// Their name as given.
    pub full: String,
}

/// The people a draft is from and to: `None` for one without a name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct People {
    /// From.
    pub from: Option<Named>,
    /// To, in order.
    pub to: Vec<Option<Named>>,
    /// Cc.
    pub cc: Vec<Option<Named>>,
    /// Bcc.
    pub bcc: Vec<Option<Named>>,
}

/// Each language's greetings: to first names, joined with `joiner`, and to one person's
/// last name.
struct Greetings {
    language: &'static str,
    joiner: &'static str,
    first_name: &'static [&'static str],
    last_name: &'static [&'static str],
}

const GREETINGS: &[Greetings] = &[
    Greetings {
        language: "en",
        joiner: "and",
        first_name: &[
            "Hey",
            "Hi",
            "Hello",
            "Greetings",
            "Good afternoon",
            "Good morning",
            "Good evening",
            "Dear",
            "Thank you",
            "Thanks",
            "Thank you so much",
            "Many thanks",
        ],
        last_name: &["Mr.", "Mrs.", "Mx.", "Miss", "Ms.", "Dr.", "Prof."],
    },
    Greetings {
        language: "fr",
        joiner: "et",
        first_name: &[
            "Bonjour",
            "Bonsoir",
            "Coucou",
            "Salut",
            "Merci",
            "Merci beaucoup",
            "Merci mille fois",
            "Merci bien",
            "Cimer",
        ],
        last_name: &["Cher", "Chère", "Mme."],
    },
    Greetings {
        language: "es",
        joiner: "y",
        first_name: &[
            "Hola",
            "Buenas",
            "Buenos días",
            "Buenas tardes",
            "Querido",
            "Querida",
            "Querido/a",
            "Gracias",
            "Muchas gracias",
            "Estoy muy agradecido",
            "Estoy muy agradecida",
            "Te lo agradezco",
            "Te lo agradecemos",
            "Estamos muy agradecidos",
            "Estamos muy agradecidas",
        ],
        last_name: &["Estimado/a", "Estimado", "Estimada"],
    },
    Greetings {
        language: "no",
        joiner: "og",
        first_name: &[
            "Hei",
            "Hallo",
            "God morgen",
            "God dag",
            "God kveld",
            "God ettermiddag",
            "Takk",
            "Tusen takk",
            "Mange takk",
            "Takk skal du ha",
        ],
        last_name: &["Kjære", "Herr", "Frue"],
    },
    Greetings {
        language: "sv",
        joiner: "och",
        first_name: &[
            "Hei",
            "Hallo",
            "God morgen",
            "God dag",
            "God kveld",
            "God ettermiddag",
            "Tack",
            "Tack så mycket",
            "Tackar tackar",
            "Tack ska du ha",
            "Stort tack",
            "Tack själv",
        ],
        last_name: &["Raring", "Min käre"],
    },
    Greetings {
        language: "de",
        joiner: "und",
        first_name: &[
            "Hallo",
            "Lieber",
            "Liebe",
            "Guten Morgen",
            "Guten Tag",
            "Guten Abend",
            "Danke",
            "Vielen Dank",
            "Dankeschön",
            "Ich danke Ihnen",
            "Danke sehr",
        ],
        last_name: &["Sehr geehrter", "Sehr geehrte", "Herr", "Frau"],
    },
    Greetings {
        language: "pt",
        joiner: "e",
        first_name: &[
            "Bom dia",
            "Boa tarde",
            "Boa noite",
            "Querido",
            "Querida",
            "Querido/a",
            "Caro",
            "Cara",
            "Caro/a",
            "Obrigado",
            "Obrigada",
            "Muito obrigado",
            "Muito obrigada",
        ],
        last_name: &["Prezado", "Prezada", "Prezado/a"],
    },
    Greetings {
        language: "pt-br",
        joiner: "e",
        first_name: &[
            "Bom dia",
            "Boa tarde",
            "Boa noite",
            "Querido",
            "Querida",
            "Querido/a",
            "Caro",
            "Cara",
            "Caro/a",
            "Obrigado",
            "Obrigada",
            "Muito obrigado",
            "Muito obrigada",
        ],
        last_name: &["Prezado", "Prezada", "Prezado/a"],
    },
    Greetings {
        language: "nl",
        joiner: "en",
        first_name: &[
            "Goededag",
            "Goedemorgen",
            "Goedemiddag",
            "Goedeavond",
            "Dag",
            "Beste",
            "Hallo",
            "Hoi",
            "Dankjewel",
            "Dankuwel",
            "Bedankt",
            "Heel erg bedankt",
        ],
        last_name: &["Geachte heer", "Geachte mevrouw"],
    },
    Greetings {
        language: "it",
        joiner: "e",
        first_name: &[
            "Ciao",
            "Ehi",
            "Salve",
            "Buongiorno",
            "Buona giornata",
            "Buon pomeriggio",
            "Buonasera",
            "Caro",
            "Cara",
            "Caro/a",
            "Gentile",
            "Grazie",
            "Grazie mille",
            "Molte grazie",
            "Grazie di tutto",
            "La ringrazio",
            "La ringrazio molto",
            "Ti ringrazio",
        ],
        last_name: &["Egregio", "Egregia", "Egregio/a", "Sig.", "Sig.ra"],
    },
    Greetings {
        language: "pl",
        joiner: "i",
        first_name: &[
            "Dzień dobry",
            "Dobry dzień",
            "Dobry wieczór",
            "Cześć",
            "Drogi",
            "Witam",
            "Dzięki",
            "Dziękuję",
            "Dziękuję bardzo",
            "Stokrotne dzięki",
            "Wielkie dzięki",
        ],
        last_name: &["Szanowny Panie", "Szanowna Pani", "Szanowna Pani/e"],
    },
];

/// The languages whose greetings are offered.
pub const GREETING_LANGUAGES: [&str; 11] = [
    "en", "es", "fr", "de", "it", "no", "nl", "pl", "pt", "pt-br", "sv",
];

/// Names joined as `A`, `A and B`, `A, B, and C`.
fn join_names(names: &[&str], joiner: &str) -> String {
    match names {
        [one] => (*one).to_owned(),
        [a, b] => format!("{a} {joiner} {b}"),
        [rest @ .., last] => format!("{}, {joiner} {last}", rest.join(", ")),
        [] => String::new(),
    }
}

/// Whether `text` ends with a dash and a name (`- Maya`), which nothing completes.
fn signs_off(text: &str) -> bool {
    let name = text.trim_end_matches(|ch: char| ch.is_alphabetic() || is_mark(ch));
    if name.len() == text.len() {
        return false;
    }
    let dash = name.trim_end_matches(char::is_whitespace);
    dash.strip_suffix(['-', '—'])
        .is_some_and(|before| before.chars().last().is_none_or(char::is_whitespace))
}

/// A combining mark (`\p{Mark}`), as far as names need.
fn is_mark(ch: char) -> bool {
    matches!(ch, '\u{300}'..='\u{36f}' | '\u{1ab0}'..='\u{1aff}' | '\u{1dc0}'..='\u{1dff}' | '\u{20d0}'..='\u{20ff}' | '\u{fe20}'..='\u{fe2f}')
}

/// What the editor's own predictions make of the text before the caret.
enum Local {
    /// Nothing yet, and no word completion either.
    Hold,
    /// This follows the caret; a greeting if `.1`.
    Predict(String, bool),
}

/// A marked word.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Annotation {
    block: usize,
    range: Range<usize>,
    /// The text marked: the mark goes once it reads otherwise.
    word: String,
    mark: Mark,
    /// Its dots wait for typing to pause.
    hidden: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Mark {
    /// A correction, from `original`; `near` while its dots show, which is until the caret leaves
    /// it; `capital` for a capitalisation.
    Corrected {
        original: String,
        near: bool,
        capital: bool,
    },
    /// Misspelled, with no correction, or one `undone`.
    Incorrect { undone: bool },
}

/// What the suggestion pill offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestionKind {
    /// `UNDO`: the word as typed, before its correction.
    Undo,
    /// Corrections for a marked misspelling, then `Learn "…"`.
    Multiple,
    /// The correction of the misspelled word being typed.
    Single,
}

/// The suggestion pill as it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestions {
    /// What it offers.
    pub kind: SuggestionKind,
    /// The word it's for.
    pub word: String,
    /// Its items: the original word, or corrections.
    pub items: Vec<String>,
    /// Whether `Learn "{word}"` follows them.
    pub learn: bool,
    /// The item chosen with the keys (`Learn` after the items); none until ↓.
    pub selected: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pill {
    pub(crate) suggestions: Suggestions,
    /// The word's block and characters.
    pub(crate) block: usize,
    pub(crate) range: Range<usize>,
}

/// A word marked by autocorrect, as [`Editor::autocorrect_marks`] lists them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkedWord {
    /// Its block.
    pub block: usize,
    /// Its characters in the block.
    pub range: Range<usize>,
    /// For a correction, the word as typed.
    pub corrected_from: Option<String>,
    /// Whether its dots show.
    pub dotted: bool,
}

/// Whether `ch` is part of a word.
fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '\'' | '’')
}

/// The word ending at `end` in `chars`, without apostrophes around it.
fn word_before(chars: &[char], end: usize) -> Option<Range<usize>> {
    let end = end.min(chars.len());
    let mut start = end;
    while start > 0 && is_word_char(chars[start - 1]) {
        start -= 1;
    }
    let (mut from, mut to) = (start, end);
    while from < to && !chars[from].is_alphanumeric() {
        from += 1;
    }
    while to > from && !chars[to - 1].is_alphanumeric() {
        to -= 1;
    }
    (from < to && chars[from..to].iter().any(|ch| ch.is_alphabetic())).then_some(from..to)
}

/// Whether `range` of `chars` reads `word`, whole: no word characters run on past its ends.
fn reads(chars: &[char], range: &Range<usize>, word: &str) -> bool {
    if range.end > chars.len() || chars[range.clone()].iter().copied().ne(word.chars()) {
        return false;
    }
    let edge = |at: usize| chars[at].is_alphanumeric();
    let before = range.start > 0 && edge(range.start) && is_word_char(chars[range.start - 1]);
    let after = range.end < chars.len() && edge(range.end - 1) && is_word_char(chars[range.end]);
    !before && !after
}

/// Whether `with` is `word` in other case.
fn is_capitalisation(word: &str, with: &str) -> bool {
    with != word && with.to_lowercase() == word.to_lowercase()
}

/// How many characters `a` and `b` have in common at the start and, after that, at the end.
fn common_ends(a: &[char], b: &[char]) -> (usize, usize) {
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    (prefix, suffix)
}

impl Editor {
    /// Turns autocorrect on or off; its marks go either way.
    pub fn set_autocorrect(&mut self, on: bool) {
        if self.autocorrect.on != on {
            self.autocorrect.on = on;
            self.autocorrect.annotations.clear();
            self.autocorrect.pill = None;
            self.autocorrect.looked = None;
            self.autocorrect.punctuation = None;
        }
    }

    /// Whether autocorrect is on.
    pub fn autocorrects(&self) -> bool {
        self.autocorrect.on
    }

    /// Turns autocomplete on or off.
    pub fn set_autocomplete(&mut self, on: bool) {
        if self.autocorrect.complete != on {
            self.autocorrect.complete = on;
            self.ghost = None;
            self.autocorrect.completion = None;
        }
    }

    /// Words autocorrect leaves alone: the names of the people the draft may be to, which a
    /// spellchecker doesn't know.
    pub fn set_names(&mut self, words: impl IntoIterator<Item = String>) {
        self.autocorrect.names = words.into_iter().map(|word| word.to_lowercase()).collect();
    }

    /// The people the draft is from and to, whose names autocomplete offers.
    pub fn set_people(&mut self, people: People) {
        self.autocorrect.people = people;
    }

    /// The languages whose greetings autocomplete offers (of [`GREETING_LANGUAGES`]; English
    /// until set).
    pub fn set_greeting_languages(&mut self, languages: Vec<String>) {
        self.autocorrect.languages = languages;
    }

    /// The words autocorrect has marked.
    pub fn autocorrect_marks(&mut self) -> Vec<MarkedWord> {
        self.sync_marks();
        self.autocorrect
            .annotations
            .iter()
            .map(|annotation| {
                let (corrected_from, dotted) = match &annotation.mark {
                    Mark::Corrected { original, near, .. } => (Some(original.clone()), *near),
                    Mark::Incorrect { .. } => (None, !annotation.hidden),
                };
                MarkedWord {
                    block: annotation.block,
                    range: annotation.range.clone(),
                    corrected_from,
                    dotted,
                }
            })
            .collect()
    }

    /// The suggestion pill, while it shows.
    pub fn suggestions(&self) -> Option<&Suggestions> {
        self.autocorrect.pill.as_ref().map(|pill| &pill.suggestions)
    }

    /// Block `index`'s dotted words: each one's characters and whether it's a correction (grey)
    /// rather than a misspelling (red).
    pub(crate) fn dotted_words(
        &self,
        index: usize,
    ) -> impl Iterator<Item = (Range<usize>, bool)> + '_ {
        self.autocorrect
            .annotations
            .iter()
            .filter(move |annotation| annotation.block == index)
            .filter_map(|annotation| match annotation.mark {
                Mark::Corrected { near: true, .. } => Some((annotation.range.clone(), true)),
                Mark::Corrected { near: false, .. } => None,
                Mark::Incorrect { .. } => {
                    (!annotation.hidden).then(|| (annotation.range.clone(), false))
                }
            })
    }

    /// The pill's word's block and characters, for placing it.
    pub(crate) fn pill_word(&self) -> Option<(usize, Range<usize>)> {
        self.autocorrect
            .pill
            .as_ref()
            .map(|pill| (pill.block, pill.range.clone()))
    }

    /// Moves the marks to where the edits since they were placed left their words, dropping
    /// those whose words changed.
    fn sync_marks(&mut self) {
        if self.autocorrect.revision == Some(self.revision) {
            return;
        }
        self.autocorrect.dropped.clear();
        let texts: Vec<String> = self.doc.blocks.iter().map(|block| block.text()).collect();
        if !self.autocorrect.annotations.is_empty() {
            let old = &self.autocorrect.texts;
            let prefix = old.iter().zip(&texts).take_while(|(a, b)| a == b).count();
            let most = old.len().min(texts.len()) - prefix;
            let suffix = old
                .iter()
                .rev()
                .zip(texts.iter().rev())
                .take(most)
                .take_while(|(a, b)| a == b)
                .count();
            let (old_changed, new_changed) =
                (prefix..old.len() - suffix, prefix..texts.len() - suffix);
            let chars: Vec<Vec<char>> = texts.iter().map(|text| text.chars().collect()).collect();
            let moved = |annotation: &Annotation| -> Option<(usize, Range<usize>)> {
                if annotation.block < old_changed.start {
                    return Some((annotation.block, annotation.range.clone()));
                }
                if annotation.block >= old_changed.end {
                    return Some((
                        annotation.block + texts.len() - old.len(),
                        annotation.range.clone(),
                    ));
                }
                let before: Vec<char> = old[annotation.block].chars().collect();
                // Within the first block changed, before what changed.
                if annotation.block == old_changed.start && !new_changed.is_empty() {
                    let (common, _) = common_ends(&before, &chars[new_changed.start]);
                    if annotation.range.end <= common {
                        return Some((new_changed.start, annotation.range.clone()));
                    }
                }
                // Within the last, after it (and after what the first kept, if it's the same).
                if annotation.block + 1 == old_changed.end && !new_changed.is_empty() {
                    let last = new_changed.end - 1;
                    let after = &chars[last];
                    let suffix = if old_changed.len() == 1 && new_changed.len() == 1 {
                        common_ends(&before, after).1
                    } else {
                        before
                            .iter()
                            .rev()
                            .zip(after.iter().rev())
                            .take_while(|(x, y)| x == y)
                            .count()
                    };
                    if annotation.range.start >= before.len() - suffix {
                        let shift = after.len() as isize - before.len() as isize;
                        let start = (annotation.range.start as isize + shift) as usize;
                        return Some((last, start..start + annotation.range.len()));
                    }
                }
                None
            };
            let annotations = std::mem::take(&mut self.autocorrect.annotations);
            let (mut kept, mut dropped, mut rejected) = (Vec::new(), Vec::new(), Vec::new());
            for annotation in annotations {
                match moved(&annotation).filter(|(block, range)| {
                    chars
                        .get(*block)
                        .is_some_and(|chars| reads(chars, range, &annotation.word))
                }) {
                    Some((block, range)) => kept.push(Annotation {
                        block,
                        range,
                        ..annotation
                    }),
                    None => {
                        dropped.push(new_changed.start.min(texts.len().saturating_sub(1)));
                        if let Mark::Corrected {
                            original,
                            capital: true,
                            ..
                        } = &annotation.mark
                        {
                            rejected.push(original.to_lowercase());
                        }
                    }
                }
            }
            self.autocorrect.annotations = kept;
            self.autocorrect.dropped = dropped;
            self.autocorrect.rejected_capitals.extend(rejected);
            if self
                .autocorrect
                .skip
                .as_ref()
                .is_some_and(|(block, range)| {
                    !self
                        .autocorrect
                        .annotations
                        .iter()
                        .any(|annotation| annotation.block == *block && annotation.range == *range)
                })
            {
                self.autocorrect.skip = None;
            }
        }
        self.autocorrect.texts = texts;
        self.autocorrect.revision = Some(self.revision);
    }

    fn annotate(&mut self, block: usize, range: Range<usize>, mark: Mark) {
        self.sync_marks();
        let word: String = self.doc.blocks[block]
            .text()
            .chars()
            .skip(range.start)
            .take(range.len())
            .collect();
        self.autocorrect.annotations.retain(|annotation| {
            annotation.block != block
                || annotation.range.end <= range.start
                || annotation.range.start >= range.end
        });
        let at = self.autocorrect.annotations.partition_point(|annotation| {
            (annotation.block, annotation.range.start) < (block, range.start)
        });
        self.autocorrect.annotations.insert(
            at,
            Annotation {
                block,
                range,
                word,
                mark,
                hidden: false,
            },
        );
    }

    /// Replaces the word at `range` of block `block` with `with`, one undo step, the caret kept
    /// where it was in the text around it (after the word if it was in it). Returns the new
    /// word's characters.
    fn replace_word(&mut self, block: usize, range: Range<usize>, with: &str) -> Range<usize> {
        self.sync_marks();
        self.undo.save(&self.doc, self.selection);
        let marks = self.doc.blocks[block].marks_at(range.start + 1);
        self.doc
            .delete(Pos::new(block, range.start), Pos::new(block, range.end));
        let end = self
            .doc
            .insert_text(Pos::new(block, range.start), with, marks);
        let len = end.offset - range.start;
        let place = |pos: Pos| match pos {
            pos if pos.block != block || pos.offset < range.start => pos,
            pos if pos.offset >= range.end => Pos::new(block, pos.offset - range.len() + len),
            _ => end,
        };
        self.selection = Selection::new(place(self.selection.anchor), place(self.selection.focus));
        self.changed();
        range.start..range.start + len
    }

    /// The word ending at `end` was finished: corrected, marked, or left be.
    pub(crate) fn finish_word(&mut self, end: Pos) {
        if !self.autocorrect.on || self.spelling.engine.is_none() || self.autocorrect.after_snippet
        {
            return;
        }
        self.sync_marks();
        let Some(block) = self
            .doc
            .blocks
            .get(end.block)
            .filter(|block| block.is_inline() && !block.in_code())
        else {
            return;
        };
        let text = block.text();
        let chars: Vec<char> = text.chars().collect();
        let Some(range) = word_before(&chars, end.offset) else {
            return;
        };
        if self
            .autocorrect
            .annotations
            .iter()
            .any(|annotation| annotation.block == end.block && annotation.range == range)
        {
            return;
        }
        let word: String = chars[range.clone()].iter().collect();
        let key = word.to_lowercase();
        if self.autocorrect.names.contains(&key) {
            return;
        }
        let Some(engine) = self.spelling.engine.as_mut() else {
            return;
        };
        let replacement = engine.replacement(&word).filter(|with| *with != word);
        if replacement.is_none() && !engine.check(&text).contains(&range) {
            return;
        }
        // A capitalisation: the correction, or else macOS's likeliest guess, is the word in other
        // case.
        let correction = replacement.or_else(|| {
            engine
                .correction(&text, range.clone())
                .filter(|with| *with != word)
        });
        let correction = correction.or_else(|| {
            engine
                .guesses(&text, range.clone())
                .into_iter()
                .next()
                .filter(|guess| is_capitalisation(&word, guess))
        });
        let capital = correction
            .as_ref()
            .is_some_and(|with| is_capitalisation(&word, with));
        // One the user undid by editing it, or one already made of the word in its paragraph,
        // isn't made again.
        if capital
            && (self.autocorrect.rejected_capitals.contains(&key)
                || self.capitalised_in(end.block, &key))
        {
            return;
        }
        if !capital && self.autocorrect.undone.contains(&key) {
            self.annotate(end.block, range, Mark::Incorrect { undone: true });
            return;
        }
        match correction {
            Some(with) => {
                let corrected = self.replace_word(end.block, range, &with);
                self.annotate(
                    end.block,
                    corrected,
                    Mark::Corrected {
                        original: word,
                        near: true,
                        capital,
                    },
                );
            }
            None => self.annotate(end.block, range, Mark::Incorrect { undone: false }),
        }
    }

    /// Whether block `block` has had `key` (lower-cased) capitalised.
    fn capitalised_in(&self, block: usize, key: &str) -> bool {
        self.autocorrect.annotations.iter().any(|annotation| {
            annotation.block == block
                && annotation.word.to_lowercase() == key
                && matches!(
                    &annotation.mark,
                    Mark::Corrected { capital: true, .. } | Mark::Incorrect { undone: true }
                )
        })
    }

    /// A snippet was inserted at the caret: the words it ends with aren't corrected as Space or
    /// Enter follow it, until another key is typed.
    pub fn snippet_inserted(&mut self) {
        self.autocorrect.after_snippet = true;
    }

    /// An edit at the caret (a character, Backspace, Delete) inside a word, or onto a marked one:
    /// the word is checked again, a misspelling marked (never corrected), its dots once typing
    /// pauses.
    pub(crate) fn recheck_edited_word(&mut self) {
        if !self.autocorrect.on || self.autocorrect.after_snippet || !self.selection.is_collapsed()
        {
            return;
        }
        self.sync_marks();
        let caret = self.selection.focus;
        let Some(block) = self
            .doc
            .blocks
            .get(caret.block)
            .filter(|block| block.is_inline() && !block.in_code())
        else {
            return;
        };
        let text = block.text();
        let chars: Vec<char> = text.chars().collect();
        let word_at = |at: usize| chars.get(at).copied().is_some_and(is_word_char);
        let inside = caret.offset > 0 && word_at(caret.offset - 1) && word_at(caret.offset);
        let onto_mark = self.autocorrect.dropped.contains(&caret.block)
            && (caret.offset > 0 && word_at(caret.offset - 1) || word_at(caret.offset));
        if !inside && !onto_mark {
            return;
        }
        let end = (caret.offset..chars.len())
            .find(|at| !word_at(*at))
            .unwrap_or(chars.len());
        let Some(range) = word_before(&chars, end).filter(|range| range.start <= caret.offset)
        else {
            return;
        };
        let word: String = chars[range.clone()].iter().collect();
        let key = word.to_lowercase();
        if self.autocorrect.names.contains(&key)
            || self
                .autocorrect
                .annotations
                .iter()
                .any(|annotation| annotation.block == caret.block && annotation.range == range)
        {
            return;
        }
        let Some(engine) = self.spelling.engine.as_mut() else {
            return;
        };
        if engine.check(&text).contains(&range) {
            self.annotate(
                caret.block,
                range.clone(),
                Mark::Incorrect { undone: false },
            );
            if let Some(annotation) = self
                .autocorrect
                .annotations
                .iter_mut()
                .find(|annotation| annotation.block == caret.block && annotation.range == range)
            {
                annotation.hidden = true;
            }
        }
    }

    /// A character was typed at the caret: `-` finishes the word before it, punctuation does
    /// once typing pauses; autocomplete follows.
    pub(crate) fn typed_for_assistance(&mut self, ch: char) {
        if ch != ' ' {
            self.autocorrect.greeted = false;
        }
        let caret = self.selection.focus;
        if self.autocorrect.on && caret.offset > 0 {
            if ch == '-' {
                self.finish_word(Pos::new(caret.block, caret.offset - 1));
            } else if PUNCTUATION.contains(&ch) {
                self.autocorrect.punctuation = Some((self.revision, self.selection.focus));
            }
        }
        // The key after a snippet that isn't Space or Enter.
        if ch != ' ' {
            self.autocorrect.after_snippet = false;
        }
        if is_word_char(ch) {
            self.recheck_edited_word();
        }
        self.complete();
    }

    /// Backspace at the caret: an edited word is checked again, autocomplete follows.
    pub(crate) fn deleted_for_assistance(&mut self) {
        self.recheck_edited_word();
        self.complete();
    }

    /// Ghost text at the end of the caret's line, after a character or a Backspace: while what's
    /// typed is still the start of what shows, it stays (typing its next letter takes it, Backspace
    /// gives it back); else the editor's own prediction, a name or a greeting; else the rest of the
    /// first of the spellchecker's completions for the word being typed.
    pub(crate) fn complete(&mut self) {
        self.ghost = None;
        let shown = self.autocorrect.completion.take();
        if !self.autocorrect.complete || !self.selection.is_collapsed() || self.trigger.is_some() {
            return;
        }
        let caret = self.selection.focus;
        let block = &self.doc.blocks[caret.block];
        // Only at the end of a paragraph.
        if !block.is_inline() || caret.offset != block.len() {
            return;
        }
        let text = block.text();
        let chars: Vec<char> = text.chars().collect();
        let before: String = chars[..caret.offset].iter().collect();
        if signs_off(&before) {
            return;
        }
        if let Some(shown) = shown.filter(|shown| shown.start < caret.offset) {
            let typed: String = chars[shown.start..caret.offset].iter().collect();
            if let Some(rest) = shown
                .full
                .strip_prefix(&typed)
                .filter(|rest| !rest.is_empty())
            {
                self.ghost = Some(rest.to_owned());
                self.autocorrect.completion = Some(shown);
                return;
            }
        }
        match self.local_prediction(caret.block == 0, &before) {
            Some(Local::Hold) => return,
            Some(Local::Predict(rest, greeting)) => {
                self.autocorrect.completion = Some(Completion {
                    start: caret.offset,
                    full: rest.clone(),
                    greeting,
                });
                self.ghost = Some(rest);
                return;
            }
            None => {}
        }
        let Some(range) = word_before(&chars, caret.offset)
            .filter(|range| range.end == caret.offset && range.len() >= COMPLETE_AFTER)
        else {
            return;
        };
        let partial: String = chars[range.clone()].iter().collect();
        let Some(engine) = self.spelling.engine.as_mut() else {
            return;
        };
        let lower = partial.to_lowercase();
        let found = engine
            .completions(&text, range.clone())
            .into_iter()
            .find(|word| {
                word.to_lowercase().starts_with(&lower)
                    && word.chars().count() > partial.chars().count()
            });
        if let Some(word) = found {
            let rest: String = word.chars().skip(partial.chars().count()).collect();
            // No punctuation at its end.
            let rest = rest
                .trim_end_matches(|ch: char| !ch.is_alphabetic())
                .to_owned();
            if !rest.is_empty() {
                self.autocorrect.completion = Some(Completion {
                    start: range.start,
                    full: format!("{partial}{rest}"),
                    greeting: false,
                });
                self.ghost = Some(rest);
            }
        }
    }

    /// The editor's own prediction for `before`, the caret's line up to it: a name, then on the
    /// body's `first_line` a greeting.
    fn local_prediction(&self, first_line: bool, before: &str) -> Option<Local> {
        self.name_prediction(before).or_else(|| {
            first_line
                .then(|| self.greeting_prediction(before))
                .flatten()
        })
    }

    /// The word being typed, capitalised, as the start of a name of the draft's people, the first
    /// of them, their last names and whole names in turn; with one letter typed, nothing yet.
    fn name_prediction(&self, before: &str) -> Option<Local> {
        if self.autocorrect.greeted {
            return None;
        }
        // The last word, with any punctuation it has.
        let word = before
            .rsplit(char::is_whitespace)
            .next()
            .unwrap_or_default();
        let token = word.trim_matches(|ch: char| !ch.is_alphanumeric());
        if token
            .chars()
            .next()
            .is_none_or(|first| first.is_lowercase())
        {
            return None;
        }
        let people = &self.autocorrect.people;
        let named = people.from.iter().chain(
            people
                .to
                .iter()
                .chain(&people.cc)
                .chain(&people.bcc)
                .flatten(),
        );
        let mut names = named
            .flat_map(|person| [&person.first, &person.last, &person.full])
            .map(|name| name.trim())
            .filter(|name| name.starts_with(word) && *name != word);
        let first = names.next()?;
        Some(if token.chars().count() < 2 {
            Local::Hold
        } else {
            Local::Predict(first[word.len()..].to_owned(), false)
        })
    }

    /// A greeting to the one or two people the draft is to, all named, by first name (with one,
    /// also with those it copies), or to one by last name, once its first word is typed; until
    /// then, nothing.
    fn greeting_prediction(&self, before: &str) -> Option<Local> {
        let people = &self.autocorrect.people;
        let to: Vec<&Named> = people
            .to
            .iter()
            .map(Option::as_ref)
            .collect::<Option<_>>()?;
        let cc_named = people.cc.iter().all(Option::is_some);
        let firsts: Vec<&str> = to.iter().map(|person| person.first.as_str()).collect();
        let with_cc: Vec<&str> = firsts
            .iter()
            .copied()
            .chain(
                people
                    .cc
                    .iter()
                    .flatten()
                    .map(|person| person.first.as_str()),
            )
            .collect();
        let english = ["en".to_owned()];
        let languages = if self.autocorrect.languages.is_empty() {
            &english[..]
        } else {
            &self.autocorrect.languages[..]
        };
        let mut candidates = Vec::new();
        for greetings in GREETINGS.iter().filter(|greetings| {
            languages
                .iter()
                .any(|language| language == greetings.language)
        }) {
            for greeting in greetings.first_name {
                if matches!(to.len(), 1 | 2) {
                    candidates.push(format!(
                        "{greeting} {}",
                        join_names(&firsts, greetings.joiner)
                    ));
                }
                if to.len() == 1 && cc_named && !people.cc.is_empty() {
                    candidates.push(format!(
                        "{greeting} {}",
                        join_names(&with_cc, greetings.joiner)
                    ));
                }
            }
            if let [one] = to[..] {
                candidates.extend(
                    greetings
                        .last_name
                        .iter()
                        .map(|greeting| format!("{greeting} {}", one.last)),
                );
            }
        }
        candidates.retain(|candidate| candidate.starts_with(before) && candidate != before);
        // Its first word typed whole.
        let typed = candidates.iter().find(|candidate| {
            before.starts_with(
                candidate
                    .split(|ch: char| !ch.is_alphabetic() && !is_mark(ch))
                    .next()
                    .unwrap_or_default(),
            )
        });
        match typed {
            Some(greeting) => Some(Local::Predict(greeting[before.len()..].to_owned(), true)),
            None => (!candidates.is_empty()).then_some(Local::Hold),
        }
    }

    /// Ghost text was taken: a greeting's keeps names from being offered until something but a
    /// space or Backspace is typed.
    pub(crate) fn took_completion(&mut self) {
        self.autocorrect.greeted = self
            .autocorrect
            .completion
            .take()
            .is_some_and(|completion| completion.greeting);
    }

    /// The marks and the pill this frame, `idle` seconds since the caret or the document last
    /// changed, the editor having the keyboard if `focused`. Returns how long until something
    /// is due, if anything is.
    pub(crate) fn assist(&mut self, focused: bool, idle: f64) -> Option<f64> {
        if !self.autocorrect.on {
            return None;
        }
        self.sync_marks();
        let caret = self.selection.focus;
        for annotation in &mut self.autocorrect.annotations {
            if let Mark::Corrected { near, .. } = &mut annotation.mark
                && (annotation.block != caret.block
                    || caret.offset < annotation.range.start
                    || caret.offset > annotation.range.end + NEAR)
            {
                *near = false;
            }
        }
        if self
            .autocorrect
            .skip
            .as_ref()
            .is_some_and(|(block, range)| {
                *block != caret.block || caret.offset < range.start || caret.offset > range.end
            })
        {
            self.autocorrect.skip = None;
        }
        // A re-checked word's dots show once typing has paused.
        let hidden = self
            .autocorrect
            .annotations
            .iter()
            .any(|annotation| annotation.hidden);
        if hidden && idle >= REVEAL {
            self.autocorrect
                .annotations
                .iter_mut()
                .for_each(|annotation| annotation.hidden = false);
        }
        let reveal = (hidden && idle < REVEAL).then_some(REVEAL - idle);
        // The pill goes with any change or caret move, and is looked for again once the caret
        // rests.
        let now = (self.revision, self.selection);
        if self.autocorrect.looked != Some(now) {
            self.autocorrect.pill = None;
        }
        if !focused {
            self.autocorrect.pill = None;
            return reveal;
        }
        let wait = DEBOUNCE - idle;
        if let Some((revision, at)) = self.autocorrect.punctuation
            && (revision != self.revision || at != caret)
        {
            self.autocorrect.punctuation = None;
        }
        if wait > 0.0 {
            let due = (self.autocorrect.punctuation.is_some()
                || self.autocorrect.looked != Some(now))
            .then_some(wait);
            return due.into_iter().chain(reveal).reduce(f64::min);
        }
        if self.autocorrect.punctuation.take().is_some() {
            let chars: Vec<char> = self.doc.blocks[caret.block].text().chars().collect();
            let mut end = caret.offset;
            while end > 0 && PUNCTUATION.contains(&chars[end - 1]) {
                end -= 1;
            }
            self.finish_word(Pos::new(caret.block, end));
        }
        let now = (self.revision, self.selection);
        if self.autocorrect.looked != Some(now) {
            self.autocorrect.looked = Some(now);
            self.autocorrect.pill = self.find_pill();
            if self.autocorrect.pill.is_some() {
                self.ghost = None;
            }
        }
        reveal
    }

    /// What the pill offers for the caret where it is.
    fn find_pill(&mut self) -> Option<Pill> {
        let caret = self.selection.focus;
        if !self.selection.is_collapsed() || self.trigger.is_some() {
            return None;
        }
        let on = |range: &Range<usize>| range.start <= caret.offset && caret.offset <= range.end;
        let marked = self
            .autocorrect
            .annotations
            .iter()
            .find(|annotation| annotation.block == caret.block && on(&annotation.range))
            .cloned();
        let text = self.doc.blocks[caret.block].text();
        if let Some(annotation) = marked {
            if self.autocorrect.skip.as_ref() == Some(&(annotation.block, annotation.range.clone()))
            {
                return None;
            }
            let (kind, items, learn) = match &annotation.mark {
                Mark::Corrected {
                    original,
                    near: true,
                    ..
                } => (SuggestionKind::Undo, vec![original.clone()], false),
                Mark::Corrected { near: false, .. } => return None,
                Mark::Incorrect { .. } => {
                    let guesses = self
                        .spelling
                        .engine
                        .as_mut()?
                        .guesses(&text, annotation.range.clone());
                    let items: Vec<String> = guesses
                        .into_iter()
                        .filter(|guess| *guess != annotation.word)
                        .take(SUGGESTIONS)
                        .collect();
                    (SuggestionKind::Multiple, items, true)
                }
            };
            return Some(Pill {
                suggestions: Suggestions {
                    kind,
                    word: annotation.word.clone(),
                    items,
                    learn,
                    selected: None,
                },
                block: annotation.block,
                range: annotation.range,
            });
        }
        // The misspelled word just typed, its correction.
        if !self.spelling.typing {
            return None;
        }
        let chars: Vec<char> = text.chars().collect();
        let range = word_before(&chars, caret.offset).filter(|range| {
            range.end == caret.offset && chars.get(caret.offset).is_none_or(|ch| !is_word_char(*ch))
        })?;
        let word: String = chars[range.clone()].iter().collect();
        if self.autocorrect.names.contains(&word.to_lowercase())
            || self.autocorrect.undone.contains(&word.to_lowercase())
        {
            return None;
        }
        let engine = self.spelling.engine.as_mut()?;
        if !engine.check(&text).contains(&range) {
            return None;
        }
        let correction = engine
            .correction(&text, range.clone())
            .filter(|with| *with != word)?;
        Some(Pill {
            suggestions: Suggestions {
                kind: SuggestionKind::Single,
                word,
                items: vec![correction],
                learn: false,
                selected: None,
            },
            block: caret.block,
            range,
        })
    }

    /// A key while the pill shows: whether it took it. ↓ starts choosing, ←/→ go round the items,
    /// Enter or Tab takes the chosen one, ↑ puts the pill away; Escape does too unless it offers
    /// only `Learn`.
    pub(crate) fn pill_key(&mut self, key: Key) -> bool {
        let Some(pill) = &mut self.autocorrect.pill else {
            return false;
        };
        let count = pill.suggestions.items.len() + usize::from(pill.suggestions.learn);
        let selected = pill.suggestions.selected;
        match (key, selected) {
            (Key::ArrowDown, None) => pill.suggestions.selected = Some(0),
            (Key::ArrowDown, Some(_)) => {}
            (Key::ArrowUp, Some(_)) => self.dismiss_pill(),
            (Key::ArrowLeft, Some(at)) => {
                pill.suggestions.selected = Some((at + count - 1) % count)
            }
            (Key::ArrowRight, Some(at)) => pill.suggestions.selected = Some((at + 1) % count),
            (Key::Enter | Key::Tab, Some(at)) => self.pick_suggestion(at),
            (Key::Escape, _) if !pill.suggestions.items.is_empty() => self.dismiss_pill(),
            _ => return false,
        }
        true
    }

    /// Whether the pill takes Tab now (an item is chosen).
    pub(crate) fn pill_wants_tab(&self) -> bool {
        self.autocorrect
            .pill
            .as_ref()
            .is_some_and(|pill| pill.suggestions.selected.is_some())
    }

    /// Whether the pill takes Escape now.
    pub(crate) fn pill_wants_escape(&self) -> bool {
        self.autocorrect
            .pill
            .as_ref()
            .is_some_and(|pill| !pill.suggestions.items.is_empty())
    }

    /// Hovering item `index` with the mouse moving chooses it.
    pub(crate) fn hover_suggestion(&mut self, index: usize) {
        if let Some(pill) = &mut self.autocorrect.pill {
            pill.suggestions.selected = Some(index);
        }
    }

    /// Puts the pill away: a word being typed is marked as a correction turned down; a marked
    /// word's pill doesn't come back until the caret leaves it.
    fn dismiss_pill(&mut self) {
        let Some(pill) = self.autocorrect.pill.take() else {
            return;
        };
        if pill.suggestions.kind == SuggestionKind::Single {
            self.autocorrect
                .undone
                .insert(pill.suggestions.word.to_lowercase());
            self.annotate(
                pill.block,
                pill.range.clone(),
                Mark::Incorrect { undone: true },
            );
        }
        self.autocorrect.skip = Some((pill.block, pill.range));
    }

    /// Takes item `index` of the pill: the original word back, marked as misspelled; a correction,
    /// marked as one; or `Learn`, which adds the word to the user's dictionary and unmarks it
    /// everywhere.
    pub(crate) fn pick_suggestion(&mut self, index: usize) {
        let Some(pill) = self.autocorrect.pill.take() else {
            return;
        };
        self.autocorrect.looked = None;
        let Pill {
            suggestions,
            block,
            range,
        } = pill;
        if self.doc.blocks.get(block).is_none_or(|text| {
            text.text()
                .chars()
                .skip(range.start)
                .take(range.len())
                .ne(suggestions.word.chars())
        }) {
            return;
        }
        let Some(item) = suggestions.items.get(index) else {
            if suggestions.learn {
                self.sync_marks();
                // `[data-incorrect="{word}"]`: the word wherever it's marked or was corrected.
                self.autocorrect
                    .annotations
                    .retain(|annotation| match &annotation.mark {
                        Mark::Corrected { original, .. } => *original != suggestions.word,
                        Mark::Incorrect { .. } => annotation.word != suggestions.word,
                    });
                self.learn_spelling(&suggestions.word);
                self.autocorrect.learned = Some(suggestions.word);
            }
            return;
        };
        let word = self.replace_word(block, range, item);
        let after = Pos::new(block, word.end);
        self.selection = Selection::caret(after);
        match suggestions.kind {
            SuggestionKind::Undo => {
                self.autocorrect.undone.insert(item.to_lowercase());
                self.annotate(block, word, Mark::Incorrect { undone: true });
            }
            SuggestionKind::Multiple | SuggestionKind::Single => {
                let capital = is_capitalisation(&suggestions.word, item);
                self.annotate(
                    block,
                    word,
                    Mark::Corrected {
                        original: suggestions.word,
                        near: true,
                        capital,
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    #[test]
    fn the_word_before_leaves_out_apostrophes_and_numbers() {
        assert_eq!(word_before(&chars("I saw teh"), 9), Some(6..9));
        assert_eq!(word_before(&chars("don't "), 5), Some(0..5));
        assert_eq!(word_before(&chars("'quoted' "), 8), Some(1..7));
        assert_eq!(
            word_before(&chars("at 10"), 5),
            None,
            "numbers aren't words"
        );
        assert_eq!(word_before(&chars("a, "), 3), None);
    }

    #[test]
    fn a_word_reads_whole_or_not_at_all() {
        let text = chars("the cat, them");
        assert!(reads(&text, &(0..3), "the"));
        assert!(reads(&text, &(4..7), "cat"), "punctuation after it");
        assert!(!reads(&text, &(9..12), "the"), "a word runs on");
    }
}
