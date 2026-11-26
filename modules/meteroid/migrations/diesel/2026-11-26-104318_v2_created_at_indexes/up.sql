-- API v2 lists filter by `created_after` / `created_before` (ADR-0013 §3): a tenant's rows by
-- when they were created, so a narrow window reads its rows rather than walking the whole table in
-- id order. Credit notes are read by their tenant index (`idx_credit_note_tenant_id`).
SET LOCAL lock_timeout = '5s';

CREATE INDEX IF NOT EXISTS subscription_tenant_created_at_idx ON subscription (tenant_id, created_at);
CREATE INDEX IF NOT EXISTS invoice_tenant_created_at_idx ON invoice (tenant_id, created_at);
CREATE INDEX IF NOT EXISTS customer_tenant_created_at_idx ON customer (tenant_id, created_at);
