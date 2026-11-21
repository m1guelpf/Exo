//! What compose's body, the spellchecked fields (the subject, a snippet's name, the Out of Office
//! subject, an event's title) and the other editors (About, the Out of Office message) share of
//! spelling: the shell's editable menu for a right-click in one, and what's chosen in it done to
//! it.
//!
//! The other editors check spelling as a body does: autocorrected while autocorrect is on,
//! spellchecked otherwise. The one-line fields are spellchecked either way
//! ([`exo_editor::FieldSpelling`]).

use exo_editor::{ContextMenu, Editor, FieldSpelling, Spelling};

use crate::{
    Mail,
    desktop::context_menu,
    page_menu::{self, Choice, Editable, Target},
};

/// What a choice in an editable menu left to do.
pub(crate) enum Done {
    Nothing,
    /// The text changed.
    Edited,
    /// A link to open ([`Mail::open_chosen`]).
    Open(String),
}

impl Mail {
    /// The shell's editable menu for a right-click `menu` reported, and the item chosen in it.
    pub(crate) fn editable_menu(&mut self, menu: &ContextMenu) -> Option<Choice> {
        let misspelling = menu.misspelling.as_ref();
        let target = Target {
            link: menu.link.clone(),
            image: None,
            editable: Some(Editable {
                misspelled: misspelling.map(|word| word.word.clone()),
                suggestions: misspelling
                    .map(|word| word.guesses.clone())
                    .unwrap_or_default(),
            }),
            selection: menu.selection.clone(),
        };
        let entries = page_menu::entries(&target);
        let items: Vec<context_menu::MenuItem> =
            entries.iter().map(|(item, _)| item.clone()).collect();
        (self.context_menu.borrow_mut())(&items, menu.at)
            .and_then(|index| entries.into_iter().nth(index))
            .and_then(|(_, choice)| choice)
    }

    /// Each frame, before anything draws: Split Inbox's sender picker gets its spellchecker, and a
    /// right-click last frame in it, or in any field without spelling (every other input of the
    /// app's: the shell's menu is any editable element's, `params.isEditable`), shows the shell's
    /// editable menu and does what's chosen in it.
    pub(crate) fn field_menus(&mut self, ctx: &egui::Context) {
        let make = &*self.spelling;
        if let Some(picker) = self.settings.sender_picker_mut() {
            check_field(&mut picker.spelling, make);
            if let Some(menu) = picker.spelling.take_menu() {
                let Some(choice) = self.editable_menu(&menu) else {
                    return;
                };
                let done = self
                    .settings
                    .sender_picker_mut()
                    .map_or(Done::Nothing, |picker| {
                        picker.spelling_choice(ctx, &menu, choice)
                    });
                if let Done::Open(url) = done {
                    self.open_chosen(&url);
                }
                return;
            }
        }
        let Some(field) = exo_ui::take_field_menu(ctx, None) else {
            return;
        };
        let menu = ContextMenu {
            at: field.at,
            link: None,
            misspelling: None,
            selection: field.selected,
            selection_baseline: field.baseline,
            text_size: field.text_size,
        };
        let Some(choice) = self.editable_menu(&menu) else {
            return;
        };
        if let Done::Open(url) = shared_choice(ctx, &menu, choice) {
            self.open_chosen(&url);
        }
    }

    /// A right-click last frame in the link input of the editor `editor` finds: the shell's
    /// editable menu, its spelling first, and what's chosen done to the input. Returns whether
    /// there was one.
    pub(crate) fn link_input_menu(
        &mut self,
        ctx: &egui::Context,
        editor: fn(&mut Mail) -> Option<&mut Editor>,
    ) -> bool {
        let Some(menu) = editor(self).and_then(|editor| editor.link_spelling_mut().take_menu())
        else {
            return false;
        };
        let Some(choice) = self.editable_menu(&menu) else {
            return true;
        };
        let done = editor(self)
            .and_then(Editor::link_field)
            .map_or(Done::Nothing, |(spelling, text)| {
                field_choice(ctx, spelling, text, &menu, choice)
            });
        if let Done::Open(url) = done {
            self.open_chosen(&url);
        }
        true
    }

