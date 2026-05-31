//! API-key management endpoints.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Json, Router};
use common::{AppError, AppResult};
use uuid::Uuid;

use crate::auth::{generate_key, AuthBusiness};
use crate::domain::api_key::{CreateApiKeyRequest, CreateApiKeyResponse};
use crate::repositories::api_key_repo;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/api-keys", axum::routing::post(create).get(list))
        .route("/v1/api-keys/:id", axum::routing::delete(revoke))
}

async fn create(
    State(state): State<AppState>,
    AuthBusiness(business_id): AuthBusiness,
    Json(req): Json<CreateApiKeyRequest>,
) -> AppResult<impl IntoResponse> {
    let env = req.env.as_deref().unwrap_or("test");
    let key = generate_key(env);
    let id = api_key_repo::insert(&state.pool, business_id, req.name.as_deref(), &key).await?;
    Ok((
        StatusCode::CREATED,
        Json(CreateApiKeyResponse {
            id,
            prefix: key.prefix,
            key: key.full,
        }),
    ))
}

async fn list(
    State(state): State<AppState>,
    AuthBusiness(business_id): AuthBusiness,
) -> AppResult<impl IntoResponse> {
    let keys = api_key_repo::list(&state.pool, business_id).await?;
    Ok(Json(keys))
}

async fn revoke(
    State(state): State<AppState>,
    AuthBusiness(business_id): AuthBusiness,
    Path(id): Path<Uuid>,
) -> AppResult<impl IntoResponse> {
    if api_key_repo::revoke(&state.pool, business_id, id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound("no active api key with that id"))
    }
}
