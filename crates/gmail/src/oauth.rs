//! Google OAuth 2.0 for installed apps: authorization URL with PKCE, code exchange, refresh, and
//! the userinfo profile.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use sha2::{Digest, Sha256};
use url::Url;

use crate::endpoints::OAuthEndpoints;
use crate::error::Error;

/// How long an OAuth request may take. Google answers in well under a second; one that hangs
/// (say, on a pooled connection that died while the Mac slept) mustn't hold up every request
/// waiting on its refresh.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Scopes Exo requests.
pub mod scopes {
    /// Full Gmail access (needed for permanent delete).
    pub const MAIL: &str = "https://mail.google.com/";
    /// Read, compose, send and label mail.
    pub const GMAIL_MODIFY: &str = "https://www.googleapis.com/auth/gmail.modify";
    /// Manage basic mail settings (filters, vacation, signatures).
    pub const GMAIL_SETTINGS_BASIC: &str = "https://www.googleapis.com/auth/gmail.settings.basic";
    /// Read and write calendars.
    pub const CALENDAR: &str = "https://www.googleapis.com/auth/calendar";
    /// Read contacts.
    pub const CONTACTS_READONLY: &str = "https://www.googleapis.com/auth/contacts.readonly";
    /// Read "other contacts" (people you've emailed).
    pub const CONTACTS_OTHER_READONLY: &str =
        "https://www.googleapis.com/auth/contacts.other.readonly";
    /// Read the Workspace directory.
    pub const DIRECTORY_READONLY: &str = "https://www.googleapis.com/auth/directory.readonly";
    /// The user's email address.
    pub const USERINFO_EMAIL: &str = "https://www.googleapis.com/auth/userinfo.email";
    /// The user's basic profile.
    pub const USERINFO_PROFILE: &str = "https://www.googleapis.com/auth/userinfo.profile";
    /// OpenID Connect (adds an `id_token` to token responses).
    pub const OPENID: &str = "openid";
}

/// An OAuth client registration plus the endpoints to use it against.
#[derive(Clone)]
pub struct OAuthClient {
    http: reqwest::Client,
    endpoints: OAuthEndpoints,
    client_id: String,
    client_secret: Option<String>,
}

/// Leaves the secret out, so logging a client never prints it.
impl std::fmt::Debug for OAuthClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthClient")
            .field("endpoints", &self.endpoints)
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

/// A PKCE verifier and its S256 challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    /// Kept secret until the code exchange.
    pub verifier: String,
    /// Sent in the authorization URL.
    pub challenge: String,
}

impl Pkce {
    /// A fresh random verifier (43 characters, the RFC 7636 minimum entropy).
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("the OS random source is available");
        Self::from_verifier(URL_SAFE_NO_PAD.encode(bytes))
    }

    /// The pair for a known verifier.
    #[must_use]
    pub fn from_verifier(verifier: String) -> Self {
        let challenge = s256_challenge(&verifier);
        Self {
            verifier,
            challenge,
        }
    }
}

/// `BASE64URL(SHA256(verifier))`, the `S256` code challenge.
#[must_use]
pub fn s256_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// A successful `/token` response.
#[skip_serializing_none]
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenResponse {
    /// Bearer token for API calls.
    pub access_token: String,
    /// Lifetime of `access_token` in seconds.
    pub expires_in: u64,
    /// Long-lived refresh token. Only on the first code exchange (with `access_type=offline`).
    pub refresh_token: Option<String>,
    /// Space-separated granted scopes.
    pub scope: Option<String>,
    /// Always `Bearer`.
    pub token_type: String,
    /// OpenID Connect ID token, when `openid` was granted.
    pub id_token: Option<String>,
}

/// Leaves the tokens out, so logging a response never prints them.
impl std::fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenResponse")
            .field("expires_in", &self.expires_in)
            .field("scope", &self.scope)
            .field("token_type", &self.token_type)
            .finish_non_exhaustive()
    }
}

/// An error from the `/token` endpoint (`{"error": "invalid_grant", ...}`), which does not use
/// the standard API error envelope.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{error}: {}", error_description.as_deref().unwrap_or("no description"))]
pub struct OAuthError {
    /// Error code, e.g. `invalid_grant`, `invalid_client`, `unsupported_grant_type`.
    pub error: String,
    /// Human-readable detail.
    pub error_description: Option<String>,
}

