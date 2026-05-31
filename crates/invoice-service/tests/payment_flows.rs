//! The three required tests: concurrency, idempotency, and PSP-failure safety.
//! Each boots a real ephemeral Postgres + the full app + a call-counting mock
//! PSP, and drives the public HTTP API.

#[path = "harness.rs"]
mod harness;

use std::time::{Duration, Instant};

use futures::future::join_all;
use harness::TestApp;

/// Concurrency: N simultaneous /pay for the same invoice. At most one may
/// succeed, the PSP must be charged at most once, and the invoice ends `paid`.
#[tokio::test]
async fn concurrent_pays_charge_at_most_once() {
    let app = TestApp::start(Duration::from_secs(5)).await;
    let customer = app.create_customer().await;
    let invoice = app.open_invoice(customer).await;

    // 12 concurrent requests, each with a DISTINCT idempotency key (so this
    // exercises the single-active-charge guard, not idempotency dedupe).
    let futures = (0..12).map(|i| {
        let app = &app;
        let inv = invoice;
        async move {
            let key = format!("concurrent-{i}");
            app.pay(inv, &key, "tok_success").await.status().as_u16()
        }
    });
    let statuses = join_all(futures).await;

    let successes = statuses.iter().filter(|&&s| s == 200).count();
    let conflicts = statuses.iter().filter(|&&s| s == 409).count();

    assert_eq!(successes, 1, "exactly one payment may succeed: {statuses:?}");
    assert_eq!(successes + conflicts, 12, "all others must be 409: {statuses:?}");
    assert_eq!(app.psp.count(), 1, "the PSP must be charged exactly once");
    assert_eq!(app.get_invoice_state(invoice).await, "paid");
}

/// Idempotency: the same key + body twice returns the same result and does NOT
/// call the PSP a second time.
#[tokio::test]
async fn idempotent_retry_returns_same_result_without_second_charge() {
    let app = TestApp::start(Duration::from_secs(5)).await;
    let customer = app.create_customer().await;
    let invoice = app.open_invoice(customer).await;

    let first = app.pay(invoice, "idem-key-1", "tok_success").await;
    assert_eq!(first.status(), 200);
    let first_body: serde_json::Value = first.json().await.unwrap();

    let second = app.pay(invoice, "idem-key-1", "tok_success").await;
    assert_eq!(second.status(), 200, "replay returns the cached 200");
    let second_body: serde_json::Value = second.json().await.unwrap();

    assert_eq!(
        first_body["attempt_id"], second_body["attempt_id"],
        "replay returns the same attempt"
    );
    assert_eq!(app.psp.count(), 1, "no second PSP call on replay");
    assert_eq!(app.get_invoice_state(invoice).await, "paid");
}

/// PSP failure: a 30s-class timeout must not hang the endpoint and must not
/// corrupt invoice state. We use a short PSP client timeout so the test is fast.
#[tokio::test]
async fn psp_timeout_does_not_hang_or_corrupt_state() {
    let app = TestApp::start(Duration::from_millis(500)).await;
    let customer = app.create_customer().await;
    let invoice = app.open_invoice(customer).await;

    let start = Instant::now();
    let resp = app.pay(invoice, "timeout-key", "tok_timeout").await;
    let elapsed = start.elapsed();

    // Returned promptly (well under the mock's 5s sleep), with 202 Accepted.
    assert!(elapsed < Duration::from_secs(3), "endpoint hung for {elapsed:?}");
    assert_eq!(resp.status(), 202, "unknown outcome -> 202 Accepted");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "pending");

    // The invoice is NOT stuck in a bad state: still payable (open), not paid.
    assert_eq!(app.get_invoice_state(invoice).await, "open");

    // The attempt is recorded as pending (the reconciler would resolve it).
    let attempt_id = body["attempt_id"].as_str().unwrap();
    let poll = app
        .client
        .get(format!("{}/v1/invoices/{invoice}/payments/{attempt_id}", app.base_url))
        .bearer_auth(harness::SEED_KEY)
        .send()
        .await
        .unwrap();
    let poll_body: serde_json::Value = poll.json().await.unwrap();
    assert_eq!(poll_body["status"], "pending");
}
