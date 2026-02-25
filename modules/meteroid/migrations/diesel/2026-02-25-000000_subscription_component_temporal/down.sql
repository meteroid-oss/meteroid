DROP INDEX IF EXISTS idx_sub_component_period_overlap;
DROP INDEX IF EXISTS idx_sub_component_active;

ALTER TABLE subscription_component
  DROP COLUMN IF EXISTS effective_to,
  DROP COLUMN IF EXISTS effective_from;
