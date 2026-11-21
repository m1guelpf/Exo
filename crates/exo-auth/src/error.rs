//! What can go wrong signing in and keeping an account signed in.

use std::path::PathBuf;

/// Everything that can go wrong in this crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No OAuth client file where Exo looks for one.
    #[error("{}", missing_client(path, *from_env))]
    MissingClient {
        /// Where Exo looked.
        path: PathBuf,
        /// Whether `path` came from `EXO_GOOGLE_OAUTH_CLIENT`.
        from_env: bool,
    },
    /// The OAuth client file can't be read or isn't a Desktop app client.
    #[error("the Google OAuth client file at {} {reason}", path.display())]
    InvalidClient {
        /// The file.
        path: PathBuf,
        /// What's wrong with it.
        reason: String,
    },
    /// The system browser couldn't be opened.
    #[error("couldn't open the browser: {0}")]
    Browser(#[source] std::io::Error),
    /// The loopback listener for Google's redirect failed.
    #[error("waiting for Google's redirect: {0}")]
    Loopback(#[source] std::io::Error),
    /// The browser didn't come back within [`crate::TIMEOUT`].
    #[error("sign-in timed out")]
    TimedOut,
    /// The user declined on Google's consent screen.
    #[error("access was denied")]
    AccessDenied,
    /// Google redirected back with an error other than `access_denied`.
    #[error("Google refused the sign-in: {error}")]
    Authorization {
        /// Google's error code, e.g. `admin_policy_enforced`.
        error: String,
    },
    /// The user unticked some of [`crate::SCOPES`] on the consent screen.
    #[error("Google granted only some permissions; missing {}", .0.join(", "))]
    MissingScopes(Vec<String>),
    /// The Google account has no Gmail mailbox.
    #[error("this Google account has no Gmail mailbox")]
    NoMailbox,
    /// The code exchange returned no refresh token.
    #[error("Google returned no refresh token")]
    NoRefreshToken,
    /// Google's grant for this account was revoked or expired: the user has to sign in again.
    #[error("{email} needs to sign in again")]
    SignInAgain {
        /// The account.
        email: String,
    },
    /// No account with this address has signed in.
    #[error("no account {0}")]
    UnknownAccount(String),
    /// A sign-in asked for one account and Google signed in another: nothing was saved.
    #[error("signed in another account than {expected}")]
    WrongAccount {
        /// The account asked for.
        expected: String,
    },
    /// Talking to Google failed.
    #[error(transparent)]
    Google(#[from] gmail::Error),
    /// The account list couldn't be read or written.
    #[error("account list at {}: {source}", path.display())]
    Accounts {
        /// The file.
        path: PathBuf,
        /// What failed.
        #[source]
        source: std::io::Error,
    },
    /// An account's profile photo couldn't be read or written.
    #[error("profile photo at {}: {source}", path.display())]
    ProfilePicture {
        /// The file.
        path: PathBuf,
        /// What failed.
        #[source]
        source: std::io::Error,
    },
    /// The secret store (the Keychain) failed.
    #[error("keychain: {0}")]
    Secrets(#[source] Box<dyn std::error::Error + Send + Sync>),
}

fn missing_client(path: &std::path::Path, from_env: bool) -> String {
    let (path, env) = (path.display(), crate::config::ENV);
    if from_env {
        format!("{env} is set to {path}, but there's no file there")
    } else {
        format!(
            "Exo has no Google OAuth client. In Google Cloud Console, create an OAuth client of \
             type \"Desktop app\", download its JSON and save it as {path} (or set {env} to its \
             path)"
        )
    }
}
