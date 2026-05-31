//! Customer use-cases.

use common::error::FieldError;
use common::pagination::{clamp_limit, decode_cursor, encode_cursor, Page};
use common::{AppError, AppResult};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::customer::{CreateCustomerRequest, CustomerResponse};
use crate::repositories::{customer_repo, is_unique_violation};

pub async fn create(
    pool: &PgPool,
    business_id: Uuid,
    req: CreateCustomerRequest,
) -> AppResult<CustomerResponse> {
    let mut errors = Vec::new();
    if let Some(email) = &req.email {
        // Deliberately lightweight: a single '@' with text either side. We are
        // not in the business of RFC 5322 validation.
        if !is_plausible_email(email) {
            errors.push(FieldError::new("email", "must be a valid email address"));
        }
    }
    if req.email.is_none() && req.name.is_none() {
        errors.push(FieldError::new("name", "provide at least a name or an email"));
    }
    if !errors.is_empty() {
        return Err(AppError::Validation(errors));
    }

    match customer_repo::insert(pool, business_id, req.email.as_deref(), req.name.as_deref()).await {
        Ok(c) => Ok(c.into()),
        Err(e) if is_unique_violation(&e) => Err(AppError::conflict(
            "customer_email_exists",
            "a customer with this email already exists for this business",
        )),
        Err(e) => Err(e.into()),
    }
}

pub async fn get(pool: &PgPool, business_id: Uuid, id: Uuid) -> AppResult<CustomerResponse> {
    customer_repo::get(pool, business_id, id)
        .await?
        .map(Into::into)
        .ok_or(AppError::NotFound("customer not found"))
}

pub async fn list(
    pool: &PgPool,
    business_id: Uuid,
    limit: Option<i64>,
    cursor: Option<String>,
) -> AppResult<Page<CustomerResponse>> {
    let limit = clamp_limit(limit);
    let decoded = cursor.as_deref().and_then(decode_cursor);
    // Fetch one extra to know whether there's a next page.
    let mut rows = customer_repo::list(pool, business_id, limit + 1, decoded).await?;

    let next_cursor = if rows.len() as i64 > limit {
        let last = &rows[limit as usize - 1];
        let c = encode_cursor(last.created_at, last.id);
        rows.truncate(limit as usize);
        Some(c)
    } else {
        None
    };

    Ok(Page::new(rows.into_iter().map(Into::into).collect(), next_cursor))
}

fn is_plausible_email(s: &str) -> bool {
    let mut parts = s.splitn(2, '@');
    matches!((parts.next(), parts.next()), (Some(a), Some(b)) if !a.is_empty() && b.contains('.'))
}
