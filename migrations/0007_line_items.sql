-- Invoice line items. amount_cents is a STORED GENERATED column: the database
-- itself does the integer multiply, so there is no path where a client total or
-- a float can sneak in. The invoice total is SUM(amount_cents) over these rows,
-- computed server-side at create/finalize.
CREATE TABLE invoice_line_items (
    id                 uuid PRIMARY KEY,
    invoice_id         uuid NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,
    description        text NOT NULL,
    quantity           bigint NOT NULL CHECK (quantity > 0),
    unit_amount_cents  bigint NOT NULL CHECK (unit_amount_cents >= 0),
    amount_cents       bigint NOT NULL
                        GENERATED ALWAYS AS (quantity * unit_amount_cents) STORED,
    position           integer NOT NULL DEFAULT 0,
    created_at         timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX ix_line_items_invoice ON invoice_line_items (invoice_id, position);
