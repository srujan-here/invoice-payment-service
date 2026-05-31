-- Invoices belong to a customer; business_id is denormalized so every auth
-- filter is a single-table predicate (no join to enforce tenancy).
--
-- total_cents is SERVER-COMPUTED from line items (see 0007) and never read from
-- the client. `version` is an optimistic guard for the lifecycle transitions
-- (finalize/void/uncollectible) so two admins can't race a transition.
CREATE TABLE invoices (
    id            uuid PRIMARY KEY,
    business_id   uuid NOT NULL REFERENCES businesses(id),
    customer_id   uuid NOT NULL REFERENCES customers(id),
    state         invoice_state NOT NULL DEFAULT 'draft',
    currency      char(3) NOT NULL DEFAULT 'USD' CHECK (currency = 'USD'),
    total_cents   bigint NOT NULL DEFAULT 0 CHECK (total_cents >= 0),
    finalized_at  timestamptz,
    paid_at       timestamptz,
    voided_at     timestamptz,
    version       integer NOT NULL DEFAULT 0,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER invoices_set_updated_at
    BEFORE UPDATE ON invoices
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- The list-filterable-by-state query: scope + filter + keyset sort in one index.
CREATE INDEX ix_invoices_business_state_created
    ON invoices (business_id, state, created_at DESC, id DESC);

CREATE INDEX ix_invoices_customer ON invoices (customer_id);
