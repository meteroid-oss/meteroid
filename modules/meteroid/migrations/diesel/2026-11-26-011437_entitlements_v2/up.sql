-- Entitlements v2 (ADR 0011): features carry a kind, policy and level; rows an effect, level,
-- condition, dates and append-only history; resources and app counts. v1 rows are converted.
-- Rollout: while main and v2 overlap, main's batch create (ON CONFLICT) fails. Once v2 has written
-- rows main can't read them: roll back the schema together with the code, never the code alone.
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

-- EE monitors move from per-subscription to per-customer ids; state is recomputed. Alerts are
-- copied, not moved, so neither release fires a threshold again (the previous one, still up during
-- the rollout, reads the old ids). Runs first, before any table is locked.
CREATE TEMP TABLE monitor_rekey ON COMMIT DROP AS
SELECT *
FROM (SELECT pg_temp.uuid_v5(f.id, uuid_send(s.id))          AS old_id,
             pg_temp.uuid_v5(f.id, uuid_send(s.customer_id)) AS new_id,
             s.tenant_id,
             s.customer_id
      FROM subscription s
               JOIN feature f ON f.tenant_id = s.tenant_id AND f.feature_type = 'METERED'
      WHERE s.status IN ('ACTIVE', 'TRIAL_ACTIVE')
        AND (s.customer_id IN (SELECT customer_id FROM usage_monitor_state WHERE monitor_kind = 'entitlement')
          OR s.tenant_id IN (SELECT tenant_id FROM usage_alert_state))
        AND EXISTS (SELECT 1 FROM entitlement e WHERE e.feature_id = f.id)) c
WHERE old_id IN (SELECT monitor_id FROM usage_monitor_state UNION SELECT monitor_id FROM usage_alert_state);

INSERT
INTO usage_alert_state (monitor_id, window_key, level, tenant_id, fired_at, generation, rearmed, window_start,
                        window_end)
SELECT DISTINCT ON (r.new_id, a.window_key, a.level) r.new_id,
                                                     a.window_key,
                                                     a.level,
                                                     a.tenant_id,
                                                     a.fired_at,
                                                     a.generation,
                                                     a.rearmed,
                                                     a.window_start,
                                                     a.window_end
FROM usage_alert_state a
         JOIN monitor_rekey r ON a.monitor_id = r.old_id
ORDER BY r.new_id, a.window_key, a.level, a.rearmed, a.fired_at DESC
ON CONFLICT (monitor_id, window_key, level) DO NOTHING;

WITH dropped AS (
    DELETE FROM usage_monitor_state ms
        USING monitor_rekey r
        WHERE ms.monitor_id = r.old_id
        RETURNING r.tenant_id, r.customer_id)
INSERT
INTO credit_recompute_mark (tenant_id, customer_id)
SELECT DISTINCT tenant_id, customer_id
FROM dropped
ON CONFLICT (tenant_id, customer_id) DO NOTHING;

CREATE TYPE "EntitlementEffectEnum" AS ENUM ('SET', 'ADD', 'RAISE_TO', 'CAP_AT');
CREATE TYPE "EntitlementLevelEnum" AS ENUM ('FEATURE', 'CUSTOMER', 'TYPE');
CREATE TYPE "FeatureKindEnum" AS ENUM ('BOOLEAN', 'VALUE', 'LIMIT');
CREATE TYPE "FeatureValueTypeEnum" AS ENUM ('NUMBER', 'TEXT', 'SELECT', 'JSON');
CREATE TYPE "FeatureLimitCountedByEnum" AS ENUM ('EVENTS', 'APP');
CREATE TYPE "AppCountOpEnum" AS ENUM ('SET', 'ADD');

-- Resource types one level below the customer; key position 0 is canonical, the others aliases.
CREATE TABLE resource_type
(
    id             UUID PRIMARY KEY,
    tenant_id      UUID        NOT NULL,
    code           TEXT        NOT NULL,
    name           TEXT        NOT NULL,
    plural_name    TEXT        NOT NULL,
    parent_type_id UUID REFERENCES resource_type (id) CHECK (parent_type_id IS NULL),
    attributes     JSONB       NOT NULL DEFAULT '{}',
    archived_at    TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, code)
);

