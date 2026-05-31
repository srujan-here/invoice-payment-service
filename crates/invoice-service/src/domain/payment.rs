//! Payment attempt entity, status enum, and DTOs.

use chrono::{DateTime, Utc};
use common::Money;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Mirrors the `payment_status` Postgres enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "payment_status", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum PaymentStatus {
    /// In-flight, or outcome unknown (PSP timeout / our crash). The reconciler
    /// owns these. A pending attempt pins the invoice via the partial unique index.
    Pending,
    Succeeded,
    Failed,
    /// Only reachable on a PSP-confirmed "not charged".
    Expired,
}

impl PaymentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PaymentStatus::Pending => "pending",
            PaymentStatus::Succeeded => "succeeded",
            PaymentStatus::Failed => "failed",
            PaymentStatus::Expired => "expired",
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PaymentAttempt {
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub business_id: Uuid,
    pub idempotency_key: String,
    pub amount_cents: Money,
    pub currency: String,
    pub card_token: String,
    pub status: PaymentStatus,
    pub psp_ref: Option<String>,
    pub failure_code: Option<String>,
    pub last_error: Option<String>,
    pub attempt_count: i32,
    pub next_poll_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct PayRequest {
    /// A mock card token, e.g. "tok_success". The amount is taken from the
    /// invoice, never from this request.
    pub card_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentAttemptResponse {
    pub attempt_id: Uuid,
    pub invoice_id: Uuid,
    pub status: PaymentStatus,
    pub amount_cents: i64,
    pub invoice_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub psp_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poll_url: Option<String>,
}
