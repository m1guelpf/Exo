//! Access tokens: caching, refresh on a 401, coalescing, revoked grants.

mod support;

use std::collections::HashSet;
use std::time::Duration;

use exo_auth::{Error, SecretStore};
use gmail::TokenProvider;
use support::{EMAIL, Harness, HeldGoogle};

/// The sign-in error inside a provider's failure, if that's what it is.
fn sign_in_again(error: &gmail::Error) -> Option<&str> {
    let gmail::Error::Token(inner) = error else {
        return None;
    };
    match inner.downcast_ref::<Error>()? {
        Error::SignInAgain { email } => Some(email),
        _ => None,
    }
}

#[tokio::test]
async fn reuses_the_token_until_invalidated() {
    let harness = Harness::new().await;
    harness.sign_in().await;
    let tokens = harness.auth.tokens(EMAIL).unwrap();
    let first = tokens.access_token().await.unwrap();
    assert_eq!(tokens.access_token().await.unwrap(), first);
    assert_eq!(
        harness
            .auth
            .tokens(EMAIL)
            .unwrap()
            .access_token()
            .await
            .unwrap(),
        first
    );

    tokens.invalidate(&first);
    let second = tokens.access_token().await.unwrap();
    assert_ne!(second, first);
    assert_eq!(tokens.access_token().await.unwrap(), second);
}

