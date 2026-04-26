CREATE TYPE "FeatureTypeEnum" AS ENUM (
    'BOOLEAN',
    'METERED'
);

CREATE TYPE "FeatureStatusEnum" AS ENUM (
    'ACTIVE',
    'DISABLED',
    'ARCHIVED'
);

CREATE TYPE "EntitlementEntityTypeEnum" AS ENUM (
  'FEATURE',
  'PLAN_VERSION',
  'ADD_ON',
  'PLAN',
  'SUBSCRIPTION',
  'QUOTE'
  );

-- Internal composition mode. Resolved automatically server-side from the owning entity
-- (e.g. add-on with max_instances_per_subscription > 1 → STACK; otherwise → OVERRIDE).
-- Not exposed in the public API.
CREATE TYPE "EntitlementModeEnum" AS ENUM (
  'OVERRIDE',
  'STACK'
  );

CREATE TABLE feature (
    id           UUID                PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id    UUID                NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    product_id   UUID                REFERENCES product(id) ON DELETE SET NULL,
    name         TEXT                NOT NULL,
    description  TEXT,
    feature_type "FeatureTypeEnum"   NOT NULL DEFAULT 'BOOLEAN',
    status       "FeatureStatusEnum" NOT NULL DEFAULT 'ACTIVE',
    metric_id    UUID                REFERENCES billable_metric(id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ         NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_by   UUID                NOT NULL,
    updated_at   TIMESTAMPTZ         NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (tenant_id, name)
);

CREATE INDEX idx_feature_tenant ON feature(tenant_id);
CREATE INDEX idx_feature_product ON feature(product_id) WHERE product_id IS NOT NULL;

CREATE TABLE entitlement (
    id           UUID                        PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id    UUID                        NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    feature_id   UUID                        NOT NULL REFERENCES feature(id) ON DELETE CASCADE,
    entity_id    UUID                        NOT NULL,
    entity_type  "EntitlementEntityTypeEnum" NOT NULL,
    mode         "EntitlementModeEnum"       NOT NULL,
    value        JSONB                       NOT NULL,
    -- Metered-only fields live inside the `value` JSONB so the type system can refuse to
    -- set them on Boolean entitlements.
    created_at   TIMESTAMPTZ                 NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_by   UUID                        NOT NULL,
    updated_at   TIMESTAMPTZ                 NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (feature_id, entity_id, entity_type)
);

CREATE INDEX idx_entitlement_entity ON entitlement(tenant_id, entity_id, entity_type);
CREATE INDEX idx_entitlement_feature ON entitlement(tenant_id, feature_id);

-- subscription_component had no index on subscription_id; needed for product-level entitlement resolution.
CREATE INDEX idx_subscription_component_subscription_id ON subscription_component(subscription_id);
