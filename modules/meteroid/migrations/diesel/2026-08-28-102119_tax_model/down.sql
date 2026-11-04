-- Reverse of up.sql. Postgres cannot drop enum values once added, so the
-- 'TAX' (ConnectorTypeEnum) and 'EXTERNAL' (TaxResolverEnum) values are
-- intentionally left in place.

DROP INDEX IF EXISTS custom_tax_tenant_tax_code_key;

ALTER TABLE custom_tax
  DROP CONSTRAINT IF EXISTS custom_tax_tenant_id_fkey;

ALTER TABLE custom_tax
  DROP COLUMN IF EXISTS tenant_id;

ALTER TABLE customer
  ADD COLUMN is_tax_exempt BOOLEAN NOT NULL DEFAULT false;

UPDATE customer SET is_tax_exempt = true WHERE tax_status = 'EXEMPT';

ALTER TABLE customer
  DROP COLUMN tax_status,
  DROP COLUMN exemption_reason;

DROP TYPE "CustomerTaxStatusEnum";

DROP INDEX IF EXISTS custom_tax_category_idx;
ALTER TABLE custom_tax DROP COLUMN IF EXISTS tax_category_id;

ALTER TABLE invoicing_entity DROP COLUMN IF EXISTS tax_provider_id;
ALTER TABLE invoicing_entity DROP COLUMN IF EXISTS default_tax_category_id;

ALTER TABLE product DROP COLUMN IF EXISTS tax_category_id;

DROP TABLE IF EXISTS tax_category;
