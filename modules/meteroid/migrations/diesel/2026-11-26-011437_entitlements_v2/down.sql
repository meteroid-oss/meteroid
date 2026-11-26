-- Rollback (ADR 0011 §7): rows the v1 model can't express are lost, and v1 rows edited under v2
-- take the feature's policy; ids held by clients keep working.
SET LOCAL lock_timeout = '10s';

-- UUID v5 over raw bytes, as `Uuid::new_v5`.
CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE OR REPLACE FUNCTION pg_temp.uuid_v5(ns UUID, name BYTEA) RETURNS UUID
    LANGUAGE sql IMMUTABLE AS
$$
SELECT encode(set_byte(set_byte(substr(h, 1, 16), 6, (get_byte(h, 6) & 15) | 80), 8, (get_byte(h, 8) & 63) | 128),
              'hex')::uuid
FROM (SELECT digest(uuid_send(ns) || name, 'sha1') AS h) s
$$;

-- EE monitors: a customer-level limit's announcements go back to the monitor of the customer's
-- anchor subscription (the earliest started active one), when it has one. First, before any lock.
CREATE TEMP TABLE monitor_rekey_back ON COMMIT DROP AS
SELECT *
FROM (SELECT pg_temp.uuid_v5(f.id, uuid_send(s.customer_id)) AS level_id,
             pg_temp.uuid_v5(f.id, uuid_send(s.id))          AS anchor_id,
             s.tenant_id,
             s.customer_id
      FROM (SELECT DISTINCT ON (customer_id) id, customer_id, tenant_id
            FROM subscription
            WHERE status IN ('ACTIVE', 'TRIAL_ACTIVE')
              AND (customer_id IN (SELECT customer_id FROM usage_monitor_state WHERE monitor_kind = 'entitlement')
                OR tenant_id IN (SELECT tenant_id FROM usage_alert_state))
            ORDER BY customer_id, start_date, id) s
               JOIN feature f ON f.tenant_id = s.tenant_id AND f.kind = 'LIMIT'
      WHERE EXISTS (SELECT 1 FROM entitlement e WHERE e.feature_id = f.id)) c
WHERE level_id IN (SELECT monitor_id FROM usage_monitor_state UNION SELECT monitor_id FROM usage_alert_state);

-- The anchor's copy the deploy left gives way to the customer level's, which is newer.
DELETE
FROM usage_alert_state o
    USING monitor_rekey_back r, usage_alert_state a
WHERE o.monitor_id = r.anchor_id
  AND a.monitor_id = r.level_id
  AND a.window_key = o.window_key
  AND a.level = o.level;

UPDATE usage_alert_state a
SET monitor_id = r.anchor_id
FROM monitor_rekey_back r
WHERE a.monitor_id = r.level_id;

WITH dropped AS (
    DELETE FROM usage_monitor_state ms
        USING monitor_rekey_back r
        WHERE ms.monitor_id = r.level_id
        RETURNING r.tenant_id, r.customer_id)
INSERT
INTO credit_recompute_mark (tenant_id, customer_id)
SELECT DISTINCT tenant_id, customer_id
FROM dropped
ON CONFLICT (tenant_id, customer_id) DO NOTHING;

DROP TABLE entitlement_app_count_op;
DROP TABLE entitlement_app_count;
DROP TABLE entitlement_schedule_cursor;


-- 1. Rows the old model can't express.
DELETE
FROM entitlement e
    USING feature f
WHERE f.id = e.feature_id
  AND (e.level <> 'FEATURE' OR e.resource_id IS NOT NULL OR e.condition IS NOT NULL
    OR e.valid_from IS NOT NULL OR e.valid_until IS NOT NULL
    OR (e.effect <> 'SET' AND e.entity_type <> 'ADD_ON')
    OR f.level_resource_type_id IS NOT NULL OR f.limit_counted_by = 'APP');

-- A limit the app counts has no metric: the previous release fails reading any of the tenant's
-- features with one left, so it goes with its rows.
DELETE
FROM feature
WHERE feature_type = 'METERED'
  AND metric_id IS NULL;

-- 2. An edited lineage's open row takes back the lineage's first id; history goes.
DELETE
FROM entitlement r
    USING entitlement o
WHERE o.revoked_at IS NULL
  AND o.id <> o.lineage_id
  AND r.lineage_id = o.lineage_id
  AND r.revoked_at IS NOT NULL;
UPDATE entitlement
SET id = lineage_id
WHERE revoked_at IS NULL
  AND id <> lineage_id;
