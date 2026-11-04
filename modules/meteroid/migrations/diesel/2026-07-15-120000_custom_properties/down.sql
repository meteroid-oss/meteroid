DROP INDEX IF EXISTS "invoice_custom_properties_idx";
DROP INDEX IF EXISTS "subscription_custom_properties_idx";
DROP INDEX IF EXISTS "customer_custom_properties_idx";

ALTER TABLE "invoice"      DROP COLUMN IF EXISTS "custom_properties";
ALTER TABLE "subscription" DROP COLUMN IF EXISTS "custom_properties";
ALTER TABLE "customer"     DROP COLUMN IF EXISTS "custom_properties";

DROP TABLE IF EXISTS "custom_property_definition";

DROP TYPE IF EXISTS "CustomPropertyTypeEnum";
DROP TYPE IF EXISTS "CustomPropertyEntityTypeEnum";
