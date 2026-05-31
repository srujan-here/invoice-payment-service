//! Mock Payment Service Provider.
//!
//! A standalone HTTP service that stands in for a real PSP. The invoice service
//! talks to it over the network and must treat it as a real external dependency
//! — including its slow and failing behaviours.
//!
//! Outcome is determined purely by the card token:
//!   tok_success           -> 200 { status: "succeeded", psp_ref } after ~100ms
//!   tok_insufficient_funds -> 200 { status: "failed", code: "insufficient_funds" } after ~100ms
//!   tok_card_declined     -> 200 { status: "failed", code: "card_declined" } after ~100ms
//!   tok_timeout           -> sleeps 30s then succeeds (caller must time out first)
//!   tok_network_error     -> 500 (a real network drop, from the caller's POV)
//!   anything else         -> 400 unknown_token
//!
//! It also honours an `idempotency_key`: a repeat key returns the original
//! recorded outcome. This is load-bearing for the invoice service's reconciler
//! — re-querying a charge with the same key never produces a second charge.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Default)]
struct PspState {
    /// idempotency_key -> previously returned outcome.
    seen: Arc<Mutex<HashMap<String, ChargeOutcome>>>,
}

#[derive(Debug, Deserialize)]
struct ChargeRequest {
    card_token: String,
    #[serde(default)]
    amount_cents: i64,
    #[serde(default)]
    idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ChargeOutcome {
    Succeeded { psp_ref: String },
    Failed { code: String },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,mock_psp=debug".into()),
        )
        .init();

    let bind = std::env::var("MOCK_PSP_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:9090".into());
    let state = PspState::default();

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/charge", post(charge))
        .with_state(state)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .expect("bind mock-psp");
    tracing::info!(%bind, "mock-psp listening");
    axum::serve(listener, app).await.expect("serve mock-psp");
}

async fn charge(State(state): State<PspState>, Json(req): Json<ChargeRequest>) -> Response {
    tracing::debug!(token = %req.card_token, amount_cents = req.amount_cents, "charge requested");

    // Idempotent replay: a known key returns its original recorded outcome,
    // without re-running the (possibly money-moving) effect.
    if let Some(key) = req.idempotency_key.as_ref() {
        if let Some(prev) = state.seen.lock().unwrap().get(key).cloned() {
            tracing::debug!(%key, "idempotent replay");
            return (StatusCode::OK, Json(prev)).into_response();
        }
    }

    let outcome: ChargeOutcome = match req.card_token.as_str() {
        "tok_success" => {
            tokio::time::sleep(Duration::from_millis(100)).await;
            ChargeOutcome::Succeeded {
                psp_ref: Uuid::new_v4().to_string(),
            }
        }
        "tok_insufficient_funds" => {
            tokio::time::sleep(Duration::from_millis(100)).await;
            ChargeOutcome::Failed {
                code: "insufficient_funds".into(),
            }
        }
        "tok_card_declined" => {
            tokio::time::sleep(Duration::from_millis(100)).await;
            ChargeOutcome::Failed {
                code: "card_declined".into(),
            }
        }
        "tok_timeout" => {
            // The caller's HTTP client must abandon us before this returns.
            tokio::time::sleep(Duration::from_secs(30)).await;
            ChargeOutcome::Succeeded {
                psp_ref: Uuid::new_v4().to_string(),
            }
        }
        "tok_network_error" => {
            tracing::debug!("simulating network error (500)");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "psp_unavailable" })),
            )
                .into_response();
        }
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "unknown_token",
                    "message": format!("unknown card token: {other}"),
                })),
            )
                .into_response();
        }
    };

    // Record the outcome under the idempotency key so a re-query is stable.
    if let Some(key) = req.idempotency_key {
        state.seen.lock().unwrap().insert(key, outcome.clone());
    }

    (StatusCode::OK, Json(outcome)).into_response()
}
