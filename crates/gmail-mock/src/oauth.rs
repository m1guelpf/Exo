//! Google OAuth 2.0 at Google's paths: consent (auto-approved), the token endpoint with PKCE,
//! revocation, userinfo, and the bearer check every API request goes through.
//!
//! Consent signs in the mailbox's owner, or the address in `login_hint` when it's another one:
//! that account shares the one mailbox but `userinfo` and Gmail's profile report its address,
//! which is enough for testing an app with several accounts on one mock.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Extension;
use axum::Router;
use axum::extract::{Form, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use gmail::oauth::{OAuthError, TokenResponse, UserInfo, s256_challenge};
use serde::Deserialize;

use crate::error::Failure;
use crate::http::extract::Params;
use crate::world::{App, World};

/// An access token that is always valid, for tools, tests and the browser harness. Followed by
/// `:` and an address (`ya29.mock-dev-token:bea@example.com`), it signs in that address
/// instead, as a `login_hint` would.
pub const DEV_ACCESS_TOKEN: &str = "ya29.mock-dev-token";

/// Access tokens live as long as Google's.
const TOKEN_LIFETIME_SECS: u64 = 3599;

/// Issued codes and tokens.
#[derive(Debug, Default)]
pub struct Sessions {
    codes: HashMap<String, Grant>,
    /// Access token → (expiry (epoch ms), who it signs in).
    access: HashMap<String, (i64, Identity)>,
    /// Refresh token → (client ID, scope, who it signs in).
    refresh: HashMap<String, (String, String, Identity)>,
    issued: u64,
}

/// Who a token signs in: the mailbox's owner (`None`), or another address that shares its
/// mailbox. The bearer check hands it to the handlers as a request extension.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identity(pub Option<String>);

#[derive(Debug, Clone)]
struct Grant {
    client_id: String,
    redirect_uri: String,
    scope: String,
    /// `(method, challenge)`.
    challenge: Option<(String, String)>,
    identity: Identity,
}

impl Sessions {
    fn next(&mut self) -> u64 {
        self.issued += 1;
        self.issued
    }

    /// Who `token` signs in, while it's valid.
    fn identity(&self, token: &str, now_ms: i64) -> Option<Identity> {
        if let Some(rest) = token.strip_prefix(DEV_ACCESS_TOKEN) {
            match rest.strip_prefix(':') {
                Some(email) if !email.is_empty() => return Some(Identity(Some(email.to_owned()))),
                _ if rest.is_empty() => return Some(Identity::default()),
                _ => {}
            }
        }
        self.access
            .get(token)
            .filter(|(expiry, _)| *expiry > now_ms)
            .map(|(_, identity)| identity.clone())
    }
}

/// `/o/oauth2/v2/auth`, `/token` and `/revoke`, which don't take bearer tokens.
pub fn router() -> Router<Arc<App>> {
    Router::new()
        .route("/o/oauth2/v2/auth", get(authorize))
        .route("/token", post(token))
        .route("/revoke", post(revoke))
}

/// Authenticated OAuth endpoints, mounted with the APIs.
pub fn api_router() -> Router<Arc<App>> {
    Router::new().route("/oauth2/v2/userinfo", get(userinfo))
}

/// Rejects API requests without a valid `Authorization: Bearer` (or `access_token` parameter),
/// and tells the rest who the token signs in ([`Identity`]).
pub async fn authenticate(
    State(app): State<Arc<App>>,
    mut request: Request,
    next: Next,
) -> Response {
    let header_token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
        })
        .map(str::to_owned);
    let params = Params::parse(request.uri().query().unwrap_or_default());
    let token = header_token.or_else(|| {
        params
            .text("access_token")
            .or_else(|| params.text("oauth_token"))
            .map(str::to_owned)
    });
    let identity = token.as_deref().map(|token| {
        let world = app.lock();
        world.sessions.identity(token, world.clock.now_ms())
    });
    match identity {
        Some(Some(identity)) => {
            request.extensions_mut().insert(identity);
            next.run(request).await
        }
        present => Failure::unauthenticated(present.is_some(), service(request.uri().path()))
            .into_response(),
    }
}

/// The service name Google reports in auth errors for a path.
fn service(path: &str) -> &'static str {
    if path.starts_with("/calendar/") {
        "calendar-json.googleapis.com"
    } else if path.starts_with("/v1/") {
        "people.googleapis.com"
    } else if path.starts_with("/oauth2/") {
        "oauth2.googleapis.com"
    } else {
        "gmail.googleapis.com"
    }
}