CREATE TABLE resource_type_key
(
    tenant_id        UUID NOT NULL,
    resource_type_id UUID NOT NULL REFERENCES resource_type (id) ON DELETE CASCADE,
    key              TEXT NOT NULL,
    position         INT  NOT NULL CHECK (position >= 0),
    PRIMARY KEY (resource_type_id, position),
    UNIQUE (tenant_id, key)
);

CREATE TABLE resource
(
    id               UUID PRIMARY KEY,
    tenant_id        UUID        NOT NULL,
    customer_id      UUID        NOT NULL,
    resource_type_id UUID        NOT NULL REFERENCES resource_type (id),
    key_value        TEXT        NOT NULL,
    name             TEXT,
    attributes       JSONB       NOT NULL DEFAULT '{}',
    archived_at      TIMESTAMPTZ,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, customer_id, resource_type_id, key_value),
    UNIQUE (customer_id, id)
);
CREATE INDEX resource_type_idx ON resource (resource_type_id);

ALTER TABLE feature
    ADD COLUMN kind                    "FeatureKindEnum",
    ADD COLUMN value_type              "FeatureValueTypeEnum",
    ADD COLUMN unit                    TEXT,
    ADD COLUMN number_higher_is_better BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN select_options          JSONB,
    ADD COLUMN select_ordered          BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN limit_counted_by        "FeatureLimitCountedByEnum",
    ADD COLUMN level_resource_type_id  UUID REFERENCES resource_type (id),
    ADD COLUMN policy                  JSONB;

ALTER TABLE entitlement
    ADD COLUMN lineage_id         UUID,
    ADD COLUMN lineage_created_at TIMESTAMPTZ,
    ADD COLUMN lineage_anchor     TIMESTAMPTZ,
    ADD COLUMN effect             "EntitlementEffectEnum",
    ADD COLUMN level              "EntitlementLevelEnum" NOT NULL DEFAULT 'FEATURE',
    ADD COLUMN level_type_id      UUID REFERENCES resource_type (id),
    ADD COLUMN resource_id        UUID REFERENCES resource (id) ON DELETE RESTRICT,
    ADD COLUMN condition          JSONB,
    ADD COLUMN condition_hash     TEXT                   NOT NULL DEFAULT '',
    ADD COLUMN valid_from         TIMESTAMPTZ,
    ADD COLUMN valid_until        TIMESTAMPTZ,
    ADD COLUMN reason             TEXT,
    ADD COLUMN created_by         UUID,
    ADD COLUMN created_by_kind    "ActorTypeEnum",
    ADD COLUMN revoked_at         TIMESTAMPTZ,
    ADD COLUMN revoked_by         UUID,
    ADD COLUMN policy_override    JSONB;

-- A config boolean is a boolean feature; a config number, text, select or json is a value.
UPDATE feature
SET kind             = CASE
                           WHEN feature_type = 'METERED' THEN 'LIMIT'
                           WHEN feature_type = 'BOOLEAN' OR config_value_type = 'BOOLEAN' THEN 'BOOLEAN'
                           ELSE 'VALUE' END::"FeatureKindEnum",
    value_type       = CASE
                           WHEN feature_type <> 'CONFIG' OR config_value_type = 'BOOLEAN' THEN NULL
                           WHEN config_value_type IN ('NUMBER', 'SELECT') THEN config_value_type::text
                           WHEN config_value_type IN ('MAP', 'JSON') THEN 'JSON'
                           ELSE 'TEXT' END::"FeatureValueTypeEnum",
    select_options   = CASE WHEN config_value_type = 'SELECT' THEN COALESCE(to_jsonb(config_options), '[]') END,
    limit_counted_by = CASE WHEN feature_type = 'METERED' THEN 'EVENTS' END::"FeatureLimitCountedByEnum";

