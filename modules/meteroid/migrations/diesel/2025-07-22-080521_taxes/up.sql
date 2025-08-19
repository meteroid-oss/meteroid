

CREATE TABLE custom_tax (
           id UUID PRIMARY KEY NOT NULL,
           invoicing_entity_id UUID NOT NULL REFERENCES invoicing_entity(id) ON DELETE CASCADE,
           name TEXT NOT NULL,
           tax_code TEXT NOT NULL,
           rules JSONB NOT NULL DEFAULT '{}' --country, (state,) rate
);

ALTER TABLE invoice
  DROP COLUMN tax_rate, -- tax rate is on line items
  ADD COLUMN tax_breakdown JSONB NOT NULL DEFAULT '{}';

ALTER TABLE customer
  ADD COLUMN is_tax_exempt BOOLEAN NOT NULL DEFAULT false,
--   ADD COLUMN tax_exemption_reason TEXT,
  ADD COLUMN custom_tax_rate NUMERIC(7,4),
  ADD COLUMN vat_number_format_valid BOOLEAN NOT NULL DEFAULT false,
  DROP COLUMN custom_vat_rate;

-- for each invoicing entity you can have a custom tax rate & code for a product (different country & different accounting software integration)
CREATE TABLE product_accounting (
  product_id UUID NOT NULL REFERENCES product(id) ON DELETE CASCADE,
  invoicing_entity_id UUID REFERENCES invoicing_entity(id) ON DELETE CASCADE,
  custom_tax_id UUID REFERENCES custom_tax(id) ON DELETE SET NULL,
  product_code TEXT,
  ledger_account_code TEXT,
  PRIMARY KEY (product_id, invoicing_entity_id)
);


CREATE TYPE "TaxResolverEnum" AS ENUM ('NONE', 'MANUAL', 'METEROID_EU_VAT');

ALTER TABLE invoicing_entity
  ADD COLUMN tax_resolver "TaxResolverEnum" NOT NULL DEFAULT 'NONE'
;

ALTER TABLE invoice
  DROP COLUMN IF EXISTS coupons;
ALTER TABLE invoice
  ADD COLUMN coupons JSONB NOT NULL DEFAULT '[]';
