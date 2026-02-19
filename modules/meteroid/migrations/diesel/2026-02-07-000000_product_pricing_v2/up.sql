-- 1. Product pricing infrastructure
CREATE TYPE "FeeTypeEnum" AS ENUM ('RATE','SLOT','CAPACITY','USAGE','EXTRA_RECURRING','ONE_TIME');
ALTER TABLE product ADD COLUMN fee_type "FeeTypeEnum";
ALTER TABLE product ADD COLUMN fee_structure JSONB;

CREATE TABLE price (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    product_id UUID NOT NULL REFERENCES product(id),
    cadence "BillingPeriodEnum" NOT NULL,
    currency TEXT NOT NULL,
    pricing JSONB NOT NULL,
    tenant_id UUID NOT NULL REFERENCES tenant(id),
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    created_by UUID NOT NULL,
    archived_at TIMESTAMP
);
CREATE INDEX idx_price_product_currency ON price(product_id, currency);
CREATE INDEX idx_price_tenant ON price(tenant_id);

-- 2. Junction table for plan components (multi-price)
CREATE TABLE plan_component_price (
    plan_component_id UUID NOT NULL REFERENCES price_component(id) ON DELETE CASCADE,
    price_id UUID NOT NULL REFERENCES price(id),
    PRIMARY KEY (plan_component_id, price_id)
);

-- 3. Plan version flag
ALTER TABLE plan_version ADD COLUMN uses_product_pricing BOOLEAN NOT NULL DEFAULT false;

-- 4. Rename fee → legacy_fee (make nullable) on 5 tables (not add_on — see below)
ALTER TABLE price_component RENAME COLUMN fee TO legacy_fee;
ALTER TABLE price_component ALTER COLUMN legacy_fee DROP NOT NULL;

ALTER TABLE subscription_component RENAME COLUMN fee TO legacy_fee;
ALTER TABLE subscription_component ALTER COLUMN legacy_fee DROP NOT NULL;

ALTER TABLE subscription_add_on RENAME COLUMN fee TO legacy_fee;
ALTER TABLE subscription_add_on ALTER COLUMN legacy_fee DROP NOT NULL;

ALTER TABLE quote_component RENAME COLUMN fee TO legacy_fee;
ALTER TABLE quote_component ALTER COLUMN legacy_fee DROP NOT NULL;

ALTER TABLE quote_add_on RENAME COLUMN fee TO legacy_fee;
ALTER TABLE quote_add_on ALTER COLUMN legacy_fee DROP NOT NULL;

-- 5. Restructure add_on: drop fee entirely (no existing data), add direct FKs
ALTER TABLE add_on DROP COLUMN fee;
ALTER TABLE add_on ADD COLUMN plan_version_id UUID REFERENCES plan_version(id);
ALTER TABLE add_on ADD COLUMN product_id UUID REFERENCES product(id) ON DELETE SET NULL;
ALTER TABLE add_on ADD COLUMN price_id UUID REFERENCES price(id);

-- 6. Add price_id where missing
-- subscription_component: already has product_id (nullable), add price_id
ALTER TABLE subscription_component ADD COLUMN price_id UUID REFERENCES price(id);

-- subscription_add_on: add product_id + price_id
ALTER TABLE subscription_add_on ADD COLUMN product_id UUID REFERENCES product(id);
ALTER TABLE subscription_add_on ADD COLUMN price_id UUID REFERENCES price(id);

-- quote_component: already has product_id, add price_id
ALTER TABLE quote_component ADD COLUMN price_id UUID REFERENCES price(id);

-- quote_add_on: add product_id + price_id
ALTER TABLE quote_add_on ADD COLUMN product_id UUID REFERENCES product(id);
ALTER TABLE quote_add_on ADD COLUMN price_id UUID REFERENCES price(id);

-- Delete products without pricing (no valid use without fee_type/fee_structure)
DELETE FROM product WHERE fee_type IS NULL OR fee_structure IS NULL;
ALTER TABLE product ALTER COLUMN fee_type SET NOT NULL;
ALTER TABLE product ALTER COLUMN fee_structure SET NOT NULL;

-- FK indexes for new columns
CREATE INDEX idx_add_on_plan_version ON add_on(plan_version_id);
CREATE INDEX idx_add_on_product ON add_on(product_id);
CREATE INDEX idx_add_on_price ON add_on(price_id);
CREATE INDEX idx_subscription_component_price ON subscription_component(price_id);
CREATE INDEX idx_subscription_add_on_product ON subscription_add_on(product_id);
CREATE INDEX idx_subscription_add_on_price ON subscription_add_on(price_id);
CREATE INDEX idx_quote_component_price ON quote_component(price_id);
CREATE INDEX idx_quote_add_on_product ON quote_add_on(product_id);
CREATE INDEX idx_quote_add_on_price ON quote_add_on(price_id);
