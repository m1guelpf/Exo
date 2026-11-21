//! The Out of Office dialog's spelling: its subject spellchecked as a single-line field, its
//! message as compose's body is ([`crate::spelling`]), and a right-click in either showing the
//! shell's editable menu.

use exo_commands::Hint;

use crate::{
    Mail,
    spelling::{self, Done},
};

impl Mail {
    /// Each frame, before anything draws: the open dialog's subject and message get their
    /// spellcheckers, the message follows the account's autocorrect and autocomplete, and a
    /// right-click in either last frame shows its menu and does what's chosen in it.
    pub(crate) fn out_of_office_spelling(&mut self, ctx: &egui::Context) {
        let (autocorrect, autocomplete) =
            (self.autocorrect(), self.account.settings().autocomplete());
        let names = if self
            .out_of_office
            .dialog
            .as_ref()
            .is_some_and(|dialog| !dialog.editor().has_spelling())
        {
            self.contact_names()
        } else {
            Vec::new()
        };
        let Some(dialog) = self.out_of_office.dialog.as_mut() else {
            return;
        };
        for field in [
            &mut dialog.subject_spelling,
            &mut dialog.start_spelling,
            &mut dialog.end_spelling,
        ] {
            spelling::check_field(field, &*self.spelling);
        }
        spelling::assist(
            dialog.editor_mut(),
            &*self.spelling,
            autocorrect,
            autocomplete,
            || names,
        );
        if let Some(word) = dialog.message_learned.take() {
            self.commands
                .hint(Hint::new().text(format!("Learned \"{word}\"")));
        }
        if self.link_input_menu(ctx, |mail| {
            mail.out_of_office
                .dialog
                .as_mut()
                .map(|dialog| dialog.editor_mut())
        }) {
            return;
        }
        let Some(dialog) = self.out_of_office.dialog.as_mut() else {
            return;
        };
        let menus = [
            dialog
                .subject_spelling
                .take_menu()
                .map(|menu| (menu, Menu::Subject)),
            dialog.message_menu.take().map(|menu| (menu, Menu::Message)),
            dialog
                .start_spelling
                .take_menu()
                .map(|menu| (menu, Menu::Start)),
            dialog
                .end_spelling
                .take_menu()
                .map(|menu| (menu, Menu::End)),
        ];
        let Some((menu, which)) = menus.into_iter().flatten().next() else {
            return;
        };
        let Some(choice) = self.editable_menu(&menu) else {
            return;
        };
        let Some(dialog) = self.out_of_office.dialog.as_mut() else {
            return;
        };
        let done = match which {
            Menu::Message => {
                let done = spelling::editor_choice(ctx, dialog.editor_mut(), &menu, choice);
                dialog.form.body = dialog.editor().html();
                done
            }
            Menu::Subject => spelling::field_choice(
                ctx,
                &mut dialog.subject_spelling,
                &mut dialog.form.subject,
                &menu,
                choice,
            ),
            Menu::Start | Menu::End => dialog.date_choice(ctx, which == Menu::Start, &menu, choice),
        };
        if let Done::Open(url) = done {
            self.open_chosen(&url);
        }
    }
}

/// Which of the dialog's widgets a menu is for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Menu {
    Subject,
    Message,
    Start,
    End,
}
