DROP TABLE product_accounting;

ALTER TABLE customer
  ADD COLUMN custom_vat_rate integer,
  DROP COLUMN vat_number_format_valid,
  DROP COLUMN is_tax_exempt,
  DROP COLUMN custom_tax_rate;

ALTER TABLE invoice
  DROP COLUMN tax_breakdown;
ALTER TABLE invoice
  ADD COLUMN tax_rate INTEGER NOT NULL DEFAULT 0;

DROP TABLE custom_tax;

ALTER TABLE invoicing_entity
  DROP COLUMN tax_resolver;

DROP TYPE "TaxResolverEnum";

ALTER TABLE invoice
  DROP COLUMN IF EXISTS coupons;
ALTER TABLE invoice
  ADD COLUMN coupons JSONB NOT NULL DEFAULT '[]';
