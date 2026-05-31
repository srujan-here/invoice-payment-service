//! The webhook dispatcher: a background Tokio task that fans events out to
//! deliveries and sends them with retries. It is the *only* component that makes
//! outbound HTTP for webhooks, so the API request path is never blocked by a
//! slow or down receiver.
//!
//! Per tick: (1) fan-out — create delivery rows for new events × matching active
//! endpoints (idempotent via the unique index); (2) claim a batch of due
//! deliveries with `FOR UPDATE SKIP LOCKED` + a lease, send them concurrently,
//! and record the outcome with exponential backoff + full jitter.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use rand::Rng;
use sqlx::{PgPool, Row};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::signing;

const TICK: Duration = Duration::from_secs(1);
const BATCH: i64 = 50;
const MAX_CONCURRENCY: usize = 16;
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
const BACKOFF_BASE_SECS: u64 = 10;
const BACKOFF_CAP_SECS: u64 = 3600;

#[derive(Clone)]
pub struct Dispatcher {
    pool: PgPool,
    http: reqwest::Client,
    max_attempts: i32,
}

impl Dispatcher {
    pub fn new(pool: PgPool, max_attempts: i32) -> Self {
        let http = reqwest::Client::builder()
            .timeout(SEND_TIMEOUT)
            .build()
            .expect("build webhook http client");
        Self {
            pool,
            http,
            max_attempts,
        }
    }

