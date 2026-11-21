//! Find in page: the find field in the tab strip ([`bar`]), its keys, and the macOS find
//! pasteboard. What it finds is the page's text, which exo-ui searches ([`exo_ui::find`]).
//!
//! The keys work whatever has the keyboard: ⌘F opens the field with its text selected, ⌘G and
//! ⌘⇧G open it and move to the next or previous match, and, as in other Mac apps, ⌘E puts the
//! selected text in it ([`use_selection`]). While the field has the keyboard the page gets no
//! keys: Enter and ⇧Enter step through the matches, Escape or ⌃[ close the field.

mod bar;
mod pasteboard;

use std::sync::Arc;

use egui::{Context, Event, Id, Key, Modifiers, mutex::Mutex};

pub(crate) use bar::{layout, show};
pub use pasteboard::Pasteboard;

/// Edit ▸ Find… (⌘F): opens the find field, its text selected, with the keyboard.
pub fn open(ctx: &Context) {
    command(ctx, Command::Open);
}

/// Edit ▸ Find Next (⌘G): opens the find field and moves to the next match.
pub fn next(ctx: &Context) {
    command(ctx, Command::Step(true));
}

/// Edit ▸ Find Previous (⌘⇧G): opens the find field and moves to the previous match.
pub fn previous(ctx: &Context) {
    command(ctx, Command::Step(false));
}

/// Edit ▸ Use Selection for Find (⌘E): the selected text becomes the text to find, shared with
/// other apps; with the field open, it's searched for.
pub fn use_selection(ctx: &Context) {
    command(ctx, Command::UseSelection);
}

/// Shares the text to find with other apps from now on, through `pasteboard`, the macOS find
/// pasteboard: what's typed goes there, and what's there when the window comes back to the front
/// becomes the text to find.
pub fn share_find_text(ctx: &Context, pasteboard: Pasteboard) {
    with(ctx, |find| find.pasteboard = Some(pasteboard));
}

/// What the find field is doing.
#[derive(Default)]
struct Find {
    /// The field shows (`findInPageState.active`).
    open: bool,
    /// The field has the keyboard.
    focused: bool,
    /// Its text is to be selected on the next frame.
    select: bool,
    /// The text to find, which the field shows.
    query: String,
    /// The text is being searched for: typing, Enter or ⌘G search; adopting the find
    /// pasteboard's text, and closing, stop.
    searching: bool,
    /// What the menu and the keys asked for, not done yet.
    commands: Vec<Command>,
    /// The keys and text the field has this frame, taken from the page.
    events: Vec<Event>,
    /// Where else the text to find goes.
    pasteboard: Option<Pasteboard>,
    /// Whether the window was in front last frame.
    window_focused: bool,
    /// Who had the keyboard before the field took it, to give it back when the field closes.
    returns_focus_to: Option<Id>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    Open,
    Step(bool),
    UseSelection,
}

fn command(ctx: &Context, command: Command) {
    with(ctx, |find| find.commands.push(command));
    ctx.request_repaint();
}

fn with<R>(ctx: &Context, f: impl FnOnce(&mut Find) -> R) -> R {
    let id = exo_ui::id(ctx, "exo::find");
    let find = ctx.data_mut(|data| data.get_temp_mut_or_default::<Arc<Mutex<Find>>>(id).clone());
    let mut find = find.lock();
    f(&mut find)
}

/// The id of the field's text edit.
fn field_id(ctx: &egui::Context) -> Id {
    exo_ui::id(ctx, "exo::find::field")
}

/// Takes this frame's find keys, and every key while the field has the keyboard, before the
/// page sees them, and does what was asked. Call at the start of every frame, before anything
/// reads the keyboard; `available` is false on the sign-in screen, which has no find and closes
/// the field.
pub(crate) fn begin(ctx: &Context, available: bool) {
    let window_focused = ctx.input(|input| input.viewport().focused.unwrap_or(input.focused));
    let regained = with(ctx, |find| {
        !std::mem::replace(&mut find.window_focused, window_focused) && window_focused
    });
    if regained {
        adopt_pasteboard(ctx);
    }
    let keys = take_shortcuts(ctx);
    with(ctx, |find| find.commands.extend(keys));
    let commands = with(ctx, |find| std::mem::take(&mut find.commands));
    for command in commands {
        match command {
            _ if !available => {}
            Command::Open => reveal(ctx),
            Command::Step(forward) => {
                // The query, if any, searched on from the current match.
                let query = with(ctx, |find| {
                    find.searching |= !find.query.is_empty();
                    find.searching.then(|| find.query.clone())
                });
                if let Some(query) = query {
                    exo_ui::find::search(ctx, Some(&query));
                    exo_ui::find::step(ctx, forward);
                }
                reveal(ctx);
            }
            Command::UseSelection => with(ctx, |find| find.commands.push(Command::UseSelection)),
        }
    }
    if !available && with(ctx, |find| find.open) {
        close(ctx);
    }
    if with(ctx, |find| find.focused) {
        take_field_keys(ctx);
    }
}

/// Whether ⌘E asked for the selected text this frame: hand it over with [`selected`].
pub(crate) fn wants_selection(ctx: &Context) -> bool {
    with(ctx, |find| find.commands.contains(&Command::UseSelection))
}

