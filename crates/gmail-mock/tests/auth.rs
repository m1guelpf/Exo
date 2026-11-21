//! Bearer authentication and the OAuth 2.0 flow (consent redirect, PKCE code exchange,
//! refresh, userinfo) through the `gmail` client.

mod support;

use gmail::oauth::{OAuthClient, Pkce, scopes};
use gmail::{Client, Error, StaticToken};

const REDIRECT: &str = "http://127.0.0.1:9004/callback";

#[tokio::test]
async fn unknown_tokens_get_401s() {
    let server = support::server().await;
    let client = Client::new(server.endpoints(), StaticToken("ya29.revoked".into()));
    let error = client.gmail().profile().await.unwrap_err();
    let Error::Api(api) = &error else {
        panic!("{error}")
    };
    assert_eq!(
        (api.code, api.status.as_deref()),
        (401, Some("UNAUTHENTICATED"))
    );
    assert_eq!(api.errors[0].reason, "authError");
    assert_eq!(
        client
            .calendar()
            .event("primary", "x")
            .await
            .unwrap_err()
            .status(),
        Some(401)
    );
    assert_eq!(
        client.people().me("names").await.unwrap_err().status(),
        Some(401)
    );
}

async fn authorize(
    server: &gmail_mock::MockServer,
    oauth: &OAuthClient,
    pkce: &Pkce,
) -> (String, url::Url) {
    authorize_as(server, oauth, pkce, None).await
}

/// [`authorize`], picking `login_hint` at consent.
async fn authorize_as(
    server: &gmail_mock::MockServer,
    oauth: &OAuthClient,
    pkce: &Pkce,
    login_hint: Option<&str>,
) -> (String, url::Url) {
    let mut url = oauth.authorization_url(
        REDIRECT,
        &[scopes::GMAIL_MODIFY, scopes::OPENID, scopes::USERINFO_EMAIL],
        "xyzzy",
        pkce,
    );
    if let Some(hint) = login_hint {
        url.query_pairs_mut().append_pair("login_hint", hint);
    }
    assert!(
        url.as_str()
            .starts_with(&format!("{}o/oauth2/v2/auth?", server.url()))
    );
    let no_redirects = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = no_redirects.get(url).send().await.unwrap();
    assert_eq!(response.status(), 302);
    let location = url::Url::parse(response.headers()["location"].to_str().unwrap()).unwrap();
    assert!(location.as_str().starts_with(REDIRECT));
    let code = location
        .query_pairs()
        .find(|(k, _)| k == "code")
        .unwrap()
        .1
        .into_owned();
    (code, location)
}

