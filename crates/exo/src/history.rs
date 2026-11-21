//! Back and forward through where the user has been: ⌘[ or ⌘← goes back, ⌘] or ⌘→ forward, and
//! Backspace in the list goes back. The history has a place for each list shown and each
//! conversation opened, up to 50; going back to one shows that list, with that conversation open
//! or none. Escape from a conversation goes back to the list it was opened from rather than
//! adding the list again, so forward opens the conversation again.
//!
//! The keys still reach the rest of the app, and do nothing while the keyboard is in a text field
//! (the editor, a recipient field, the search bar, the palette) or a dialog. Search and the
//! calendar's week view aren't places in it, and the keys do nothing while either is up.

use egui::{Key, Modifiers};
use exo_store::ListId;

/// As many places as the history keeps.
const MAX_PLACES: usize = 50;

/// Where the user is: a list, and the conversation open in it if one is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Place {
    pub list: ListId,
    pub thread: Option<String>,
}

/// The places visited, oldest first, and which one is on screen.
#[derive(Debug, Default)]
pub(crate) struct History {
    places: Vec<Place>,
    current: usize,
}

impl History {
    /// `place` is on screen. A new one goes after the current place, dropping those ahead of it,
    /// except a conversation's list after it, which goes back to where the conversation was
    /// opened (and the conversations after it, from J and K).
    pub(crate) fn visit(&mut self, place: Place) {
        if self.places.get(self.current) == Some(&place) {
            return;
        }
        if place.thread.is_none() && !self.places.is_empty() {
            let opened_from = self.places[..=self.current]
                .iter()
                .rposition(|visited| visited.list != place.list || visited.thread.is_none());
            if let Some(index) =
                opened_from.filter(|index| *index < self.current && self.places[*index] == place)
            {
                self.current = index;
                return;
            }
        }
        self.places.truncate(self.current + 1);
        self.places.push(place);
        if self.places.len() > MAX_PLACES {
            self.places.remove(0);
        }
        self.current = self.places.len() - 1;
    }

    /// The place before the current one, now current.
    pub(crate) fn back(&mut self) -> Option<Place> {
        self.current = self.current.checked_sub(1)?;
        self.places.get(self.current).cloned()
    }

    /// The place after the current one, now current.
    pub(crate) fn forward(&mut self) -> Option<Place> {
        let next = self.places.get(self.current + 1).cloned()?;
        self.current += 1;
        Some(next)
    }
}

/// Which way a key goes: ⌘[ and ⌘← back, ⌘] and ⌘→ forward, with ⌘ alone.
fn direction(event: &egui::Event) -> Option<isize> {
    let egui::Event::Key {
        key,
        pressed: true,
        modifiers,
        ..
    } = event
    else {
        return None;
    };
    if !modifiers.matches_exact(Modifiers::COMMAND) {
        return None;
    }
    match key {
        Key::OpenBracket | Key::ArrowLeft => Some(-1),
        Key::CloseBracket | Key::ArrowRight => Some(1),
        _ => None,
    }
}

impl crate::Mail {
    /// Where the mail is, if it's somewhere history keeps.
    fn place(&self) -> Option<Place> {
        if self.search.is_some() || self.calendar_week().is_some() {
            return None;
        }
        Some(Place {
            list: self.list.clone(),
            thread: self.thread.as_ref().map(|thread| thread.id().to_owned()),
        })
    }

    /// Goes back or forward for this frame's ⌘[, ⌘], ⌘← and ⌘→, leaving the keys for what has
    /// the keyboard. Call before the commands take the frame's keys.
    pub(crate) fn history_keys(&mut self, ctx: &egui::Context) {
        let region = self.commands.region();
        if region.is_text_input() || region.is_overlay() || self.place().is_none() {
            return;
        }
        let steps: Vec<isize> =
            ctx.input(|input| input.events.iter().filter_map(direction).collect());
        for step in steps {
            let place = if step < 0 {
                self.history.back()
            } else {
                self.history.forward()
            };
            if let Some(place) = place {
                self.go_to(place);
            }
        }
    }

