DROP INDEX IF EXISTS "credit_note_custom_properties_idx";

ALTER TABLE "credit_note" DROP COLUMN IF EXISTS "custom_properties";

-- Note: the 'CREDIT_NOTE' value added to "CustomPropertyEntityTypeEnum" is intentionally left in
-- place; PostgreSQL cannot drop a single enum value without recreating the type.