/// Consent is automatic: validate, then redirect straight back with a code.
async fn authorize(State(app): State<Arc<App>>, params: Params) -> Response {
    let (Some(client_id), Some(redirect_uri)) =
        (params.text("client_id"), params.text("redirect_uri"))
    else {
        return consent_error("Missing required parameter: client_id or redirect_uri");
    };
    if params.get("response_type") != Some("code") {
        return consent_error("Unsupported response_type; only `code` is supported");
    }
    let Ok(mut redirect) = url::Url::parse(redirect_uri) else {
        return consent_error("Invalid parameter value for redirect_uri");
    };
    let challenge = params.text("code_challenge").map(|challenge| {
        let method = params.text("code_challenge_method").unwrap_or("plain");
        (method.to_owned(), challenge.to_owned())
    });
    let scope = params.text("scope").unwrap_or_default().to_owned();
    let code = {
        let mut world = app.lock();
        let identity = Identity(
            params
                .text("login_hint")
                .filter(|hint| !hint.eq_ignore_ascii_case(&world.account.email))
                .map(str::to_owned),
        );
        let code = format!("4/0AeMock{:08x}", world.sessions.next());
        world.sessions.codes.insert(
            code.clone(),
            Grant {
                client_id: client_id.to_owned(),
                redirect_uri: redirect_uri.to_owned(),
                scope: scope.clone(),
                challenge,
                identity,
            },
        );
        code
    };
    {
        let mut query = redirect.query_pairs_mut();
        if let Some(state) = params.get("state") {
            query.append_pair("state", state);
        }
        query
            .append_pair("code", &code)
            .append_pair("scope", &scope)
            .append_pair("authuser", "0")
            .append_pair("prompt", "consent");
    }
    // Google answers consent with a 302.
    (
        StatusCode::FOUND,
        [(header::LOCATION, redirect.to_string())],
    )
        .into_response()
}

