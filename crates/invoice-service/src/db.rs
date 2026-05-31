//! Database pool setup and migrations.

use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Connect with a bounded pool and run migrations. Called once at startup so
/// `docker compose up` needs no manual migration step.
pub async fn connect_and_migrate(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(Duration::from_secs(10))
        .connect(database_url)
        .await?;

    sqlx::migrate!("../../migrations").run(&pool).await?;
    tracing::info!("migrations applied");
    Ok(pool)
}
