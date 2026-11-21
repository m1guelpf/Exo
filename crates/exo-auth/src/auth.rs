//! The accounts on this device: signing them in and out, and their tokens.

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use gmail::Endpoints;
use gmail::oauth::OAuthClient;
use url::Url;

use crate::accounts::Registry;
use crate::profile_pictures::ProfilePictures;
use crate::sign_in::{self, Grant, Purpose};
use crate::{Account, ClientConfig, Error, SecretStore, Tokens};

/// Exo's signed-in accounts. Cheap to clone; clones share everything.
#[derive(Clone)]
pub struct Auth(Arc<Inner>);

struct Inner {
    client: ClientConfig,
    oauth: OAuthClient,
    secrets: Box<dyn SecretStore>,
    pictures: ProfilePictures,
    state: Mutex<State>,
}

struct State {
    registry: Registry,
    /// One provider per account, so a sign-in reaches the clients already using it.
    tokens: HashMap<String, Tokens>,
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Auth")
            .field("client", &self.0.client)
            .finish_non_exhaustive()
    }
}

impl Auth {
    /// The accounts saved in `state_dir` (Exo's state folder), signing in with `client` and
    /// keeping refresh tokens in `secrets` (the [`Keychain`](crate::Keychain)).
    ///
    /// # Errors
    /// [`Error::Accounts`] when the account list can't be read.
    pub fn open(
        client: ClientConfig,
        state_dir: &Path,
        secrets: impl SecretStore,
    ) -> Result<Self, Error> {
        Ok(Self(Arc::new(Inner {
            oauth: client.oauth(),
            client,
            secrets: Box::new(secrets),
            pictures: ProfilePictures::new(state_dir),
            state: Mutex::new(State {
                registry: Registry::load(state_dir)?,
                tokens: HashMap::new(),
            }),
        })))
    }

    /// The Google APIs the accounts' clients talk to.
    #[must_use]
    pub fn endpoints(&self) -> &Endpoints {
        self.0.client.endpoints()
    }

    /// The accounts, in the user's order. Signed-out ones (whose [`Tokens`] need a new sign-in)
    /// stay listed.
    #[must_use]
    pub fn accounts(&self) -> Vec<Account> {
        self.state().registry.accounts().to_vec()
    }

    /// The account to open on launch: the one used last, or else the first not signed out.
    #[must_use]
    pub fn default_account(&self) -> Option<Account> {
        self.state().registry.default_account().cloned()
    }

    /// `email` is the account on screen now, so launch opens it next time.
    ///
    /// # Errors
    /// [`Error::UnknownAccount`], or [`Error::Accounts`] when the list can't be saved.
    pub fn use_account(&self, email: &str) -> Result<(), Error> {
        self.state().registry.use_account(email)
    }

    /// Google refused `email`'s grant ([`Tokens::needs_sign_in`]): it stays listed but opens
    /// the sign-in screen until it signs in again.
    ///
    /// # Errors
    /// [`Error::UnknownAccount`], or [`Error::Accounts`] when the list can't be saved.
    pub fn mark_signed_out(&self, email: &str) -> Result<(), Error> {
        self.state().registry.sign_out(email)
    }

    /// Sets the color `email` shows in, by name (`None`: computed from its address).
    ///
    /// # Errors
    /// [`Error::UnknownAccount`], or [`Error::Accounts`] when the list can't be saved.
    pub fn set_color(&self, email: &str, color: Option<String>) -> Result<(), Error> {
        self.state().registry.set_color(email, color)
    }

    /// `email`'s profile photo as last saved ([`Auth::set_profile_picture`]), `None` without
    /// one.
    ///
    /// # Errors
    /// [`Error::ProfilePicture`] when it can't be read.
    pub fn profile_picture(&self, email: &str) -> Result<Option<Vec<u8>>, Error> {
        self.0.pictures.get(email)
    }

    /// Saves `email`'s profile photo, the image as Google serves it (`None`: Google has none
    /// for it). It stays until it's replaced or the account signs out ([`Auth::forget`]).
    ///
    /// # Errors
    /// [`Error::UnknownAccount`], also for an account that signed out while its photo was
    /// fetched, or [`Error::ProfilePicture`] when it can't be written.
    pub fn set_profile_picture(&self, email: &str, photo: Option<&[u8]>) -> Result<(), Error> {
        // Under the list's lock, so a sign-out can't come between the check and the write.
        let state = self.state();
        if !state.registry.contains(email) {
            return Err(Error::UnknownAccount(email.to_owned()));
        }
        self.0.pictures.set(email, photo)
    }

