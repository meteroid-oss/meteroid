-- 1. Create junction table
CREATE TABLE plan_version_add_on (
    id UUID PRIMARY KEY,
    plan_version_id UUID NOT NULL REFERENCES plan_version(id),
    add_on_id UUID NOT NULL REFERENCES add_on(id) ON DELETE CASCADE,
    price_id UUID REFERENCES price(id),
    self_serviceable BOOLEAN,
    max_instances_per_subscription INTEGER,
    tenant_id UUID NOT NULL REFERENCES tenant(id),
    created_at TIMESTAMP NOT NULL DEFAULT now(),
    UNIQUE(plan_version_id, add_on_id)
);

-- 2. Migrate existing data (1 junction row per existing add_on)
INSERT INTO plan_version_add_on (id, plan_version_id, add_on_id, tenant_id)
SELECT gen_random_uuid(), plan_version_id, id, tenant_id
FROM add_on WHERE plan_version_id IS NOT NULL;

-- 3. Add new columns to add_on
ALTER TABLE add_on ADD COLUMN description TEXT;
ALTER TABLE add_on ADD COLUMN self_serviceable BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE add_on ADD COLUMN max_instances_per_subscription INTEGER;

-- 4. Remove plan_version_id (now in junction)
ALTER TABLE add_on DROP COLUMN plan_version_id;

-- 5. Enforce product/price NOT NULL
DELETE FROM add_on WHERE product_id IS NULL OR price_id IS NULL;
ALTER TABLE add_on ALTER COLUMN product_id SET NOT NULL;
ALTER TABLE add_on ALTER COLUMN price_id SET NOT NULL;

-- 6. Add quantity to subscription/quote add-ons
ALTER TABLE subscription_add_on ADD COLUMN quantity INTEGER NOT NULL DEFAULT 1;
ALTER TABLE quote_add_on ADD COLUMN quantity INTEGER NOT NULL DEFAULT 1;

-- 7. Indexes
CREATE INDEX idx_plan_version_add_on_plan_version ON plan_version_add_on(plan_version_id);
CREATE INDEX idx_plan_version_add_on_add_on ON plan_version_add_on(add_on_id);
CREATE INDEX idx_add_on_tenant ON add_on(tenant_id);
