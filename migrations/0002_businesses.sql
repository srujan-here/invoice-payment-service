-- The tenant root. Everything else hangs off business_id.
CREATE TABLE businesses (
    id          uuid PRIMARY KEY,
    name        text NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER businesses_set_updated_at
    BEFORE UPDATE ON businesses
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- A seed business + a well-known API key so the service is usable the moment
-- `docker compose up` finishes, with zero manual steps (see README curl flow).
-- key_hash is SHA-256 of the plaintext key below; prefix is its lookup handle.
--   plaintext: dpk_test_seedkey0000000000000000000000000000
INSERT INTO businesses (id, name)
VALUES ('018f0000-0000-7000-8000-000000000001', 'Acme Demo Co');