    /// Signs an account in: opens Google's consent screen with `open` (which should show the URL
    /// in the system browser), waits up to [`TIMEOUT`](crate::TIMEOUT) for the redirect, and saves
    /// the account. A new account goes at the end of the list; one that's already listed (a
    /// re-authentication) keeps its place, and its [`Tokens`] pick up the new grant. Either way
    /// it becomes the account used last.
    ///
    /// `login_hint` preselects an account at Google (`Sign in as {email}`). The user can still
    /// pick another one, which is the account returned. Drop the future to cancel.
    ///
    /// # Errors
    /// [`Error::AccessDenied`] when the user declines (the sign-in screen says nothing then),
    /// [`Error::TimedOut`], [`Error::MissingScopes`], [`Error::NoMailbox`], and the transport
    /// and storage errors. Redirects carrying another attempt's `state` are turned away without
    /// ending this one.
    pub async fn sign_in(
        &self,
        login_hint: Option<&str>,
        open: impl FnOnce(&Url) -> io::Result<()>,
    ) -> Result<Account, Error> {
        sign_in::authorize(
            &self.0.oauth,
            self.endpoints(),
            Purpose::Mail,
            login_hint,
            open,
            |grant| self.save(grant),
        )
        .await
    }

    /// Signs a Google account in for mail account `owner`'s calendar: the same flow as
    /// [`Auth::sign_in`], asking for [`CALENDAR_SCOPES`](crate::CALENDAR_SCOPES) alone. Its grant
    /// is `owner`'s, kept until [`Auth::forget_calendar`] or `owner` signs out; signing it in again
    /// replaces it, and its [`Tokens`] pick the new one up. Returns the address Google signed in;
    /// `owner`'s own needs no grant of its own (its mail's covers its calendar), so nothing is
    /// saved for it.
    ///
    /// # Errors
    /// [`Error::WrongAccount`] when `login_hint` names an account and the user picked another,
    /// [`Error::UnknownAccount`] for an `owner` that isn't listed, and [`Auth::sign_in`]'s.
    pub async fn sign_in_calendar(
        &self,
        owner: &str,
        login_hint: Option<&str>,
        open: impl FnOnce(&Url) -> io::Result<()>,
    ) -> Result<String, Error> {
        sign_in::authorize(
            &self.0.oauth,
            self.endpoints(),
            Purpose::Calendar,
            login_hint,
            open,
            |grant| {
                let email = grant.account.email;
                if let Some(expected) =
                    login_hint.filter(|expected| !expected.eq_ignore_ascii_case(&email))
                {
                    return Err(Error::WrongAccount {
                        expected: expected.to_owned(),
                    });
                }
                if email.eq_ignore_ascii_case(owner) {
                    return Ok(email);
                }
                if !self.state().registry.contains(owner) {
                    return Err(Error::UnknownAccount(owner.to_owned()));
                }
                let key = calendar_key(owner, &email);
                self.0.secrets.set(&key, &grant.refresh_token)?;
                let mut state = self.state();
                state.registry.add_calendar(owner, &email)?;
                state
                    .tokens
                    .entry(key.clone())
                    .or_insert_with(|| Tokens::new(&email, self.0.oauth.clone(), None))
                    .replace(grant.refresh_token, grant.access);
                log::info!("signed a calendar account in");
                Ok(email)
            },
        )
        .await
    }

    /// Calendar account `email`'s access tokens, for `owner`'s Calendar calls; every call
    /// returns the same provider. Reads the Keychain the first time, as [`Auth::tokens`] does.
    ///
    /// # Errors
    /// [`Error::UnknownAccount`] when `owner` holds no grant for `email`, or
    /// [`Error::Secrets`].
    pub fn calendar_tokens(&self, owner: &str, email: &str) -> Result<Tokens, Error> {
        let key = calendar_key(owner, email);
        let holds = |state: &State| {
            state.registry.accounts().iter().any(|account| {
                account.email == owner && account.calendars.iter().any(|known| known == email)
            })
        };
        {
            let state = self.state();
            if !holds(&state) {
                return Err(Error::UnknownAccount(email.to_owned()));
            }
            if let Some(tokens) = state.tokens.get(&key) {
                return Ok(tokens.clone());
            }
        }
        let grant = self.0.secrets.get(&key)?;
        let mut state = self.state();
        if !holds(&state) {
            return Err(Error::UnknownAccount(email.to_owned()));
        }
        Ok(state
            .tokens
            .entry(key)
            .or_insert_with(|| Tokens::new(email, self.0.oauth.clone(), grant))
            .clone())
    }

