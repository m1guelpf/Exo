//! The account list: persistence, removal and order.

mod support;

use exo_auth::{Error, SecretStore};
use support::{EMAIL, Harness};

#[tokio::test]
async fn accounts_persist_across_launches() {
    let harness = Harness::new().await;
    let account = harness.sign_in().await;
    assert_eq!(harness.relaunch().accounts(), [account]);
}

#[tokio::test]
async fn forgetting_an_account_leaves_its_grant_at_google() {
    let harness = Harness::new().await;
    harness.sign_in().await;
    let grant = harness.secrets.get(EMAIL).unwrap().unwrap();
    let tokens = harness.auth.tokens(EMAIL).unwrap();

    harness.auth.forget(EMAIL).unwrap();
    assert!(harness.auth.accounts().is_empty());
    assert!(harness.relaunch().accounts().is_empty());
    assert_eq!(harness.secrets.get(EMAIL).unwrap(), None);
    assert!(
        tokens.needs_sign_in(),
        "a running store stops getting tokens"
    );
    harness
        .oauth()
        .refresh(&grant)
        .await
        .expect("Google still honors the grant");

    assert!(matches!(
        harness.auth.tokens(EMAIL),
        Err(Error::UnknownAccount(_))
    ));
    assert!(matches!(
        harness.auth.forget(EMAIL),
        Err(Error::UnknownAccount(_))
    ));
}

#[tokio::test]
async fn moving_an_unknown_account_fails() {
    let harness = Harness::new().await;
    harness.sign_in().await;
    harness.auth.move_account(EMAIL, 3).unwrap();
    assert!(matches!(
        harness.auth.move_account("nobody@example.com", 0),
        Err(Error::UnknownAccount(_))
    ));
}

#[tokio::test]
async fn a_profile_photo_is_kept_until_the_account_signs_out() {
    let harness = Harness::new().await;
    harness.sign_in().await;
    assert_eq!(harness.auth.profile_picture(EMAIL).unwrap(), None);
    harness
        .auth
        .set_profile_picture(EMAIL, Some(b"photo"))
        .unwrap();
    assert_eq!(
        harness
            .relaunch()
            .profile_picture(EMAIL)
            .unwrap()
            .as_deref(),
        Some(&b"photo"[..]),
        "kept across launches"
    );
    harness.auth.set_profile_picture(EMAIL, None).unwrap();
    assert_eq!(
        harness.auth.profile_picture(EMAIL).unwrap(),
        None,
        "Google has none any more"
    );

    harness
        .auth
        .set_profile_picture(EMAIL, Some(b"photo"))
        .unwrap();
    harness.auth.forget(EMAIL).unwrap();
    assert_eq!(harness.relaunch().profile_picture(EMAIL).unwrap(), None);
    assert!(
        matches!(
            harness.auth.set_profile_picture(EMAIL, Some(b"late")),
            Err(Error::UnknownAccount(_))
        ),
        "a fetch that ends after signing out saves nothing"
    );
    let folder = std::fs::read_dir(harness.dir.path().join("profile-pictures")).unwrap();
    assert_eq!(folder.count(), 0, "nothing is left of it");
}
