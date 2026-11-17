-- One writer per customer (Phase 6): the controller runs a customer's billing steps when its
-- next wake is due or an input changed (dirty), under a lease so replicas never run it twice.
CREATE TABLE billing_controller (
    tenant_id    UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    customer_id  UUID        NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    next_wake_at TIMESTAMPTZ,
    dirty_since  TIMESTAMPTZ,
    lease_until  TIMESTAMPTZ,
    attempts     INTEGER     NOT NULL DEFAULT 0,
    last_error   TEXT,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, customer_id)
);

CREATE INDEX billing_controller_wake ON billing_controller (next_wake_at)
    WHERE next_wake_at IS NOT NULL;
CREATE INDEX billing_controller_dirty ON billing_controller (dirty_since)
    WHERE dirty_since IS NOT NULL;
CREATE INDEX billing_controller_customer ON billing_controller (customer_id);
CREATE INDEX billing_controller_due ON billing_controller (LEAST(next_wake_at, dirty_since));

-- Every customer with a subscription or credit still holding value runs once, which computes its
-- first wake.
INSERT INTO billing_controller (tenant_id, customer_id, dirty_since)
SELECT tenant_id, customer_id, now() FROM subscription
UNION
SELECT tenant_id, customer_id, now() FROM customer_credit_grant WHERE remaining > 0
ON CONFLICT DO NOTHING;