    /// Forgets `owner`'s grant for calendar account `email`: its Keychain item goes and its
    /// [`Tokens`] stop working. Google's grant is left alone, as [`Auth::forget`] leaves it.
    ///
    /// # Errors
    /// [`Error::UnknownAccount`] for an `owner` that isn't listed, or a storage error.
    pub fn forget_calendar(&self, owner: &str, email: &str) -> Result<(), Error> {
        let key = calendar_key(owner, email);
        self.0.secrets.delete(&key)?;
        let mut state = self.state();
        if let Some(tokens) = state.tokens.remove(&key) {
            tokens.revoke();
        }
        state.registry.remove_calendar(owner, email)
    }

    fn save(&self, grant: Grant) -> Result<Account, Error> {
        let Grant {
            account,
            refresh_token,
            access,
        } = grant;
        let email = account.email.clone();
        self.0.secrets.set(&email, &refresh_token)?;
        let mut state = self.state();
        state.registry.sign_in(account)?;
        state
            .tokens
            .entry(email.clone())
            .or_insert_with(|| Tokens::new(&email, self.0.oauth.clone(), None))
            .replace(refresh_token, access);
        log::info!("signed an account in");
        let saved = state
            .registry
            .accounts()
            .iter()
            .find(|known| known.email == email);
        Ok(saved.cloned().expect("the account was just saved"))
    }

    /// `email`'s access tokens, for its Gmail, Calendar and People clients. Every call for an
    /// account returns the same provider. Reads the Keychain the first time, which can block on
    /// an access prompt, so call it off the UI thread.
    ///
    /// # Errors
    /// [`Error::UnknownAccount`], or [`Error::Secrets`] when the Keychain can't be read.
    pub fn tokens(&self, email: &str) -> Result<Tokens, Error> {
        if let Some(tokens) = self.known(email)? {
            return Ok(tokens);
        }
        let grant = self.0.secrets.get(email)?;
        let mut state = self.state();
        if !state.registry.contains(email) {
            return Err(Error::UnknownAccount(email.to_owned()));
        }
        let tokens = state
            .tokens
            .entry(email.to_owned())
            .or_insert_with(|| Tokens::new(email, self.0.oauth.clone(), grant));
        Ok(tokens.clone())
    }

    /// The provider already made for `email`, if any.
    fn known(&self, email: &str) -> Result<Option<Tokens>, Error> {
        let state = self.state();
        if !state.registry.contains(email) {
            return Err(Error::UnknownAccount(email.to_owned()));
        }
        Ok(state.tokens.get(email).cloned())
    }

    /// Signs `email` out of Exo: forgets its refresh token and its calendar accounts' grants,
    /// drops it from the list and deletes its profile photo. Its [`Tokens`] stop working. Google's grant is left alone: nothing
    /// here holds it any more, and revoking it would also sign out other copies of Exo on the
    /// same OAuth client. The account's local mail store is the caller's to delete.
    ///
    /// # Errors
    /// [`Error::UnknownAccount`], or a storage error.
    pub fn forget(&self, email: &str) -> Result<(), Error> {
        // Its calendar accounts' grants go with it.
        let calendars = self
            .state()
            .registry
            .accounts()
            .iter()
            .find(|account| account.email == email)
            .map(|account| account.calendars.clone())
            .unwrap_or_default();
        for calendar in calendars {
            self.forget_calendar(email, &calendar)?;
        }
        // The secret goes first: a failure after that leaves a listed account that needs a
        // sign-in, never a refresh token nobody can see.
        self.0.secrets.delete(email)?;
        let removed = {
            let mut state = self.state();
            if let Some(tokens) = state.tokens.remove(email) {
                tokens.revoke();
            }
            let removed = state.registry.remove(email)?;
            // A photo that can't be deleted doesn't keep the account signed in.
            if removed && let Err(error) = self.0.pictures.set(email, None) {
                log::error!("deleting a signed-out account's profile photo: {error}");
            }
            removed
        };
        if !removed {
            return Err(Error::UnknownAccount(email.to_owned()));
        }
        log::info!("removed an account");
        Ok(())
    }

    /// Moves `email` to position `to` in the list (the end when `to` is past it).
    ///
    /// # Errors
    /// [`Error::UnknownAccount`], or [`Error::Accounts`] when the list can't be saved.
    pub fn move_account(&self, email: &str, to: usize) -> Result<(), Error> {
        self.state().registry.move_to(email, to)
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.0
            .state
            .lock()
            .expect("account state is never poisoned")
    }
}

/// Where `owner`'s grant for calendar account `email` is kept, in the [`SecretStore`] and the
/// tokens: apart from `email`'s own mail grant, if it has one.
fn calendar_key(owner: &str, email: &str) -> String {
    format!("calendar {email} for {owner}")
}
