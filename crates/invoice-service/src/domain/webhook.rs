//! Webhook entities and DTOs: endpoints, the outbox event, and per-endpoint
//! delivery work items.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The canonical event-type strings. Kept as constants so producers and the
/// OpenAPI doc never drift.
pub mod event_types {
    pub const INVOICE_CREATED: &str = "invoice.created";
    pub const INVOICE_PAID: &str = "invoice.paid";
    pub const INVOICE_PAYMENT_FAILED: &str = "invoice.payment_failed";
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct WebhookEndpoint {
    pub id: Uuid,
    pub business_id: Uuid,
    pub url: String,
    pub secret: Vec<u8>,
    pub event_types: Vec<String>,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateEndpointRequest {
    pub url: String,
    /// Optional subscription filter. Empty/absent = all event types.
    #[serde(default)]
    pub event_types: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateEndpointResponse {
    pub id: Uuid,
    pub url: String,
    pub event_types: Vec<String>,
    /// The signing secret, hex-encoded. Shown once at creation.
    pub secret: String,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct WebhookEventRow {
    pub id: Uuid,
    pub event_type: String,
    pub aggregate_id: Uuid,
    pub payload: serde_json::Value,
    pub sequence: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct WebhookDeliveryRow {
    pub id: Uuid,
    pub event_id: Uuid,
    pub endpoint_id: Uuid,
    pub status: String,
    pub attempts: i32,
    pub next_attempt_at: DateTime<Utc>,
    pub last_status_code: Option<i32>,
    pub last_error: Option<String>,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}
