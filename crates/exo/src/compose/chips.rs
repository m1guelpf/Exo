//! Recipient chips beyond typing them: editing one in place, selecting several, removing, copying
//! and moving them between To, Cc and Bcc, and their undo and redo.
//!
//! A list's selection is [`Input::selected`] and [`Input::anchor`], the edge a ⇧← or ⇧→ moves;
//! nothing selected is the caret in the input at the end. Editing a chip puts the input in its
//! place ([`Input::editing`]), its text the chip's `Name <address>`, all selected; finishing puts
//! it back as a chip, or removes it if it's emptied. Chips have no menu or hover card.
//!
//! [`Input::selected`]: super::draft::Input::selected
//! [`Input::anchor`]: super::draft::Input::anchor
//! [`Input::editing`]: super::draft::Input::editing

use exo_commands::{Command, Commands, Region};
use exo_store::Address;

use super::{Action, Open, contacts, draft::List, recipients};
use crate::commands;

/// To, Cc and Bcc.
type Recipients = [Vec<Address>; 3];

fn recipients(open: &Open) -> Recipients {
    [
        open.record.to.clone(),
        open.record.cc.clone(),
        open.record.bcc.clone(),
    ]
}

fn set(open: &mut Open, [to, cc, bcc]: Recipients) {
    open.record.to = to;
    open.record.cc = cc;
    open.record.bcc = bcc;
}

/// A draft's recipient undo and redo: every change the user makes, whichever way (typing, removing,
/// moving, Instant Intro, a mention), is one step; Auto Bcc's own changes aren't.
#[derive(Clone, Debug, Default)]
pub(crate) struct History {
    undo: Vec<Recipients>,
    redo: Vec<Recipients>,
    /// The recipients as last noted.
    seen: Option<Recipients>,
}

/// Takes what changed in `open`'s recipients since the last note as one undo step, clearing
/// redo.
pub(crate) fn note(open: &mut Open) {
    let now = recipients(open);
    match open.history.seen.replace(now.clone()) {
        Some(seen) if seen != now => {
            open.history.undo.push(seen);
            open.history.redo.clear();
        }
        _ => {}
    }
}

/// Takes `open`'s recipients as they are now without an undo step (Auto Bcc's changes).
pub(crate) fn settle(open: &mut Open) {
    open.history.seen = Some(recipients(open));
}

/// ⌘Z in a recipient list. Returns whether there was a step to undo.
pub(crate) fn undo(open: &mut Open) -> bool {
    note(open);
    let Some(previous) = open.history.undo.pop() else {
        return false;
    };
    open.history.redo.push(recipients(open));
    set(open, previous);
    settle(open);
    true
}

/// ⌘⇧Z or ⌘Y in a recipient list.
pub(crate) fn redo(open: &mut Open) -> bool {
    note(open);
    let Some(next) = open.history.redo.pop() else {
        return false;
    };
    open.history.undo.push(recipients(open));
    set(open, next);
    settle(open);
    true
}

/// What a key does to a list's chips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    /// Enter with chips selected: the first is edited.
    Edit,
    /// Backspace or Delete with chips selected: they go.
    Remove,
    /// ⇧← (-1) or ⇧→ (1): the selection grows or shrinks at its edge.
    Extend(isize),
    /// ⌘A with nothing typed: every chip.
    SelectAll,
    /// ⌘C: the selected chips as `Name <address>, …`.
    Copy,
    /// ⌘X: copied, then removed.
    Cut,
    /// ⌥↑ (-1) or ⌥↓ (1): the selected chips go to the field before or after (To, Cc, Bcc).
    MoveField(isize),
    /// ⌘Z.
    Undo,
    /// ⌘⇧Z or ⌘Y.
    Redo,
    /// Escape with something typed: it becomes a chip.
    Finish,
}

