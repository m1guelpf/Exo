//! The device's accounts as the mail on screen sees them: switching, adding and signing out. The
//! mail asks the app ([`crate::App`]), which owns the account list, the Keychain and the stores.
//! Before signing out it checks for changes Gmail doesn't have yet and confirms.

use egui::Color32;
use exo_commands::{Command, Commands, SubPalette};
use exo_store::Unsent;

use crate::{Mail, commands, splits};

/// What the mail on screen asks of the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Sign this account out: forget its grant and delete what Exo keeps of it. The user was
    /// asked, or it has nothing unsent.
    SignOut(String),
    /// Sign this account, not the one on screen, out, asking first when it has changes Gmail
    /// doesn't have yet.
    CheckSignOut(String),
    /// Show this account's mail, or the sign-in screen when Google signed it out.
    Switch(String),
    /// Sign another account in.
    Add,
    /// Move this account to this place in the list.
    Move {
        /// The account.
        email: String,
        /// Its new place.
        to: usize,
    },
    /// Show this account in this color, by its name in [`COLORS`].
    Color {
        /// The account.
        email: String,
        /// The color's name.
        color: &'static str,
    },
    /// Open one of Exo's links, clicked in a message, in a new tab.
    Open(crate::links::Link),
}

/// What an account command, key or row does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// `Sign Out` of the account on screen.
    SignOut,
    /// `Switch Accounts`: the Accounts palette.
    OpenSwitcher,
    /// Switch to the account in this place in the list (ctrl+1…9, a row of the palette).
    Switch(usize),
    /// `Add Account`, ctrl+0.
    Add,
}

/// The account colors by name, in the order an address's default is picked from.
pub(crate) const COLORS: [(&str, Color32); 12] = [
    ("thistle", Color32::from_rgb(0xc4, 0xbb, 0xd3)),
    ("orchid", Color32::from_rgb(0xe4, 0xcd, 0xe0)),
    ("bubblegum", Color32::from_rgb(0xef, 0xcb, 0xe9)),
    ("sea", Color32::from_rgb(0xb2, 0xca, 0xe5)),
    ("water", Color32::from_rgb(0xae, 0xd8, 0xea)),
    ("shell", Color32::from_rgb(0xcf, 0xdf, 0xe1)),
    ("woods", Color32::from_rgb(0xae, 0xc8, 0xcb)),
    ("grass", Color32::from_rgb(0xb4, 0xe3, 0xcc)),
    ("leaf", Color32::from_rgb(0xd1, 0xd6, 0xb7)),
    ("wine", Color32::from_rgb(0xd8, 0xba, 0xba)),
    ("blush", Color32::from_rgb(0xed, 0xbc, 0xb5)),
    ("honey", Color32::from_rgb(0xf5, 0xd2, 0xaa)),
];

/// An account's default color: the sum of the address's UTF-16 units picks one of twelve.
pub(crate) fn default_color(email: &str) -> Color32 {
    COLORS[default_index(email)].1
}

fn default_index(email: &str) -> usize {
    let sum: u32 = email.encode_utf16().map(u32::from).sum();
    sum as usize % COLORS.len()
}

/// The color `account` shows in: the one picked for it, else its default.
pub(crate) fn color(account: &exo_auth::Account) -> Color32 {
    let picked = account
        .color
        .as_deref()
        .and_then(|name| COLORS.iter().find(|(known, _)| *known == name));
    picked.map_or_else(|| default_color(&account.email), |(_, color)| *color)
}

/// The name of the color `email` shows in, `account` being its listing if it has one.
pub(crate) fn color_name(email: &str, account: Option<&exo_auth::Account>) -> &'static str {
    let picked = account
        .and_then(|account| account.color.as_deref())
        .and_then(|name| COLORS.iter().find(|(known, _)| *known == name));
    picked.unwrap_or(&COLORS[default_index(email)]).0
}

/// The letter an account's placeholder avatar shows: the address's first, upper-cased.
pub(crate) fn initial(email: &str) -> String {
    email
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default()
}

/// The alert that confirms signing out of an account with something unsent.
pub(crate) const CONFIRM_TITLE: &str = "One last thing…";
/// Its actions, right to left: `ok`, then `cancel`.
pub(crate) const CONFIRM_ACTIONS: [&str; 2] = ["Sign Out", "Cancel"];

