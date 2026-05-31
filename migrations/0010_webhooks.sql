-- Webhook subsystem: a transactional outbox (events) + per-endpoint work items
-- (deliveries), split so that one immutable fact can fan out to N endpoints and
-- be reconciled independently of delivery state.

-- Receiver registrations.
CREATE TABLE webhook_endpoints (
    id           uuid PRIMARY KEY,
    business_id  uuid NOT NULL REFERENCES businesses(id),
    url          text NOT NULL,
    secret       bytea NOT NULL,              -- per-endpoint HMAC signing secret
    event_types  text[] NOT NULL DEFAULT '{}',-- subscription filter; empty = all
    status       text NOT NULL DEFAULT 'active'
                 CHECK (status IN ('active', 'disabled')),
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER webhook_endpoints_set_updated_at
    BEFORE UPDATE ON webhook_endpoints
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE INDEX ix_webhook_endpoints_business ON webhook_endpoints (business_id);

-- The outbox: an immutable, append-only record of "something happened". Written
-- in the SAME transaction as the state change, so events are never lost and
-- never phantom. `id` is sent as X-Webhook-Id (receiver idempotency key).
-- `sequence` gives a monotonic ordering aid for receiver-side sequencing.
CREATE TABLE webhook_events (
    id            uuid PRIMARY KEY,
    business_id   uuid NOT NULL REFERENCES businesses(id),
    event_type    text NOT NULL,             -- invoice.created | invoice.paid | invoice.payment_failed
    aggregate_id  uuid NOT NULL,             -- the invoice id
    payload       jsonb NOT NULL,            -- frozen body that gets signed & sent
    sequence      bigserial NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX ix_webhook_events_business_created
    ON webhook_events (business_id, created_at DESC, id DESC);
CREATE INDEX ix_webhook_events_aggregate ON webhook_events (aggregate_id, sequence);

-- Per-(event x endpoint) delivery work item. Mutable: carries retry state and a
-- lease (locked_until) so a crashed worker's in-flight rows are reclaimable.
CREATE TABLE webhook_deliveries (
    id                uuid PRIMARY KEY,
    event_id          uuid NOT NULL REFERENCES webhook_events(id),
    endpoint_id       uuid NOT NULL REFERENCES webhook_endpoints(id),
    business_id       uuid NOT NULL REFERENCES businesses(id),
    status            text NOT NULL DEFAULT 'pending'
                      CHECK (status IN ('pending', 'in_flight', 'succeeded', 'failed', 'dead')),
    attempts          integer NOT NULL DEFAULT 0,
    next_attempt_at   timestamptz NOT NULL DEFAULT now(),
    locked_until      timestamptz,
    last_status_code  integer,
    last_error        text,
    last_attempt_at   timestamptz,
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER webhook_deliveries_set_updated_at
    BEFORE UPDATE ON webhook_deliveries
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- Idempotent fan-out: re-running fan-out can't create duplicate work.
CREATE UNIQUE INDEX uq_delivery_event_endpoint
    ON webhook_deliveries (event_id, endpoint_id);

-- The dispatcher's claim query scans only due, not-yet-terminal rows.
CREATE INDEX ix_deliveries_due
    ON webhook_deliveries (next_attempt_at)
    WHERE status IN ('pending', 'failed', 'in_flight');

CREATE INDEX ix_deliveries_business ON webhook_deliveries (business_id, created_at DESC);
