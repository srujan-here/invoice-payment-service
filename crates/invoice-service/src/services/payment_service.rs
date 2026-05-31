//! The payment flow — the correctness core of the service.
//!
//! Two independent guards, two short transactions, and **no DB transaction held
//! across the PSP HTTP call**:
//!
//!   * Idempotency (same client request retried)  -> `idempotency_keys` unique key.
//!   * Single-active-charge (distinct requests racing one invoice) -> the partial
//!     unique index `uq_active_attempt_per_invoice` on `payment_attempts`.
//!
//! Order of checks: idempotency replay FIRST, invoice-state SECOND. A replay of
//! a completed key returns the cached response regardless of current state.
//!
//! See DESIGN.md §3 for the full failure-mode walkthrough that this implements.

use axum::http::StatusCode;
use common::ids::new_id;
use common::{AppError, AppResult};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::domain::invoice::InvoiceState;
use crate::domain::payment::{PayRequest, PaymentAttemptResponse, PaymentStatus};
use crate::domain::webhook::event_types;
use crate::psp::PspResult;
use crate::repositories::is_unique_violation;
use crate::services::webhook_service;
use crate::state::AppState;

/// Outcome carrying the HTTP status the route should emit.
pub type PayOutcome = (StatusCode, PaymentAttemptResponse);