/// What the alert says about `unsent`, or nothing when everything is sent.
pub(crate) fn confirm_text(unsent: Unsent) -> Option<String> {
    Some(match unsent {
        Unsent {
            sends: 0,
            changes: 0,
        } => return None,
        Unsent { sends: 1, .. } => "1 email is still sending. Ignore and sign out anyway?".into(),
        Unsent { sends: 0, changes } => format!(
            "Exo is still uploading {changes} change{}. Ignore and sign out anyway?",
            if changes == 1 { "" } else { "s" }
        ),
        Unsent { sends, .. } => {
            format!("{sends} emails are still sending. Ignore and sign out anyway?")
        }
    })
}

/// Registers the account commands and keys: ctrl+1…9 switch to an account by its place in the
/// list (Alt elsewhere), ctrl+0 adds one (Alt+Shift+0 elsewhere).
pub(crate) fn register(commands: &mut Commands<commands::Action>, os: egui::os::OperatingSystem) {
    let act = commands::Action::Accounts;
    commands.add(
        Command::new(
            "SWITCH_ACCOUNTS",
            "Switch Accounts",
            act(Action::OpenSwitcher),
        )
        .aliases(["Profiles"])
        .default_score(1.1)
        .icon("data/switch-accounts")
        .operative_word("Account"),
    );
    commands.add(
        Command::new("ADD_ACCOUNT", "Add Account", act(Action::Add))
            .aliases(["Authorize Account", "Add Email", "Sign In", "Log In"])
            .default_score(0.0)
            .icon("data/switch-accounts")
            .operative_word("add")
            .multiple(),
    );
    commands.add(
        Command::new("SIGN_OUT", "Sign Out", act(Action::SignOut))
            .aliases(["Log Out", "Remove Account"])
            .default_score(0.0)
            .icon("data/switch-accounts")
            .follow("SIGNATURE_SETTINGS")
            .multiple(),
    );
    // All nine are taken: a key past the last account does nothing.
    for n in 1..=9 {
        commands.add(
            Command::shortcut(format!("SWITCH_ACCOUNT_{n}"), act(Action::Switch(n - 1)))
                .key(format!("ctralt+{n}")),
        );
    }
    let add = if os == egui::os::OperatingSystem::Mac {
        "ctrl+0"
    } else {
        "alt+shift+0"
    };
    commands.add(Command::shortcut("ADD_ACCOUNT_KEY", act(Action::Add)).key(add));
}

/// The Accounts palette: no input, a row per
/// account in the list's order with its avatar, a tick on the one on screen and its ctrl+N for
/// the first nine, and `Add Account` under them. A row's avatar is the account's profile photo
/// as soon as it's loaded, its initial on its color until then or without one.
fn switcher(
    ctx: &egui::Context,
    accounts: &[exo_auth::Account],
    current: &str,
) -> SubPalette<commands::Action> {
    let (ctx, accounts, current) = (ctx.clone(), accounts.to_vec(), current.to_owned());
    let rows = move |_: &str| {
        let row = |(index, account): (usize, &exo_auth::Account)| {
            let photo = crate::profile_pictures::photo(&ctx, &account.email);
            let row = Command::new(
                account.email.clone(),
                account.email.clone(),
                commands::Action::Accounts(Action::Switch(index)),
            )
            .avatar(initial(&account.email), color(account), photo);
            let row = if index < 9 {
                row.key_code(["ctralt".to_owned(), (index + 1).to_string()])
            } else {
                row
            };
            if account.email == current {
                row.in_use()
            } else {
                row
            }
        };
        accounts.iter().enumerate().map(row).collect()
    };
    SubPalette::ordered("SWITCH_ACCOUNTS", "Accounts", "data/switch-accounts", rows)
        .hide_input()
        .footer(|_| {
            Some(
                Command::new(
                    "ADD_ACCOUNT_FOOTER",
                    "Add Account",
                    commands::Action::Accounts(Action::Add),
                )
                .icon("data/add"),
            )
        })
}

impl Mail {
    /// The device's accounts, in the user's order, for the switchers and the tab title.
    pub fn set_accounts(&mut self, accounts: Vec<exo_auth::Account>) {
        self.accounts = accounts;
    }

    /// The accounts the switchers list.
    pub fn accounts(&self) -> &[exo_auth::Account] {
        &self.accounts
    }

    /// When the account on screen first signed in to Exo on this Mac, Exo's join date; `None`
    /// when the device's list doesn't say.
    pub(crate) fn joined(&self) -> Option<jiff::Timestamp> {
        self.accounts
            .iter()
            .find(|account| account.email == self.account.email())
            .and_then(|account| account.joined)
    }

