//! Google's installed-app flow (RFC 8252 with PKCE): the system browser, a loopback redirect, the
//! code exchange, and checks that the account can be used.

use std::io;
use std::net::Ipv4Addr;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use gmail::oauth::{OAuthClient, Pkce, scopes};
use gmail::{Endpoints, StaticToken};
use jiff::Timestamp;
use tokio::net::TcpListener;
use url::Url;

use crate::tokens::Access;
use crate::{Account, CALENDAR_SCOPES, Error, SCOPES, loopback};

/// How long sign-in waits for the browser to come back. It's long enough to type a password and
/// a second factor; the sign-in screen can give up sooner by dropping the future.
pub const TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// What an account signs in for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    /// Its mail, and everything else Exo does with it ([`SCOPES`]).
    Mail,
    /// Its calendars alone ([`CALENDAR_SCOPES`]), for another account's calendar.
    Calendar,
}

impl Purpose {
    fn scopes(self) -> &'static [&'static str] {
        match self {
            Self::Mail => SCOPES,
            Self::Calendar => CALENDAR_SCOPES,
        }
    }
}

/// A signed-in account and its tokens.
pub(crate) struct Grant {
    pub account: Account,
    pub refresh_token: String,
    pub access: Access,
}

/// Runs the flow for `purpose`: opens the browser at Google's consent screen, waits for the
/// redirect, exchanges the code, hands the grant to `commit`, and tells the browser how it went.
pub(crate) async fn authorize<T>(
    oauth: &OAuthClient,
    endpoints: &Endpoints,
    purpose: Purpose,
    login_hint: Option<&str>,
    open: impl FnOnce(&Url) -> io::Result<()>,
    commit: impl FnOnce(Grant) -> Result<T, Error>,
) -> Result<T, Error> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(Error::Loopback)?;
    let port = listener.local_addr().map_err(Error::Loopback)?.port();
    // The loopback IP rather than `localhost` (RFC 8252, section 8.3), which Google accepts on
    // any port for Desktop app clients.
    let redirect_uri = format!("http://127.0.0.1:{port}");
    let pkce = Pkce::generate();
    let state = random_state();
    let mut url = oauth.authorization_url(&redirect_uri, purpose.scopes(), &state, &pkce);
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("include_granted_scopes", "true");
        if let Some(hint) = login_hint {
            query.append_pair("login_hint", hint);
        }
    }
    open(&url).map_err(Error::Browser)?;

    let (redirect, stream) = tokio::time::timeout(TIMEOUT, loopback::receive(&listener, &state))
        .await
        .map_err(|_| Error::TimedOut)?
        .map_err(Error::Loopback)?;
    let result = match exchange(oauth, endpoints, purpose, &redirect, &pkce, &redirect_uri).await {
        Ok(grant) => commit(grant),
        Err(error) => Err(error),
    };
    loopback::respond(stream, &result).await;
    result
}

/// Exchanges the redirect's code (its `state` is this attempt's), and checks the account it
/// signed in: every scope `purpose` asks for, and for mail, a mailbox.
async fn exchange(
    oauth: &OAuthClient,
    endpoints: &Endpoints,
    purpose: Purpose,
    redirect: &Url,
    pkce: &Pkce,
    redirect_uri: &str,
) -> Result<Grant, Error> {
    let param = |name| loopback::param(redirect, name);
    match param("error") {
        Some(error) if error == "access_denied" => return Err(Error::AccessDenied),
        Some(error) => return Err(Error::Authorization { error }),
        None => {}
    }
    let Some(code) = param("code") else {
        return Err(Error::Authorization {
            error: "no authorization code".into(),
        });
    };

    let issued = Timestamp::now();
    let tokens = oauth
        .exchange_code(&code, &pkce.verifier, redirect_uri)
        .await?;
    // Without `scope`, Google granted exactly what was asked (RFC 6749, section 5.1).
    if let Some(granted) = &tokens.scope {
        let granted: Vec<&str> = granted.split_whitespace().collect();
        let missing: Vec<String> = purpose
            .scopes()
            .iter()
            .filter(|scope| !granted.contains(scope))
            .map(|scope| (*scope).to_owned())
            .collect();
        if !missing.is_empty() {
            return Err(Error::MissingScopes(missing));
        }
    }
    let refresh_token = tokens.refresh_token.ok_or(Error::NoRefreshToken)?;

    let google = gmail::Client::new(endpoints.clone(), StaticToken(tokens.access_token.clone()));
    let mailbox = async {
        if purpose == Purpose::Calendar {
            return Ok(());
        }
        google
            .gmail()
            .profile()
            .into_future()
            .await
            .map(drop)
            .map_err(|error| match &error {
                gmail::Error::Api(api)
                    if api.code == 400 && api.message.contains("Mail service not enabled") =>
                {
                    Error::NoMailbox
                }
                _ => error.into(),
            })
    };
    let (me, mailbox) = tokio::join!(google.userinfo().into_future(), mailbox);
    mailbox?;
    let me = me?;
    let email = me
        .email
        .ok_or_else(|| Error::MissingScopes(vec![scopes::USERINFO_EMAIL.to_owned()]))?;
    Ok(Grant {
        account: Account {
            email,
            name: me.name,
            ..Account::default()
        },
        refresh_token,
        access: Access::new(tokens.access_token, issued, tokens.expires_in),
    })
}

/// 128 random bits, base64url.
fn random_state() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("the OS random source is available");
    URL_SAFE_NO_PAD.encode(bytes)
}