pub async fn pay(
    state: &AppState,
    business_id: Uuid,
    invoice_id: Uuid,
    idempotency_key: String,
    req: PayRequest,
) -> AppResult<PayOutcome> {
    if req.card_token.trim().is_empty() {
        return Err(AppError::Validation(vec![common::error::FieldError::new(
            "card_token",
            "must not be empty",
        )]));
    }

    // Read the invoice once (no lock) to learn its amount + state. A missing
    // invoice is a 404 regardless of the idempotency key.
    let inv = sqlx::query(
        "SELECT state, total_cents FROM invoices WHERE id = $1 AND business_id = $2",
    )
    .bind(invoice_id)
    .bind(business_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some(inv) = inv else {
        return Err(AppError::NotFound("invoice not found"));
    };
    let invoice_state: InvoiceState = inv.try_get("state")?;
    let amount_cents: i64 = inv.try_get("total_cents")?;

    let fingerprint = fingerprint(invoice_id, &req.card_token, amount_cents);

    // ---- Transaction 1: claim idempotency key + create the pending attempt ----
    let mut tx = state.pool.begin().await?;

    let claimed: Option<bool> = sqlx::query_scalar(
        r#"
        INSERT INTO idempotency_keys (id, business_id, idempotency_key, request_fingerprint)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (business_id, idempotency_key) DO NOTHING
        RETURNING true
        "#,
    )
    .bind(new_id())
    .bind(business_id)
    .bind(&idempotency_key)
    .bind(&fingerprint)
    .fetch_optional(&mut *tx)
    .await?;

    if claimed.is_none() {
        // Key already exists -> this is a replay (or a mismatch). Resolve it
        // without ever calling the PSP a second time.
        tx.rollback().await?;
        return replay(state, business_id, &idempotency_key, &fingerprint, invoice_id).await;
    }

    // Fresh key. The invoice must be payable.
    if invoice_state != InvoiceState::Open {
        // Case (e) / not-finalized: not payable. Release the key (rollback) so a
        // later legitimate attempt can reuse it once the invoice is open.
        tx.rollback().await?;
        return Err(AppError::conflict(
            "invoice_not_payable",
            format!("invoice is in state '{}', not 'open'", invoice_state.as_str()),
        ));
    }

    // Create the single active attempt. The partial unique index makes this the
    // concurrency guard: a racing request gets 23505 here.
    let attempt_id = new_id();
    let insert_attempt = sqlx::query(
        r#"
        INSERT INTO payment_attempts
            (id, invoice_id, business_id, idempotency_key, amount_cents, card_token, status, attempt_count)
        VALUES ($1, $2, $3, $4, $5, $6, 'pending', 1)
        "#,
    )
    .bind(attempt_id)
    .bind(invoice_id)
    .bind(business_id)
    .bind(&idempotency_key)
    .bind(amount_cents)
    .bind(&req.card_token)
    .execute(&mut *tx)
    .await;

    if let Err(e) = insert_attempt {
        tx.rollback().await?;
        if is_unique_violation(&e) {
            // A concurrent /pay already has an active (pending/succeeded) attempt.
            return Err(AppError::conflict(
                "payment_in_progress",
                "another payment attempt is already in progress for this invoice",
            ));
        }
        return Err(e.into());
    }

    // Link the key to the attempt so a replay can find the in-flight attempt.
    sqlx::query("UPDATE idempotency_keys SET attempt_id = $1 WHERE business_id = $2 AND idempotency_key = $3")
        .bind(attempt_id)
        .bind(business_id)
        .bind(&idempotency_key)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    // ---- PSP call: outside any transaction, with a bounded timeout ----
    let result = state
        .psp
        .charge(&req.card_token, amount_cents, &idempotency_key)
        .await;

    // ---- Transaction 2: finalize based on the outcome ----
    finalize(state, business_id, invoice_id, attempt_id, &idempotency_key, amount_cents, result)
        .await
}

/// Fetch a single payment attempt (for the poll URL).
pub async fn get_attempt(
    state: &AppState,
    business_id: Uuid,
    invoice_id: Uuid,
    attempt_id: Uuid,
) -> AppResult<PaymentAttemptResponse> {
    use crate::domain::payment::PaymentAttempt;
    let attempt = sqlx::query_as::<_, PaymentAttempt>(
        r#"
        SELECT id, invoice_id, business_id, idempotency_key, amount_cents, currency,
               card_token, status, psp_ref, failure_code, last_error, attempt_count,
               next_poll_at, created_at, updated_at
        FROM payment_attempts
        WHERE id = $1 AND invoice_id = $2 AND business_id = $3
        "#,
    )
    .bind(attempt_id)
    .bind(invoice_id)
    .bind(business_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound("payment attempt not found"))?;

    let invoice_state: String =
        sqlx::query_scalar::<_, InvoiceState>("SELECT state FROM invoices WHERE id = $1")
            .bind(invoice_id)
            .fetch_one(&state.pool)
            .await?
            .as_str()
            .into();

    Ok(PaymentAttemptResponse {
        attempt_id: attempt.id,
        invoice_id: attempt.invoice_id,
        status: attempt.status,
        amount_cents: attempt.amount_cents.cents(),
        invoice_state,
        psp_ref: attempt.psp_ref,
        failure_code: attempt.failure_code,
        message: None,
        poll_url: None,
    })
}

/// Resolve a replayed idempotency key.
async fn replay(
    state: &AppState,
    business_id: Uuid,
    idempotency_key: &str,
    fingerprint: &[u8],
    invoice_id: Uuid,
) -> AppResult<PayOutcome> {
    let row = sqlx::query(
        r#"
        SELECT request_fingerprint, status, attempt_id, response_status, response_body
        FROM idempotency_keys
        WHERE business_id = $1 AND idempotency_key = $2
        "#,
    )
    .bind(business_id)
    .bind(idempotency_key)
    .fetch_one(&state.pool)
    .await?;

    let stored_fp: Vec<u8> = row.try_get("request_fingerprint")?;
    if stored_fp != fingerprint {
        // Case (d): same key, different request. Refuse — we will not silently
        // charge a different thing under a reused key.
        return Err(AppError::conflict(
            "idempotency_key_reuse",
            "this idempotency key was already used with different request parameters",
        ));
    }

    let status: String = row.try_get("status")?;
    if status == "completed" {
        let code: i32 = row.try_get("response_status")?;
        let body: serde_json::Value = row.try_get("response_body")?;
        let resp: PaymentAttemptResponse = serde_json::from_value(body)
            .map_err(|e| AppError::Internal(e.into()))?;
        let status = StatusCode::from_u16(code as u16).unwrap_or(StatusCode::OK);
        return Ok((status, resp));
    }

    // Still in progress (e.g. the original call timed out or we crashed). Report
    // the pending attempt; the reconciler will resolve it.
    let attempt_id: Option<Uuid> = row.try_get("attempt_id")?;
    let resp = PaymentAttemptResponse {
        attempt_id: attempt_id.unwrap_or_default(),
        invoice_id,
        status: PaymentStatus::Pending,
        amount_cents: 0,
        invoice_state: "open".into(),
        psp_ref: None,
        failure_code: None,
        message: Some("payment is still being processed".into()),
        poll_url: attempt_id.map(|a| format!("/v1/invoices/{invoice_id}/payments/{a}")),
    };
    Ok((StatusCode::ACCEPTED, resp))
}

/// Reconciler entry point: apply a PSP outcome to a pending attempt using the
/// exact same finalize path as the request handler. The HTTP tuple is discarded.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn finalize_for_reconciler(
    state: &AppState,
    business_id: Uuid,
    invoice_id: Uuid,
    attempt_id: Uuid,
    idempotency_key: &str,
    amount_cents: i64,
    result: PspResult,
) -> AppResult<()> {
    finalize(
        state,
        business_id,
        invoice_id,
        attempt_id,
        idempotency_key,
        amount_cents,
        result,
    )
    .await
    .map(|_| ())
}

