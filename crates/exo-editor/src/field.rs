//! A one-line text field spellchecked as Chromium checks an `<input>` on a Mac: the compose
//! subject, a snippet's name, the Out of Office subject, an event's title. These are spellchecked
//! whether or not autocorrect is on.
//!
//! As Chromium's idle-time checker does (its hot and cold modes): a field is checked
//! while it has the keyboard, the whole of it once it has changed since it was last checked, or a
//! second after typing in a word, which shows that word's mark only then; typing a space or
//! pasting checks it at once. The marks go when the field loses the keyboard, and come back with
//! the next edit. A right-click selects the word under it, as on a Mac ([`exo_ui::text_field`]),
//! and the app's editable menu ([`FieldSpelling::take_menu`]) has the word's spelling.

use std::ops::Range;

use egui::{
    Color32, Event, Id, Key, Rect, Response, Ui,
    text::{CCursor, CCursorRange},
    text_edit::TextEditState,
};
use exo_ui::TextStyle;

use crate::{ContextMenu, Misspelling, Spelling};

/// How long after typing in a word the field is checked whole (`kColdModeTimerInterval`).
const COLD: f64 = 1.0;

/// A field's spellchecker and the marks it shows.
#[derive(Default)]
pub struct FieldSpelling {
    engine: Option<Box<dyn Spelling>>,
    /// The text as last checked: focusing the field again checks it only once it has changed.
    checked: Option<String>,
    /// The misspellings marked, as character ranges, while the field has the keyboard.
    marks: Vec<Range<usize>>,
    /// The text last frame, to see edits by.
    last: Option<String>,
    /// It had the keyboard last frame.
    had_focus: bool,
    /// When the field is next checked whole, after typing in a word.
    due: Option<f64>,
    /// The field's widget, for a correction's caret.
    id: Option<Id>,
    menu: Option<ContextMenu>,
}

impl std::fmt::Debug for FieldSpelling {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FieldSpelling")
            .field("checks", &self.engine.is_some())
            .field("marks", &self.marks)
            .finish_non_exhaustive()
    }
}

/// Whether `ch` is part of a word.
fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '\'' | '’')
}

impl FieldSpelling {
    /// Checks with `engine` from now on.
    pub fn set_engine(&mut self, engine: Box<dyn Spelling>) {
        *self = Self {
            engine: Some(engine),
            ..Self::default()
        };
    }

    /// Whether it has a spellchecker.
    pub fn has_engine(&self) -> bool {
        self.engine.is_some()
    }

    /// The misspellings marked now, as character ranges.
    pub fn marks(&self) -> &[Range<usize>] {
        &self.marks
    }

    /// A right-click since this was last asked, for the app's menu.
    pub fn take_menu(&mut self) -> Option<ContextMenu> {
        self.menu.take()
    }