-- Add-on rows become Add (multi-instance) or Raise to, folded after the plan's rows: v1 let a plan
-- version's row replace them, and a config number's last row win. Unordered values (text, select,
-- json) only take Set to; a config boolean is ordered.
CREATE OR REPLACE FUNCTION pg_temp.legacy_effect(entity_type "EntitlementEntityTypeEnum",
                                                 mode "EntitlementModeEnum",
                                                 value JSONB) RETURNS "EntitlementEffectEnum"
    LANGUAGE sql IMMUTABLE AS
$$
SELECT CASE
           WHEN entity_type <> 'ADD_ON' THEN 'SET'
           WHEN value ->> 'type' = 'config'
               AND COALESCE(value -> 'value' ->> 'kind', '') NOT IN ('number', 'boolean') THEN 'SET'
           WHEN mode = 'STACK' THEN 'ADD'
           ELSE 'RAISE_TO' END::"EntitlementEffectEnum"
$$;

UPDATE entitlement
SET lineage_id         = id,
    lineage_created_at = created_at,
    lineage_anchor     = created_at,
    effect             = pg_temp.legacy_effect(entity_type, mode, value);

-- Every whole-number term a policy reads is one, or the conversion stops on the row.
DO
$$
    DECLARE
        bad RECORD;
    BEGIN
        SELECT e.id INTO bad
        FROM entitlement e
        WHERE e.value ->> 'type' = 'metered'
          AND (jsonb_typeof(COALESCE(e.value -> 'thresholds', 'null')) NOT IN ('array', 'null')
            OR EXISTS (SELECT 1
                       FROM jsonb_array_elements_text(CASE jsonb_typeof(e.value -> 'thresholds')
                                                          WHEN 'array' THEN e.value -> 'thresholds'
                                                          ELSE '[]' END) t
                       WHERE t !~ '^[0-9]{1,9}$')
            OR e.value ->> 'warning_threshold_pct' !~ '^[0-9]{1,9}$'
            OR e.value -> 'overage_behavior' ->> 'grace_period_pct' !~ '^[0-9]{1,9}$'
            OR e.value -> 'reset_period' ->> 'interval' !~ '^[0-9]{1,9}$')
        LIMIT 1;
        IF FOUND THEN
            RAISE EXCEPTION 'entitlement % has a threshold, grace or interval that is not a whole number: fix its value, then migrate', bad.id;
        END IF;
    END
$$;

-- A metered value's limit terms as a feature policy; its window keeps its unit and interval.
CREATE OR REPLACE FUNCTION pg_temp.legacy_policy(value JSONB) RETURNS JSONB
    LANGUAGE sql IMMUTABLE AS
$$
WITH r AS (SELECT value -> 'reset_period' AS p)
SELECT jsonb_strip_nulls(jsonb_build_object(
        'resets', CASE p ->> 'type'
                      WHEN 'billing_cycle' THEN 'BILLING_CYCLE'
                      WHEN 'calendar' THEN 'CALENDAR'
                      WHEN 'fixed_window' THEN 'FIXED'
                      WHEN 'sliding_window' THEN 'SLIDING'
                      ELSE 'NEVER' END,
        'period', CASE
                      WHEN p ->> 'type' IN ('calendar', 'fixed_window', 'sliding_window')
                          THEN jsonb_build_object('unit', COALESCE(p ->> 'unit', 'day'), 'interval',
                                                  COALESCE((p ->> 'interval')::int, 1)) END,
        'at_limit', CASE WHEN value -> 'overage_behavior' ->> 'type' = 'allow' THEN 'ALLOW_AND_BILL' ELSE 'DENY' END,
        'grace_pct', CASE
                         WHEN value -> 'overage_behavior' ->> 'type' = 'allow' THEN 0
                         ELSE COALESCE((value -> 'overage_behavior' ->> 'grace_period_pct')::int, 0) END,
        'thresholds', (SELECT COALESCE(jsonb_agg(t ORDER BY t), '[]')
                       FROM (SELECT t::int AS t
                             FROM jsonb_array_elements_text(CASE jsonb_typeof(value -> 'thresholds')
                                                                WHEN 'array' THEN value -> 'thresholds'
                                                                ELSE '[]' END) t
                             UNION
                             SELECT (value ->> 'warning_threshold_pct')::int
                             WHERE value ->> 'warning_threshold_pct' IS NOT NULL
                             ORDER BY 1
                             LIMIT 10) ts)))
