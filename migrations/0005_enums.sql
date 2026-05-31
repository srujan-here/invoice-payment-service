-- Invoice lifecycle states. draft -> open (finalize) -> paid; open -> void;
-- open -> uncollectible. paid/void/uncollectible are terminal.
CREATE TYPE invoice_state AS ENUM ('draft', 'open', 'paid', 'void', 'uncollectible');

-- Payment attempt lifecycle. 'pending' is the in-flight/unknown state that a
-- PSP timeout or crash can leave behind; the reconciler resolves it. 'expired'
-- is only reachable on a PSP-confirmed "not charged".
CREATE TYPE payment_status AS ENUM ('pending', 'succeeded', 'failed', 'expired');
