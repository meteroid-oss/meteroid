-- Remove indexes
DROP INDEX IF EXISTS idx_plan_version_add_on_add_on;
DROP INDEX IF EXISTS idx_plan_version_add_on_plan_version;

-- Remove quantity from subscription/quote add-ons
ALTER TABLE quote_add_on DROP COLUMN quantity;
ALTER TABLE subscription_add_on DROP COLUMN quantity;

-- Make product_id and price_id nullable again
ALTER TABLE add_on ALTER COLUMN product_id DROP NOT NULL;
ALTER TABLE add_on ALTER COLUMN price_id DROP NOT NULL;

-- Re-add plan_version_id column
ALTER TABLE add_on ADD COLUMN plan_version_id UUID REFERENCES plan_version(id);

-- Restore plan_version_id from junction table
UPDATE add_on SET plan_version_id = pva.plan_version_id
FROM plan_version_add_on pva WHERE pva.add_on_id = add_on.id;

-- Remove new columns from add_on
ALTER TABLE add_on DROP COLUMN archived_at;
ALTER TABLE add_on DROP COLUMN max_instances_per_subscription;
ALTER TABLE add_on DROP COLUMN self_serviceable;
ALTER TABLE add_on DROP COLUMN description;

-- Drop junction table
DROP TABLE plan_version_add_on;
