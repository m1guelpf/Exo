//! What the connection says as Gmail goes away and comes back, rate limits the account or says
//! it has no mail: the offline bar's states.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use exo_store::{Error, Outage, Store};
use gmail::{TokenFuture, TokenProvider};
use gmail_mock::DEV_ACCESS_TOKEN;
use serde_json::{Value, json};
use support::{ACCOUNT, DEBOUNCE, Harness, TempDir, options};

fn outage(store: &Store) -> Option<Outage> {
    store.connection().borrow().outage
}

#[tokio::test]
async fn gmail_going_away_and_coming_back() {
    let mut harness = Harness::synced().await;
    let store = harness.store.clone();
    assert!(store.connection().borrow().is_online());

    harness.server.disconnect().await;
    assert!(matches!(store.sync().await, Err(Error::Offline(_))));
    assert_eq!(
        outage(&store),
        Some(Outage::Gmail),
        "the sync got no answer"
    );
    // The OS says the network changed, and the check gets no answer either.
    store.network_changed().await;
    assert_eq!(outage(&store), Some(Outage::Offline));
    assert!(!store.connection().borrow().disk_low);

    harness
        .server
        .reconnect()
        .await
        .expect("the mock comes back");
    store.check_connection().await;
    assert_eq!(
        outage(&store),
        Some(Outage::Offline),
        "down too briefly to believe it's back"
    );
    tokio::time::sleep(DEBOUNCE.down).await;
    store.check_connection().await;
    assert_eq!(outage(&store), None);

    // Gone again at once: back only a while after it last came back, even though a sync gets
    // through.
    harness.server.disconnect().await;
    assert!(store.sync().await.is_err());
    assert_eq!(outage(&store), Some(Outage::Gmail));
    harness
        .server
        .reconnect()
        .await
        .expect("the mock comes back");
    tokio::time::sleep(DEBOUNCE.down).await;
    assert!(store.sync().await.is_ok());
    assert_eq!(
        outage(&store),
        Some(Outage::Gmail),
        "it bounced too recently"
    );
    tokio::time::sleep(DEBOUNCE.bounce).await;
    store.check_connection().await;
    assert_eq!(outage(&store), None);
}

#[tokio::test]
async fn calendar_and_people_getting_no_answer_say_the_network_is_down() {
    let mut harness = Harness::synced().await;
    let store = harness.store.clone();
    harness.server.disconnect().await;
    assert!(matches!(
        store.sync_calendars(ACCOUNT).await,
        Err(Error::Offline(_))
    ));
    assert_eq!(
        outage(&store),
        Some(Outage::Offline),
        "Calendar got no answer"
    );
    harness
        .server
        .reconnect()
        .await
        .expect("the mock comes back");
    tokio::time::sleep(DEBOUNCE.down).await;
    store
        .sync_calendars(ACCOUNT)
        .await
        .expect("Calendar answers");
    assert_eq!(outage(&store), None);

    harness.server.disconnect().await;
    let query = gmail::people::PeopleQuery::default();
    assert!(matches!(
        store
            .google(1, |client| client.people().connections("names", &query))
            .await,
        Err(Error::Offline(_))
    ));
    assert_eq!(
        outage(&store),
        Some(Outage::Offline),
        "People got no answer"
    );
}

#[tokio::test]
async fn a_network_change_with_gmail_answering_changes_nothing() {
    let harness = Harness::synced().await;
    let connection = harness.store.connection();
    harness.store.network_changed().await;
    assert!(!connection.has_changed().expect("open"));
}

/// Gmail's rate-limit answers.
fn rate_limits() -> [Value; 3] {
    let every = |mut fault: Value| {
        fault["path"] = json!("/gmail/v1/");
        fault["times"] = json!(1000);
        fault
    };
    [
        every(json!({ "status": 429 })),
        every(
            json!({ "status": 403, "message": "Quota exceeded for quota metric 'Queries'", "reason": "userRateLimitExceeded" }),
        ),
        every(
            json!({ "status": 503, "message": "Too many concurrent requests", "reason": "CommandConcurrencyLimitReached" }),
        ),
    ]
}

