//! API key DTOs. The entity itself never leaves the auth layer.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: Option<String>,
    /// "live" or "test". Defaults to "test".
    #[serde(default)]
    pub env: Option<String>,
}

/// Returned exactly once at creation: this is the only time the full secret is
/// visible.
#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    pub id: Uuid,
    pub prefix: String,
    pub key: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ApiKeyListItem {
    pub id: Uuid,
    pub name: Option<String>,
    pub prefix: String,
    pub last_four: String,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}