#[tokio::test]
async fn oauth_flow_with_pkce_and_refresh() {
    let server = support::server().await;
    let endpoints = server.endpoints();
    let oauth = OAuthClient::new(
        endpoints.oauth.clone(),
        "exo-desktop.apps.example",
        Some("secret".into()),
    );
    let pkce = Pkce::generate();

    let (code, location) = authorize(&server, &oauth, &pkce).await;
    let query: Vec<(String, String)> = location
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    assert_eq!(query[0], ("state".into(), "xyzzy".into()));
    assert!(
        query
            .iter()
            .any(|(k, v)| k == "scope" && v.contains("gmail.modify"))
    );

    let tokens = oauth
        .exchange_code(&code, &pkce.verifier, REDIRECT)
        .await
        .unwrap();
    assert_eq!(
        (tokens.token_type.as_str(), tokens.expires_in),
        ("Bearer", 3599)
    );
    let refresh_token = tokens
        .refresh_token
        .clone()
        .expect("offline access returns a refresh token");
    let id_token = tokens.id_token.clone().expect("openid returns an ID token");
    assert_eq!(id_token.split('.').count(), 3);

    let reused = oauth
        .exchange_code(&code, &pkce.verifier, REDIRECT)
        .await
        .unwrap_err();
    assert!(
        matches!(&reused, Error::OAuth(e) if e.error == "invalid_grant"),
        "codes are single-use: {reused}"
    );

    let client = Client::new(endpoints.clone(), StaticToken(tokens.access_token.clone()));
    assert_eq!(
        client
            .gmail()
            .profile()
            .await
            .unwrap()
            .email_address
            .as_deref(),
        Some("maya@brightmoor.co")
    );
    let me = client.userinfo().await.unwrap();
    assert_eq!(
        (me.name.as_deref(), me.verified_email),
        (Some("Maya Okafor"), Some(true))
    );

    let refreshed = oauth.refresh(&refresh_token).await.unwrap();
    assert!(
        refreshed.refresh_token.is_none(),
        "refreshes don't rotate the refresh token"
    );
    assert_ne!(refreshed.access_token, tokens.access_token);
    let client = Client::new(endpoints, StaticToken(refreshed.access_token));
    assert!(client.gmail().labels().await.is_ok());

    let revoked = oauth.refresh("1//0bogus").await.unwrap_err();
    assert!(matches!(revoked, Error::OAuth(e) if e.error == "invalid_grant"));

    oauth.revoke(&refresh_token).await.unwrap();
    let revoked = oauth.refresh(&refresh_token).await.unwrap_err();
    assert!(matches!(revoked, Error::OAuth(e) if e.error == "invalid_grant"));
    let again = oauth.revoke(&refresh_token).await.unwrap_err();
    assert!(matches!(again, Error::OAuth(e) if e.error == "invalid_token"));
}

#[tokio::test]
async fn a_login_hint_signs_in_another_address_on_the_same_mailbox() {
    let server = support::server().await;
    let oauth = OAuthClient::new(server.endpoints().oauth, "exo-desktop.apps.example", None);
    let address = |token: String| async {
        let client = Client::new(server.endpoints(), StaticToken(token));
        let profile = client.gmail().profile().await.unwrap();
        let me = client.userinfo().await.unwrap();
        assert_eq!(profile.email_address, me.email);
        (me.email.unwrap(), profile.threads_total)
    };
    let mut signed_in = Vec::new();
    for hint in [Some("bea@example.com"), Some("MAYA@brightmoor.co"), None] {
        let pkce = Pkce::generate();
        let (code, _) = authorize_as(&server, &oauth, &pkce, hint).await;
        let tokens = oauth
            .exchange_code(&code, &pkce.verifier, REDIRECT)
            .await
            .unwrap();
        let refreshed = oauth
            .refresh(tokens.refresh_token.as_deref().unwrap())
            .await
            .unwrap();
        let (email, threads) = address(tokens.access_token).await;
        assert_eq!(
            address(refreshed.access_token).await.0,
            email,
            "refreshes keep it"
        );
        signed_in.push((email, threads));
    }
    assert_eq!(signed_in[0].0, "bea@example.com");
    assert_eq!(signed_in[1].0, "maya@brightmoor.co");
    assert_eq!(signed_in[2].0, "maya@brightmoor.co");
    assert_eq!(signed_in[0].1, signed_in[2].1, "one mailbox");
}