fn consent_error(detail: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Html(format!(
            "<!DOCTYPE html><title>Error 400: invalid_request</title>\
             <h1>Access blocked: Authorization Error</h1><p>Error 400: invalid_request</p><p>{detail}</p>"
        )),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
struct TokenRequest {
    grant_type: Option<String>,
    client_id: Option<String>,
    code: Option<String>,
    code_verifier: Option<String>,
    redirect_uri: Option<String>,
    refresh_token: Option<String>,
}

async fn token(State(app): State<Arc<App>>, Form(request): Form<TokenRequest>) -> Response {
    let mut world = app.lock();
    match exchange(&mut world, request) {
        Ok(tokens) => json_response(StatusCode::OK, &tokens),
        Err((status, error)) => json_response(status, &error),
    }
}

fn exchange(
    world: &mut World,
    request: TokenRequest,
) -> Result<TokenResponse, (StatusCode, OAuthError)> {
    let fail = |status: StatusCode, error: &str, description: &str| {
        (
            status,
            OAuthError {
                error: error.to_owned(),
                error_description: Some(description.to_owned()),
            },
        )
    };
    let Some(client_id) = request.client_id else {
        return Err(fail(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Could not determine client ID from request.",
        ));
    };
    let (scope, refresh_token, identity) = match request.grant_type.as_deref() {
        Some("authorization_code") => {
            let grant = request
                .code
                .and_then(|code| world.sessions.codes.remove(&code))
                .ok_or_else(|| {
                    fail(
                        StatusCode::BAD_REQUEST,
                        "invalid_grant",
                        "Malformed auth code.",
                    )
                })?;
            if grant.client_id != client_id {
                return Err(fail(
                    StatusCode::UNAUTHORIZED,
                    "invalid_client",
                    "Unauthorized",
                ));
            }
            if request.redirect_uri.as_deref() != Some(grant.redirect_uri.as_str()) {
                return Err(fail(
                    StatusCode::BAD_REQUEST,
                    "redirect_uri_mismatch",
                    "Bad Request",
                ));
            }
            if let Some((method, challenge)) = &grant.challenge {
                let verifier = request.code_verifier.ok_or_else(|| {
                    fail(
                        StatusCode::BAD_REQUEST,
                        "invalid_grant",
                        "Missing code verifier.",
                    )
                })?;
                let derived = if method.eq_ignore_ascii_case("S256") {
                    s256_challenge(&verifier)
                } else {
                    verifier
                };
                if &derived != challenge {
                    return Err(fail(
                        StatusCode::BAD_REQUEST,
                        "invalid_grant",
                        "Invalid code verifier.",
                    ));
                }
            }
            let refresh = format!("1//0dMock{:08x}", world.sessions.next());
            world.sessions.refresh.insert(
                refresh.clone(),
                (
                    client_id.clone(),
                    grant.scope.clone(),
                    grant.identity.clone(),
                ),
            );
            (grant.scope, Some(refresh), grant.identity)
        }
        Some("refresh_token") => {
            let (owner, scope, identity) = request
                .refresh_token
                .and_then(|token| world.sessions.refresh.get(&token).cloned())
                .ok_or_else(|| {
                    fail(
                        StatusCode::BAD_REQUEST,
                        "invalid_grant",
                        "Token has been expired or revoked.",
                    )
                })?;
            if owner != client_id {
                return Err(fail(
                    StatusCode::UNAUTHORIZED,
                    "invalid_client",
                    "Unauthorized",
                ));
            }
            (scope, None, identity)
        }
        other => {
            return Err(fail(
                StatusCode::BAD_REQUEST,
                "unsupported_grant_type",
                &format!("Invalid grant_type: {}", other.unwrap_or_default()),
            ));
        }
    };
    let access = format!("ya29.a0Mock{:08x}", world.sessions.next());
    let expiry =
        world.clock.now_ms() + i64::try_from(TOKEN_LIFETIME_SECS * 1000).unwrap_or(i64::MAX);
    world
        .sessions
        .access
        .insert(access.clone(), (expiry, identity));
    let id_token = scope
        .split_whitespace()
        .any(|s| s == "openid")
        .then(|| id_token(world, &client_id));
    Ok(TokenResponse {
        access_token: access,
        expires_in: TOKEN_LIFETIME_SECS,
        refresh_token,
        scope: Some(scope),
        token_type: "Bearer".into(),
        id_token,
    })
}

#[derive(Debug, Deserialize)]
struct RevokeRequest {
    token: Option<String>,
}

/// Forgets a refresh or access token. Unlike Google, revoking a refresh token leaves the access
/// tokens minted from it working until they expire.
async fn revoke(State(app): State<Arc<App>>, Form(request): Form<RevokeRequest>) -> Response {
    let mut world = app.lock();
    let sessions = &mut world.sessions;
    let known = request.token.is_some_and(|token| {
        sessions.refresh.remove(&token).is_some() || sessions.access.remove(&token).is_some()
    });
    if known {
        json_response(StatusCode::OK, &serde_json::json!({}))
    } else {
        json_response(
            StatusCode::BAD_REQUEST,
            &OAuthError {
                error: "invalid_token".into(),
                error_description: Some("Token expired or revoked".into()),
            },
        )
    }
}

/// An unsigned-looking OpenID ID token with the claims Google sets. The signature is fake.
fn id_token(world: &World, client_id: &str) -> String {
    let now = world.clock.now().as_second();
    let account = &world.account;
    let encode = |value: serde_json::Value| URL_SAFE_NO_PAD.encode(value.to_string());
    let header = encode(serde_json::json!({"alg": "RS256", "kid": "mock", "typ": "JWT"}));
    let claims = encode(serde_json::json!({
        "iss": "https://accounts.google.com",
        "azp": client_id,
        "aud": client_id,
        "sub": account.google_id,
        "email": account.email,
        "email_verified": true,
        "name": account.name(),
        "given_name": account.given_name,
        "family_name": account.family_name,
        "iat": now,
        "exp": now + 3600,
    }));
    format!(
        "{header}.{claims}.{}",
        URL_SAFE_NO_PAD.encode("mock-signature")
    )
}

async fn userinfo(
    State(app): State<Arc<App>>,
    Extension(identity): Extension<Identity>,
) -> axum::Json<UserInfo> {
    let world = app.lock();
    let account = &world.account;
    if let Identity(Some(email)) = identity {
        return axum::Json(UserInfo {
            hd: email.split_once('@').map(|(_, domain)| domain.to_owned()),
            email: Some(email),
            verified_email: Some(true),
            locale: Some("en".into()),
            ..UserInfo::default()
        });
    }
    axum::Json(UserInfo {
        id: Some(account.google_id.clone()),
        email: Some(account.email.clone()),
        verified_email: Some(true),
        name: Some(account.name()),
        given_name: Some(account.given_name.clone()),
        family_name: Some(account.family_name.clone()),
        // Userinfo's photo URLs ask for 96px, cropped square.
        picture: Some(account.photo.as_ref().map_or_else(
            || crate::people::DEFAULT_PHOTO.to_owned(),
            |photo| format!("{photo}=s96-c"),
        )),
        locale: Some("en".into()),
        hd: account
            .email
            .split_once('@')
            .map(|(_, domain)| domain.to_owned()),
    })
}

fn json_response(status: StatusCode, body: &impl serde::Serialize) -> Response {
    let text = serde_json::to_string_pretty(body).expect("token bodies serialize");
    (
        status,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        text,
    )
        .into_response()
}