FROM r
$$;

-- Each limit takes the most common policy among its rows, limited rows first (an unlimited row's
-- terms act on nothing); a limit without rows gets the default.
UPDATE feature f
SET policy = v.policy
FROM (SELECT DISTINCT ON (feature_id) feature_id, policy
      FROM (SELECT e.feature_id,
                   pg_temp.legacy_policy(e.value)                              AS policy,
                   count(*) FILTER (WHERE e.value ->> 'limit' IS NOT NULL)      AS limited,
                   count(*)                                                     AS n,
                   bool_or(e.entity_type = 'FEATURE')                           AS is_default,
                   min(e.created_at)                                            AS first_at
            FROM entitlement e
            WHERE e.value ->> 'type' = 'metered'
            GROUP BY 1, 2) votes
      ORDER BY feature_id, limited DESC, n DESC, is_default DESC, first_at) v
WHERE f.id = v.feature_id;
UPDATE feature
SET policy = '{"resets": "BILLING_CYCLE", "at_limit": "DENY", "grace_pct": 0, "thresholds": []}'
WHERE kind = 'LIMIT'
  AND policy IS NULL;

-- A limited Set to row whose policy differs keeps it as an override; other rows lose it.
UPDATE entitlement e
SET policy_override = pg_temp.legacy_policy(e.value)
FROM feature f
WHERE f.id = e.feature_id
  AND e.value ->> 'type' = 'metered'
  AND e.value ->> 'limit' IS NOT NULL
  AND e.effect = 'SET'
  AND pg_temp.legacy_policy(e.value) <> f.policy;

ALTER TABLE feature
    ALTER COLUMN kind SET NOT NULL;

ALTER TABLE entitlement
    ALTER COLUMN lineage_id SET NOT NULL,
    ALTER COLUMN lineage_created_at SET NOT NULL,
    ALTER COLUMN lineage_anchor SET NOT NULL,
    ALTER COLUMN effect SET NOT NULL,
    DROP CONSTRAINT entitlement_feature_id_entity_id_entity_type_key,
    ADD CONSTRAINT entitlement_level_type_check CHECK ((level = 'TYPE') = (level_type_id IS NOT NULL)),
    ADD CONSTRAINT entitlement_policy_override_check CHECK (policy_override IS NULL OR effect = 'SET'),
    ADD CONSTRAINT entitlement_condition_hash_check CHECK ((condition IS NULL) = (condition_hash = '')),
    ADD CONSTRAINT entitlement_validity_check CHECK (valid_from IS NULL OR valid_until IS NULL OR valid_from < valid_until);

CREATE UNIQUE INDEX entitlement_open_lineage_key ON entitlement (lineage_id) WHERE revoked_at IS NULL;
-- Open rows of one key may not overlap in validity: checked by the writers under a key lock.
CREATE INDEX entitlement_open_key_idx ON entitlement (feature_id, entity_type, entity_id) WHERE revoked_at IS NULL;
CREATE INDEX entitlement_lineage_idx ON entitlement (lineage_id, created_at);
CREATE INDEX entitlement_valid_from_idx ON entitlement (valid_from) WHERE valid_from IS NOT NULL AND revoked_at IS NULL;
CREATE INDEX entitlement_valid_until_idx ON entitlement (valid_until) WHERE valid_until IS NOT NULL AND revoked_at IS NULL;
CREATE INDEX entitlement_resource_idx ON entitlement (resource_id) WHERE resource_id IS NOT NULL;