#[tokio::test]
async fn a_photo_of_its_own_is_listed_and_served_in_any_size() {
    let config = gmail_mock::Config {
        photo: true,
        ..gmail_mock::Config::default()
    };
    let server = gmail_mock::MockServer::start(config).await.unwrap();
    let maya = support::client(&server);
    let photo = maya.people().me("photos").await.unwrap().photos.remove(0);
    assert_eq!(
        (
            photo.default,
            photo.metadata.and_then(|metadata| metadata.primary)
        ),
        (None, Some(true))
    );
    let url = photo.url.unwrap();
    assert!(
        url.starts_with(server.url().as_str()) && url.ends_with("=s100"),
        "{url}"
    );
    let picture = maya.userinfo().await.unwrap().picture;
    assert_eq!(
        picture,
        Some(url.replace("=s100", "=s96-c")),
        "userinfo's is the same photo"
    );

    // Public, as Google's image server is, and as large as `=s{n}` says.
    for (url, side) in [(url.clone(), 100u32), (url.replace("=s100", "=s48-c"), 48)] {
        let response = reqwest::get(&url).await.unwrap();
        assert_eq!(response.headers()["content-type"], "image/png");
        let png = response.bytes().await.unwrap();
        assert_eq!(
            png[16..24],
            [side.to_be_bytes(), side.to_be_bytes()].concat(),
            "IHDR's width and height"
        );
    }
    let elsewhere = url.replace("=s100", "x=s100");
    assert_eq!(reqwest::get(&elsewhere).await.unwrap().status(), 404);

    // Another address on the mailbox has Google's generated avatar.
    let oauth = OAuthClient::new(server.endpoints().oauth, "exo-desktop.apps.example", None);
    let pkce = Pkce::generate();
    let (code, _) = authorize_as(&server, &oauth, &pkce, Some("bea@example.com")).await;
    let tokens = oauth
        .exchange_code(&code, &pkce.verifier, REDIRECT)
        .await
        .unwrap();
    let bea = Client::new(server.endpoints(), StaticToken(tokens.access_token));
    let me = bea.people().me("emailAddresses,photos").await.unwrap();
    assert_eq!(
        me.email_addresses[0].value.as_deref(),
        Some("bea@example.com")
    );
    assert_eq!(me.photos[0].default, Some(true));
}

#[tokio::test]
async fn without_a_photo_the_account_has_googles_default() {
    let server = support::server().await;
    let photo = support::client(&server)
        .people()
        .me("photos")
        .await
        .unwrap()
        .photos
        .remove(0);
    assert_eq!(photo.default, Some(true));
    let mock_photo = server
        .url()
        .join("/__mock/photo/108452913874620153827=s100")
        .unwrap();
    assert_eq!(
        reqwest::get(mock_photo).await.unwrap().status(),
        404,
        "nothing is served"
    );
}

#[tokio::test]
async fn pkce_is_verified() {
    let server = support::server().await;
    let oauth = OAuthClient::new(server.endpoints().oauth, "exo-desktop.apps.example", None);
    let pkce = Pkce::generate();
    let (code, _) = authorize(&server, &oauth, &pkce).await;
    let wrong = oauth
        .exchange_code(&code, "not-the-verifier", REDIRECT)
        .await
        .unwrap_err();
    assert!(
        matches!(&wrong, Error::OAuth(e) if e.error_description.as_deref() == Some("Invalid code verifier.")),
        "{wrong}"
    );

    let (code, _) = authorize(&server, &oauth, &pkce).await;
    let mismatch = oauth
        .exchange_code(&code, &pkce.verifier, "http://elsewhere/")
        .await
        .unwrap_err();
    assert!(matches!(mismatch, Error::OAuth(e) if e.error == "redirect_uri_mismatch"));
}

/// Records the tokens Google rejected.
#[derive(Default)]
struct Rejections(std::sync::Mutex<Vec<String>>);

impl gmail::TokenProvider for Rejections {
    fn access_token(&self) -> gmail::TokenFuture<'_> {
        Box::pin(async { Ok("ya29.revoked".to_owned()) })
    }

    fn invalidate(&self, token: &str) {
        self.0.lock().unwrap().push(token.to_owned());
    }
}

#[tokio::test]
async fn a_401_hands_the_rejected_token_back() {
    let server = support::server().await;
    let tokens = std::sync::Arc::new(Rejections::default());
    let client = Client::new(server.endpoints(), tokens.clone());
    assert_eq!(
        client.gmail().profile().await.unwrap_err().status(),
        Some(401)
    );
    let gmail = client.gmail();
    let batch = gmail.batch(vec![gmail.labels()]).await;
    assert!(batch.is_err() || batch.unwrap()[0].is_err());
    assert_eq!(*tokens.0.lock().unwrap(), ["ya29.revoked", "ya29.revoked"]);
}