#[allow(clippy::too_many_arguments)]
async fn finalize(
    state: &AppState,
    business_id: Uuid,
    invoice_id: Uuid,
    attempt_id: Uuid,
    idempotency_key: &str,
    amount_cents: i64,
    result: PspResult,
) -> AppResult<PayOutcome> {
    let mut tx = state.pool.begin().await?;

    let outcome = match result {
        PspResult::Succeeded { psp_ref } => {
            sqlx::query(
                "UPDATE payment_attempts SET status='succeeded', psp_ref=$1 WHERE id=$2 AND status='pending'",
            )
            .bind(&psp_ref)
            .bind(attempt_id)
            .execute(&mut *tx)
            .await?;

            // open -> paid, atomic with the success.
            sqlx::query(
                "UPDATE invoices SET state='paid', paid_at=now(), version=version+1 WHERE id=$1 AND state='open'",
            )
            .bind(invoice_id)
            .execute(&mut *tx)
            .await?;

            let resp = PaymentAttemptResponse {
                attempt_id,
                invoice_id,
                status: PaymentStatus::Succeeded,
                amount_cents,
                invoice_state: "paid".into(),
                psp_ref: Some(psp_ref),
                failure_code: None,
                message: None,
                poll_url: None,
            };
            emit(&mut tx, business_id, event_types::INVOICE_PAID, invoice_id, &resp, amount_cents).await?;
            complete_key(&mut tx, business_id, idempotency_key, StatusCode::OK, &resp).await?;
            (StatusCode::OK, resp)
        }

        PspResult::Declined { code } => {
            sqlx::query(
                "UPDATE payment_attempts SET status='failed', failure_code=$1 WHERE id=$2 AND status='pending'",
            )
            .bind(&code)
            .bind(attempt_id)
            .execute(&mut *tx)
            .await?;
            // Invoice stays 'open' -> retryable with a new idempotency key.

            let resp = PaymentAttemptResponse {
                attempt_id,
                invoice_id,
                status: PaymentStatus::Failed,
                amount_cents,
                invoice_state: "open".into(),
                psp_ref: None,
                failure_code: Some(code),
                message: Some("the payment was declined".into()),
                poll_url: None,
            };
            emit(&mut tx, business_id, event_types::INVOICE_PAYMENT_FAILED, invoice_id, &resp, amount_cents).await?;
            complete_key(&mut tx, business_id, idempotency_key, StatusCode::PAYMENT_REQUIRED, &resp).await?;
            (StatusCode::PAYMENT_REQUIRED, resp)
        }

        PspResult::Unknown { reason } => {
            // Outcome genuinely unknown. Leave the attempt 'pending' (it still
            // pins the invoice) and let the reconciler resolve it. Do NOT
            // complete the idempotency key.
            sqlx::query(
                r#"
                UPDATE payment_attempts
                SET last_error = $1,
                    attempt_count = attempt_count + 1,
                    next_poll_at = now() + interval '5 seconds'
                WHERE id = $2 AND status = 'pending'
                "#,
            )
            .bind(reason)
            .bind(attempt_id)
            .execute(&mut *tx)
            .await?;

            let resp = PaymentAttemptResponse {
                attempt_id,
                invoice_id,
                status: PaymentStatus::Pending,
                amount_cents,
                invoice_state: "open".into(),
                psp_ref: None,
                failure_code: None,
                message: Some("payment is being processed; poll for the result".into()),
                poll_url: Some(format!("/v1/invoices/{invoice_id}/payments/{attempt_id}")),
            };
            (StatusCode::ACCEPTED, resp)
        }
    };

    tx.commit().await?;
    Ok(outcome)
}

async fn emit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    business_id: Uuid,
    event_type: &str,
    invoice_id: Uuid,
    resp: &PaymentAttemptResponse,
    amount_cents: i64,
) -> AppResult<()> {
    webhook_service::emit_event(
        &mut **tx,
        business_id,
        event_type,
        invoice_id,
        serde_json::json!({
            "invoice_id": invoice_id,
            "attempt_id": resp.attempt_id,
            "status": resp.status,
            "amount_cents": amount_cents,
            "failure_code": resp.failure_code,
        }),
    )
    .await?;
    Ok(())
}

async fn complete_key(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    business_id: Uuid,
    idempotency_key: &str,
    status: StatusCode,
    resp: &PaymentAttemptResponse,
) -> AppResult<()> {
    let body = serde_json::to_value(resp).map_err(|e| AppError::Internal(e.into()))?;
    sqlx::query(
        r#"
        UPDATE idempotency_keys
        SET status='completed', response_status=$1, response_body=$2
        WHERE business_id=$3 AND idempotency_key=$4
        "#,
    )
    .bind(status.as_u16() as i32)
    .bind(body)
    .bind(business_id)
    .bind(idempotency_key)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// SHA-256 over the canonical request so a key reused with a different body is
/// detectable.
fn fingerprint(invoice_id: Uuid, card_token: &str, amount_cents: i64) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(invoice_id.as_bytes());
    h.update([0u8]);
    h.update(card_token.as_bytes());
    h.update([0u8]);
    h.update(amount_cents.to_le_bytes());
    h.finalize().to_vec()
}
