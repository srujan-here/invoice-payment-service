//! The single error type and wire envelope for the whole API.
//!
//! Every handler returns `AppResult<T>`; every error — domain or infrastructure
//! — funnels through [`AppError`], which is the *only* place that decides the
//! HTTP status, the stable machine-readable `code`, and the coarse `type`. That
//! keeps the contract DRY and lets handlers stay thin (`?` everywhere).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

pub type AppResult<T> = Result<T, AppError>;

/// A single field-level validation problem, surfaced under `error.details`.
#[derive(Debug, Clone, Serialize)]
pub struct FieldError {
    pub field: String,
    pub issue: String,
}

impl FieldError {
    pub fn new(field: impl Into<String>, issue: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            issue: issue.into(),
        }
    }
}

/// The body shape returned for *every* error response.
#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    /// Stable, snake_case, machine-readable. Clients branch on this; it never changes.
    pub code: String,
    /// Coarse category (authentication_error, invalid_request_error, ...).
    pub r#type: String,
    /// Human-readable, safe to surface to the caller.
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<FieldError>,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("authentication required or invalid credentials")]
    Unauthorized,

    #[error("{0}")]
    NotFound(&'static str),

    #[error("validation failed")]
    Validation(Vec<FieldError>),

    /// A semantic conflict that is not a state-machine transition (e.g. duplicate
    /// email, idempotency-key reuse with a different body).
    #[error("{code}: {message}")]
    Conflict {
        code: &'static str,
        message: String,
    },

    /// An illegal invoice state transition. Rendered as 409 with a precise code.
    #[error("cannot transition invoice from {from} to {to}")]
    InvalidStateTransition { from: String, to: String },

    /// Payment was attempted but the PSP declined it (a real business outcome,
    /// not a client error) — rendered as 402.
    #[error("payment declined: {0}")]
    PaymentDeclined(String),

    /// Catch-all for infrastructure failures. The real cause is logged against
    /// the request id; the client gets a scrubbed message.
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    pub fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        AppError::Conflict {
            code,
            message: message.into(),
        }
    }

    /// Single source of truth for (variant -> status, code, type). Pinned by a
    /// golden test so codes never drift silently.
    fn parts(&self) -> (StatusCode, &'static str, &'static str) {
        use AppError::*;
        match self {
            Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized", "authentication_error"),
            NotFound(_) => (StatusCode::NOT_FOUND, "not_found", "invalid_request_error"),
            Validation(_) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_error",
                "invalid_request_error",
            ),
            Conflict { code, .. } => (StatusCode::CONFLICT, code, "invalid_request_error"),
            InvalidStateTransition { .. } => (
                StatusCode::CONFLICT,
                "invalid_state_transition",
                "invalid_request_error",
            ),
            PaymentDeclined(_) => (
                StatusCode::PAYMENT_REQUIRED,
                "payment_declined",
                "card_error",
            ),
            Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "api_error",
            ),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, r#type) = self.parts();

        let (message, details) = match &self {
            AppError::Internal(err) => {
                // Log the real cause; return a scrubbed message to the client.
                tracing::error!(error = ?err, "internal error");
                ("an internal error occurred".to_string(), Vec::new())
            }
            AppError::Validation(fields) => ("one or more fields are invalid".to_string(), fields.clone()),
            // The code is already carried in `code`; surface only the message.
            AppError::Conflict { message, .. } => (message.clone(), Vec::new()),
            other => (other.to_string(), Vec::new()),
        };

        let body = ErrorBody {
            error: ErrorDetail {
                code: code.to_string(),
                r#type: r#type.to_string(),
                message,
                details,
            },
        };
        (status, Json(body)).into_response()
    }
}

/// Map sqlx errors. A `RowNotFound` becomes a 404; everything else is internal.
/// (Unique-violation handling for the payment path is done explicitly at the
/// call site where we know which constraint means what.)
impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::RowNotFound => AppError::NotFound("resource not found"),
            other => AppError::Internal(other.into()),
        }
    }
}