/// Registers a list's chip keys, for the field `open` has the keyboard in.
pub(super) fn register(commands: &mut Commands<commands::Action>, open: &Open, list: List) {
    const LISTS: &[Region] = &[Region::RecipientList];
    let key = |id: &'static str, key: Key, keys: &[&'static str]| {
        Command::shortcut(id, commands::Action::Compose(Action::Chip(key)))
            .keys(keys.iter().copied())
            .regions(LISTS)
    };
    let input = open.input(list);
    let typed = !input.text.is_empty();
    if !input.selected.is_empty() {
        // Only while no chip is being edited.
        commands.add(key("CHIP_EDIT", Key::Edit, &["enter"]));
        commands.add(key("CHIP_REMOVE", Key::Remove, &["backspace", "del"]));
        commands.add(key("CHIP_COPY", Key::Copy, &["mod+c"]));
        commands.add(key("CHIP_CUT", Key::Cut, &["mod+x"]));
    }
    // In typed text, ⇧← and ⇧→ select text but at its edges.
    if !typed || input.caret == Some(0) {
        commands.add(key("CHIP_EXTEND_LEFT", Key::Extend(-1), &["shift+left"]));
    }
    if !typed || input.caret == Some(input.text.chars().count()) {
        commands.add(key("CHIP_EXTEND_RIGHT", Key::Extend(1), &["shift+right"]));
    }
    commands.add(key("CHIP_MOVE_UP", Key::MoveField(-1), &["alt+up"]));
    commands.add(key("CHIP_MOVE_DOWN", Key::MoveField(1), &["alt+down"]));
    // Right after `Remove Suggestion`, ⌘Z brings it back, typed text or not.
    if let Some(excluded) = &input.just_excluded {
        let undo = commands::Action::Compose(Action::UndoRemoveSuggestion(excluded.email.clone()));
        commands.add(
            Command::shortcut("CHIP_UNDO_REMOVE_SUGGESTION", undo)
                .key("mod+z")
                .regions(LISTS),
        );
    }
    if !typed {
        commands.add(key("CHIP_SELECT_ALL", Key::SelectAll, &["mod+a"]));
        if input.just_excluded.is_none() {
            commands.add(key("CHIP_UNDO", Key::Undo, &["mod+z"]));
        }
        commands.add(key("CHIP_REDO", Key::Redo, &["mod+shift+z", "mod+y"]));
    } else if !input.suggesting() {
        commands.add(key("CHIP_FINISH", Key::Finish, &["esc"]));
    }
}

/// The list with the keyboard.
fn list(open: &Open) -> Option<List> {
    open.state.field.list()
}

/// The chip at the edge of the selection in the list with the keyboard, if chips are selected.
pub(crate) fn edge(open: &Open) -> Option<&Address> {
    let list = list(open)?;
    let input = open.input(list);
    let at = input.anchor.filter(|_| !input.selected.is_empty())?;
    open.list(list).get(at)
}

/// Chip `index` of `list` becomes the input, its text all selected.
pub(crate) fn start_edit(open: &mut Open, list: List, index: usize) {
    let Some(address) = open.list(list).get(index) else {
        return;
    };
    let text = recipients::format(address);
    let input = open.input_mut(list);
    input.editing = Some(index);
    input.text = text;
    input.selected.clear();
    input.anchor = Some(index);
    input.select_text = true;
}

