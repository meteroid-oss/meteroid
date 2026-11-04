-- Config features now declare their value type authoritatively (was only implied by config_options).
CREATE TYPE "ConfigValueTypeEnum" AS ENUM ('NUMBER', 'BOOLEAN', 'TEXT', 'MAP', 'JSON', 'SELECT');

ALTER TABLE feature ADD COLUMN config_value_type "ConfigValueTypeEnum";

-- Backfill: options present ⇒ a select feature.
UPDATE feature SET config_value_type = 'SELECT'
WHERE feature_type = 'CONFIG' AND config_options IS NOT NULL;

-- Backfill: derive the kind from the feature-level default entitlement's stored config value.
-- EntitlementValue is `{"type":"config","value":{"kind":"number","value":...}}`.
UPDATE feature f
SET config_value_type = (
  CASE e.value -> 'value' ->> 'kind'
    WHEN 'number' THEN 'NUMBER'
    WHEN 'boolean' THEN 'BOOLEAN'
    WHEN 'text' THEN 'TEXT'
    WHEN 'json' THEN 'JSON'
    ELSE 'TEXT'
  END
)::"ConfigValueTypeEnum"
FROM entitlement e
WHERE e.feature_id = f.id
  AND e.entity_type = 'FEATURE'
  AND e.value ->> 'type' = 'config'
  AND f.feature_type = 'CONFIG'
  AND f.config_value_type IS NULL;

-- Backfill: config features with no default entitlement default to a free-form text value.
UPDATE feature SET config_value_type = 'TEXT'
WHERE feature_type = 'CONFIG' AND config_value_type IS NULL;
