-- API keys, scoped to one business.
--
-- We store the key's PREFIX (non-secret lookup handle) and a SHA-256 of the
-- full secret -- never the secret itself. SHA-256 (not argon2/bcrypt) is the
-- right call here: keys are 32+ bytes of CSPRNG entropy, so a slow KDF buys no
-- security and would cost CPU on every authenticated request. (Defended in
-- DESIGN.md section 5.)
CREATE TABLE api_keys (
    id            uuid PRIMARY KEY,
    business_id   uuid NOT NULL REFERENCES businesses(id),
    name          text,
    prefix        text NOT NULL,          -- e.g. dpk_test_seedkey0  (namespace + first 8 of random)
    key_hash      bytea NOT NULL,         -- SHA-256(full key)
    last_four     char(4) NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    last_used_at  timestamptz,
    revoked_at    timestamptz,            -- soft revoke; NULL = active
    expires_at    timestamptz             -- optional rotation deadline
);

-- Prefix is the unique lookup path: parse it from the bearer token, hit this
-- index for a single row, then constant-time compare the hash.
CREATE UNIQUE INDEX uq_api_keys_prefix ON api_keys (prefix);
CREATE INDEX ix_api_keys_business ON api_keys (business_id);

-- Seed key for the demo business (plaintext in README):
--   dpk_test_seedkey0000000000000000000000000000
INSERT INTO api_keys (id, business_id, name, prefix, key_hash, last_four)
VALUES (
    '018f0000-0000-7000-8000-000000000002',
    '018f0000-0000-7000-8000-000000000001',
    'seed-key',
    'dpk_test_seedkey0',
    decode('9121441a46784f879f47389c73bd47bc5a488c323b9f365460b34105434dce91', 'hex'),
    '0000'
);