/// Finishes a chip edited in place: emptied, it goes; otherwise the highlighted suggestion or
/// what's typed takes its place, selected (an edit doesn't advance to the end). Returns whether a
/// chip was being edited.
pub(crate) fn finish_in_place(
    open: &mut Open,
    list: List,
    book: Option<&contacts::Index>,
    now: f64,
) -> bool {
    let Some(index) = open.input_mut(list).editing.take() else {
        return false;
    };
    let input = open.input_mut(list);
    let text = std::mem::take(&mut input.text).trim().to_owned();
    let picked = input
        .suggesting()
        .then(|| input.suggestions[input.highlight.min(input.suggestions.len() - 1)].clone());
    input.suggestions.clear();
    input.query.clear();
    let chips = open.list_mut(list);
    if index >= chips.len() {
        return true;
    }
    if text.is_empty() {
        chips.remove(index);
        let input = open.input_mut(list);
        input.selected.clear();
        input.anchor = None;
    } else {
        let mut address = picked.unwrap_or_else(|| recipients::parse(&text));
        if address.name.is_none() && !address.email.is_empty() {
            address.name = book.and_then(|book| book.name_for(&address.email));
        }
        // The address once, in the edited chip's place.
        chips[index] = address.clone();
        let mut at = index;
        let mut kept = 0;
        chips.retain(|other| {
            let keep = kept == index
                || other.email.is_empty()
                || !other.email.eq_ignore_ascii_case(&address.email);
            if !keep && kept < index {
                at -= 1;
            }
            kept += 1;
            keep
        });
        let input = open.input_mut(list);
        input.selected = vec![at];
        input.anchor = Some(at);
    }
    open.touch(now);
    true
}

/// A mouse-down on chip `index`: ⇧ extends the selection to it, ⌘ adds or takes it, otherwise it
/// alone is selected (unless it already is).
pub(crate) fn press(
    open: &mut Open,
    list: List,
    index: usize,
    modifiers: egui::Modifiers,
    book: Option<&contacts::Index>,
    now: f64,
) {
    finish_in_place(open, list, book, now);
    let input = open.input_mut(list);
    if modifiers.shift {
        extend_to(open, list, index);
    } else if modifiers.command {
        if let Some(at) = input
            .selected
            .iter()
            .position(|selected| *selected == index)
        {
            input.selected.remove(at);
        } else {
            input.selected.push(index);
            input.selected.sort_unstable();
            input.anchor = Some(index);
        }
    } else if !input.selected.contains(&index) {
        input.selected = vec![index];
        input.anchor = Some(index);
    }
}

/// Extends the selection from the edge to `to`.
fn extend_to(open: &mut Open, list: List, to: usize) {
    let count = open.list(list).len();
    let input = open.input_mut(list);
    let Some(edge) = input.anchor.filter(|_| to < count) else {
        input.selected = vec![to.min(count.saturating_sub(1))];
        input.anchor = input.selected.first().copied();
        return;
    };
    let (low, high) = (edge.min(to), edge.max(to));
    input.selected.extend(low..=high);
    input.selected.sort_unstable();
    input.selected.dedup();
    input.anchor = Some(to);
}

/// Carries out `key` in the list with the keyboard. Returns text to copy, for ⌘C and ⌘X.
pub(crate) fn run(
    open: &mut Open,
    key: Key,
    book: Option<&contacts::Index>,
    now: f64,
) -> Option<String> {
    let list = list(open)?;
    match key {
        Key::Edit => {
            if let Some(first) = open.input(list).selected.first().copied() {
                start_edit(open, list, first);
            }
        }
        Key::Remove => remove_selected(open, list, now),
        Key::Extend(delta) => {
            finish_in_place(open, list, book, now);
            let input = open.input(list);
            if input.selected.is_empty() {
                // At the end, ⇧← is ←.
                super::form::move_selection(open, delta, book, now);
                return None;
            }
            let edge = input.anchor.unwrap_or(input.selected[0]);
            let next = edge
                .checked_add_signed(delta)
                .filter(|next| *next < open.list(list).len())?;
            let input = open.input_mut(list);
            if input.selected.contains(&next) {
                // Shrinking: the edge comes off.
                input.selected.retain(|selected| *selected != edge);
                input.anchor = Some(next);
            } else {
                extend_to(open, list, next);
            }
        }
        Key::SelectAll => {
            let count = open.list(list).len();
            if count > 0 {
                let input = open.input_mut(list);
                input.selected = (0..count).collect();
                input.anchor = Some(count - 1);
            }
        }
        Key::Copy | Key::Cut => {
            let input = open.input(list);
            let text = input
                .selected
                .iter()
                .filter_map(|index| open.list(list).get(*index))
                .map(recipients::format)
                .collect::<Vec<_>>()
                .join(", ");
            if key == Key::Cut {
                remove_selected(open, list, now);
            }
            return Some(text);
        }
        Key::MoveField(delta) => {
            finish_in_place(open, list, book, now);
            move_field(open, list, delta, now);
        }
        Key::Undo => {
            if undo(open) {
                open.touch(now);
            }
        }
        Key::Redo => {
            if redo(open) {
                open.touch(now);
            }
        }
        Key::Finish => super::form::commit_input(open, now, book),
    }
    None
}

