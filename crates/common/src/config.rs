//! Environment-driven config, loaded once at startup. No secrets are logged.

use std::env;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub database_url: String,
    pub bind_addr: String,
    pub psp_base_url: String,
    pub psp_timeout: Duration,
    pub webhook_max_attempts: i32,
}

impl ServiceConfig {
    /// Read config from the environment, applying sane defaults where a missing
    /// value is not fatal. `DATABASE_URL` is the only hard requirement.
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            database_url: env::var("DATABASE_URL")
                .map_err(|_| anyhow::anyhow!("DATABASE_URL must be set"))?,
            bind_addr: env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into()),
            psp_base_url: env::var("PSP_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:9090".into()),
            psp_timeout: Duration::from_millis(
                env::var("PSP_TIMEOUT_MS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(5_000),
            ),
            webhook_max_attempts: env::var("WEBHOOK_MAX_ATTEMPTS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8),
        })
    }
}