    /// Backspace in the list: back, as ⌘[ goes.
    pub(crate) fn history_back(&mut self) {
        if let Some(place) = self.history.back() {
            self.go_to(place);
        }
    }

    /// Notes where the mail is. Call once the frame has gone wherever it goes.
    pub(crate) fn history_visit(&mut self) {
        if let Some(place) = self.place() {
            self.history.visit(place);
        }
    }

    /// Shows `place`: its list, with its conversation open or none.
    pub(crate) fn go_to(&mut self, place: Place) {
        if place.list != self.list {
            self.close_thread();
            self.show(place.list);
        }
        match place.thread {
            Some(id) if self.thread.as_ref().is_none_or(|open| open.id() != id) => {
                self.open_thread(id)
            }
            Some(_) => {}
            None => self.close_thread(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(list: ListId) -> Place {
        Place { list, thread: None }
    }

    fn thread(list: ListId, id: &str) -> Place {
        Place {
            list,
            thread: Some(id.to_owned()),
        }
    }

    #[test]
    fn back_and_forward_walk_the_places_visited() {
        let mut history = History::default();
        history.visit(list(ListId::Important));
        history.visit(list(ListId::Done));
        history.visit(list(ListId::Starred));
        assert_eq!(history.back(), Some(list(ListId::Done)));
        assert_eq!(history.back(), Some(list(ListId::Important)));
        assert_eq!(history.back(), None, "nothing before the first");
        assert_eq!(history.forward(), Some(list(ListId::Done)));
        // Somewhere new from there drops what was ahead.
        history.visit(list(ListId::Sent));
        assert_eq!(history.forward(), None);
        assert_eq!(history.back(), Some(list(ListId::Done)));
    }

    #[test]
    fn leaving_a_conversation_goes_back_to_where_it_was_opened() {
        let mut history = History::default();
        history.visit(list(ListId::Important));
        history.visit(thread(ListId::Important, "a"));
        // J: the next conversation.
        history.visit(thread(ListId::Important, "b"));
        // Escape.
        history.visit(list(ListId::Important));
        assert_eq!(history.forward(), Some(thread(ListId::Important, "a")));
        assert_eq!(history.forward(), Some(thread(ListId::Important, "b")));
        assert_eq!(history.back(), Some(thread(ListId::Important, "a")));
        assert_eq!(history.back(), Some(list(ListId::Important)));
        // Going to the list again from elsewhere is somewhere new.
        let mut history = History::default();
        history.visit(list(ListId::Important));
        history.visit(list(ListId::Done));
        history.visit(list(ListId::Important));
        assert_eq!(history.back(), Some(list(ListId::Done)));
    }

    #[test]
    fn fifty_places_at_most() {
        let mut history = History::default();
        for n in 0..60 {
            history.visit(thread(ListId::Important, &n.to_string()));
        }
        let mut back = 0;
        while history.back().is_some() {
            back += 1;
        }
        assert_eq!(back, MAX_PLACES - 1);
    }

    #[test]
    fn only_command_and_a_bracket_or_arrow_go() {
        let press = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let command = Modifiers {
            mac_cmd: true,
            command: true,
            ..Modifiers::NONE
        };
        assert_eq!(direction(&press(Key::OpenBracket, command)), Some(-1));
        assert_eq!(direction(&press(Key::ArrowLeft, command)), Some(-1));
        assert_eq!(direction(&press(Key::CloseBracket, command)), Some(1));
        assert_eq!(direction(&press(Key::ArrowRight, command)), Some(1));
        assert_eq!(
            direction(&press(Key::OpenBracket, command | Modifiers::SHIFT)),
            None,
            "⌘⇧[ is the previous tab"
        );
        assert_eq!(direction(&press(Key::ArrowLeft, Modifiers::NONE)), None);
    }
}