/// `GET /oauth2/v2/userinfo`. Note the snake_case field names.
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserInfo {
    /// Google account ID.
    pub id: Option<String>,
    /// Primary email address.
    pub email: Option<String>,
    /// Whether the address is verified.
    pub verified_email: Option<bool>,
    /// Full name.
    pub name: Option<String>,
    /// Given name.
    pub given_name: Option<String>,
    /// Family name.
    pub family_name: Option<String>,
    /// Avatar URL.
    pub picture: Option<String>,
    /// BCP 47 locale.
    pub locale: Option<String>,
    /// Workspace domain, for Workspace accounts.
    pub hd: Option<String>,
}

impl OAuthClient {
    /// A client registration. Google requires the secret even for installed apps using PKCE.
    pub fn new(
        endpoints: OAuthEndpoints,
        client_id: impl Into<String>,
        client_secret: Option<String>,
    ) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(TIMEOUT)
                .build()
                .expect("the TLS backend initializes, as reqwest::Client::new assumes"),
            endpoints,
            client_id: client_id.into(),
            client_secret,
        }
    }

    /// Where to send the user to grant `scopes`. Requests offline access so the first exchange
    /// returns a refresh token.
    #[must_use]
    pub fn authorization_url(
        &self,
        redirect_uri: &str,
        scopes: &[&str],
        state: &str,
        pkce: &Pkce,
    ) -> Url {
        let mut url = self.endpoints.authorization.clone();
        url.query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", &scopes.join(" "))
            .append_pair("state", state)
            .append_pair("code_challenge", &pkce.challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent");
        url
    }

    /// Exchanges an authorization code for tokens.
    ///
    /// # Errors
    /// [`Error::OAuth`] if Google rejects the grant, [`Error::Http`] on transport failure.
    pub async fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> Result<TokenResponse, Error> {
        self.token(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
        ])
        .await
    }

    /// Mints a new access token from a refresh token.
    ///
    /// # Errors
    /// [`Error::OAuth`] (`invalid_grant` when the refresh token was revoked), [`Error::Http`].
    pub async fn refresh(&self, refresh_token: &str) -> Result<TokenResponse, Error> {
        self.token(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])
        .await
    }

    /// Revokes a refresh token (and the access tokens minted from it) or an access token.
    ///
    /// # Errors
    /// [`Error::OAuth`] (`invalid_token` when it was already revoked), [`Error::Http`].
    pub async fn revoke(&self, token: &str) -> Result<(), Error> {
        self.post::<serde::de::IgnoredAny>(&self.endpoints.revocation, &[("token", token)])
            .await
            .map(drop)
    }

    async fn token(&self, grant: &[(&str, &str)]) -> Result<TokenResponse, Error> {
        let mut form = vec![("client_id", self.client_id.as_str())];
        if let Some(secret) = &self.client_secret {
            form.push(("client_secret", secret));
        }
        form.extend_from_slice(grant);
        self.post(&self.endpoints.token, &form).await
    }

    /// POSTs a form to an OAuth endpoint, which answers errors with [`OAuthError`] bodies.
    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        url: &Url,
        form: &[(&str, &str)],
    ) -> Result<T, Error> {
        let response = self.http.post(url.clone()).form(form).send().await?;
        let status = response.status();
        let body = response.bytes().await?;
        if status.is_success() {
            // Unlike `decode`'s, this error doesn't quote the body: it holds the tokens.
            return crate::client::parse(&body).map_err(|error| Error::Decode {
                status: status.as_u16(),
                reason: error.to_string(),
            });
        }
        match serde_json::from_slice::<OAuthError>(&body) {
            Ok(error) => Err(Error::OAuth(error)),
            Err(_) => crate::client::decode(status, &body),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_leaves_the_secret_out() {
        let client = OAuthClient::new(
            crate::Endpoints::google().oauth,
            "id",
            Some("hunter2".into()),
        );
        assert!(!format!("{client:?}").contains("hunter2"));
    }

    #[test]
    fn debug_leaves_tokens_out() {
        let response = TokenResponse {
            access_token: "ya29.secret".into(),
            expires_in: 3599,
            refresh_token: Some("1//secret".into()),
            scope: None,
            token_type: "Bearer".into(),
            id_token: Some("eyJ.secret".into()),
        };
        let debug = format!("{response:?}");
        assert!(!debug.contains("secret"), "{debug}");
        assert!(debug.contains("3599"));
        let token = crate::StaticToken("ya29.secret".into());
        assert!(!format!("{token:?}").contains("secret"));
    }

    #[test]
    fn s256_matches_rfc_7636_appendix_b() {
        assert_eq!(
            s256_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
}