#[tokio::test]
async fn rate_limiting_shows_until_gmail_answers() {
    for fault in rate_limits() {
        let harness = Harness::synced().await;
        harness.fault(fault.clone()).await;
        assert!(
            matches!(harness.store.sync().await, Err(Error::Throttled { .. })),
            "{fault}"
        );
        assert_eq!(outage(&harness.store), Some(Outage::Throttled), "{fault}");

        harness.clear_faults().await;
        tokio::time::sleep(DEBOUNCE.down).await;
        harness.store.check_connection().await;
        assert_eq!(outage(&harness.store), None, "{fault}");
    }
}

#[tokio::test]
async fn a_pause_longer_than_a_call_waits_shows_at_once() {
    let harness = Harness::synced().await;
    let at = jiff::Timestamp::now() + jiff::SignedDuration::from_mins(2);
    harness
        .fault(json!({ "path": "/gmail/v1/", "status": 429, "message": format!("User-rate limit exceeded.  Retry after {at}"), "times": 1 }))
        .await;
    assert!(harness.store.sync().await.is_err());
    assert_eq!(outage(&harness.store), Some(Outage::Throttled));
    // The check waits for Gmail too: it fails without asking.
    harness.store.check_connection().await;
    assert_eq!(outage(&harness.store), Some(Outage::Throttled));
}

#[tokio::test]
async fn a_5xx_that_outlasts_its_retries_is_no_outage() {
    let harness = Harness::synced().await;
    harness
        .fault(json!({ "path": "/gmail/v1/", "status": 503, "times": 1000 }))
        .await;
    assert!(matches!(harness.store.sync().await, Err(Error::Gmail(_))));
    assert_eq!(
        outage(&harness.store),
        None,
        "Gmail answered, and only no answer counts"
    );
}

#[tokio::test]
async fn a_disabled_gmail_says_so_over_everything_else() {
    let mut harness = Harness::synced().await;
    let store = harness.store.clone();
    harness
        .fault(json!({ "path": "/gmail/v1/", "status": 400, "message": "Mail service not enabled", "reason": "failedPrecondition", "times": 1000 }))
        .await;
    assert!(store.sync().await.is_err());
    assert_eq!(outage(&store), Some(Outage::GmailDisabled));
    harness.server.disconnect().await;
    store.network_changed().await;
    assert_eq!(
        outage(&store),
        Some(Outage::GmailDisabled),
        "still the one the bar names"
    );

    harness
        .server
        .reconnect()
        .await
        .expect("the mock comes back");
    harness.clear_faults().await;
    tokio::time::sleep(DEBOUNCE.down).await;
    store.check_connection().await;
    assert_eq!(outage(&store), None);
}

/// Tokens from a sign-in service that stops answering once `gone`.
#[derive(Clone)]
struct Refreshing {
    gone: Arc<AtomicBool>,
    token: reqwest::Url,
}

impl TokenProvider for Refreshing {
    fn access_token(&self) -> TokenFuture<'_> {
        Box::pin(async move {
            if self.gone.load(Ordering::SeqCst) {
                // A refresh at a token endpoint nothing listens on.
                return Err(gmail::Error::Http(
                    reqwest::get(self.token.clone())
                        .await
                        .expect_err("nothing listens"),
                ));
            }
            Ok(DEV_ACCESS_TOKEN.to_owned())
        })
    }
}

#[tokio::test]
async fn a_sign_in_service_that_doesnt_answer_is_googles_oauth() {
    let harness = Harness::synced().await;
    let dir = TempDir::new();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("a free port")
        .port();
    let mut endpoints = harness.server.endpoints();
    endpoints.oauth.token = format!("http://127.0.0.1:{port}/token")
        .parse()
        .expect("a URL");
    let tokens = Refreshing {
        gone: Arc::default(),
        token: endpoints.oauth.token.clone(),
    };
    let store = Store::open_with(dir.path(), ACCOUNT, endpoints, tokens.clone(), options())
        .await
        .expect("store opens");
    store.check_connection().await;
    assert_eq!(outage(&store), None);

    tokens.gone.store(true, Ordering::SeqCst);
    assert!(matches!(store.sync().await, Err(Error::Offline(_))));
    assert_eq!(outage(&store), Some(Outage::Auth));

    tokens.gone.store(false, Ordering::SeqCst);
    tokio::time::sleep(DEBOUNCE.down + Duration::from_millis(10)).await;
    store.check_connection().await;
    assert_eq!(outage(&store), None);
}
