-- Idempotency for POST /invoices/{id}/pay. Separate from the single-active-
-- charge guard on purpose: this protects against the SAME client request being
-- retried, keyed by the client-supplied Idempotency-Key.
--
-- request_fingerprint = SHA-256 of the canonical request (invoice + token +
-- amount + currency). Reusing a key with a DIFFERENT body is a 422 -- we refuse
-- to silently charge a different thing under the same key.
CREATE TABLE idempotency_keys (
    id                   uuid PRIMARY KEY,
    business_id          uuid NOT NULL REFERENCES businesses(id),
    idempotency_key      text NOT NULL,
    request_fingerprint  bytea NOT NULL,
    status               text NOT NULL DEFAULT 'in_progress'
                          CHECK (status IN ('in_progress', 'completed')),
    attempt_id           uuid REFERENCES payment_attempts(id),
    response_status      integer,          -- cached HTTP status for replay
    response_body        jsonb,            -- cached response body for replay
    created_at           timestamptz NOT NULL DEFAULT now(),
    expires_at           timestamptz NOT NULL DEFAULT now() + interval '24 hours'
);

-- The dedupe guarantee. INSERT ... ON CONFLICT DO NOTHING on this constraint is
-- how we atomically distinguish a fresh request from a replay.
CREATE UNIQUE INDEX uq_idempotency_business_key
    ON idempotency_keys (business_id, idempotency_key);

CREATE INDEX ix_idempotency_expires ON idempotency_keys (expires_at);
