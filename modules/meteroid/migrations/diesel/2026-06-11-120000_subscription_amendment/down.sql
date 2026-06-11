ALTER TABLE subscription_add_on
  DROP COLUMN IF EXISTS effective_to,
  DROP COLUMN IF EXISTS effective_from;

-- Note: PostgreSQL cannot remove an enum value; APPLY_AMENDMENT remains on
-- "ScheduledEventTypeEnum" but is harmless if unused.

ALTER TABLE scheduled_event DROP COLUMN IF EXISTS created_by_customer;

DROP INDEX IF EXISTS idx_sub_add_on_lineage;
DROP INDEX IF EXISTS idx_sub_component_lineage;

ALTER TABLE subscription_add_on DROP COLUMN IF EXISTS lineage_id;
ALTER TABLE subscription_component DROP COLUMN IF EXISTS lineage_id;