#[tokio::test]
async fn a_401_refreshes_the_token() {
    let harness = Harness::new().await;
    harness.sign_in().await;
    let tokens = harness.auth.tokens(EMAIL).unwrap();
    let gmail = gmail::Client::new(harness.auth.endpoints().clone(), tokens.clone());
    assert!(gmail.gmail().profile().await.is_ok());

    // Google expired the token early (the mock's clock jumped past its expiry): the client
    // hands the rejected token back, so the caller's one retry gets a fresh one.
    harness.advance_clock(2).await;
    let error = gmail.gmail().profile().await.unwrap_err();
    assert_eq!(error.status(), Some(401));
    assert!(gmail.gmail().profile().await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_requests_share_one_refresh() {
    let harness = Harness::new().await;
    harness.sign_in().await;
    let tokens = harness.auth.tokens(EMAIL).unwrap();
    let before = tokens.access_token().await.unwrap();
    tokens.invalidate(&before);

    let mut requests = tokio::task::JoinSet::new();
    for _ in 0..16 {
        let tokens = tokens.clone();
        requests.spawn(async move { tokens.access_token().await.unwrap() });
    }
    let minted: HashSet<String> = requests.join_all().await.into_iter().collect();
    assert_eq!(minted.len(), 1, "one refresh for all of them: {minted:?}");
    assert!(!minted.contains(&before));
}

#[tokio::test]
async fn a_revoked_grant_asks_to_sign_in_again() {
    let harness = Harness::new().await;
    harness.sign_in().await;
    let tokens = harness.auth.tokens(EMAIL).unwrap();
    let rejected = tokens.access_token().await.unwrap();
    harness.revoke_at_google().await;
    tokens.invalidate(&rejected);

    let error = tokens.access_token().await.unwrap_err();
    assert_eq!(sign_in_again(&error), Some(EMAIL), "{error}");
    assert!(tokens.needs_sign_in());

    // Settled without asking Google again: it fails the same way with Google gone.
    let Harness { server, auth, .. } = harness;
    drop(server);
    let error = tokens.access_token().await.unwrap_err();
    assert_eq!(sign_in_again(&error), Some(EMAIL), "{error}");
    assert!(
        auth.accounts().iter().any(|account| account.email == EMAIL),
        "it stays listed"
    );
}

#[tokio::test]
async fn network_failures_are_not_sign_in_errors() {
    let harness = Harness::new().await;
    harness.sign_in().await;
    let tokens = harness.auth.tokens(EMAIL).unwrap();
    let rejected = tokens.access_token().await.unwrap();
    tokens.invalidate(&rejected);
    let Harness { server, .. } = harness;
    drop(server);

    let error = tokens.access_token().await.unwrap_err();
    assert!(matches!(error, gmail::Error::Http(_)), "{error}");
    assert!(!tokens.needs_sign_in());
}

#[tokio::test]
async fn the_next_launch_refreshes_from_the_stored_grant() {
    let harness = Harness::new().await;
    harness.sign_in().await;
    let auth = harness.relaunch();
    let tokens = auth.tokens(EMAIL).unwrap();
    let gmail = gmail::Client::new(auth.endpoints().clone(), tokens);
    assert!(gmail.gmail().profile().await.is_ok());
}

#[tokio::test]
async fn a_missing_keychain_item_needs_sign_in() {
    let harness = Harness::new().await;
    harness.sign_in().await;
    harness.secrets.delete(EMAIL).unwrap();
    let tokens = harness.relaunch().tokens(EMAIL).unwrap();
    assert!(tokens.needs_sign_in());
    let error = tokens.access_token().await.unwrap_err();
    assert_eq!(sign_in_again(&error), Some(EMAIL), "{error}");
}

/// What the held token endpoint answers a refresh with.
const MINTED: &str = r#"{"access_token":"ya29.held","expires_in":3599,"token_type":"Bearer"}"#;

#[tokio::test]
async fn a_late_401_for_a_replaced_token_refreshes_nothing() {
    let harness = Harness::new().await;
    harness.sign_in().await;
    let tokens = harness.auth.tokens(EMAIL).unwrap();
    // Two requests went out with this token and both come back 401, the second after the
    // first one's refresh.
    let rejected = tokens.access_token().await.unwrap();
    tokens.invalidate(&rejected);
    let fresh = tokens.access_token().await.unwrap();
    assert_ne!(fresh, rejected);
    tokens.invalidate(&rejected);
    assert_eq!(
        tokens.access_token().await.unwrap(),
        fresh,
        "one refresh serves both"
    );
}

#[tokio::test]
async fn an_account_removed_mid_refresh_gets_no_token() {
    let harness = Harness::new().await;
    harness.sign_in().await;
    let google = HeldGoogle::start(MINTED).await;
    let auth = google.auth(&harness);
    let tokens = auth.tokens(EMAIL).unwrap();
    let refreshing = tokio::spawn({
        let tokens = tokens.clone();
        async move { tokens.access_token().await }
    });
    google.arrived().await;
    auth.forget(EMAIL).unwrap();
    google.answer();
    let error = refreshing.await.unwrap().unwrap_err();
    assert_eq!(sign_in_again(&error), Some(EMAIL), "{error}");
}

#[tokio::test]
async fn a_refresh_google_never_answers_gives_up() {
    let harness = Harness::new().await;
    harness.sign_in().await;
    let google = HeldGoogle::start(MINTED).await;
    let tokens = google.auth(&harness).tokens(EMAIL).unwrap();
    tokio::time::pause();
    let waited = tokio::time::timeout(Duration::from_secs(600), tokens.access_token()).await;
    let error = waited.expect("the refresh gives up").unwrap_err();
    assert!(matches!(error, gmail::Error::Http(_)), "{error}");
    assert!(
        !tokens.needs_sign_in(),
        "a network failure isn't a revocation"
    );
}

#[tokio::test]
async fn a_garbled_token_response_stays_out_of_errors() {
    let harness = Harness::new().await;
    harness.sign_in().await;
    let google = HeldGoogle::start(
        r#"{"access_token":"ya29.leaked","expires_in":"soon","token_type":"Bearer"}"#,
    )
    .await;
    let tokens = google.auth(&harness).tokens(EMAIL).unwrap();
    google.answer();
    let error = tokens.access_token().await.unwrap_err();
    assert!(
        !format!("{error} {error:?}").contains("ya29.leaked"),
        "{error}"
    );
}