DELETE
FROM entitlement
WHERE revoked_at IS NOT NULL;

-- 3. Rows written by v2 get their policy back into the value, config booleans their config shape;
-- converted v1 rows kept theirs.
UPDATE entitlement e
SET value = e.value || jsonb_build_object(
        'reset_period', CASE p ->> 'resets'
                            WHEN 'BILLING_CYCLE' THEN '{"type": "billing_cycle"}'::jsonb
                            WHEN 'CALENDAR' THEN jsonb_build_object('type', 'calendar') || (p -> 'period')
                            WHEN 'FIXED' THEN jsonb_build_object('type', 'fixed_window') || (p -> 'period')
                            WHEN 'SLIDING' THEN jsonb_build_object('type', 'sliding_window') || (p -> 'period')
                            ELSE '{"type": "never"}' END,
        'overage_behavior', CASE
                                WHEN p ->> 'at_limit' = 'ALLOW_AND_BILL' THEN '{"type": "allow"}'::jsonb
                                ELSE jsonb_strip_nulls(jsonb_build_object(
                                        'type', 'block',
                                        'grace_period_pct', NULLIF((p ->> 'grace_pct')::int, 0))) END,
        'thresholds', COALESCE(p -> 'thresholds', '[]'))
FROM (SELECT e2.id, COALESCE(e2.policy_override, f.policy, '{}') AS p
      FROM entitlement e2
               JOIN feature f ON f.id = e2.feature_id) pol
WHERE pol.id = e.id
  AND e.value ->> 'type' = 'metered'
  AND NOT e.value ? 'overage_behavior';

UPDATE entitlement e
SET value = jsonb_build_object('type', 'config', 'value',
                               jsonb_build_object('kind', 'boolean', 'value', COALESCE((e.value ->> 'enabled')::boolean, false)))
FROM feature f
WHERE f.id = e.feature_id
  AND f.feature_type = 'CONFIG'
  AND e.value ->> 'type' = 'boolean';


-- 4. One row per (feature, owner) again.
DROP INDEX entitlement_open_key_idx;
DROP INDEX entitlement_open_lineage_key;
DROP INDEX entitlement_lineage_idx;
DROP INDEX entitlement_valid_from_idx;
DROP INDEX entitlement_valid_until_idx;
ALTER TABLE entitlement
    ADD CONSTRAINT entitlement_feature_id_entity_id_entity_type_key UNIQUE (feature_id, entity_id, entity_type);

-- 5. The columns.
ALTER TABLE entitlement
    DROP CONSTRAINT entitlement_level_type_check,
    DROP CONSTRAINT entitlement_policy_override_check,
    DROP CONSTRAINT entitlement_condition_hash_check,
    DROP CONSTRAINT entitlement_validity_check,
    DROP COLUMN lineage_id,
    DROP COLUMN lineage_created_at,
    DROP COLUMN lineage_anchor,
    DROP COLUMN effect,
    DROP COLUMN level,
    DROP COLUMN level_type_id,
    DROP COLUMN resource_id,
    DROP COLUMN condition,
    DROP COLUMN condition_hash,
    DROP COLUMN valid_from,
    DROP COLUMN valid_until,
    DROP COLUMN reason,
    DROP COLUMN created_by,
    DROP COLUMN created_by_kind,
    DROP COLUMN revoked_at,
    DROP COLUMN revoked_by,
    DROP COLUMN policy_override;

ALTER TABLE feature
    DROP COLUMN kind,
    DROP COLUMN value_type,
    DROP COLUMN unit,
    DROP COLUMN number_higher_is_better,
    DROP COLUMN select_options,
    DROP COLUMN select_ordered,
    DROP COLUMN limit_counted_by,
    DROP COLUMN level_resource_type_id,
    DROP COLUMN policy;

DROP TABLE resource;
DROP TABLE resource_type_key;
DROP TABLE resource_type;

ALTER TABLE tenant
    DROP COLUMN max_resources_per_customer_type,
    DROP COLUMN entitlement_op_retention_days,
    DROP COLUMN entitlement_op_log_complete_from;
ALTER TABLE portal_token
    DROP COLUMN client_claims;

DROP TYPE "EntitlementEffectEnum";
DROP TYPE "EntitlementLevelEnum";
DROP TYPE "FeatureKindEnum";
DROP TYPE "FeatureValueTypeEnum";
DROP TYPE "FeatureLimitCountedByEnum";
DROP TYPE "AppCountOpEnum";
