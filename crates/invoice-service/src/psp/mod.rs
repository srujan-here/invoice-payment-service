//! HTTP client for the mock PSP. We treat the PSP as a real, untrusted external
//! dependency: a bounded timeout (shorter than the PSP's 30s `tok_timeout`), and
//! a tri-state outcome so an *unknown* result (timeout / network error) is never
//! conflated with a definite failure.

use std::time::Duration;

use serde::Deserialize;

/// The result of a charge call, from the invoice service's point of view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PspResult {
    Succeeded { psp_ref: String },
    /// A definite decline (the card was bad). Safe to mark the attempt failed.
    Declined { code: String },
    /// We do not know whether money moved (timeout, dropped connection, 5xx).
    /// The attempt must stay `pending`; the reconciler resolves it later by
    /// re-querying with the same idempotency key.
    Unknown { reason: &'static str },
}

#[derive(Clone)]
pub struct PspClient {
    http: reqwest::Client,
    base_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum PspResponse {
    Succeeded { psp_ref: String },
    Failed { code: String },
}

impl PspClient {
    pub fn new(base_url: impl Into<String>, timeout: Duration) -> Self {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("build reqwest client");
        Self {
            http,
            base_url: base_url.into(),
        }
    }

    /// Attempt a charge. `idempotency_key` is forwarded so the PSP dedupes — a
    /// re-call with the same key returns the original single outcome.
    pub async fn charge(
        &self,
        card_token: &str,
        amount_cents: i64,
        idempotency_key: &str,
    ) -> PspResult {
        let body = serde_json::json!({
            "card_token": card_token,
            "amount_cents": amount_cents,
            "currency": "USD",
            "idempotency_key": idempotency_key,
        });

        let resp = match self
            .http
            .post(format!("{}/charge", self.base_url))
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                // Distinguish a timeout (PSP too slow) from any other transport
                // error (connection drop, DNS, etc). Both are "unknown".
                let reason = if e.is_timeout() {
                    "psp_timeout"
                } else {
                    "psp_network_error"
                };
                tracing::warn!(error = %e, reason, "PSP call failed");
                return PspResult::Unknown { reason };
            }
        };

        if resp.status().is_server_error() {
            tracing::warn!(status = %resp.status(), "PSP returned 5xx");
            return PspResult::Unknown {
                reason: "psp_network_error",
            };
        }

        match resp.json::<PspResponse>().await {
            Ok(PspResponse::Succeeded { psp_ref }) => PspResult::Succeeded { psp_ref },
            Ok(PspResponse::Failed { code }) => PspResult::Declined { code },
            Err(e) => {
                tracing::warn!(error = %e, "could not parse PSP response");
                PspResult::Unknown {
                    reason: "psp_network_error",
                }
            }
        }
    }
}
