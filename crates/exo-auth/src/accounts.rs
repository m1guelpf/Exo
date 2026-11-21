//! The signed-in accounts, in the user's order, kept in `accounts.json` in the state folder.
//!
//! New accounts are appended, signing in again keeps an account's place, and the order is the one
//! ctrl+1…9 and the switchers use. An account Google signed out stays listed until it signs in
//! again, and one account is the one launch opens. Only metadata lives here; refresh tokens are in
//! the [`SecretStore`](crate::SecretStore).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;

/// The file's name in the state folder.
const FILE_NAME: &str = "accounts.json";

/// A Google account that signed in to Exo.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// The Gmail address, which also names the account's local store.
    pub email: String,
    /// The account's full name, when Google has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Google refused its grant: it's listed, but has to sign in again before its mail opens.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub signed_out: bool,
    /// The account used last, which launch opens.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub last_used: bool,
    /// The color picked for it in the Accounts dialog, by name (`thistle`, `sea`, …); without one
    /// it's computed from the address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// When it first signed in to Exo on this Mac: the account's join date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joined: Option<jiff::Timestamp>,
    /// The other Google accounts signed in for its calendars alone, whose grants Exo keeps for it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calendars: Vec<String>,
}

/// The account list and the file it's saved in.
#[derive(Debug)]
pub(crate) struct Registry {
    path: PathBuf,
    accounts: Vec<Account>,
}

impl Registry {
    /// The list saved in `dir`, empty when there's none yet.
    pub(crate) fn load(dir: &Path) -> Result<Self, Error> {
        let path = dir.join(FILE_NAME);
        let accounts = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| Error::Accounts {
                path: path.clone(),
                source: error.into(),
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(source) => return Err(Error::Accounts { path, source }),
        };
        Ok(Self { path, accounts })
    }

    pub(crate) fn accounts(&self) -> &[Account] {
        &self.accounts
    }

    pub(crate) fn contains(&self, email: &str) -> bool {
        self.accounts.iter().any(|account| account.email == email)
    }

    /// The account launch opens: the one used last, unless it's signed out; then the first that
    /// isn't.
    pub(crate) fn default_account(&self) -> Option<&Account> {
        let signed_in = |account: &&Account| !account.signed_out;
        self.accounts
            .iter()
            .filter(signed_in)
            .find(|account| account.last_used)
            .or_else(|| self.accounts.iter().find(signed_in))
    }

    /// Records a sign-in: a new account goes at the end, joined now; one already listed keeps its
    /// place, its color, its join date and whether it's the last used, and is signed in again.
    /// Either way its name is Google's latest, and it becomes the one used last.
    pub(crate) fn sign_in(&mut self, account: Account) -> Result<(), Error> {
        let email = account.email.clone();
        match self
            .accounts
            .iter_mut()
            .find(|known| known.email == account.email)
        {
            Some(known) => {
                known.name = account.name;
                known.signed_out = false;
            }
            None => self.accounts.push(Account {
                joined: Some(jiff::Timestamp::now()),
                ..account
            }),
        }
        self.use_account(&email)
    }

    /// Makes `email` the one used last.
    pub(crate) fn use_account(&mut self, email: &str) -> Result<(), Error> {
        let account = self.find(email)?;
        if account.last_used {
            return Ok(());
        }
        for account in &mut self.accounts {
            account.last_used = account.email == email;
        }
        self.save()
    }

    /// Marks `email` signed out by Google.
    pub(crate) fn sign_out(&mut self, email: &str) -> Result<(), Error> {
        let account = self.find(email)?;
        if account.signed_out {
            return Ok(());
        }
        account.signed_out = true;
        self.save()
    }

    /// Sets the color `email` shows in (`None`: the one computed from its address).
    pub(crate) fn set_color(&mut self, email: &str, color: Option<String>) -> Result<(), Error> {
        self.find(email)?.color = color;
        self.save()
    }

    /// `owner` holds a grant for calendar account `email`.
    pub(crate) fn add_calendar(&mut self, owner: &str, email: &str) -> Result<(), Error> {
        let account = self.find(owner)?;
        if account.calendars.iter().any(|known| known == email) {
            return Ok(());
        }
        account.calendars.push(email.to_owned());
        self.save()
    }

    /// `owner` no longer holds a grant for calendar account `email`.
    pub(crate) fn remove_calendar(&mut self, owner: &str, email: &str) -> Result<(), Error> {
        let account = self.find(owner)?;
        let before = account.calendars.len();
        account.calendars.retain(|known| known != email);
        if account.calendars.len() == before {
            return Ok(());
        }
        self.save()
    }

    fn find(&mut self, email: &str) -> Result<&mut Account, Error> {
        self.accounts
            .iter_mut()
            .find(|account| account.email == email)
            .ok_or_else(|| Error::UnknownAccount(email.to_owned()))
    }

    /// Takes `email` out of the list. Whether it was there.
    pub(crate) fn remove(&mut self, email: &str) -> Result<bool, Error> {
        let before = self.accounts.len();
        self.accounts.retain(|account| account.email != email);
        if self.accounts.len() == before {
            return Ok(false);
        }
        self.save().map(|()| true)
    }

