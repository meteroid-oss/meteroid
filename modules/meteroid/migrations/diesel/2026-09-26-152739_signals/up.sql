-- Customer signals (ADR-0002, ADR-0004).

-- Account ownership (ADR-0002). A deleted user frees their accounts; org membership is checked in
-- the store, since the FK to "user" cannot express it.
ALTER TABLE "customer"
  ADD COLUMN "owner_user_id" UUID NULL REFERENCES "user" ("id") ON DELETE SET NULL;
CREATE INDEX "customer_tenant_owner_idx" ON "customer" ("tenant_id", "owner_user_id");

-- Built-in signals, and a custom signal's category: where it shows on Signals and whether its
-- events carry a revenue impact (expansion is upside not priced; the others put the customer's MRR
-- at stake). The category is NULL on built-ins (their code decides) and on metric monitors.
ALTER TABLE alert_definition
    ADD COLUMN builtin_code TEXT,
    ADD COLUMN settings JSONB,
    ADD COLUMN builtin_cursor UUID,
    ADD COLUMN category TEXT
        CHECK (category IN ('expansion', 'risk', 'contracts', 'billing'));

CREATE UNIQUE INDEX uq_alert_definition_tenant_builtin
    ON alert_definition (tenant_id, builtin_code)
    WHERE builtin_code IS NOT NULL;

ALTER TABLE alert_event
    ADD COLUMN evidence JSONB,
    ADD COLUMN impact_cents BIGINT,
    ADD COLUMN impact_currency TEXT;

-- Saved views are keyed by the list they belong to, not by the custom-property entity type: the
-- signals list has views but no custom properties.
CREATE TYPE "EntityListTypeEnum" AS ENUM ('CUSTOMER', 'SUBSCRIPTION', 'INVOICE', 'CREDIT_NOTE', 'QUOTE', 'SIGNAL');

ALTER TABLE "entity_list_view"
  ALTER COLUMN "entity_type" TYPE "EntityListTypeEnum" USING "entity_type"::text::"EntityListTypeEnum";
ALTER TABLE "entity_list_view_default"
  ALTER COLUMN "entity_type" TYPE "EntityListTypeEnum" USING "entity_type"::text::"EntityListTypeEnum";

-- The sandbox seed used to create two custom signals that the built-ins supersede: "Overdue
-- balance" duplicates payment_failing and "Shrinking accounts" flags a contraction only after it
-- is booked. Drop them from sandbox tenants; their events and memberships cascade.
DELETE FROM alert_definition d
USING tenant t
WHERE t.id = d.tenant_id
  AND t.environment = 'SANDBOX'
  AND d.builtin_code IS NULL
  AND d.name IN ('Overdue balance — collection risk', 'Shrinking accounts — MRR contracting');

-- Existing custom signals take their category from their kind. Keep this statement last: the
-- integration tests replay it from here to the end of the file.
UPDATE alert_definition
SET category = CASE WHEN kind = 'opportunity' THEN 'expansion' ELSE 'risk' END
WHERE builtin_code IS NULL
  AND metric_key = 'segment';
