//! Customer endpoints.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Json, Router};
use common::AppResult;
use uuid::Uuid;

use crate::auth::AuthBusiness;
use crate::domain::customer::CreateCustomerRequest;
use crate::routes::ListQuery;
use crate::services::customer_service;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/customers", axum::routing::post(create).get(list))
        .route("/v1/customers/:id", axum::routing::get(get))
}

async fn create(
    State(state): State<AppState>,
    AuthBusiness(business_id): AuthBusiness,
    Json(req): Json<CreateCustomerRequest>,
) -> AppResult<impl IntoResponse> {
    let c = customer_service::create(&state.pool, business_id, req).await?;
    Ok((StatusCode::CREATED, Json(c)))
}

async fn get(
    State(state): State<AppState>,
    AuthBusiness(business_id): AuthBusiness,
    Path(id): Path<Uuid>,
) -> AppResult<impl IntoResponse> {
    Ok(Json(customer_service::get(&state.pool, business_id, id).await?))
}

async fn list(
    State(state): State<AppState>,
    AuthBusiness(business_id): AuthBusiness,
    Query(q): Query<ListQuery>,
) -> AppResult<impl IntoResponse> {
    Ok(Json(
        customer_service::list(&state.pool, business_id, q.limit, q.cursor).await?,
    ))
}