    /// [`exo_ui::text_field`] `id` editing `text` on `line`, checked and marked.
    #[expect(
        clippy::too_many_arguments,
        reason = "the field's own arguments, as `text_field` takes them"
    )]
    pub fn show(
        &mut self,
        ui: &mut Ui,
        id: Id,
        text: &mut String,
        line: Rect,
        style: &TextStyle,
        color: Color32,
        selection: Color32,
    ) -> Response {
        let ctx = ui.ctx().clone();
        self.id = Some(id);
        let (typed, pasted) = ui.input(|input| {
            let typed = input.events.iter().any(|event| {
                matches!(
                    event,
                    Event::Text(_)
                        | Event::Key {
                            key: Key::Backspace | Key::Delete,
                            pressed: true,
                            ..
                        }
                )
            });
            (
                typed,
                input
                    .events
                    .iter()
                    .any(|event| matches!(event, Event::Paste(_) | Event::Cut)),
            )
        });
        let response = exo_ui::text_field(ui, id, text, line, style, color, selection);
        if let Some(menu) = exo_ui::take_field_menu(&ctx, Some(id)) {
            self.right_click(text, menu);
        }
        let now = ui.input(|input| input.time);
        let gained = !std::mem::replace(&mut self.had_focus, response.has_focus());
        if !response.has_focus() {
            self.marks.clear();
            self.due = None;
            self.last = Some(text.clone());
            return response;
        }
        let edited = self.last.as_ref().is_some_and(|last| last != text);
        if edited {
            self.follow_edit(self.last.clone().unwrap_or_default().as_str(), text);
        }
        let caret = TextEditState::load(&ctx, id)
            .and_then(|state| state.cursor.char_range())
            .filter(|range| range.primary == range.secondary)
            .map(|range| range.primary.index.0);
        let chars: Vec<char> = text.chars().collect();
        let in_word =
            caret.is_some_and(|at| at > 0 && chars.get(at - 1).copied().is_some_and(is_word_char));
        if edited && typed && !pasted && in_word {
            // `CurrentWordIfTypingInPartialWord`: the word being typed isn't marked yet.
            let caret = caret.unwrap_or_default();
            self.marks
                .retain(|mark| !(mark.start <= caret && caret <= mark.end));
            self.due = Some(now + COLD);
        } else if edited
            || (gained && self.checked.as_deref() != Some(text.as_str()))
            || self.due.is_some_and(|due| due <= now)
        {
            self.check(text);
        }
        if let Some(due) = self.due {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64((due - now).max(0.0)));
        }
        self.last = Some(text.clone());
        let top = line.top() + exo_ui::misspelling_top(style, &ctx);
        let painter = ui
            .painter()
            .with_clip_rect(ui.clip_rect().intersect(Rect::from_x_y_ranges(
                line.x_range(),
                line.top()..=line.bottom() + 4.0,
            )));
        for mark in &self.marks {
            let x = |index| exo_ui::field_x(&ctx, text, line, style, index);
            exo_ui::paint_misspelling(&painter, x(mark.start)..x(mark.end), top, line.left());
        }
        response
    }

    /// Checks all of `text` now.
    fn check(&mut self, text: &str) {
        let Some(engine) = self.engine.as_mut() else {
            return;
        };
        self.marks = engine.check(text);
        self.checked = Some(text.to_owned());
        self.due = None;
    }

    /// Moves the marks over an edit from `before` to `after`, dropping those it touched.
    fn follow_edit(&mut self, before: &str, after: &str) {
        let (old, new): (Vec<char>, Vec<char>) =
            (before.chars().collect(), after.chars().collect());
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(new[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let (changed_end, shift) = (old.len() - suffix, new.len() as isize - old.len() as isize);
        self.marks = std::mem::take(&mut self.marks)
            .into_iter()
            .filter_map(|mark| match mark {
                mark if mark.end < prefix => Some(mark),
                mark if mark.start > changed_end => Some(
                    (mark.start as isize + shift) as usize..(mark.end as isize + shift) as usize,
                ),
                _ => None,
            })
            .collect();
    }

    /// The menu for a right-click the field reported, with the misspelled word it selected.
    fn right_click(&mut self, text: &str, menu: exo_ui::FieldMenu) {
        let misspelling = self
            .marks
            .iter()
            .find(|mark| **mark == menu.selection)
            .cloned()
            .and_then(|range| {
                let word: String = text.chars().skip(range.start).take(range.len()).collect();
                let guesses = self.engine.as_mut()?.guesses(text, range.clone());
                Some(Misspelling {
                    block: 0,
                    range,
                    word,
                    guesses,
                })
            });
        self.menu = Some(ContextMenu {
            at: menu.at,
            link: None,
            misspelling,
            selection: menu.selected,
            selection_baseline: menu.baseline,
            text_size: menu.text_size,
        });
    }

    /// Replaces `misspelling` in `text` with `correction`, the caret after it, checked again at
    /// once. Does nothing if the word has changed since.
    pub fn correct(
        &mut self,
        ctx: &egui::Context,
        text: &mut String,
        misspelling: &Misspelling,
        correction: &str,
    ) {
        let chars: Vec<char> = text.chars().collect();
        if chars
            .get(misspelling.range.clone())
            .is_none_or(|word| word.iter().copied().ne(misspelling.word.chars()))
        {
            return;
        }
        *text = chars[..misspelling.range.start]
            .iter()
            .copied()
            .chain(correction.chars())
            .chain(chars[misspelling.range.end..].iter().copied())
            .collect();
        if let Some(id) = self.id {
            let end = CCursor::new(misspelling.range.start + correction.chars().count());
            set_selection(ctx, id, CCursorRange::one(end));
        }
        self.check(text);
        self.last = Some(text.clone());
    }

    /// Adds `word` to the user's dictionary (`Add to Dictionary`), checked again at once.
    pub fn learn(&mut self, word: &str, text: &str) {
        if let Some(engine) = self.engine.as_mut() {
            engine.learn(word);
            self.check(text);
        }
    }

    /// Stops marking `word` in this field (`Ignore Spelling`), checked again at once.
    pub fn ignore(&mut self, word: &str, text: &str) {
        if let Some(engine) = self.engine.as_mut() {
            engine.ignore(word);
            self.check(text);
        }
    }
}

fn set_selection(ctx: &egui::Context, id: Id, range: CCursorRange) {
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(range));
    state.store(ctx, id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_follow_an_edit_and_go_where_it_touched() {
        let mut spelling = FieldSpelling {
            marks: vec![2..5, 10..13],
            ..FieldSpelling::default()
        };
        spelling.follow_edit("I teh cat and dgo", "Oh I teh cat and dgo");
        assert_eq!(spelling.marks, [5..8, 13..16], "both after the insertion");
        spelling.follow_edit("Oh I teh cat and dgo", "Oh I tehx cat and dgo");
        assert_eq!(
            spelling.marks,
            std::slice::from_ref(&(14..17)),
            "the one typed onto goes"
        );
    }
}
