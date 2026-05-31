//! Shared application state, injected into every handler via Axum.

use std::sync::Arc;

use common::config::ServiceConfig;
use sqlx::PgPool;

use crate::psp::PspClient;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub psp: PspClient,
    pub config: Arc<ServiceConfig>,
}
