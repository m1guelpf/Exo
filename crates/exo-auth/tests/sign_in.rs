//! The loopback flow end to end against gmail-mock's OAuth endpoints.

mod support;

use exo_auth::{Auth, ClientConfig, Error, MemorySecrets, SCOPES, SecretStore};
use gmail::TokenProvider;
use support::{EMAIL, Harness, browser, param, redirect};

#[tokio::test]
async fn signs_in_through_the_loopback_redirect() {
    let harness = Harness::new().await;
    let (open, page) = browser(Clone::clone);
    let account = harness.auth.sign_in(None, open).await.unwrap();
    assert_eq!(account.email, EMAIL);
    assert_eq!(account.name.as_deref(), Some("Maya Okafor"));

    let page = page.await.unwrap();
    assert_eq!(page.status, 200);
    assert!(
        page.text.contains("Thanks for allowing Exo!"),
        "{}",
        page.text
    );
    let url = &page.opened;
    assert!(param(url, "redirect_uri").starts_with("http://127.0.0.1:"));
    assert_eq!(param(url, "scope"), SCOPES.join(" "));
    assert_eq!(param(url, "code_challenge_method"), "S256");
    assert_eq!(param(url, "access_type"), "offline");
    assert_eq!(param(url, "prompt"), "consent");
    assert_eq!(param(url, "include_granted_scopes"), "true");
    assert!(url.query_pairs().all(|(key, _)| key != "login_hint"));

    assert_eq!(harness.auth.accounts(), [account]);
    assert!(
        harness
            .secrets
            .get(EMAIL)
            .unwrap()
            .is_some_and(|grant| grant.starts_with("1//"))
    );
    let tokens = harness.auth.tokens(EMAIL).unwrap();
    assert!(!tokens.needs_sign_in());
    let gmail = gmail::Client::new(harness.auth.endpoints().clone(), tokens);
    let profile = gmail.gmail().profile().await.unwrap();
    assert_eq!(profile.email_address.as_deref(), Some(EMAIL));
}

#[tokio::test]
async fn a_hint_preselects_the_account() {
    let harness = Harness::new().await;
    let (open, page) = browser(Clone::clone);
    harness.auth.sign_in(Some(EMAIL), open).await.unwrap();
    assert_eq!(param(&page.await.unwrap().opened, "login_hint"), EMAIL);
}

#[tokio::test]
async fn declining_consent_is_access_denied() {
    let harness = Harness::new().await;
    let (open, page) = browser(|url| {
        redirect(
            url,
            &format!("error=access_denied&state={}", param(url, "state")),
        )
    });
    let error = harness.auth.sign_in(None, open).await.unwrap_err();
    assert!(matches!(error, Error::AccessDenied), "{error}");
    let page = page.await.unwrap();
    assert_eq!(page.status, 400);
    assert!(
        page.text
            .contains("Sorry, something went wrong! Please try again later.")
    );
    assert!(harness.auth.accounts().is_empty());
}

#[tokio::test]
async fn other_google_errors_are_reported() {
    let harness = Harness::new().await;
    let (open, _page) = browser(|url| {
        redirect(
            url,
            &format!("error=admin_policy_enforced&state={}", param(url, "state")),
        )
    });
    let error = harness.auth.sign_in(None, open).await.unwrap_err();
    assert!(
        matches!(&error, Error::Authorization { error } if error == "admin_policy_enforced"),
        "{error}"
    );
}

#[tokio::test]
async fn redirects_from_other_attempts_are_turned_away() {
    let harness = Harness::new().await;
    // Before the user approves, a stale tab or a page that found the port sends redirects
    // without this attempt's `state`: they're answered, and the attempt keeps waiting.
    let (sender, pages) = tokio::sync::oneshot::channel();
    let open = move |url: &url::Url| {
        let forged = [
            redirect(url, "code=4%2F0AeMockforged&state=someone-else"),
            redirect(url, "error=access_denied&state=someone-else"),
            redirect(url, "code=4%2F0AeMockforged"),
        ];
        let consent = url.clone();
        tokio::spawn(async move {
            let mut answers = Vec::new();
            for url in forged {
                let response = reqwest::get(url).await.unwrap();
                answers.push((response.status().as_u16(), response.text().await.unwrap()));
            }
            let approved = reqwest::get(consent).await.unwrap().status().as_u16();
            let _ = sender.send((answers, approved));
        });
        Ok(())
    };
    let account = harness.auth.sign_in(None, open).await.unwrap();
    assert_eq!(account.email, EMAIL);
    let (answers, approved) = pages.await.unwrap();
    for (status, text) in answers {
        assert_eq!(status, 400);
        assert!(
            text.contains("Sorry, this login attempt has expired. Please try again."),
            "{text}"
        );
    }
    assert_eq!(approved, 200);
}

