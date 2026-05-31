//! Invoice use-cases: creation with a server-computed total, retrieval,
//! listing, and the admin-driven state transitions (finalize / void /
//! mark_uncollectible). The `invoice.created` outbox event is written in the
//! same transaction as the invoice so it can never be lost or phantom.

use common::error::FieldError;
use common::pagination::{clamp_limit, decode_cursor, encode_cursor, Page};
use common::{AppError, AppResult};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::invoice::{
    CreateInvoiceRequest, Invoice, InvoiceResponse, InvoiceState,
};
use crate::domain::webhook::event_types;
use crate::repositories::invoice_repo;
use crate::services::webhook_service;

pub async fn create(
    pool: &PgPool,
    business_id: Uuid,
    req: CreateInvoiceRequest,
) -> AppResult<InvoiceResponse> {
    validate_line_items(&req)?;

    // Customer must belong to this business.
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM customers WHERE id = $1 AND business_id = $2)",
    )
    .bind(req.customer_id)
    .bind(business_id)
    .fetch_one(pool)
    .await?;
    if !exists {
        return Err(AppError::NotFound("customer not found"));
    }

    let mut tx = pool.begin().await?;

    let invoice = invoice_repo::insert_invoice(&mut *tx, business_id, req.customer_id).await?;
    for (pos, li) in req.line_items.iter().enumerate() {
        invoice_repo::insert_line_item(
            &mut *tx,
            invoice.id,
            &li.description,
            li.quantity,
            li.unit_amount_cents,
            pos as i32,
        )
        .await?;
    }
    // Server computes the total from the persisted items (DB integer SUM).
    let total = invoice_repo::sync_total(&mut *tx, invoice.id).await?;
    let items = invoice_repo::list_line_items(&mut *tx, invoice.id).await?;

    // invoice.created — atomic with the insert.
    webhook_service::emit_event(
        &mut *tx,
        business_id,
        event_types::INVOICE_CREATED,
        invoice.id,
        serde_json::json!({
            "id": invoice.id,
            "customer_id": invoice.customer_id,
            "state": "draft",
            "total_cents": total.cents(),
            "currency": "USD",
        }),
    )
    .await?;

    tx.commit().await?;

    let mut invoice = invoice;
    invoice.total_cents = total;
    Ok(InvoiceResponse::from_parts(invoice, items))
}

pub async fn get(pool: &PgPool, business_id: Uuid, id: Uuid) -> AppResult<InvoiceResponse> {
    let invoice = invoice_repo::get(pool, business_id, id)
        .await?
        .ok_or(AppError::NotFound("invoice not found"))?;
    let items = invoice_repo::list_line_items(pool, id).await?;
    Ok(InvoiceResponse::from_parts(invoice, items))
}

pub async fn list(
    pool: &PgPool,
    business_id: Uuid,
    state: Option<InvoiceState>,
    limit: Option<i64>,
    cursor: Option<String>,
) -> AppResult<Page<InvoiceResponse>> {
    let limit = clamp_limit(limit);
    let decoded = cursor.as_deref().and_then(decode_cursor);
    let mut rows = invoice_repo::list(pool, business_id, state, limit + 1, decoded).await?;

    let next_cursor = if rows.len() as i64 > limit {
        let last = &rows[limit as usize - 1];
        let c = encode_cursor(last.created_at, last.id);
        rows.truncate(limit as usize);
        Some(c)
    } else {
        None
    };

    // List view omits line items for brevity; GET by id includes them.
    let data = rows
        .into_iter()
        .map(|inv| InvoiceResponse::from_parts(inv, Vec::new()))
        .collect();
    Ok(Page::new(data, next_cursor))
}

pub async fn finalize(pool: &PgPool, business_id: Uuid, id: Uuid) -> AppResult<InvoiceResponse> {
    transition(pool, business_id, id, InvoiceState::Open).await
}

pub async fn void(pool: &PgPool, business_id: Uuid, id: Uuid) -> AppResult<InvoiceResponse> {
    transition(pool, business_id, id, InvoiceState::Void).await
}

pub async fn mark_uncollectible(
    pool: &PgPool,
    business_id: Uuid,
    id: Uuid,
) -> AppResult<InvoiceResponse> {
    transition(pool, business_id, id, InvoiceState::Uncollectible).await
}

/// Shared transition logic: load, validate the transition against the state
/// machine, then apply it with a status-conditional UPDATE. Invalid transitions
/// are rejected here with a precise 409.
async fn transition(
    pool: &PgPool,
    business_id: Uuid,
    id: Uuid,
    to: InvoiceState,
) -> AppResult<InvoiceResponse> {
    let current = invoice_repo::get(pool, business_id, id)
        .await?
        .ok_or(AppError::NotFound("invoice not found"))?;

    if current.state == to {
        // Idempotent no-op would be surprising for these admin actions; treat as
        // an invalid transition so the caller knows nothing changed.
        return Err(AppError::InvalidStateTransition {
            from: current.state.as_str().into(),
            to: to.as_str().into(),
        });
    }
    if !current.state.can_transition_to(to) {
        return Err(AppError::InvalidStateTransition {
            from: current.state.as_str().into(),
            to: to.as_str().into(),
        });
    }

    let updated: Option<Invoice> =
        invoice_repo::transition(pool, business_id, id, current.state, to).await?;
    let updated = updated.ok_or(AppError::InvalidStateTransition {
        // Lost the race: someone else moved the invoice out of `from`.
        from: current.state.as_str().into(),
        to: to.as_str().into(),
    })?;

    let items = invoice_repo::list_line_items(pool, id).await?;
    Ok(InvoiceResponse::from_parts(updated, items))
}

fn validate_line_items(req: &CreateInvoiceRequest) -> AppResult<()> {
    let mut errors = Vec::new();
    if req.line_items.is_empty() {
        errors.push(FieldError::new("line_items", "at least one line item is required"));
    }
    for (i, li) in req.line_items.iter().enumerate() {
        if li.description.trim().is_empty() {
            errors.push(FieldError::new(
                format!("line_items[{i}].description"),
                "must not be empty",
            ));
        }
        if li.quantity <= 0 {
            errors.push(FieldError::new(
                format!("line_items[{i}].quantity"),
                "must be greater than 0",
            ));
        }
        if li.unit_amount_cents < 0 {
            errors.push(FieldError::new(
                format!("line_items[{i}].unit_amount_cents"),
                "must not be negative",
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::Validation(errors))
    }
}