/// `remove({ranks: selection})`: the selected chips go, and the one before them is selected (the
/// first chip, or the caret at the end when there are none left).
fn remove_selected(open: &mut Open, list: List, now: f64) {
    let selected = std::mem::take(&mut open.input_mut(list).selected);
    let Some(first) = selected.first().copied() else {
        return;
    };
    let mut index = 0;
    open.list_mut(list).retain(|_| {
        index += 1;
        !selected.contains(&(index - 1))
    });
    let count = open.list(list).len();
    let input = open.input_mut(list);
    let next = first.saturating_sub(1);
    input.selected = if next < count { vec![next] } else { Vec::new() };
    input.anchor = input.selected.first().copied();
    open.touch(now);
}

/// The selected chips go to the end of the field before or after, selected there, which gets the
/// keyboard with Cc and Bcc showing.
fn move_field(open: &mut Open, list: List, delta: isize, now: f64) {
    let Some(target) = (list as usize)
        .checked_add_signed(delta)
        .and_then(|at| List::ALL.get(at).copied())
    else {
        return;
    };
    let selected = std::mem::take(&mut open.input_mut(list).selected);
    if selected.is_empty() {
        return;
    }
    let moved: Vec<Address> = selected
        .iter()
        .filter_map(|index| open.list(list).get(*index).cloned())
        .collect();
    let mut index = 0;
    open.list_mut(list).retain(|_| {
        index += 1;
        !selected.contains(&(index - 1))
    });
    open.input_mut(list).anchor = None;
    let into = open.list_mut(target);
    // Each address once; one already there makes way.
    into.retain(|kept| {
        !moved.iter().any(|address| {
            !address.email.is_empty() && address.email.eq_ignore_ascii_case(&kept.email)
        })
    });
    let start = into.len();
    into.extend(moved);
    let end = into.len();
    let input = open.input_mut(target);
    input.selected = (start..end).collect();
    input.anchor = end.checked_sub(1);
    open.state.expand(target.field());
    open.touch(now);
}

/// A paste while chips are selected: what's pasted replaces them, split into addresses as a
/// pasted list is, and the selection moves past what came in.
pub(crate) fn paste_over(
    ui: &egui::Ui,
    open: &mut Open,
    list: List,
    book: Option<&contacts::Index>,
    now: f64,
) {
    let pasted: Vec<String> = ui.ctx().input(|input| {
        input
            .events
            .iter()
            .filter_map(|event| {
                if let egui::Event::Paste(text) = event {
                    Some(text.clone())
                } else {
                    None
                }
            })
            .collect()
    });
    for text in pasted {
        let selected = std::mem::take(&mut open.input_mut(list).selected);
        let Some(at) = selected.first().copied() else {
            return;
        };
        let mut index = 0;
        open.list_mut(list).retain(|_| {
            index += 1;
            !selected.contains(&(index - 1))
        });
        let text = text.replace("mailto:", "");
        let mut parts = recipients::split_pasted(&text);
        // What doesn't make an address at the end stays typed.
        let mut leftover = Vec::new();
        while parts
            .last()
            .is_some_and(|part| !recipients::sendable(&recipients::parse(part)))
        {
            leftover.insert(0, parts.pop().unwrap_or_default());
        }
        let added: Vec<Address> = parts
            .iter()
            .map(|part| {
                let mut address = recipients::parse(part);
                if address.name.is_none() {
                    address.name = book.and_then(|book| book.name_for(&address.email));
                }
                address
            })
            .collect();
        let chips = open.list_mut(list);
        chips.retain(|kept| {
            !added.iter().any(|address| {
                !address.email.is_empty() && address.email.eq_ignore_ascii_case(&kept.email)
            })
        });
        let at = at.min(chips.len());
        let count = added.len();
        chips.splice(at..at, added);
        let total = chips.len();
        let input = open.input_mut(list);
        input.text = leftover.join(" ");
        input.selected = if at + count < total {
            vec![at + count]
        } else {
            Vec::new()
        };
        input.anchor = input.selected.first().copied();
        open.touch(now);
    }
}

