//! The event form's spelling: its title spellchecked as a one-line field, About as compose's body
//! is ([`crate::spelling`]), and a right-click in either showing the shell's editable menu.

use exo_commands::Hint;

use crate::{
    Mail,
    spelling::{self, Done},
};

impl Mail {
    /// Each frame, before anything draws: the open form's title and About get their
    /// spellcheckers, About follows the account's autocorrect and autocomplete, and a
    /// right-click in either last frame shows its menu and does what's chosen in it.
    pub(crate) fn calendar_spelling(&mut self, ctx: &egui::Context) {
        let (autocorrect, autocomplete) =
            (self.autocorrect(), self.account.settings().autocomplete());
        let names = if self
            .calendar
            .form
            .as_ref()
            .is_some_and(|form| !form.about().has_spelling())
        {
            self.contact_names()
        } else {
            Vec::new()
        };
        let Some(form) = self.calendar.form.as_mut() else {
            return;
        };
        spelling::check_field(&mut form.title_spelling, &*self.spelling);
        spelling::assist(
            form.about_mut(),
            &*self.spelling,
            autocorrect,
            autocomplete,
            || names,
        );
        if let Some(word) = form.about_learned.take() {
            self.commands
                .hint(Hint::new().text(format!("Learned \"{word}\"")));
        }
        if self.link_input_menu(ctx, |mail| {
            mail.calendar.form.as_mut().map(|form| form.about_mut())
        }) {
            return;
        }
        let Some(form) = self.calendar.form.as_mut() else {
            return;
        };
        let (title, about) = (form.title_spelling.take_menu(), form.about_menu.take());
        let Some((menu, in_about)) = title
            .map(|menu| (menu, false))
            .or(about.map(|menu| (menu, true)))
        else {
            return;
        };
        let Some(choice) = self.editable_menu(&menu) else {
            return;
        };
        let Some(form) = self.calendar.form.as_mut() else {
            return;
        };
        let done = if in_about {
            let done = spelling::editor_choice(ctx, form.about_mut(), &menu, choice);
            form.sync_about();
            done
        } else {
            spelling::field_choice(
                ctx,
                &mut form.title_spelling,
                &mut form.values.title,
                &menu,
                choice,
            )
        };
        if let Done::Open(url) = done {
            self.open_chosen(&url);
        }
    }
}
