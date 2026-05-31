-- A payment attempt records one try at paying an invoice. amount_cents is a
-- SNAPSHOT of the invoice total at claim time (never trusted from the request),
-- so a later invoice edit can't retroactively change what was charged.
CREATE TABLE payment_attempts (
    id               uuid PRIMARY KEY,
    invoice_id       uuid NOT NULL REFERENCES invoices(id),
    business_id      uuid NOT NULL REFERENCES businesses(id),
    idempotency_key  text NOT NULL,
    amount_cents     bigint NOT NULL CHECK (amount_cents >= 0),
    currency         char(3) NOT NULL DEFAULT 'USD' CHECK (currency = 'USD'),
    card_token       text NOT NULL,
    status           payment_status NOT NULL DEFAULT 'pending',
    psp_ref          text,                 -- set on success
    failure_code     text,                 -- card_declined | insufficient_funds
    last_error       text,                 -- psp_timeout | psp_network_error
    attempt_count    integer NOT NULL DEFAULT 1,
    next_poll_at     timestamptz,          -- reconciler schedule (set while pending/unknown)
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER payment_attempts_set_updated_at
    BEFORE UPDATE ON payment_attempts
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- THE single-active-charge guard. At most one attempt per invoice may be
-- 'pending' or 'succeeded' at a time. Two concurrent /pay requests both try to
-- INSERT a 'pending' row; exactly one wins, the other gets a 23505 unique
-- violation -> 409. A 'failed' attempt is OUTSIDE this predicate, so a declined
-- invoice is immediately retryable. This is the concurrency mechanism. (See
-- DESIGN.md section 3.)
CREATE UNIQUE INDEX uq_active_attempt_per_invoice
    ON payment_attempts (invoice_id) WHERE status IN ('pending', 'succeeded');

-- The reconciler scans only due pending rows; partial index keeps it tiny.
CREATE INDEX ix_payment_attempts_pending_poll
    ON payment_attempts (next_poll_at) WHERE status = 'pending';

CREATE INDEX ix_payment_attempts_invoice ON payment_attempts (invoice_id, created_at DESC);