    /// Moves `email` to position `to` (clamped to the end), shifting the accounts in between.
    pub(crate) fn move_to(&mut self, email: &str, to: usize) -> Result<(), Error> {
        let from = self
            .accounts
            .iter()
            .position(|account| account.email == email)
            .ok_or_else(|| Error::UnknownAccount(email.to_owned()))?;
        if from == to.min(self.accounts.len() - 1) {
            return Ok(());
        }
        let account = self.accounts.remove(from);
        self.accounts.insert(to.min(self.accounts.len()), account);
        self.save()
    }

    /// Writes the list through a temporary file, so a crash never leaves half of it.
    fn save(&self) -> Result<(), Error> {
        let failed = |source| Error::Accounts {
            path: self.path.clone(),
            source,
        };
        let json = serde_json::to_vec_pretty(&self.accounts).expect("accounts always serialize");
        let partial = self.path.with_extension("json.partial");
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(failed)?;
        }
        std::fs::write(&partial, json).map_err(failed)?;
        std::fs::rename(&partial, &self.path).map_err(failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(email: &str) -> Account {
        Account {
            email: email.into(),
            ..Account::default()
        }
    }

    fn emails(registry: &Registry) -> Vec<&str> {
        registry
            .accounts()
            .iter()
            .map(|account| account.email.as_str())
            .collect()
    }

    #[test]
    fn keeps_order_across_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = Registry::load(dir.path()).unwrap();
        assert!(registry.accounts().is_empty());
        for email in ["a@x.com", "b@x.com", "c@x.com"] {
            registry.sign_in(account(email)).unwrap();
        }
        registry
            .sign_in(Account {
                name: Some("Bea".into()),
                ..account("b@x.com")
            })
            .unwrap();
        registry.move_to("c@x.com", 0).unwrap();
        assert!(registry.remove("a@x.com").unwrap());
        assert!(!registry.remove("a@x.com").unwrap());

        let reloaded = Registry::load(dir.path()).unwrap();
        assert_eq!(emails(&reloaded), ["c@x.com", "b@x.com"]);
        assert_eq!(reloaded.accounts()[1].name.as_deref(), Some("Bea"));
        assert!(!dir.path().join("accounts.json.partial").exists());
    }

    #[test]
    fn moves_clamp_to_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = Registry::load(&dir.path().join("new")).unwrap();
        for email in ["a@x.com", "b@x.com", "c@x.com"] {
            registry.sign_in(account(email)).unwrap();
        }
        registry.move_to("a@x.com", 99).unwrap();
        assert_eq!(emails(&registry), ["b@x.com", "c@x.com", "a@x.com"]);
        registry.move_to("a@x.com", 1).unwrap();
        assert_eq!(emails(&registry), ["b@x.com", "a@x.com", "c@x.com"]);
        assert!(matches!(
            registry.move_to("z@x.com", 0),
            Err(Error::UnknownAccount(email)) if email == "z@x.com"
        ));
    }

    #[test]
    fn launch_opens_the_last_used_account_unless_google_signed_it_out() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = Registry::load(dir.path()).unwrap();
        assert_eq!(registry.default_account(), None);
        for email in ["a@x.com", "b@x.com", "c@x.com"] {
            registry.sign_in(account(email)).unwrap();
        }
        let default = |registry: &Registry| {
            registry
                .default_account()
                .map(|account| account.email.clone())
        };
        assert_eq!(
            default(&registry).as_deref(),
            Some("c@x.com"),
            "the last to sign in"
        );
        registry.use_account("b@x.com").unwrap();
        assert_eq!(
            default(&Registry::load(dir.path()).unwrap()).as_deref(),
            Some("b@x.com"),
            "kept across launches"
        );

        registry.sign_out("b@x.com").unwrap();
        registry.sign_out("a@x.com").unwrap();
        assert_eq!(
            default(&registry).as_deref(),
            Some("c@x.com"),
            "the first still signed in"
        );
        registry.sign_out("c@x.com").unwrap();
        assert_eq!(default(&registry), None);

        registry.set_color("a@x.com", Some("sea".into())).unwrap();
        let joined = registry.accounts()[0].joined;
        assert!(joined.is_some(), "a new account joins as it first signs in");
        registry
            .sign_in(Account {
                name: Some("Ann".into()),
                ..account("a@x.com")
            })
            .unwrap();
        let reloaded = Registry::load(dir.path()).unwrap();
        let a = &reloaded.accounts()[0];
        assert_eq!(
            (
                a.email.as_str(),
                a.name.as_deref(),
                a.color.as_deref(),
                a.signed_out
            ),
            ("a@x.com", Some("Ann"), Some("sea"), false),
            "signing in again keeps its place and color"
        );
        assert_eq!(a.joined, joined, "and its join date, across launches");
        assert_eq!(default(&reloaded).as_deref(), Some("a@x.com"));
    }

    #[test]
    fn fields_left_out_take_their_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(FILE_NAME),
            r#"[{"email": "a@x.com", "name": "Ann"}]"#,
        )
        .unwrap();
        let registry = Registry::load(dir.path()).unwrap();
        assert_eq!(
            registry.accounts(),
            [Account {
                name: Some("Ann".into()),
                ..account("a@x.com")
            }]
        );
        assert_eq!(
            registry
                .default_account()
                .map(|account| account.email.as_str()),
            Some("a@x.com")
        );
    }

    #[test]
    fn a_corrupt_file_is_an_error_not_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE_NAME), "[{").unwrap();
        assert!(matches!(
            Registry::load(dir.path()),
            Err(Error::Accounts { .. })
        ));
    }
}
