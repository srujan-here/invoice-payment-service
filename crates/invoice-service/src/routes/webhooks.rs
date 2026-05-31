//! Webhook endpoints: registration + the reconciliation surface (events,
//! deliveries, redeliver).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Json, Router};
use common::AppResult;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::AuthBusiness;
use crate::domain::webhook::CreateEndpointRequest;
use crate::routes::ListQuery;
use crate::services::webhook_service;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    use axum::routing::{get, post};
    Router::new()
        .route("/v1/webhook-endpoints", post(create_endpoint))
        .route("/v1/events", get(list_events))
        .route("/v1/webhook-deliveries", get(list_deliveries))
        .route("/v1/webhook-deliveries/:id/redeliver", post(redeliver))
}

async fn create_endpoint(
    State(state): State<AppState>,
    AuthBusiness(business_id): AuthBusiness,
    Json(req): Json<CreateEndpointRequest>,
) -> AppResult<impl IntoResponse> {
    let ep = webhook_service::register_endpoint(&state.pool, business_id, req).await?;
    Ok((StatusCode::CREATED, Json(ep)))
}

async fn list_events(
    State(state): State<AppState>,
    AuthBusiness(business_id): AuthBusiness,
    Query(q): Query<ListQuery>,
) -> AppResult<impl IntoResponse> {
    Ok(Json(
        webhook_service::list_events(&state.pool, business_id, q.limit, q.cursor).await?,
    ))
}

#[derive(Debug, Deserialize)]
struct DeliveryQuery {
    status: Option<String>,
}

async fn list_deliveries(
    State(state): State<AppState>,
    AuthBusiness(business_id): AuthBusiness,
    Query(q): Query<DeliveryQuery>,
) -> AppResult<impl IntoResponse> {
    Ok(Json(
        webhook_service::list_deliveries(&state.pool, business_id, q.status).await?,
    ))
}

async fn redeliver(
    State(state): State<AppState>,
    AuthBusiness(business_id): AuthBusiness,
    Path(id): Path<Uuid>,
) -> AppResult<impl IntoResponse> {
    webhook_service::redeliver(&state.pool, business_id, id).await?;
    Ok(StatusCode::ACCEPTED)
}
