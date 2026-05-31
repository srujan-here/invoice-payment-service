//! Webhook use-cases: emitting outbox events (called inside the state-change
//! transaction), endpoint registration, and the reconciliation endpoints
//! (`/events`, `/webhook-deliveries`, redeliver).

use common::ids::new_id;
use common::pagination::{clamp_limit, decode_cursor, encode_cursor, Page};
use common::{AppError, AppResult};
use rand::RngCore;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::webhook::{
    CreateEndpointRequest, CreateEndpointResponse, WebhookDeliveryRow, WebhookEventRow,
};

/// Insert an outbox event. MUST be called with the same executor/transaction as
/// the state change it describes, so the event is atomic with the change — no
/// lost events, no phantom events. Fan-out to deliveries happens later in the
/// dispatcher; here we only record the immutable fact.
pub async fn emit_event<'e, E>(
    exec: E,
    business_id: Uuid,
    event_type: &str,
    aggregate_id: Uuid,
    payload: serde_json::Value,
) -> Result<Uuid, sqlx::Error>
where
    E: sqlx::PgExecutor<'e>,
{
    let id = new_id();
    sqlx::query(
        r#"
        INSERT INTO webhook_events (id, business_id, event_type, aggregate_id, payload)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(id)
    .bind(business_id)
    .bind(event_type)
    .bind(aggregate_id)
    .bind(payload)
    .execute(exec)
    .await?;
    Ok(id)
}

pub async fn register_endpoint(
    pool: &PgPool,
    business_id: Uuid,
    req: CreateEndpointRequest,
) -> AppResult<CreateEndpointResponse> {
    if !(req.url.starts_with("http://") || req.url.starts_with("https://")) {
        return Err(AppError::Validation(vec![common::error::FieldError::new(
            "url",
            "must be an http(s) URL",
        )]));
    }
    let mut secret = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut secret);
    let id = new_id();

    sqlx::query(
        r#"
        INSERT INTO webhook_endpoints (id, business_id, url, secret, event_types)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(id)
    .bind(business_id)
    .bind(&req.url)
    .bind(&secret)
    .bind(&req.event_types)
    .execute(pool)
    .await?;

    Ok(CreateEndpointResponse {
        id,
        url: req.url,
        event_types: req.event_types,
        secret: hex::encode(&secret),
    })
}

/// The reconciliation source of truth: what events happened, regardless of
/// delivery state. Keyset-paginated.
pub async fn list_events(
    pool: &PgPool,
    business_id: Uuid,
    limit: Option<i64>,
    cursor: Option<String>,
) -> AppResult<Page<WebhookEventRow>> {
    let limit = clamp_limit(limit);
    let decoded = cursor.as_deref().and_then(decode_cursor);

    let mut rows = match decoded {
        Some((ts, id)) => {
            sqlx::query_as::<_, WebhookEventRow>(
                r#"
                SELECT id, event_type, aggregate_id, payload, sequence, created_at
                FROM webhook_events
                WHERE business_id = $1 AND (created_at, id) < ($2, $3)
                ORDER BY created_at DESC, id DESC LIMIT $4
                "#,
            )
            .bind(business_id)
            .bind(ts)
            .bind(id)
            .bind(limit + 1)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<_, WebhookEventRow>(
                r#"
                SELECT id, event_type, aggregate_id, payload, sequence, created_at
                FROM webhook_events
                WHERE business_id = $1
                ORDER BY created_at DESC, id DESC LIMIT $2
                "#,
            )
            .bind(business_id)
            .bind(limit + 1)
            .fetch_all(pool)
            .await?
        }
    };

    let next_cursor = if rows.len() as i64 > limit {
        let last = &rows[limit as usize - 1];
        let c = encode_cursor(last.created_at, last.id);
        rows.truncate(limit as usize);
        Some(c)
    } else {
        None
    };
    Ok(Page::new(rows, next_cursor))
}

/// Per-delivery state: attempts, last status/error, which are dead. This is how
/// a business sees *why* a webhook didn't arrive.
pub async fn list_deliveries(
    pool: &PgPool,
    business_id: Uuid,
    status: Option<String>,
) -> AppResult<Vec<WebhookDeliveryRow>> {
    let rows = sqlx::query_as::<_, WebhookDeliveryRow>(
        r#"
        SELECT id, event_id, endpoint_id, status, attempts, next_attempt_at,
               last_status_code, last_error, last_attempt_at, created_at
        FROM webhook_deliveries
        WHERE business_id = $1 AND ($2::text IS NULL OR status = $2)
        ORDER BY created_at DESC
        LIMIT 200
        "#,
    )
    .bind(business_id)
    .bind(status)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Manual replay: reset a failed/dead delivery to be retried now.
pub async fn redeliver(pool: &PgPool, business_id: Uuid, delivery_id: Uuid) -> AppResult<()> {
    let res = sqlx::query(
        r#"
        UPDATE webhook_deliveries
        SET status = 'pending', attempts = 0, next_attempt_at = now(),
            locked_until = NULL, last_error = NULL
        WHERE id = $1 AND business_id = $2 AND status IN ('failed', 'dead')
        "#,
    )
    .bind(delivery_id)
    .bind(business_id)
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound("no failed/dead delivery with that id"));
    }
    Ok(())
}
