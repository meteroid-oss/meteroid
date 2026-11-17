-- When the facts next change a subscription's derived state (a plan window
-- opening, a boundary, a trial end, an ending), written by `lifecycle::derived::derive_state_tx`.
-- The billing controller wakes the customer then. NULL: nothing to derive until a fact changes.
SET LOCAL lock_timeout = '5s';

ALTER TABLE subscription ADD COLUMN next_wake_at TIMESTAMPTZ;

CREATE INDEX subscription_next_wake_at_idx ON subscription (tenant_id, customer_id, next_wake_at)
    WHERE next_wake_at IS NOT NULL;