#[tokio::test]
async fn unticked_permissions_fail_sign_in() {
    let harness = Harness::new().await;
    // The user unticks Calendar on Google's consent screen.
    let (open, page) = browser(|url| {
        let scopes = param(url, "scope").replace(gmail::oauth::scopes::CALENDAR, "");
        let mut visit = url.clone();
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(key, value)| {
                let value = if key == "scope" {
                    scopes.clone()
                } else {
                    value.into_owned()
                };
                (key.into_owned(), value)
            })
            .collect();
        visit.query_pairs_mut().clear().extend_pairs(pairs);
        visit
    });
    let error = harness.auth.sign_in(None, open).await.unwrap_err();
    assert!(
        matches!(&error, Error::MissingScopes(missing) if missing == &[gmail::oauth::scopes::CALENDAR]),
        "{error}"
    );
    let page = page.await.unwrap();
    assert!(
        page.text
            .contains("Please select all permissions when signing in with Google.<br>")
    );
    assert!(harness.auth.accounts().is_empty());
}

#[tokio::test]
async fn accounts_without_gmail_are_refused() {
    let harness = Harness::new().await;
    harness
        .fault(serde_json::json!({
            "path": "/gmail/v1/users/me/profile",
            "status": 400,
            "message": "Mail service not enabled",
            "reason": "failedPrecondition",
        }))
        .await;
    let (open, page) = browser(Clone::clone);
    let error = harness.auth.sign_in(None, open).await.unwrap_err();
    assert!(matches!(error, Error::NoMailbox), "{error}");
    let page = page.await.unwrap();
    assert!(
        page.text
            .contains("This Google Account does not appear to have a Gmail mailbox.")
    );
    assert!(harness.auth.accounts().is_empty());
}

#[tokio::test]
async fn a_browser_that_wont_open_fails_at_once() {
    let harness = Harness::new().await;
    let error = harness
        .auth
        .sign_in(None, |_| Err(std::io::Error::other("no browser")))
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Browser(_)), "{error}");
}

#[tokio::test]
async fn dropping_the_future_cancels_and_frees_the_port() {
    let harness = Harness::new().await;
    let (sender, opened) = tokio::sync::oneshot::channel();
    let auth = harness.auth.clone();
    let signing_in = tokio::spawn(async move {
        auth.sign_in(None, move |url| {
            let _ = sender.send(url.clone());
            Ok(())
        })
        .await
    });
    let opened = opened.await.unwrap();
    let loopback = url::Url::parse(&param(&opened, "redirect_uri")).unwrap();
    let address = (loopback.host_str().unwrap(), loopback.port().unwrap());
    assert!(tokio::net::TcpStream::connect(address).await.is_ok());

    signing_in.abort();
    assert!(signing_in.await.unwrap_err().is_cancelled());
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    assert!(harness.auth.accounts().is_empty());
}

#[tokio::test(start_paused = true)]
async fn gives_up_when_the_browser_never_comes_back() {
    // Nothing is contacted: the user never finishes in the browser.
    let dir = tempfile::tempdir().unwrap();
    let client = ClientConfig::new("id", "secret", gmail::Endpoints::google());
    let auth = Auth::open(client, dir.path(), MemorySecrets::default()).unwrap();
    let started = tokio::time::Instant::now();
    let error = auth.sign_in(None, |_| Ok(())).await.unwrap_err();
    assert!(matches!(error, Error::TimedOut), "{error}");
    assert_eq!(started.elapsed(), exo_auth::TIMEOUT);
}

#[tokio::test]
async fn signing_in_again_revives_the_same_tokens() {
    let harness = Harness::new().await;
    let first = harness.sign_in().await;
    let tokens = harness.auth.tokens(EMAIL).unwrap();
    let rejected = tokens.access_token().await.unwrap();
    harness.revoke_at_google().await;
    tokens.invalidate(&rejected);
    assert!(tokens.access_token().await.is_err());
    assert!(tokens.needs_sign_in());

    let (open, _page) = browser(Clone::clone);
    let again = harness.auth.sign_in(Some(EMAIL), open).await.unwrap();
    assert_eq!(again, first);
    assert_eq!(harness.auth.accounts(), [first]);
    // The provider a running store already holds works again.
    assert!(!tokens.needs_sign_in());
    let gmail = gmail::Client::new(harness.auth.endpoints().clone(), tokens);
    assert!(gmail.gmail().profile().await.is_ok());
}