impl crate::Mail {
    /// The draft with the keyboard's To, Cc and Bcc addresses (for tests).
    pub fn compose_recipients(&self) -> Option<[Vec<String>; 3]> {
        let open = self.compose.focused()?;
        let emails = |list: &[Address]| list.iter().map(|address| address.email.clone()).collect();
        Some([
            emails(&open.record.to),
            emails(&open.record.cc),
            emails(&open.record.bcc),
        ])
    }

    /// `Remove Suggestion` on the focused field's suggestion `index`, as its button does (for
    /// tests).
    pub fn compose_remove_suggestion(&mut self, index: usize) {
        self.remove_suggestion(index);
    }

    /// The suggestions under the focused recipient field, as `Name address` (for tests).
    pub fn compose_suggestions(&self) -> Vec<String> {
        let Some(open) = self.compose.focused() else {
            return Vec::new();
        };
        let Some(list) = open.state.field.list() else {
            return Vec::new();
        };
        open.input(list)
            .suggestions
            .iter()
            .map(|address| {
                format!(
                    "{} {}",
                    address.name.as_deref().unwrap_or(&address.email),
                    address.email
                )
            })
            .collect()
    }

    /// The draft with the keyboard's body, a line per block, and what its To field's input
    /// holds (for tests).
    pub fn compose_text(&self) -> Option<(String, String)> {
        let open = self.compose.focused()?;
        let body = open
            .editor
            .document()
            .blocks
            .iter()
            .map(|block| block.text())
            .collect::<Vec<_>>()
            .join("\n");
        Some((body, open.input(List::To).text.clone()))
    }
}

/// Chips being dragged: the list they're from and which, and how the drag image follows the
/// pointer.
#[derive(Clone, Debug)]
pub(crate) struct Drag {
    pub list: List,
    /// The chips, by index, sorted.
    pub chips: Vec<usize>,
    /// The grabbed chip's label and size, and where in it the pointer went down: one chip's
    /// image is that chip, kept under the pointer as grabbed.
    pub grabbed: (String, egui::Vec2, egui::Vec2),
}

/// A drag starting on chip `index` of `list`: an edit finishes, the chip is selected unless it
/// is already (with the rest of the selection), and Cc and Bcc show to drop into.
pub(crate) fn start_drag(
    open: &mut Open,
    list: List,
    index: usize,
    grabbed: (String, egui::Vec2, egui::Vec2),
    book: Option<&contacts::Index>,
    now: f64,
) {
    finish_in_place(open, list, book, now);
    let input = open.input_mut(list);
    if !input.selected.contains(&index) {
        input.selected = vec![index];
        input.anchor = Some(index);
    }
    let chips = input.selected.clone();
    open.state.header_expanded = true;
    open.state.cc_bcc_shown = true;
    open.chip_drag = Some(Drag {
        list,
        chips,
        grabbed,
    });
}

/// Whether two chips are one address: the same email, or with none, the same text.
fn same_chip(a: &Address, b: &Address) -> bool {
    if a.email.is_empty() || b.email.is_empty() {
        a.email.is_empty() && b.email.is_empty() && a.name == b.name
    } else {
        a.email.eq_ignore_ascii_case(&b.email)
    }
}

