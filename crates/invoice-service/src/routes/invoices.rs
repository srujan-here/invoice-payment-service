//! Invoice endpoints: CRUD, lifecycle transitions, and payment.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::{Json, Router};
use common::{AppError, AppResult};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AuthBusiness;
use crate::domain::invoice::{CreateInvoiceRequest, InvoiceState};
use crate::domain::payment::PayRequest;
use crate::services::{invoice_service, payment_service};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/v1/invoices", post(create).get(list))
        .route("/v1/invoices/:id", get(get_one))
        .route("/v1/invoices/:id/finalize", post(finalize))
        .route("/v1/invoices/:id/void", post(void))
        .route("/v1/invoices/:id/mark_uncollectible", post(mark_uncollectible))
        .route("/v1/invoices/:id/pay", post(pay))
        .route("/v1/invoices/:id/payments/:attempt_id", get(get_attempt))
}

#[derive(Debug, Deserialize)]
struct InvoiceListQuery {
    state: Option<InvoiceState>,
    limit: Option<i64>,
    cursor: Option<String>,
}

async fn create(
    State(state): State<AppState>,
    AuthBusiness(business_id): AuthBusiness,
    Json(req): Json<CreateInvoiceRequest>,
) -> AppResult<impl IntoResponse> {
    let inv = invoice_service::create(&state.pool, business_id, req).await?;
    Ok((StatusCode::CREATED, Json(inv)))
}

async fn get_one(
    State(state): State<AppState>,
    AuthBusiness(business_id): AuthBusiness,
    Path(id): Path<Uuid>,
) -> AppResult<impl IntoResponse> {
    Ok(Json(invoice_service::get(&state.pool, business_id, id).await?))
}

async fn list(
    State(state): State<AppState>,
    AuthBusiness(business_id): AuthBusiness,
    Query(q): Query<InvoiceListQuery>,
) -> AppResult<impl IntoResponse> {
    Ok(Json(
        invoice_service::list(&state.pool, business_id, q.state, q.limit, q.cursor).await?,
    ))
}

async fn finalize(
    State(state): State<AppState>,
    AuthBusiness(business_id): AuthBusiness,
    Path(id): Path<Uuid>,
) -> AppResult<impl IntoResponse> {
    Ok(Json(invoice_service::finalize(&state.pool, business_id, id).await?))
}

async fn void(
    State(state): State<AppState>,
    AuthBusiness(business_id): AuthBusiness,
    Path(id): Path<Uuid>,
) -> AppResult<impl IntoResponse> {
    Ok(Json(invoice_service::void(&state.pool, business_id, id).await?))
}

async fn mark_uncollectible(
    State(state): State<AppState>,
    AuthBusiness(business_id): AuthBusiness,
    Path(id): Path<Uuid>,
) -> AppResult<impl IntoResponse> {
    Ok(Json(
        invoice_service::mark_uncollectible(&state.pool, business_id, id).await?,
    ))
}

async fn pay(
    State(state): State<AppState>,
    AuthBusiness(business_id): AuthBusiness,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<PayRequest>,
) -> AppResult<impl IntoResponse> {
    let idempotency_key = headers
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            AppError::Validation(vec![common::error::FieldError::new(
                "Idempotency-Key",
                "this header is required for payments",
            )])
        })?;

    let (status, body) =
        payment_service::pay(&state, business_id, id, idempotency_key, req).await?;
    Ok((status, Json(body)))
}

async fn get_attempt(
    State(state): State<AppState>,
    AuthBusiness(business_id): AuthBusiness,
    Path((id, attempt_id)): Path<(Uuid, Uuid)>,
) -> AppResult<impl IntoResponse> {
    Ok(Json(
        payment_service::get_attempt(&state, business_id, id, attempt_id).await?,
    ))
}
