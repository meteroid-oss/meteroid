-- The sandbox signals deleted on the way up cannot be restored.

DELETE FROM "entity_list_view" WHERE "entity_type" = 'SIGNAL';

ALTER TABLE "entity_list_view_default"
  ALTER COLUMN "entity_type" TYPE "CustomPropertyEntityTypeEnum" USING "entity_type"::text::"CustomPropertyEntityTypeEnum";
ALTER TABLE "entity_list_view"
  ALTER COLUMN "entity_type" TYPE "CustomPropertyEntityTypeEnum" USING "entity_type"::text::"CustomPropertyEntityTypeEnum";

DROP TYPE IF EXISTS "EntityListTypeEnum";

ALTER TABLE alert_event
    DROP COLUMN IF EXISTS impact_currency,
    DROP COLUMN IF EXISTS impact_cents,
    DROP COLUMN IF EXISTS evidence;

DROP INDEX IF EXISTS uq_alert_definition_tenant_builtin;
-- Built-in rows would be unevaluable after the rollback; their events cascade.
DELETE FROM alert_definition WHERE builtin_code IS NOT NULL;
ALTER TABLE alert_definition
    DROP COLUMN IF EXISTS category,
    DROP COLUMN IF EXISTS builtin_cursor,
    DROP COLUMN IF EXISTS settings,
    DROP COLUMN IF EXISTS builtin_code;

DROP INDEX IF EXISTS "customer_tenant_owner_idx";
ALTER TABLE "customer" DROP COLUMN IF EXISTS "owner_user_id";