/// The drop: chips `chips` of `from` go to `into` at `at`, the drop cursor's place, as one undo
/// step. The target keeps each address once; the dropped chips are selected there. `at` is
/// counted after the chips that make way are taken out, so a chip dragged right within its list
/// lands at the cursor.
pub(crate) fn drop(open: &mut Open, from: List, chips: &[usize], into: List, at: usize, now: f64) {
    let moved: Vec<Address> = chips
        .iter()
        .filter_map(|index| open.list(from).get(*index).cloned())
        .collect();
    if moved.is_empty() {
        return;
    }
    if from != into {
        let mut index = 0;
        open.list_mut(from).retain(|_| {
            index += 1;
            !chips.contains(&(index - 1))
        });
        let input = open.input_mut(from);
        input.selected.clear();
        input.anchor = None;
    }
    let target = open.list_mut(into);
    let at = at.min(target.len());
    let makes_way = |kept: &Address| moved.iter().any(|address| same_chip(address, kept));
    let before = target[..at].iter().filter(|kept| makes_way(kept)).count();
    target.retain(|kept| !makes_way(kept));
    let at = at - before;
    let count = moved.len();
    target.splice(at..at, moved);
    let input = open.input_mut(into);
    input.selected = (at..at + count).collect();
    input.anchor = Some(at + count - 1);
    open.touch(now);
}

