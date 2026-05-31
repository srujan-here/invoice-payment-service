//! Invoice entity, line items, the state enum, and the state-machine rules.

use chrono::{DateTime, Utc};
use common::Money;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Invoice lifecycle. Mirrors the `invoice_state` Postgres enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "invoice_state", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum InvoiceState {
    Draft,
    Open,
    Paid,
    Void,
    Uncollectible,
}

impl InvoiceState {
    pub fn as_str(&self) -> &'static str {
        match self {
            InvoiceState::Draft => "draft",
            InvoiceState::Open => "open",
            InvoiceState::Paid => "paid",
            InvoiceState::Void => "void",
            InvoiceState::Uncollectible => "uncollectible",
        }
    }

    /// Terminal states accept no further transitions.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            InvoiceState::Paid | InvoiceState::Void | InvoiceState::Uncollectible
        )
    }

    /// The single source of truth for which transitions are legal. The payment
    /// path does NOT go through here — `open -> paid` is performed by a
    /// status-conditional UPDATE in the payment service so it is atomic with the
    /// charge. This governs the admin-driven transitions (finalize/void/etc).
    pub fn can_transition_to(&self, next: InvoiceState) -> bool {
        use InvoiceState::*;
        matches!(
            (self, next),
            (Draft, Open)            // finalize
                | (Draft, Void)      // discard a draft
                | (Open, Paid)       // successful payment
                | (Open, Void)       // cancel an open invoice
                | (Open, Uncollectible) // write off
        )
    }
}

/// An invoice row (without its line items).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Invoice {
    pub id: Uuid,
    pub business_id: Uuid,
    pub customer_id: Uuid,
    pub state: InvoiceState,
    pub currency: String,
    pub total_cents: Money,
    pub finalized_at: Option<DateTime<Utc>>,
    pub paid_at: Option<DateTime<Utc>>,
    pub voided_at: Option<DateTime<Utc>>,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LineItem {
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub description: String,
    pub quantity: i64,
    pub unit_amount_cents: Money,
    pub amount_cents: Money,
    pub position: i32,
}

// ---- request / response DTOs ----

#[derive(Debug, Deserialize)]
pub struct CreateInvoiceRequest {
    pub customer_id: Uuid,
    pub line_items: Vec<CreateLineItem>,
    // NOTE: there is deliberately no `total` field. The server computes it.
}

#[derive(Debug, Deserialize)]
pub struct CreateLineItem {
    pub description: String,
    pub quantity: i64,
    pub unit_amount_cents: i64,
}

#[derive(Debug, Serialize)]
pub struct LineItemResponse {
    pub description: String,
    pub quantity: i64,
    pub unit_amount_cents: i64,
    pub amount_cents: i64,
}

impl From<LineItem> for LineItemResponse {
    fn from(li: LineItem) -> Self {
        Self {
            description: li.description,
            quantity: li.quantity,
            unit_amount_cents: li.unit_amount_cents.cents(),
            amount_cents: li.amount_cents.cents(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct InvoiceResponse {
    pub id: Uuid,
    pub customer_id: Uuid,
    pub state: InvoiceState,
    pub currency: String,
    pub total_cents: i64,
    pub line_items: Vec<LineItemResponse>,
    pub finalized_at: Option<DateTime<Utc>>,
    pub paid_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl InvoiceResponse {
    pub fn from_parts(inv: Invoice, items: Vec<LineItem>) -> Self {
        Self {
            id: inv.id,
            customer_id: inv.customer_id,
            state: inv.state,
            currency: inv.currency,
            total_cents: inv.total_cents.cents(),
            line_items: items.into_iter().map(Into::into).collect(),
            finalized_at: inv.finalized_at,
            paid_at: inv.paid_at,
            created_at: inv.created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::InvoiceState::*;

    #[test]
    fn legal_transitions_only() {
        assert!(Draft.can_transition_to(Open));
        assert!(Open.can_transition_to(Paid));
        assert!(Open.can_transition_to(Void));
        assert!(Open.can_transition_to(Uncollectible));
        // illegal
        assert!(!Draft.can_transition_to(Paid));
        assert!(!Paid.can_transition_to(Open));
        assert!(!Void.can_transition_to(Paid));
        assert!(!Open.can_transition_to(Draft));
    }

    #[test]
    fn terminals() {
        assert!(Paid.is_terminal());
        assert!(Void.is_terminal());
        assert!(Uncollectible.is_terminal());
        assert!(!Draft.is_terminal());
        assert!(!Open.is_terminal());
    }
}