    /// Carries out an account command, key or row.
    pub(crate) fn accounts_run(&mut self, action: Action, now: f64) {
        match action {
            Action::SignOut => self.sign_out(),
            Action::OpenSwitcher => {
                let switcher = switcher(&self.ctx, &self.accounts, self.account.email());
                self.commands.open_sub_palette(switcher);
            }
            Action::Switch(index) => {
                let Some(email) = self
                    .accounts
                    .get(index)
                    .map(|account| account.email.clone())
                else {
                    return;
                };
                if email == self.account.email() {
                    // The account on screen goes home and to the top.
                    let home = splits::home(self.account.settings());
                    self.run(commands::Action::Go(home), now);
                } else {
                    self.request = Some(Request::Switch(email));
                }
            }
            Action::Add => self.request = Some(Request::Add),
        }
    }

    /// What the Accounts dialog asked for. Signing out of the account on screen asks it what
    /// it hasn't sent, as the palette's `Sign Out` does; the app counts another account's.
    pub(crate) fn account_settings_event(&mut self, event: crate::settings::AccountEvent) {
        use crate::settings::AccountEvent;
        self.request = Some(match event {
            AccountEvent::SignOut(email) if email == self.account.email() => {
                return self.sign_out();
            }
            AccountEvent::SignOut(email) => Request::CheckSignOut(email),
            AccountEvent::Move { email, to } => Request::Move { email, to },
            AccountEvent::Color { email, color } => Request::Color { email, color },
            AccountEvent::Add => Request::Add,
        });
    }

    /// `Sign Out`, from the palette or the settings menu: asks the account what Gmail doesn't
    /// have yet, and [`Mail::sign_out_unsent`] goes on from its answer.
    pub(crate) fn sign_out(&mut self) {
        self.signing_out = true;
        self.account.ask_unsent();
    }

    /// The account's answer to [`Mail::sign_out`]: confirm when something isn't sent yet, else
    /// ask the app to sign out.
    pub(crate) fn sign_out_unsent(&mut self, unsent: Unsent) {
        if !std::mem::take(&mut self.signing_out) {
            return;
        }
        let email = self.account.email().to_owned();
        match confirm_text(unsent) {
            Some(text) => self.pickers.confirm_sign_out(email, text),
            None => self.request = Some(Request::SignOut(email)),
        }
    }

    /// Asks before signing `email`, another account, out: `unsent` is what it hasn't sent,
    /// counted by the app. Returns whether there was anything to ask about.
    pub(crate) fn confirm_sign_out(&mut self, email: String, unsent: Unsent) -> bool {
        let Some(text) = confirm_text(unsent) else {
            return false;
        };
        self.pickers.confirm_sign_out(email, text);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_alert_counts_sends_before_other_changes() {
        let text = |sends, changes| confirm_text(Unsent { sends, changes });
        assert_eq!(text(0, 0), None);
        assert_eq!(
            text(1, 3).as_deref(),
            Some("1 email is still sending. Ignore and sign out anyway?")
        );
        assert_eq!(
            text(2, 0).as_deref(),
            Some("2 emails are still sending. Ignore and sign out anyway?")
        );
        assert_eq!(
            text(0, 1).as_deref(),
            Some("Exo is still uploading 1 change. Ignore and sign out anyway?")
        );
        assert_eq!(
            text(0, 4).as_deref(),
            Some("Exo is still uploading 4 changes. Ignore and sign out anyway?")
        );
    }

    #[test]
    fn account_colors_hash_the_address_unless_one_was_picked() {
        // `a` is 97: 97 % 12 = 1, orchid; `maya@brightmoor.co` sums to 1829: 5, shell.
        assert_eq!(default_color("a"), Color32::from_rgb(0xe4, 0xcd, 0xe0));
        let maya = exo_auth::Account {
            email: "maya@brightmoor.co".into(),
            ..exo_auth::Account::default()
        };
        assert_eq!(color(&maya), Color32::from_rgb(0xcf, 0xdf, 0xe1));
        assert_eq!(
            color(&exo_auth::Account {
                color: Some("sea".into()),
                ..maya.clone()
            }),
            Color32::from_rgb(0xb2, 0xca, 0xe5)
        );
        assert_eq!(
            color(&exo_auth::Account {
                color: Some("mauve".into()),
                ..maya
            }),
            Color32::from_rgb(0xcf, 0xdf, 0xe1),
            "an unknown name is ignored"
        );
    }

    #[test]
    fn the_switcher_lists_every_account_and_adding_one() {
        let accounts: Vec<exo_auth::Account> = ["maya@brightmoor.co", "bea@example.com"]
            .map(|email| exo_auth::Account {
                email: email.into(),
                ..exo_auth::Account::default()
            })
            .into();
        assert_eq!(
            switcher(&egui::Context::default(), &accounts, "bea@example.com").titles(""),
            ["maya@brightmoor.co", "bea@example.com", "Add Account"]
        );
    }
}
