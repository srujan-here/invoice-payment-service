//! API-key persistence (creation, listing, revocation). Verification lookups
//! live in the auth middleware (hot path).

use common::ids::new_id;
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::GeneratedKey;
use crate::domain::api_key::ApiKeyListItem;

pub async fn insert(
    pool: &PgPool,
    business_id: Uuid,
    name: Option<&str>,
    key: &GeneratedKey,
) -> Result<Uuid, sqlx::Error> {
    let id = new_id();
    sqlx::query(
        r#"
        INSERT INTO api_keys (id, business_id, name, prefix, key_hash, last_four)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(id)
    .bind(business_id)
    .bind(name)
    .bind(&key.prefix)
    .bind(&key.key_hash)
    .bind(&key.last_four)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn list(pool: &PgPool, business_id: Uuid) -> Result<Vec<ApiKeyListItem>, sqlx::Error> {
    sqlx::query_as::<_, ApiKeyListItem>(
        r#"
        SELECT id, name, prefix, last_four, created_at, revoked_at
        FROM api_keys
        WHERE business_id = $1
        ORDER BY created_at DESC
        "#,
    )
    .bind(business_id)
    .fetch_all(pool)
    .await
}

/// Soft revoke. Returns true if a (still-active) key was revoked.
pub async fn revoke(pool: &PgPool, business_id: Uuid, id: Uuid) -> Result<bool, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE api_keys SET revoked_at = now() WHERE id = $1 AND business_id = $2 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(business_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}
