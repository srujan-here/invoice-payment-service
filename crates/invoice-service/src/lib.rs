//! Invoice & Payment Service — library root.
//!
//! Exposes [`build_app`] (the Axum router) and [`spawn_background_tasks`] so both
//! the binary and the integration tests construct the service the same way.

pub mod auth;
pub mod db;
pub mod domain;
pub mod psp;
pub mod reconciler;
pub mod repositories;
pub mod routes;
pub mod services;
pub mod state;
pub mod webhooks;

use std::sync::Arc;

use axum::Router;
use common::config::ServiceConfig;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use tower_http::trace::TraceLayer;

use crate::psp::PspClient;
use crate::state::AppState;
use crate::webhooks::dispatcher::Dispatcher;

/// Build the fully-wired application state.
pub fn build_state(pool: PgPool, config: ServiceConfig) -> AppState {
    let psp = PspClient::new(config.psp_base_url.clone(), config.psp_timeout);
    AppState {
        pool,
        psp,
        config: Arc::new(config),
    }
}

/// Assemble the router: a public health check plus all `/v1` routes behind the
/// API-key middleware.
pub fn build_app(state: AppState) -> Router {
    let protected = Router::new()
        .merge(routes::api_keys::router())
        .merge(routes::customers::router())
        .merge(routes::invoices::router())
        .merge(routes::webhooks::router())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_api_key,
        ));

    Router::new()
        .route("/healthz", axum::routing::get(|| async { "ok" }))
        .merge(protected)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Spawn the webhook dispatcher and the payment reconciler. Both stop when the
/// token is cancelled.
pub fn spawn_background_tasks(state: &AppState, shutdown: CancellationToken) {
    let dispatcher = Dispatcher::new(state.pool.clone(), state.config.webhook_max_attempts);
    tokio::spawn(dispatcher.run(shutdown.clone()));
    tokio::spawn(reconciler::run(state.clone(), shutdown));
}
