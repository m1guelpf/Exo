//! Signing in from the UI thread: exo-auth's installed-app flow, run on a runtime of its own,
//! its outcome picked up on a later frame, and the signed-in account's mail opened from it.

use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use exo_auth::{Auth, Tokens};
use tokio::{runtime::Runtime, sync::oneshot, task::JoinHandle};
use url::Url;

use crate::account::{Account, Credentials, OpenError};
use crate::power::PowerSource;

/// Shows Google's consent page: the system browser in the app, a scripted one in tests.
pub type Browser = Arc<dyn Fn(&Url) -> io::Result<()> + Send + Sync>;

/// Opens `url` in the system browser.
///
/// # Errors
/// When `open` can't run or fails.
pub fn system_browser(url: &Url) -> io::Result<()> {
    let status = std::process::Command::new("/usr/bin/open")
        .arg(url.as_str())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("open exited with {status}")))
    }
}

/// What one sign-in came to.
pub type Outcome = Result<exo_auth::Account, exo_auth::Error>;

/// The device's accounts: signing them in, and opening their mail.
pub struct SignIn {
    /// Opens the account list with its OAuth client; tried again on each sign-in until it works,
    /// so a client file saved while Exo runs is picked up.
    open: Box<dyn Fn() -> Result<Auth, exo_auth::Error>>,
    auth: Option<Auth>,
    /// Where accounts' mail stores live.
    stores: PathBuf,
    browser: Browser,
    runtime: Runtime,
    attempt: Option<Attempt>,
    /// The power source the accounts it opens sync by off screen.
    power: PowerSource,
}

/// A sign-in in flight. Dropping it abandons it, closing its loopback listener.
struct Attempt {
    task: JoinHandle<()>,
    outcome: oneshot::Receiver<Outcome>,
}

impl Drop for Attempt {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl SignIn {
    /// Accounts opened by `open`, their mail stored in `stores`, consent shown by `browser`.
    ///
    /// # Errors
    /// When the sign-in runtime can't start.
    pub fn new(
        open: impl Fn() -> Result<Auth, exo_auth::Error> + 'static,
        stores: &Path,
        browser: Browser,
    ) -> io::Result<Self> {
        Ok(Self {
            open: Box::new(open),
            auth: None,
            stores: stores.to_owned(),
            browser,
            runtime: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("exo-sign-in")
                .enable_all()
                .build()?,
            attempt: None,
            power: PowerSource::default(),
        })
    }

    /// Has the accounts it opens from now on sync off screen by `power`, the Mac's
    /// ([`PowerSource::watch`]) in the app. Without, they're plugged in.
    pub fn set_power(&mut self, power: PowerSource) {
        self.power = power;
    }

    /// The account list, opened the first time that works.
    ///
    /// # Errors
    /// [`exo_auth::Error::MissingClient`] or [`exo_auth::Error::InvalidClient`] while there's no
    /// usable OAuth client, or [`exo_auth::Error::Accounts`].
    pub fn auth(&mut self) -> Result<&Auth, exo_auth::Error> {
        let auth = match self.auth.take() {
            Some(auth) => auth,
            None => (self.open)()?,
        };
        Ok(self.auth.insert(auth))
    }

    /// Starts signing an account in, abandoning the attempt in flight if there is one; `ctx`
    /// repaints when it's over. Google offers `hint` first (`login_hint`), when given.
    ///
    /// # Errors
    /// When the account list can't be opened (see [`SignIn::auth`]).
    pub fn start(
        &mut self,
        ctx: &egui::Context,
        hint: Option<&str>,
    ) -> Result<(), exo_auth::Error> {
        let auth = self.auth()?.clone();
        let (browser, ctx, hint) = (self.browser.clone(), ctx.clone(), hint.map(str::to_owned));
        let (sender, outcome) = oneshot::channel();
        let task = self.runtime.spawn(async move {
            let _ = sender.send(auth.sign_in(hint.as_deref(), |url| browser(url)).await);
            ctx.request_repaint();
        });
        self.attempt = Some(Attempt { task, outcome });
        Ok(())
    }

    /// The outcome of the attempt in flight, once it has one.
    pub fn poll(&mut self) -> Option<Outcome> {
        let outcome = self.attempt.as_mut()?.outcome.try_recv().ok()?;
        self.attempt = None;
        Some(outcome)
    }

    /// Forgets `email`'s grant, its Keychain item and its place in the list, as signing out
    /// does.
    ///
    /// # Errors
    /// When the account list can't be opened or written, or the Keychain item deleted.
    pub(crate) fn forget(&mut self, email: &str) -> Result<(), exo_auth::Error> {
        self.auth()?.forget(email)
    }

    /// What `email`'s store holds that Gmail doesn't have yet, read from the store whether or
    /// not its mail is open (a held send can outlive a quit that couldn't reach Gmail).
    ///
    /// # Errors
    /// When the account list can't be opened, or the store read.
    pub(crate) fn unsent(&mut self, email: &str) -> Result<exo_store::Unsent, MailError> {
        let endpoints = self.auth()?.endpoints().clone();
        let stores = self.stores.clone();
        // Counting makes no Gmail call: the store's client is never used.
        let counted = self.runtime.block_on(async move {
            exo_store::Store::open(&stores, email, endpoints, gmail::StaticToken(String::new()))
                .await?
                .unsent()
                .await
        });
        Ok(counted.map_err(OpenError::from)?)
    }

    /// `email` has a store: it has opened on this Mac since it last signed out.
    pub(crate) fn has_store(&self, email: &str) -> bool {
        exo_store::Store::exists(&self.stores, email)
    }

    /// Deletes `email`'s store, once its mail is closed.
    pub(crate) fn delete_store(&self, email: &str) {
        if let Err(error) = exo_store::Store::delete(&self.stores, email) {
            log::error!("deleting a signed-out account's store: {error}");
        }
    }

    /// Clears `email`'s cache, once its mail is closed: its store is made again, keeping what
    /// Gmail doesn't have yet ([`exo_store::Store::clear_cache`]), and fills from Gmail when it
    /// next opens.
    ///
    /// # Errors
    /// When the old store can't be moved aside or the new one made.
    pub(crate) fn clear_cache(&self, email: &str) -> exo_store::Result<()> {
        self.runtime
            .block_on(exo_store::Store::clear_cache(&self.stores, email))
    }

    /// Opens `email`'s mail, repainting `ctx` as it changes. Also returns its tokens, which say
    /// when Google stops honoring the grant. Reads the Keychain the first time.
    ///
    /// # Errors
    /// When the account isn't listed, its grant can't be read, or its store can't be opened.
    pub fn open_mail(
        &mut self,
        email: &str,
        ctx: &egui::Context,
    ) -> Result<(Account, Tokens), MailError> {
        let auth = self.auth()?;
        let tokens = auth.tokens(email)?;
        let credentials = Credentials {
            email: email.to_owned(),
            endpoints: auth.endpoints().clone(),
            tokens: Arc::new(tokens.clone()),
        };
        let calendars = crate::account::CalendarAuth {
            auth: auth.clone(),
            browser: self.browser.clone(),
        };
        let mut account = Account::open(&self.stores, credentials, ctx.clone())?;
        account.follow_power(&self.power);
        account.sign_in_calendars_with(calendars);
        Ok((account, tokens))
    }
}

/// Why an account's mail couldn't be opened.
#[derive(Debug, thiserror::Error)]
pub enum MailError {
    /// The account or its grant couldn't be read.
    #[error(transparent)]
    Auth(#[from] exo_auth::Error),
    /// Its store couldn't be opened.
    #[error(transparent)]
    Open(#[from] OpenError),
}
