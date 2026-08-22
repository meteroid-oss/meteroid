ALTER TABLE invoicing_entity DROP COLUMN IF EXISTS tax_provider_id;
ALTER TABLE invoicing_entity DROP COLUMN IF EXISTS default_tax_category_id;
ALTER TABLE product DROP COLUMN IF EXISTS tax_category_id;
DROP TABLE IF EXISTS tax_category;
