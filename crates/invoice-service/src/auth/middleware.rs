//! The auth middleware and the `AuthBusiness` extractor.
//!
//! `require_api_key` runs ahead of protected routes: it parses the bearer
//! token, looks the key up by prefix (single indexed row), constant-time
//! compares the SHA-256, checks revocation/expiry, and stashes the resolved
//! `business_id` in request extensions. Handlers then pull it out via
//! `AuthBusiness`, so tenant scope is impossible to forget.

use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{header::AUTHORIZATION, Request};
use axum::middleware::Next;
use axum::response::Response;
use common::AppError;
use uuid::Uuid;

use super::api_key::{constant_time_eq, derive_prefix, hash_key};
use crate::state::AppState;

/// The authenticated business id, available to any handler behind the auth layer.
#[derive(Debug, Clone, Copy)]
pub struct AuthBusiness(pub Uuid);

#[axum::async_trait]
impl<S: Send + Sync> FromRequestParts<S> for AuthBusiness {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthBusiness>()
            .copied()
            .ok_or(AppError::Unauthorized)
    }
}

/// Row we fetch for verification.
#[derive(sqlx::FromRow)]
struct KeyRow {
    business_id: Uuid,
    key_hash: Vec<u8>,
}

pub async fn require_api_key(
    State(state): State<AppState>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, AppError> {
    let token = bearer_token(&req).ok_or(AppError::Unauthorized)?;
    let prefix = derive_prefix(&token);

    let row = sqlx::query_as::<_, KeyRow>(
        r#"
        SELECT business_id, key_hash
        FROM api_keys
        WHERE prefix = $1
          AND revoked_at IS NULL
          AND (expires_at IS NULL OR expires_at > now())
        "#,
    )
    .bind(prefix)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| AppError::Internal(e.into()))?
    .ok_or(AppError::Unauthorized)?;

    if !constant_time_eq(&row.key_hash, &hash_key(&token)) {
        return Err(AppError::Unauthorized);
    }

    // Best-effort, non-blocking last_used_at touch.
    let pool = state.pool.clone();
    let p = prefix.to_string();
    tokio::spawn(async move {
        let _ = sqlx::query("UPDATE api_keys SET last_used_at = now() WHERE prefix = $1")
            .bind(p)
            .execute(&pool)
            .await;
    });

    req.extensions_mut().insert(AuthBusiness(row.business_id));
    Ok(next.run(req).await)
}

fn bearer_token(req: &Request<axum::body::Body>) -> Option<String> {
    let raw = req.headers().get(AUTHORIZATION)?.to_str().ok()?;
    raw.strip_prefix("Bearer ")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