CREATE TABLE entitlement_schedule_cursor
(
    id            BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
    checked_until TIMESTAMPTZ NOT NULL
);
INSERT INTO entitlement_schedule_cursor (checked_until)
VALUES (now());

-- App-counted limits (§3.8). `path`: '' for the customer, else '<type code>=<key value>'.
CREATE TABLE entitlement_app_count
(
    tenant_id      UUID        NOT NULL,
    customer_id    UUID        NOT NULL,
    feature_id     UUID        NOT NULL REFERENCES feature (id) ON DELETE CASCADE,
    path           TEXT        NOT NULL,
    resource_id    UUID REFERENCES resource (id) ON DELETE SET NULL,
    count          NUMERIC     NOT NULL DEFAULT 0,
    last_set_value NUMERIC,
    last_set_at    TIMESTAMPTZ,
    last_set_key   TEXT,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, customer_id, feature_id, path)
);

CREATE TABLE entitlement_app_count_op
(
    tenant_id       UUID             NOT NULL,
    customer_id     UUID             NOT NULL,
    feature_id      UUID             NOT NULL REFERENCES feature (id) ON DELETE CASCADE,
    idempotency_key TEXT             NOT NULL,
    path            TEXT             NOT NULL,
    op              "AppCountOpEnum" NOT NULL,
    amount          NUMERIC          NOT NULL,
    at              TIMESTAMPTZ      NOT NULL,
    result_count    NUMERIC          NOT NULL,
    request_hash    TEXT             NOT NULL,
    created_at      TIMESTAMPTZ      NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, customer_id, feature_id, idempotency_key)
);
CREATE INDEX entitlement_app_count_op_path_idx ON entitlement_app_count_op (tenant_id, customer_id, feature_id, path, at);
CREATE INDEX entitlement_app_count_op_created_idx ON entitlement_app_count_op (created_at);

-- Last, so tenant, portal_token and customer are locked only from here to the commit (each waited
-- for up to lock_timeout). feature and entitlement stay locked from their ALTERs to the commit,
-- through the backfill, the NOT NULL scans and the index builds above (ADR 0011 §7).
ALTER TABLE tenant
    ADD COLUMN max_resources_per_customer_type  INT NOT NULL DEFAULT 10000,
    ADD COLUMN entitlement_op_retention_days    INT NOT NULL DEFAULT 7,
    ADD COLUMN entitlement_op_log_complete_from TIMESTAMPTZ;

ALTER TABLE portal_token
    ADD COLUMN client_claims JSONB;

ALTER TABLE resource_type
    ADD CONSTRAINT resource_type_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant (id) ON DELETE CASCADE;
ALTER TABLE resource_type_key
    ADD CONSTRAINT resource_type_key_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant (id) ON DELETE CASCADE;
ALTER TABLE resource
    ADD CONSTRAINT resource_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant (id) ON DELETE CASCADE;
ALTER TABLE entitlement_app_count
    ADD CONSTRAINT entitlement_app_count_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant (id) ON DELETE CASCADE;
ALTER TABLE entitlement_app_count_op
    ADD CONSTRAINT entitlement_app_count_op_tenant_id_fkey FOREIGN KEY (tenant_id) REFERENCES tenant (id) ON DELETE CASCADE;
ALTER TABLE resource
    ADD CONSTRAINT resource_customer_id_fkey FOREIGN KEY (customer_id) REFERENCES customer (id) ON DELETE CASCADE;
ALTER TABLE entitlement_app_count
    ADD CONSTRAINT entitlement_app_count_customer_id_fkey FOREIGN KEY (customer_id) REFERENCES customer (id) ON DELETE CASCADE;
ALTER TABLE entitlement_app_count_op
    ADD CONSTRAINT entitlement_app_count_op_customer_id_fkey FOREIGN KEY (customer_id) REFERENCES customer (id) ON DELETE CASCADE;
