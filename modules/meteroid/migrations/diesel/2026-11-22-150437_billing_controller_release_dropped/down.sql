CREATE TABLE billing_controller_release (
    tenant_id   UUID UNIQUE REFERENCES tenant (id) ON DELETE CASCADE,
    released_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    released_by TEXT
);
CREATE UNIQUE INDEX billing_controller_release_all ON billing_controller_release ((1))
    WHERE tenant_id IS NULL;