    /// Opens what a menu chose to: a `mailto:` in a new draft, anything else in the browser.
    pub(crate) fn open_chosen(&mut self, url: &str) {
        if url.starts_with("mailto:") {
            self.compose_mailto(url, false);
        } else {
            self.open_url(url);
        }
    }
}

/// Gives `editor` a spellchecker made by `make` if it has none, with `names` (made only then)
/// left alone by autocorrect, and turns its autocorrect and autocomplete as the account has them.
pub(crate) fn assist(
    editor: &mut Editor,
    make: &dyn Fn() -> Option<Box<dyn Spelling>>,
    autocorrect: bool,
    autocomplete: bool,
    names: impl FnOnce() -> Vec<String>,
) {
    if !editor.has_spelling()
        && let Some(engine) = make()
    {
        editor.set_spelling(engine);
        editor.set_names(names());
    }
    editor.set_autocorrect(autocorrect);
    editor.set_autocomplete(autocomplete);
    check_field(editor.link_spelling_mut(), make);
}

/// Gives `field` a spellchecker made by `make` if it has none.
pub(crate) fn check_field(field: &mut FieldSpelling, make: &dyn Fn() -> Option<Box<dyn Spelling>>) {
    if !field.has_engine()
        && let Some(engine) = make()
    {
        field.set_engine(engine);
    }
}

/// Does `choice`, from the menu of a right-click `menu` in `editor`.
pub(crate) fn editor_choice(
    ctx: &egui::Context,
    editor: &mut Editor,
    menu: &ContextMenu,
    choice: Choice,
) -> Done {
    match choice {
        Choice::Correct(correction) => {
            let Some(misspelling) = &menu.misspelling else {
                return Done::Nothing;
            };
            editor.correct(misspelling, &correction);
            return Done::Edited;
        }
        Choice::AddToDictionary(word) => editor.learn_spelling(&word),
        Choice::IgnoreSpelling(word) => editor.ignore_spelling(&word),
        choice => return shared_choice(ctx, menu, choice),
    }
    Done::Nothing
}

/// Does `choice`, from the menu of a right-click `menu` in a field editing `text`.
pub(crate) fn field_choice(
    ctx: &egui::Context,
    field: &mut FieldSpelling,
    text: &mut String,
    menu: &ContextMenu,
    choice: Choice,
) -> Done {
    match choice {
        Choice::Correct(correction) => {
            let Some(misspelling) = &menu.misspelling else {
                return Done::Nothing;
            };
            field.correct(ctx, text, misspelling, &correction);
            return Done::Edited;
        }
        Choice::AddToDictionary(word) => field.learn(&word, text),
        Choice::IgnoreSpelling(word) => field.ignore(&word, text),
        choice => return shared_choice(ctx, menu, choice),
    }
    Done::Nothing
}

/// The items an editor and a field do alike: what the keys would do for the widget with the
/// keyboard, Look Up, and links to open.
fn shared_choice(ctx: &egui::Context, menu: &ContextMenu, choice: Choice) -> Done {
    match choice {
        Choice::LookUp => {
            context_menu::look_up(&menu.selection, menu.selection_baseline, menu.text_size)
        }
        Choice::Cut => ctx.input_mut(|input| input.events.push(egui::Event::Cut)),
        Choice::Copy => ctx.input_mut(|input| input.events.push(egui::Event::Copy)),
        Choice::Paste => {
            let modifiers = egui::Modifiers::MAC_CMD.plus(egui::Modifiers::COMMAND);
            // A paste of the clipboard's text; with none, the ⌘V key, which pastes an image.
            let event = crate::clipboard::read_text(ctx)
                .map(|text| egui::Event::Paste(text.replace("\r\n", "\n")))
                .unwrap_or(egui::Event::Key {
                    key: egui::Key::V,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                });
            ctx.input_mut(|input| input.events.push(event));
        }
        Choice::CopyText(text) => ctx.copy_text(text),
        Choice::OpenLink(url) | Choice::Search(url) => return Done::Open(url),
        // Editable menus offer no image's items; the spelling ones are each widget's.
        Choice::CopyImage
        | Choice::Correct(_)
        | Choice::AddToDictionary(_)
        | Choice::IgnoreSpelling(_) => {}
    }
    Done::Nothing
}
