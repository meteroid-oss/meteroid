-- Reverse: drop indexes
DROP INDEX IF EXISTS idx_quote_add_on_price;
DROP INDEX IF EXISTS idx_quote_add_on_product;
DROP INDEX IF EXISTS idx_quote_component_price;
DROP INDEX IF EXISTS idx_subscription_add_on_price;
DROP INDEX IF EXISTS idx_subscription_add_on_product;
DROP INDEX IF EXISTS idx_subscription_component_price;
DROP INDEX IF EXISTS idx_add_on_price;
DROP INDEX IF EXISTS idx_add_on_product;
DROP INDEX IF EXISTS idx_add_on_plan_version;

-- Reverse: make fee_type/fee_structure nullable again
ALTER TABLE product ALTER COLUMN fee_structure DROP NOT NULL;
ALTER TABLE product ALTER COLUMN fee_type DROP NOT NULL;

-- Reverse: drop new columns on quote_add_on
ALTER TABLE quote_add_on DROP COLUMN IF EXISTS price_id;
ALTER TABLE quote_add_on DROP COLUMN IF EXISTS product_id;

-- Reverse: drop new columns on quote_component
ALTER TABLE quote_component DROP COLUMN IF EXISTS price_id;

-- Reverse: drop new columns on subscription_add_on
ALTER TABLE subscription_add_on DROP COLUMN IF EXISTS price_id;
ALTER TABLE subscription_add_on DROP COLUMN IF EXISTS product_id;

-- Reverse: drop new column on subscription_component
ALTER TABLE subscription_component DROP COLUMN IF EXISTS price_id;

-- Reverse: restore add_on fee column, drop new columns
ALTER TABLE add_on DROP COLUMN IF EXISTS price_id;
ALTER TABLE add_on DROP COLUMN IF EXISTS product_id;
ALTER TABLE add_on DROP COLUMN IF EXISTS plan_version_id;
ALTER TABLE add_on ADD COLUMN fee JSONB NOT NULL DEFAULT '{}'::jsonb;

-- Reverse: backfill NULLs, rename legacy_fee → fee, restore NOT NULL on 5 tables
UPDATE quote_add_on SET legacy_fee = '{}'::jsonb WHERE legacy_fee IS NULL;
ALTER TABLE quote_add_on RENAME COLUMN legacy_fee TO fee;
ALTER TABLE quote_add_on ALTER COLUMN fee SET NOT NULL;

UPDATE quote_component SET legacy_fee = '{}'::jsonb WHERE legacy_fee IS NULL;
ALTER TABLE quote_component RENAME COLUMN legacy_fee TO fee;
ALTER TABLE quote_component ALTER COLUMN fee SET NOT NULL;

UPDATE subscription_add_on SET legacy_fee = '{}'::jsonb WHERE legacy_fee IS NULL;
ALTER TABLE subscription_add_on RENAME COLUMN legacy_fee TO fee;
ALTER TABLE subscription_add_on ALTER COLUMN fee SET NOT NULL;

UPDATE subscription_component SET legacy_fee = '{}'::jsonb WHERE legacy_fee IS NULL;
ALTER TABLE subscription_component RENAME COLUMN legacy_fee TO fee;
ALTER TABLE subscription_component ALTER COLUMN fee SET NOT NULL;

UPDATE price_component SET legacy_fee = '{}'::jsonb WHERE legacy_fee IS NULL;
ALTER TABLE price_component RENAME COLUMN legacy_fee TO fee;
ALTER TABLE price_component ALTER COLUMN fee SET NOT NULL;

-- Reverse: plan version flag
ALTER TABLE plan_version DROP COLUMN IF EXISTS uses_product_pricing;

-- Reverse: junction table
DROP TABLE IF EXISTS plan_component_price;

-- Reverse: price table
DROP TABLE IF EXISTS price;

-- Reverse: product columns
ALTER TABLE product DROP COLUMN IF EXISTS fee_structure;
ALTER TABLE product DROP COLUMN IF EXISTS fee_type;

-- Reverse: enum
DROP TYPE IF EXISTS "FeeTypeEnum";
