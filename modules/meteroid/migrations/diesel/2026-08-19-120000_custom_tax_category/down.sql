DROP INDEX IF EXISTS custom_tax_category_idx;
ALTER TABLE custom_tax DROP COLUMN IF EXISTS tax_category_id;
