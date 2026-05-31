//! Customer persistence. All queries are scoped by `business_id`.

use chrono::{DateTime, Utc};
use common::ids::new_id;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::customer::Customer;

pub async fn insert(
    pool: &PgPool,
    business_id: Uuid,
    email: Option<&str>,
    name: Option<&str>,
) -> Result<Customer, sqlx::Error> {
    sqlx::query_as::<_, Customer>(
        r#"
        INSERT INTO customers (id, business_id, email, name)
        VALUES ($1, $2, $3, $4)
        RETURNING id, business_id, email::text AS email, name, created_at, updated_at
        "#,
    )
    .bind(new_id())
    .bind(business_id)
    .bind(email)
    .bind(name)
    .fetch_one(pool)
    .await
}

pub async fn get(
    pool: &PgPool,
    business_id: Uuid,
    id: Uuid,
) -> Result<Option<Customer>, sqlx::Error> {
    sqlx::query_as::<_, Customer>(
        r#"
        SELECT id, business_id, email::text AS email, name, created_at, updated_at
        FROM customers
        WHERE id = $1 AND business_id = $2
        "#,
    )
    .bind(id)
    .bind(business_id)
    .fetch_optional(pool)
    .await
}

/// Keyset-paginated list. Returns one extra row beyond `limit` is *not* done
/// here; the service decides the cursor. Here we simply page on (created_at, id).
pub async fn list(
    pool: &PgPool,
    business_id: Uuid,
    limit: i64,
    cursor: Option<(DateTime<Utc>, Uuid)>,
) -> Result<Vec<Customer>, sqlx::Error> {
    match cursor {
        Some((ts, id)) => {
            sqlx::query_as::<_, Customer>(
                r#"
                SELECT id, business_id, email::text AS email, name, created_at, updated_at
                FROM customers
                WHERE business_id = $1
                  AND (created_at, id) < ($2, $3)
                ORDER BY created_at DESC, id DESC
                LIMIT $4
                "#,
            )
            .bind(business_id)
            .bind(ts)
            .bind(id)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
        None => {
            sqlx::query_as::<_, Customer>(
                r#"
                SELECT id, business_id, email::text AS email, name, created_at, updated_at
                FROM customers
                WHERE business_id = $1
                ORDER BY created_at DESC, id DESC
                LIMIT $2
                "#,
            )
            .bind(business_id)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
    }
}