/// ⌘E's answer: `text` (the text selected on the page, if any) becomes the text to find.
pub(crate) fn selected(ctx: &Context, text: Option<String>) {
    let open = with(ctx, |find| {
        find.commands
            .retain(|command| *command != Command::UseSelection);
        find.open
    });
    let Some(text) = text.filter(|text| !text.is_empty()) else {
        return;
    };
    set_query(ctx, text.clone(), open);
    share(ctx, &text);
}

/// The field opens (if it isn't) with the keyboard and its text selected; the page's link
/// preview goes.
fn reveal(ctx: &Context) {
    let field = field_id(ctx);
    let previous = ctx
        .memory(|memory| memory.focused())
        .filter(|id| *id != field);
    with(ctx, |find| {
        if !find.focused {
            find.returns_focus_to = previous;
        }
        (find.open, find.focused, find.select) = (true, true, true);
    });
}

/// Closes the field (`findInPage({ stop: true })`): the search stops, its highlights go, and the
/// keyboard goes back to the page. The text stays for next time.
fn close(ctx: &Context) {
    let returns = with(ctx, |find| {
        let returns = find.focused.then(|| find.returns_focus_to.take()).flatten();
        (find.open, find.focused, find.searching) = (false, false, false);
        find.events.clear();
        returns
    });
    exo_ui::find::search(ctx, None);
    let field = field_id(ctx);
    ctx.memory_mut(|memory| {
        memory.surrender_focus(field);
        if let Some(id) = returns {
            memory.request_focus(id);
        }
    });
}

/// The text to find is now `query`; searched for if `search`.
fn set_query(ctx: &Context, query: String, search: bool) {
    let searching = search && !query.is_empty();
    with(ctx, |find| {
        (find.query, find.searching) = (query.clone(), searching)
    });
    exo_ui::find::search(ctx, searching.then_some(query.as_str()));
}

/// Hands `query` to other apps.
fn share(ctx: &Context, query: &str) {
    let pasteboard = with(ctx, |find| find.pasteboard.clone());
    if let Some(pasteboard) = pasteboard {
        pasteboard.write(query);
    }
}

/// The window came back to the front: a text to find another app left on the find pasteboard
/// replaces this one, without searching for it.
fn adopt_pasteboard(ctx: &Context) {
    let Some(pasteboard) = with(ctx, |find| find.pasteboard.clone()) else {
        return;
    };
    let text = pasteboard.read();
    if with(ctx, |find| find.query != text) {
        set_query(ctx, text, false);
    }
}

/// ⌘F, ⌘G, ⌘⇧G and ⌘E, taken out of this frame's input.
fn take_shortcuts(ctx: &Context) -> Vec<Command> {
    let mut commands = Vec::new();
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            let Event::Key {
                key,
                pressed,
                modifiers,
                ..
            } = event
            else {
                return true;
            };
            let command = shortcut(*key, *modifiers);
            if *pressed {
                commands.extend(command);
            }
            command.is_none()
        });
    });
    commands
}

fn shortcut(key: Key, modifiers: Modifiers) -> Option<Command> {
    if !modifiers.command || modifiers.alt || modifiers.ctrl {
        return None;
    }
    match (key, modifiers.shift) {
        (Key::F, false) => Some(Command::Open),
        (Key::G, shift) => Some(Command::Step(!shift)),
        (Key::E, false) => Some(Command::UseSelection),
        _ => None,
    }
}

/// While the field has the keyboard: its own keys, done here, and everything else it types
/// with, kept for it, out of the page's reach. Zooming still zooms.
fn take_field_keys(ctx: &Context) {
    let mut taken = Vec::new();
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            let keyboard = matches!(event, Event::Key { .. } | Event::Text(_) | Event::Paste(_) | Event::Copy | Event::Cut | Event::Ime(_));
            let zoom = matches!(event, Event::Key { key: Key::Equals | Key::Plus | Key::Minus | Key::Num0, modifiers, .. } if modifiers.command);
            if keyboard && !zoom {
                taken.push(event.clone());
            }
            !keyboard || zoom
        });
    });
    let mut kept = Vec::new();
    for event in taken {
        match event {
            Event::Key {
                key: Key::Enter,
                pressed: true,
                modifiers,
                ..
            } => {
                // Enter finds the next match, ⇧Enter the previous.
                let query = with(ctx, |find| find.query.clone());
                if !query.is_empty() {
                    set_query(ctx, query.clone(), true);
                    exo_ui::find::step(ctx, !modifiers.shift);
                    share(ctx, &query);
                }
            }
            Event::Key {
                key: Key::Escape,
                pressed: true,
                ..
            } => close(ctx),
            Event::Key {
                key: Key::OpenBracket,
                pressed: true,
                modifiers,
                ..
            } if modifiers.ctrl => close(ctx),
            // The field's own keys go no further, nor does Tab, which would move egui's focus.
            Event::Key {
                key: Key::Enter | Key::Escape | Key::Tab,
                ..
            } => {}
            event => kept.push(event),
        }
    }
    with(ctx, |find| find.events = kept);
}
