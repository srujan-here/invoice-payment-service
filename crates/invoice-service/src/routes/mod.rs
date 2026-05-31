//! HTTP layer. Handlers do three things and no more: extract input, call one
//! service function, map the result. No SQL, no business rules.

pub mod api_keys;
pub mod customers;
pub mod invoices;
pub mod webhooks;

use serde::Deserialize;

/// Common list query params.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}
