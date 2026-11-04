-- Customer-provided unique identifier for a feature, scoped per tenant.
--
-- A feature could previously only be addressed by its generated UUID `id`, which
-- forced consumers (e.g. dog-fooded feature gates) to inject that UUID via config.
-- `code` is a customer-provided slug, unique per tenant and immutable, so a feature
-- can be referenced by a portable key. Mirrors `billable_metric.code`, but here
-- `code` is the addressing key, so it carries a UNIQUE (tenant_id, code) constraint.
-- With `code` as the addressing key, `name` no longer needs to be unique.
ALTER TABLE feature
    ADD COLUMN code TEXT;

-- Backfill existing rows with a unique, non-null value so the constraints below
-- can be applied with zero downtime. Operators may rename to a meaningful slug
-- afterwards (codes are immutable through the API but editable in the DB).
UPDATE feature
SET code = id::text
WHERE code IS NULL;

ALTER TABLE feature
    ALTER COLUMN code SET NOT NULL;

ALTER TABLE feature
    ADD CONSTRAINT feature_tenant_code_key UNIQUE (tenant_id, code);

-- `code` is now the addressing key, so per-tenant uniqueness of `name` is dropped.
ALTER TABLE feature
    DROP CONSTRAINT IF EXISTS feature_tenant_id_name_key;
