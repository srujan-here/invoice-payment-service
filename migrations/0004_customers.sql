-- Customers belong to exactly one business. Email is case-insensitive and
-- unique PER BUSINESS (two businesses may legitimately share a customer email),
-- enforced by a partial unique index that ignores NULL emails.
CREATE TABLE customers (
    id           uuid PRIMARY KEY,
    business_id  uuid NOT NULL REFERENCES businesses(id),
    email        citext,
    name         text,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now()
);

CREATE TRIGGER customers_set_updated_at
    BEFORE UPDATE ON customers
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE UNIQUE INDEX uq_customers_business_email
    ON customers (business_id, email) WHERE email IS NOT NULL;

-- Scoped list + keyset pagination on (created_at, id).
CREATE INDEX ix_customers_business_created
    ON customers (business_id, created_at DESC, id DESC);
