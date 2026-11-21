//! Calendar accounts: other Google accounts a mail account signs in for their calendars alone,
//! against gmail-mock (a `login_hint` signs in another address there, with calendars of its
//! own).

mod support;

use exo_auth::{CALENDAR_SCOPES, Error, SecretStore};
use support::{EMAIL, Harness, browser, param};
use url::Url;

/// Another account of the user's.
const PERSONAL: &str = "maya.lin@example.com";

/// A consent URL with the user picking `email` at Google.
fn picking(url: &Url, email: &str) -> Url {
    let mut visit = url.clone();
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| key != "login_hint")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    visit
        .query_pairs_mut()
        .clear()
        .extend_pairs(pairs)
        .append_pair("login_hint", email);
    visit
}

#[tokio::test]
async fn a_calendar_account_asks_for_calendar_alone_and_its_grant_is_the_mail_accounts() {
    let harness = Harness::new().await;
    harness.sign_in().await;
    // Its Gmail isn't asked about: a calendar account needs no mailbox.
    harness.fault(serde_json::json!({ "path": "/gmail/v1/users/me/profile", "status": 400, "message": "Mail service not enabled" })).await;
    let (open, page) = browser(|url| picking(url, PERSONAL));
    let email = harness
        .auth
        .sign_in_calendar(EMAIL, None, open)
        .await
        .unwrap();
    assert_eq!(email, PERSONAL);
    let page = page.await.unwrap();
    assert_eq!(param(&page.opened, "scope"), CALENDAR_SCOPES.join(" "));
    assert_eq!(param(&page.opened, "prompt"), "consent");

    let mail = &harness.auth.accounts()[0];
    assert_eq!(
        (mail.email.as_str(), mail.calendars.as_slice()),
        (EMAIL, [PERSONAL.to_owned()].as_slice()),
        "listed under the mail account, which stays the only account"
    );
    assert_eq!(harness.auth.accounts().len(), 1);
    assert!(
        harness.secrets.get(PERSONAL).unwrap().is_none(),
        "kept apart from a mail grant of its own"
    );

    // Its tokens read its own calendars, on the next launch too.
    for auth in [harness.auth.clone(), harness.relaunch()] {
        let tokens = auth.calendar_tokens(EMAIL, PERSONAL).unwrap();
        let calendar = gmail::Client::new(auth.endpoints().clone(), tokens);
        let list = calendar
            .calendar()
            .calendar_list(&gmail::calendar::CalendarListQuery::default())
            .await
            .unwrap();
        assert_eq!(
            list.items
                .iter()
                .find(|entry| entry.primary == Some(true))
                .and_then(|entry| entry.id.as_deref()),
            Some(PERSONAL)
        );
    }
    assert!(matches!(
        harness.auth.calendar_tokens(EMAIL, "someone@example.com"),
        Err(Error::UnknownAccount(_))
    ));
}

#[tokio::test]
async fn signing_in_another_account_than_the_one_asked_for_saves_nothing() {
    let harness = Harness::new().await;
    harness.sign_in().await;
    let (open, _page) = browser(|url| picking(url, "someone@example.com"));
    let error = harness
        .auth
        .sign_in_calendar(EMAIL, Some(PERSONAL), open)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, Error::WrongAccount { expected } if expected == PERSONAL),
        "{error}"
    );
    assert!(harness.auth.accounts()[0].calendars.is_empty());

    // The mail account's own address needs no grant of its own.
    let (open, _page) = browser(Clone::clone);
    assert_eq!(
        harness
            .auth
            .sign_in_calendar(EMAIL, None, open)
            .await
            .unwrap(),
        EMAIL
    );
    assert!(harness.auth.accounts()[0].calendars.is_empty());
}

#[tokio::test]
async fn removing_a_calendar_account_or_signing_its_mail_account_out_forgets_its_grant() {
    let harness = Harness::new().await;
    harness.sign_in().await;
    for email in [PERSONAL, "bea@example.com"] {
        let (open, _page) = browser(move |url| picking(url, email));
        harness
            .auth
            .sign_in_calendar(EMAIL, None, open)
            .await
            .unwrap();
    }
    let tokens = harness.auth.calendar_tokens(EMAIL, PERSONAL).unwrap();
    harness.auth.forget_calendar(EMAIL, PERSONAL).unwrap();
    assert!(tokens.needs_sign_in(), "its tokens stop");
    assert_eq!(harness.auth.accounts()[0].calendars, ["bea@example.com"]);

    let bea = harness
        .auth
        .calendar_tokens(EMAIL, "bea@example.com")
        .unwrap();
    harness.auth.forget(EMAIL).unwrap();
    assert!(
        bea.needs_sign_in(),
        "the mail account's sign-out takes its calendar accounts"
    );
    let relaunched = harness.relaunch();
    assert!(relaunched.accounts().is_empty());
}
