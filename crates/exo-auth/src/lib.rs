#![deny(
    clippy::correctness,
    clippy::suspicious,
    clippy::complexity,
    clippy::perf
)]
//! Google sign-in for Exo, the half without UI.
//!
//! Exo has no server to keep refresh tokens and hand out access tokens, so it runs Google's
//! installed-app flow itself (RFC 8252: PKCE and a loopback redirect, see [`Auth::sign_in`]),
//! keeps refresh tokens in the macOS Keychain ([`Keychain`]) and mints access tokens in-process
//! ([`Tokens`]). The list of accounts, in the user's order, lives in `accounts.json` in the app's
//! state folder, and their profile photos beside it ([`Auth::set_profile_picture`]).
//!
//! ```no_run
//! # async fn demo(config_dir: &std::path::Path, state_dir: &std::path::Path) -> Result<(), exo_auth::Error> {
//! # fn open_url(_: &url::Url) -> std::io::Result<()> { Ok(()) }
//! let client = exo_auth::ClientConfig::load(config_dir)?;
//! let auth = exo_auth::Auth::open(client, state_dir, exo_auth::Keychain)?;
//! let account = auth.sign_in(None, open_url).await?;
//! let tokens = auth.tokens(&account.email)?;
//! let gmail = gmail::Client::new(auth.endpoints().clone(), tokens);
//! # Ok(()) }
//! ```

mod accounts;
mod auth;
mod config;
mod error;
mod loopback;
mod profile_pictures;
mod secrets;
mod sign_in;
mod tokens;

pub use accounts::Account;
pub use auth::Auth;
pub use config::ClientConfig;
pub use error::Error;
#[cfg(target_os = "macos")]
pub use secrets::Keychain;
pub use secrets::{MemorySecrets, SecretStore};
pub use sign_in::TIMEOUT;
pub use tokens::Tokens;

use gmail::oauth::scopes;

/// What Exo asks Google for. Every one is required: sign-in fails when the user unticks any of
/// them on Google's consent screen.
pub const SCOPES: &[&str] = &[
    // Read, label, archive, trash, draft and send. Exo never deletes mail permanently (trash is a
    // label), so it doesn't need full access (`https://mail.google.com/`).
    scopes::GMAIL_MODIFY,
    // Filters (block sender, auto-archive), the vacation responder, send-as aliases and
    // signatures.
    scopes::GMAIL_SETTINGS_BASIC,
    // The calendar pane: reading events, RSVPs, creating and editing events.
    scopes::CALENDAR,
    // Recipient autocomplete from saved contacts, which Exo only reads.
    scopes::CONTACTS_READONLY,
    // Recipient autocomplete from "other contacts", the people you've emailed.
    scopes::CONTACTS_OTHER_READONLY,
    // Recipient autocomplete from a Workspace domain's directory (`people:listDirectoryPeople`).
    scopes::DIRECTORY_READONLY,
    // Which account signed in.
    scopes::USERINFO_EMAIL,
    // The account's name and photo, for the account switcher.
    scopes::USERINFO_PROFILE,
];

/// What a calendar account (another Google account signed in for its calendars alone) asks
/// Google for: Calendar and the address only.
pub const CALENDAR_SCOPES: &[&str] = &[scopes::CALENDAR, scopes::USERINFO_EMAIL];
