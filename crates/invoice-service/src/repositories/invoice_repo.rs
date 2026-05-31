//! Invoice + line-item persistence. Functions that participate in a larger
//! transaction (create, transition) are generic over the executor so the
//! service can run them inside the same `tx` that writes the webhook outbox.

use chrono::{DateTime, Utc};
use common::ids::new_id;
use common::Money;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::invoice::{Invoice, InvoiceState, LineItem};

const INVOICE_COLS: &str = "id, business_id, customer_id, state, currency, total_cents, \
    finalized_at, paid_at, voided_at, version, created_at, updated_at";

pub async fn insert_invoice<'e, E>(
    exec: E,
    business_id: Uuid,
    customer_id: Uuid,
) -> Result<Invoice, sqlx::Error>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as::<_, Invoice>(&format!(
        "INSERT INTO invoices (id, business_id, customer_id) VALUES ($1, $2, $3) RETURNING {INVOICE_COLS}"
    ))
    .bind(new_id())
    .bind(business_id)
    .bind(customer_id)
    .fetch_one(exec)
    .await
}

pub async fn insert_line_item<'e, E>(
    exec: E,
    invoice_id: Uuid,
    description: &str,
    quantity: i64,
    unit_amount_cents: i64,
    position: i32,
) -> Result<(), sqlx::Error>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query(
        r#"
        INSERT INTO invoice_line_items
            (id, invoice_id, description, quantity, unit_amount_cents, position)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(new_id())
    .bind(invoice_id)
    .bind(description)
    .bind(quantity)
    .bind(unit_amount_cents)
    .bind(position)
    .execute(exec)
    .await?;
    Ok(())
}

/// Recompute the invoice total from its line items (server-side SUM). Returns
/// the new total. The DB does the integer arithmetic; no float is involved.
pub async fn sync_total<'e, E>(exec: E, invoice_id: Uuid) -> Result<Money, sqlx::Error>
where
    E: sqlx::PgExecutor<'e>,
{
    let total: i64 = sqlx::query_scalar(
        r#"
        UPDATE invoices
        SET total_cents = (
            SELECT COALESCE(SUM(amount_cents), 0)
            FROM invoice_line_items WHERE invoice_id = $1
        )
        WHERE id = $1
        RETURNING total_cents
        "#,
    )
    .bind(invoice_id)
    .fetch_one(exec)
    .await?;
    Ok(Money::from_cents(total))
}

pub async fn get(pool: &PgPool, business_id: Uuid, id: Uuid) -> Result<Option<Invoice>, sqlx::Error> {
    sqlx::query_as::<_, Invoice>(&format!(
        "SELECT {INVOICE_COLS} FROM invoices WHERE id = $1 AND business_id = $2"
    ))
    .bind(id)
    .bind(business_id)
    .fetch_optional(pool)
    .await
}

pub async fn list_line_items<'e, E>(exec: E, invoice_id: Uuid) -> Result<Vec<LineItem>, sqlx::Error>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as::<_, LineItem>(
        r#"
        SELECT id, invoice_id, description, quantity, unit_amount_cents, amount_cents, position
        FROM invoice_line_items
        WHERE invoice_id = $1
        ORDER BY position, id
        "#,
    )
    .bind(invoice_id)
    .fetch_all(exec)
    .await
}

/// List invoices, optionally filtered by state, keyset-paginated.
pub async fn list(
    pool: &PgPool,
    business_id: Uuid,
    state: Option<InvoiceState>,
    limit: i64,
    cursor: Option<(DateTime<Utc>, Uuid)>,
) -> Result<Vec<Invoice>, sqlx::Error> {
    // Built dynamically but with only bound parameters — no string interpolation
    // of user input.
    let mut sql = format!("SELECT {INVOICE_COLS} FROM invoices WHERE business_id = $1");
    if state.is_some() {
        sql.push_str(" AND state = $2");
    }
    if cursor.is_some() {
        let n = if state.is_some() { ("$3", "$4") } else { ("$2", "$3") };
        sql.push_str(&format!(" AND (created_at, id) < ({}, {})", n.0, n.1));
    }
    sql.push_str(" ORDER BY created_at DESC, id DESC");
    let limit_pos = 2 + state.is_some() as usize * 1 + cursor.is_some() as usize * 2;
    sql.push_str(&format!(" LIMIT ${limit_pos}"));

    let mut q = sqlx::query_as::<_, Invoice>(&sql).bind(business_id);
    if let Some(s) = state {
        q = q.bind(s);
    }
    if let Some((ts, id)) = cursor {
        q = q.bind(ts).bind(id);
    }
    q.bind(limit).fetch_all(pool).await
}

/// Status-conditional transition with optimistic version bump. Returns the
/// updated invoice, or `None` if the row was not in `from` state (caller maps
/// `None` to "not found" vs "invalid transition" by re-reading).
pub async fn transition<'e, E>(
    exec: E,
    business_id: Uuid,
    id: Uuid,
    from: InvoiceState,
    to: InvoiceState,
) -> Result<Option<Invoice>, sqlx::Error>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as::<_, Invoice>(&format!(
        r#"
        UPDATE invoices
        SET state = $3,
            version = version + 1,
            finalized_at = CASE WHEN $3 = 'open'::invoice_state THEN now() ELSE finalized_at END,
            voided_at    = CASE WHEN $3 = 'void'::invoice_state THEN now() ELSE voided_at END
        WHERE id = $1 AND business_id = $2 AND state = $4
        RETURNING {INVOICE_COLS}
        "#
    ))
    .bind(id)
    .bind(business_id)
    .bind(to)
    .bind(from)
    .fetch_optional(exec)
    .await
}
