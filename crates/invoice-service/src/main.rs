//! Invoice & Payment Service entrypoint.

use common::config::ServiceConfig;
use invoice_service::{build_app, build_state, db, spawn_background_tasks};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,invoice_service=debug,tower_http=info".into()),
        )
        .init();

    let config = ServiceConfig::from_env()?;
    let bind_addr = config.bind_addr.clone();

    let pool = db::connect_and_migrate(&config.database_url).await?;
    let state = build_state(pool, config);

    let shutdown = CancellationToken::new();
    spawn_background_tasks(&state, shutdown.clone());

    let app = build_app(state);
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    tracing::info!(%bind_addr, "invoice-service listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(shutdown))
        .await?;

    Ok(())
}

/// Trigger graceful shutdown on Ctrl-C / SIGTERM, cancelling background tasks.
async fn shutdown_signal(token: CancellationToken) {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received");
    token.cancel();
}