    pub async fn run(self, shutdown: CancellationToken) {
        tracing::info!("webhook dispatcher started");
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    tracing::info!("webhook dispatcher shutting down");
                    return;
                }
                _ = tokio::time::sleep(TICK) => {
                    if let Err(e) = self.fan_out().await {
                        tracing::error!(error = ?e, "webhook fan-out failed");
                    }
                    if let Err(e) = self.deliver_batch().await {
                        tracing::error!(error = ?e, "webhook delivery batch failed");
                    }
                }
            }
        }
    }

    /// Create delivery rows for recent events × matching active endpoints.
    /// Idempotent: the unique (event_id, endpoint_id) index prevents duplicates.
    async fn fan_out(&self) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            INSERT INTO webhook_deliveries (id, event_id, endpoint_id, business_id)
            SELECT gen_random_uuid(), e.id, ep.id, e.business_id
            FROM webhook_events e
            JOIN webhook_endpoints ep
              ON ep.business_id = e.business_id
             AND ep.status = 'active'
             AND (cardinality(ep.event_types) = 0 OR e.event_type = ANY(ep.event_types))
            WHERE e.created_at > now() - interval '1 hour'
            ON CONFLICT (event_id, endpoint_id) DO NOTHING
            "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn deliver_batch(&self) -> anyhow::Result<()> {
        // Atomically claim + lease a batch. Also reclaims in_flight rows whose
        // lease expired (a crashed worker).
        let claimed = sqlx::query(
            r#"
            UPDATE webhook_deliveries
            SET status = 'in_flight',
                locked_until = now() + interval '30 seconds',
                attempts = attempts + 1,
                last_attempt_at = now()
            WHERE id IN (
                SELECT id FROM webhook_deliveries
                WHERE (status IN ('pending', 'failed') AND next_attempt_at <= now())
                   OR (status = 'in_flight' AND locked_until < now())
                ORDER BY next_attempt_at
                FOR UPDATE SKIP LOCKED
                LIMIT $1
            )
            RETURNING id, attempts
            "#,
        )
        .bind(BATCH)
        .fetch_all(&self.pool)
        .await?;

        if claimed.is_empty() {
            return Ok(());
        }

        let ids: Vec<Uuid> = claimed
            .iter()
            .map(|r| r.try_get::<Uuid, _>("id"))
            .collect::<Result<_, _>>()?;

        // Hydrate each claimed delivery with its event payload + endpoint.
        let work = sqlx::query(
            r#"
            SELECT d.id, d.attempts, e.id AS event_id, e.payload, ep.url, ep.secret
            FROM webhook_deliveries d
            JOIN webhook_events e ON e.id = d.event_id
            JOIN webhook_endpoints ep ON ep.id = d.endpoint_id
            WHERE d.id = ANY($1)
            "#,
        )
        .bind(&ids)
        .fetch_all(&self.pool)
        .await?;

        let sem = Arc::new(Semaphore::new(MAX_CONCURRENCY));
        let mut set = tokio::task::JoinSet::new();

        for row in work {
            let permit = sem.clone().acquire_owned().await.expect("semaphore");
            let this = self.clone();
            set.spawn(async move {
                let _permit = permit;
                let delivery_id: Uuid = row.get("id");
                let attempts: i32 = row.get("attempts");
                let event_id: Uuid = row.get("event_id");
                let payload: serde_json::Value = row.get("payload");
                let url: String = row.get("url");
                let secret: Vec<u8> = row.get("secret");
                this.send_one(delivery_id, attempts, event_id, payload, url, secret)
                    .await;
            });
        }
        while set.join_next().await.is_some() {}
        Ok(())
    }

    async fn send_one(
        &self,
        delivery_id: Uuid,
        attempts: i32,
        event_id: Uuid,
        payload: serde_json::Value,
        url: String,
        secret: Vec<u8>,
    ) {
        let body = serde_json::to_vec(&payload).unwrap_or_default();
        let ts = Utc::now().timestamp();
        let sig = signing::signature_header(&secret, ts, &body);

        let res = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .header("X-Webhook-Id", event_id.to_string())
            .header("X-Webhook-Signature", sig)
            .body(body)
            .send()
            .await;

        match res {
            Ok(resp) if resp.status().is_success() => {
                let _ = self.mark_succeeded(delivery_id, resp.status().as_u16() as i32).await;
            }
            Ok(resp) => {
                let code = resp.status().as_u16() as i32;
                // 4xx (except 408/429) is the receiver rejecting us -> fail fast.
                let non_retryable = (400..500).contains(&code) && code != 408 && code != 429;
                let _ = self
                    .mark_failed(delivery_id, attempts, Some(code), "non-2xx response", non_retryable)
                    .await;
            }
            Err(e) => {
                let _ = self
                    .mark_failed(delivery_id, attempts, None, &e.to_string(), false)
                    .await;
            }
        }
    }

    async fn mark_succeeded(&self, id: Uuid, code: i32) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE webhook_deliveries SET status='succeeded', last_status_code=$1, locked_until=NULL WHERE id=$2",
        )
        .bind(code)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn mark_failed(
        &self,
        id: Uuid,
        attempts: i32,
        code: Option<i32>,
        error: &str,
        non_retryable: bool,
    ) -> anyhow::Result<()> {
        // Exhausted budget or a fatal 4xx -> dead-letter.
        if non_retryable || attempts >= self.max_attempts {
            sqlx::query(
                "UPDATE webhook_deliveries SET status='dead', last_status_code=$1, last_error=$2, locked_until=NULL WHERE id=$3",
            )
            .bind(code)
            .bind(error)
            .bind(id)
            .execute(&self.pool)
            .await?;
            tracing::warn!(%id, attempts, "webhook delivery dead-lettered");
            return Ok(());
        }

        // Exponential backoff with full jitter: delay = rand(0, min(cap, base*2^n)).
        let exp = BACKOFF_BASE_SECS.saturating_mul(1u64 << (attempts.max(1) - 1) as u32);
        let ceiling = exp.min(BACKOFF_CAP_SECS).max(1);
        let delay = rand::thread_rng().gen_range(0..=ceiling) as i64;

        sqlx::query(
            r#"
            UPDATE webhook_deliveries
            SET status='failed', last_status_code=$1, last_error=$2,
                next_attempt_at = now() + make_interval(secs => $3), locked_until=NULL
            WHERE id=$4
            "#,
        )
        .bind(code)
        .bind(error)
        .bind(delay as f64)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