/// A list that lost the keyboard keeps what was typed in it as a chip and its selection goes back
/// to the end.
pub(crate) fn blur(open: &mut Open, has_keyboard: bool, book: Option<&contacts::Index>, now: f64) {
    for list in List::ALL {
        if has_keyboard && open.state.field == list.field() {
            continue;
        }
        let input = open.input(list);
        if input.editing.is_some() || !input.selected.is_empty() {
            finish_in_place(open, list, book, now);
            let input = open.input_mut(list);
            input.selected.clear();
            input.anchor = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use exo_store::{DraftAction, DraftRecord};

    use super::*;

    fn address(email: &str) -> Address {
        Address {
            name: None,
            email: email.into(),
        }
    }

    pub(crate) fn draft(to: &[&str]) -> Open {
        let record = DraftRecord {
            action: DraftAction::Compose,
            from: None,
            to: to.iter().map(|email| address(email)).collect(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: String::new(),
            body: String::new(),
            quoted_content: None,
            quoted_inlined: false,
            in_reply_to: None,
            in_reply_to_rfc822: None,
            references: Vec::new(),
            rfc822_id: "x@y".into(),
            fingerprint: String::new(),
            attachments: Vec::new(),
            send_later: None,
            reminder: None,
        };
        Open::new("d".into(), None, record)
    }

    fn to(open: &Open) -> Vec<&str> {
        open.record
            .to
            .iter()
            .map(|address| address.email.as_str())
            .collect()
    }

    #[test]
    fn each_change_undoes_and_redoes() {
        let mut open = draft(&["a@x.com"]);
        note(&mut open);
        open.record.to.push(address("b@x.com"));
        note(&mut open);
        open.record.to.remove(0);
        assert!(undo(&mut open));
        assert_eq!(to(&open), ["a@x.com", "b@x.com"]);
        assert!(undo(&mut open));
        assert_eq!(to(&open), ["a@x.com"]);
        assert!(!undo(&mut open), "nothing more");
        assert!(redo(&mut open));
        assert!(redo(&mut open));
        assert_eq!(to(&open), ["b@x.com"]);
        assert!(!redo(&mut open));
    }

    fn focused(mut open: Open, field: super::super::Field) -> Open {
        open.state.expand(field);
        open
    }

    #[test]
    fn a_chip_edits_in_place_and_emptied_goes() {
        let mut open = focused(
            draft(&["a@x.com", "b@x.com", "c@x.com"]),
            super::super::Field::To,
        );
        start_edit(&mut open, List::To, 1);
        assert_eq!(
            (
                open.input(List::To).text.as_str(),
                open.input(List::To).editing
            ),
            ("b@x.com", Some(1))
        );
        open.input_mut(List::To).text = "Bea <bea@x.com>".into();
        assert!(finish_in_place(&mut open, List::To, None, 0.0));
        assert_eq!(to(&open), ["a@x.com", "bea@x.com", "c@x.com"]);
        assert_eq!(
            open.input(List::To).selected,
            [1],
            "the edited chip is selected"
        );
        assert_eq!(open.record.to[1].name.as_deref(), Some("Bea"));
        run(&mut open, Key::Edit, None, 0.0);
        open.input_mut(List::To).text = "  ".into();
        finish_in_place(&mut open, List::To, None, 0.0);
        assert_eq!(to(&open), ["a@x.com", "c@x.com"]);
        start_edit(&mut open, List::To, 0);
        open.input_mut(List::To).text = "c@x.com".into();
        finish_in_place(&mut open, List::To, None, 0.0);
        assert_eq!(to(&open), ["c@x.com"], "an address already there makes way");
    }

    #[test]
    fn selecting_with_the_mouse_and_shift_arrows() {
        let mut open = focused(
            draft(&["a@x.com", "b@x.com", "c@x.com", "d@x.com"]),
            super::super::Field::To,
        );
        press(&mut open, List::To, 1, egui::Modifiers::NONE, None, 0.0);
        press(&mut open, List::To, 3, egui::Modifiers::SHIFT, None, 0.0);
        assert_eq!(open.input(List::To).selected, [1, 2, 3]);
        assert_eq!(
            edge(&open).map(|address| address.email.as_str()),
            Some("d@x.com"),
            "the contact pane shows the edge"
        );
        run(&mut open, Key::Extend(-1), None, 0.0);
        assert_eq!(open.input(List::To).selected, [1, 2], "the edge comes back");
        press(&mut open, List::To, 0, egui::Modifiers::COMMAND, None, 0.0);
        assert_eq!(open.input(List::To).selected, [0, 1, 2]);
        press(&mut open, List::To, 1, egui::Modifiers::COMMAND, None, 0.0);
        assert_eq!(open.input(List::To).selected, [0, 2]);
        assert_eq!(
            run(&mut open, Key::Copy, None, 0.0).as_deref(),
            Some("a@x.com, c@x.com")
        );
        run(&mut open, Key::Remove, None, 0.0);
        assert_eq!(to(&open), ["b@x.com", "d@x.com"]);
        assert_eq!(
            open.input(List::To).selected,
            [0],
            "the chip before is selected"
        );
        run(&mut open, Key::SelectAll, None, 0.0);
        assert_eq!(open.input(List::To).selected, [0, 1]);
    }

    #[test]
    fn alt_arrows_move_chips_between_fields() {
        let mut open = focused(draft(&["a@x.com", "b@x.com"]), super::super::Field::To);
        open.input_mut(List::To).selected = vec![1];
        run(&mut open, Key::MoveField(1), None, 0.0);
        assert_eq!(to(&open), ["a@x.com"]);
        assert_eq!(
            open.record
                .cc
                .iter()
                .map(|address| address.email.as_str())
                .collect::<Vec<_>>(),
            ["b@x.com"]
        );
        assert_eq!(
            (
                open.state.field,
                open.input(List::Cc).selected.clone(),
                open.state.cc_bcc_shown
            ),
            (super::super::Field::Cc, vec![0], true)
        );
        run(&mut open, Key::MoveField(-1), None, 0.0);
        assert_eq!(to(&open), ["a@x.com", "b@x.com"]);
        open.input_mut(List::To).selected = vec![0];
        run(&mut open, Key::MoveField(-1), None, 0.0);
        assert_eq!(to(&open), ["a@x.com", "b@x.com"], "nothing before To");
    }

    #[test]
    fn auto_bccs_changes_arent_steps() {
        let mut open = draft(&["a@x.com"]);
        note(&mut open);
        open.record.bcc.push(address("crm@x.com"));
        settle(&mut open);
        assert!(!undo(&mut open));
        open.record.to.clear();
        note(&mut open);
        open.record.to.push(address("c@x.com"));
        assert!(undo(&mut open), "a new change clears redo and steps back");
        assert!(open.record.to.is_empty());
    }
}
