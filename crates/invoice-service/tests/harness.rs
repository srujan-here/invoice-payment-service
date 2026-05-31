//! Shared test harness: an ephemeral Postgres (testcontainers), an in-process
//! invoice-service, and a tiny call-counting mock PSP. Re-exported into each
//! integration test via `mod harness;`.
//!
//! Kept in `tests/` and pulled in with `#[path = "harness.rs"] mod harness;` so
//! it is compiled per test binary without being mistaken for its own test.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use common::config::ServiceConfig;
use invoice_service::{build_app, build_state};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt};

/// The seed business + API key created by migration 0002/0003.
pub const SEED_KEY: &str = "dpk_test_seedkey0000000000000000000000000000";

/// A call-counting mock PSP. Behaviour mirrors the real mock but timeouts are
/// short so tests are fast.
#[derive(Clone, Default)]
pub struct MockPsp {
    pub calls: Arc<AtomicUsize>,
}

impl MockPsp {
    pub fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[derive(serde::Deserialize)]
struct ChargeReq {
    card_token: String,
}

async fn mock_charge(State(psp): State<MockPsp>, Json(req): Json<ChargeReq>) -> Json<serde_json::Value> {
    psp.calls.fetch_add(1, Ordering::SeqCst);
    match req.card_token.as_str() {
        "tok_success" => Json(serde_json::json!({"status":"succeeded","psp_ref": uuid::Uuid::new_v4()})),
        "tok_card_declined" => Json(serde_json::json!({"status":"failed","code":"card_declined"})),
        "tok_timeout" => {
            // Longer than the test's PSP client timeout, so the client gives up.
            tokio::time::sleep(Duration::from_secs(5)).await;
            Json(serde_json::json!({"status":"succeeded","psp_ref": uuid::Uuid::new_v4()}))
        }
        _ => Json(serde_json::json!({"status":"failed","code":"card_declined"})),
    }
}

pub struct TestApp {
    pub base_url: String,
    pub psp: MockPsp,
    pub client: reqwest::Client,
    // Held to keep the container alive for the test's lifetime.
    _pg: ContainerAsync<Postgres>,
}

impl TestApp {
    /// Boot Postgres + mock PSP + app. `psp_timeout` lets the timeout test use a
    /// short client deadline.
    pub async fn start(psp_timeout: Duration) -> TestApp {
        // Pin to PG16 (matches docker-compose). The crate's default image is too
        // old to support GENERATED ... STORED columns.
        let pg = Postgres::default()
            .with_tag("16-alpine")
            .start()
            .await
            .expect("start postgres container");
        let port = pg.get_host_port_ipv4(5432).await.expect("pg port");
        let database_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

        let pool = invoice_service::db::connect_and_migrate(&database_url)
            .await
            .expect("connect + migrate");

        // Spawn the mock PSP on a random port.
        let psp = MockPsp::default();
        let psp_router = Router::new()
            .route("/charge", post(mock_charge))
            .route("/healthz", get(|| async { "ok" }))
            .with_state(psp.clone());
        let psp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let psp_addr: SocketAddr = psp_listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(psp_listener, psp_router).await.unwrap() });

        let config = ServiceConfig {
            database_url,
            bind_addr: "127.0.0.1:0".into(),
            psp_base_url: format!("http://{psp_addr}"),
            psp_timeout,
            webhook_max_attempts: 8,
        };
        let state = build_state(pool, config);

        // Note: we deliberately do NOT spawn the reconciler here, so a pending
        // attempt stays pending and the timeout test can assert "not stuck".
        let app = build_app(state);
        let app_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let app_addr: SocketAddr = app_listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(app_listener, app).await.unwrap() });

        TestApp {
            base_url: format!("http://{app_addr}"),
            psp,
            client: reqwest::Client::new(),
            _pg: pg,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    pub async fn create_customer(&self) -> uuid::Uuid {
        let resp = self
            .client
            .post(self.url("/v1/customers"))
            .bearer_auth(SEED_KEY)
            .json(&serde_json::json!({"name":"Test Customer","email":null}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 201, "create customer");
        let body: serde_json::Value = resp.json().await.unwrap();
        uuid::Uuid::parse_str(body["id"].as_str().unwrap()).unwrap()
    }

    /// Create + finalize an invoice; returns its id. Total = qty * unit.
    pub async fn open_invoice(&self, customer_id: uuid::Uuid) -> uuid::Uuid {
        let resp = self
            .client
            .post(self.url("/v1/invoices"))
            .bearer_auth(SEED_KEY)
            .json(&serde_json::json!({
                "customer_id": customer_id,
                "line_items": [{"description":"Widget","quantity":2,"unit_amount_cents":2500}]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 201, "create invoice");
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["total_cents"], 5000, "server-computed total");
        let id = uuid::Uuid::parse_str(body["id"].as_str().unwrap()).unwrap();

        let fin = self
            .client
            .post(self.url(&format!("/v1/invoices/{id}/finalize")))
            .bearer_auth(SEED_KEY)
            .send()
            .await
            .unwrap();
        assert_eq!(fin.status(), 200, "finalize");
        id
    }

    pub async fn pay(
        &self,
        invoice_id: uuid::Uuid,
        idempotency_key: &str,
        card_token: &str,
    ) -> reqwest::Response {
        self.client
            .post(self.url(&format!("/v1/invoices/{invoice_id}/pay")))
            .bearer_auth(SEED_KEY)
            .header("Idempotency-Key", idempotency_key)
            .json(&serde_json::json!({"card_token": card_token}))
            .send()
            .await
            .unwrap()
    }

    pub async fn get_invoice_state(&self, invoice_id: uuid::Uuid) -> String {
        let resp = self
            .client
            .get(self.url(&format!("/v1/invoices/{invoice_id}")))
            .bearer_auth(SEED_KEY)
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = resp.json().await.unwrap();
        body["state"].as_str().unwrap().to_string()
    }
}
