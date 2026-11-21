//! Pacing under Gmail's per-user quota, against a mock that enforces one: a first sync that
//! would run into it within a second never does, and what the user does doesn't wait for the
//! backfill.

mod support;

use std::time::Duration;

use exo_store::{ListId, Mutation, Options, Quota};
use gmail_mock::Config;
use serde_json::Value;
use support::Harness;
use tokio::time::Instant;

/// Gmail's own, about half of the fixtures' first sync (10,800 units).
const PER_MINUTE: u32 = 6_000;

async fn start(paced: bool) -> Harness {
    let config = Config {
        quota: Some(PER_MINUTE),
        ..Config::default()
    };
    let options = Options {
        quota: paced.then_some(Quota {
            per_minute: PER_MINUTE,
        }),
        ..support::options()
    };
    Harness::start_with(config, options).await
}

/// Starts syncing in the background, as the app does while the store backfills.
fn syncing(harness: &Harness) -> tokio::task::JoinHandle<()> {
    let store = harness.store.clone();
    tokio::spawn(async move { while store.sync().await.is_ok_and(|report| report.backfilling) {} })
}

async fn spent(harness: &Harness) -> Value {
    harness
        .admin(reqwest::Method::GET, "/__mock/quota", Value::Null)
        .await
}

#[tokio::test]
async fn sending_as_fast_as_it_can_runs_into_the_quota() {
    let harness = start(false).await;
    let sync = syncing(&harness);
    tokio::time::sleep(Duration::from_secs(2)).await;
    sync.abort();
    let quota = spent(&harness).await;
    assert!(quota["refused"].as_u64() > Some(0), "{quota}");
}

#[tokio::test]
async fn a_paced_first_sync_never_does_and_a_triage_goes_first() {
    let harness = start(true).await;
    let started = Instant::now();
    let sync = syncing(&harness);
    let thread = loop {
        if let Some(thread) = harness.all(&ListId::All).await.into_iter().next() {
            break thread;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    harness
        .store
        .apply(std::slice::from_ref(&thread.id), Mutation::Star)
        .await
        .unwrap();
    let asked = Instant::now();
    let report = harness.store.flush().await.unwrap();
    assert_eq!(report.completed, 1);
    assert!(
        asked.elapsed() < Duration::from_millis(500),
        "the star waited {:?}",
        asked.elapsed()
    );

    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(!sync.is_finished(), "the first sync was still going");
    sync.abort();
    let quota = spent(&harness).await;
    assert_eq!(quota["refused"], 0, "{quota}");
    // No more than a full bucket (45 % of the quota) and its refill since (90 % a minute).
    let most = f64::from(PER_MINUTE) * (0.45 + 0.9 * started.elapsed().as_secs_f64() / 60.0);
    assert!(
        quota["units"].as_f64().unwrap() <= most,
        "{quota} in {:?}",
        started.elapsed()
    );
}
