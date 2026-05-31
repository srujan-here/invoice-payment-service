//! Payment reconciler.
//!
//! Resolves `pending` payment attempts that the request path could not finish —
//! a PSP timeout, a network error, or a crash between transaction 1 and 2. It
//! re-queries the PSP **with the original idempotency key**; because the PSP
//! dedupes on that key, the re-query returns the original single outcome and can
//! never produce a second charge. It then runs the same finalize path as the
//! request handler.
//!
//! It claims work with `FOR UPDATE SKIP LOCKED` and a lease (pushing
//! `next_poll_at` 30s into the future) so it is safe to run on multiple replicas
//! and never holds a row lock across the PSP HTTP call.

use std::time::Duration;

use sqlx::Row;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::services::payment_service;
use crate::state::AppState;

const TICK: Duration = Duration::from_secs(5);
const BATCH: i64 = 20;

pub async fn run(state: AppState, shutdown: CancellationToken) {
    tracing::info!("payment reconciler started");
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                tracing::info!("payment reconciler shutting down");
                return;
            }
            _ = tokio::time::sleep(TICK) => {
                if let Err(e) = tick(&state).await {
                    tracing::error!(error = ?e, "reconciler tick failed");
                }
            }
        }
    }
}

async fn tick(state: &AppState) -> anyhow::Result<()> {
    // Atomically lease a batch of due pending attempts (pushes next_poll_at out
    // so a concurrent reconciler won't grab the same rows). NULL next_poll_at
    // catches attempts orphaned by a crash before transaction 2.
    let rows = sqlx::query(
        r#"
        UPDATE payment_attempts
        SET next_poll_at = now() + interval '30 seconds'
        WHERE id IN (
            SELECT id FROM payment_attempts
            WHERE status = 'pending'
              AND (next_poll_at IS NULL OR next_poll_at <= now())
            ORDER BY next_poll_at NULLS FIRST
            FOR UPDATE SKIP LOCKED
            LIMIT $1
        )
        RETURNING id, invoice_id, business_id, idempotency_key, amount_cents, card_token
        "#,
    )
    .bind(BATCH)
    .fetch_all(&state.pool)
    .await?;

    if rows.is_empty() {
        return Ok(());
    }
    tracing::info!(count = rows.len(), "reconciling pending payment attempts");

    for row in rows {
        let attempt_id: Uuid = row.try_get("id")?;
        let invoice_id: Uuid = row.try_get("invoice_id")?;
        let business_id: Uuid = row.try_get("business_id")?;
        let idempotency_key: String = row.try_get("idempotency_key")?;
        let amount_cents: i64 = row.try_get("amount_cents")?;
        let card_token: String = row.try_get("card_token")?;

        // Re-query the PSP with the SAME idempotency key — safe by construction.
        let result = state
            .psp
            .charge(&card_token, amount_cents, &idempotency_key)
            .await;

        // Reuse the exact request-path finalize logic. We discard the HTTP tuple;
        // the side effects (attempt status, invoice state, outbox event, cached
        // idempotency response) are what matter.
        if let Err(e) = payment_service::finalize_for_reconciler(
            state,
            business_id,
            invoice_id,
            attempt_id,
            &idempotency_key,
            amount_cents,
            result,
        )
        .await
        {
            tracing::error!(error = ?e, %attempt_id, "failed to finalize reconciled attempt");
        }
    }

    Ok(())
}
