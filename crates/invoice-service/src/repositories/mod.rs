//! Data-access layer. Every function takes an executor and is scoped by
//! `business_id`; no business rules live here, only SQL.

pub mod api_key_repo;
pub mod customer_repo;
pub mod invoice_repo;

/// True if the error is a Postgres unique-constraint violation (SQLSTATE 23505).
/// Used by callers that need to translate a specific constraint into a domain
/// outcome (duplicate email, racing payment attempt, ...).
pub fn is_unique_violation(err: &sqlx::Error) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.code().as_deref() == Some("23505"))
}

/// The constraint name on a unique violation, if available.
pub fn constraint_name(err: &sqlx::Error) -> Option<String> {
    match err {
        sqlx::Error::Database(db) => db.constraint().map(|s| s.to_string()),
        _ => None,
    }
}
