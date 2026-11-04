DROP INDEX IF EXISTS "quote_custom_properties_idx";

ALTER TABLE "quote" DROP COLUMN IF EXISTS "custom_properties";

-- Note: the 'QUOTE' value added to "CustomPropertyEntityTypeEnum" is intentionally left in place;
-- PostgreSQL cannot drop a single enum value without recreating the type.
