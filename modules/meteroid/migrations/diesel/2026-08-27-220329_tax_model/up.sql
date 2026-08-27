-- Tax model rewrite (merged). Two merchant-facing levels:
--   1. Tax categories — what is sold (editable; standard-rated by default, `nontaxable` special-cased).
--   2. Custom rates / overrides — target a category, matched to the customer's destination country.
-- Rate class is engine-internal (world-tax) only; there is no vat_rate_class here.
-- Enum ADD VALUEs ('TAX', 'EXTERNAL') are not consumed later in this migration, so it stays single-transaction safe.

-- Tax categories: a provider-agnostic classification of what is sold.
-- Built-in rows are global (tenant_id NULL); tenant-custom categories carry a tenant_id.
CREATE TABLE tax_category (
    id         UUID PRIMARY KEY NOT NULL,
    tenant_id  UUID REFERENCES tenant(id) ON DELETE CASCADE,
    parent_id  UUID REFERENCES tax_category(id) ON DELETE SET NULL,
    key        TEXT NOT NULL,
    name       TEXT NOT NULL,
    is_builtin BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMP NOT NULL DEFAULT now()
);

-- key is unique within a tenant; built-in (global) rows share the zero-uuid bucket.
CREATE UNIQUE INDEX tax_category_tenant_key_uq
    ON tax_category (COALESCE(tenant_id, '00000000-0000-0000-0000-000000000000'::uuid), key);

-- A product's tax nature (provider-agnostic). NULL resolves to the invoicing entity default.
ALTER TABLE product
    ADD COLUMN tax_category_id UUID REFERENCES tax_category(id) ON DELETE SET NULL;

-- Best-effort default category per invoicing entity (fallback for lines with no categorised product),
-- and the external tax provider for that entity (a Tax-typed connector; NULL = use the built-in resolver).
ALTER TABLE invoicing_entity
    ADD COLUMN default_tax_category_id UUID REFERENCES tax_category(id) ON DELETE SET NULL,
    ADD COLUMN tax_provider_id UUID REFERENCES connector(id) ON DELETE SET NULL;

-- Seed the lean built-in catalog (global, flat; hierarchy is supported via parent_id for later).
-- Only distinctions the built-in EU engine treats differently: SaaS / software / digital services
-- collapse to "Digital services"; physical goods; and the special-cased non-taxable bucket.
INSERT INTO tax_category (id, tenant_id, parent_id, key, name, is_builtin) VALUES
    ('a0000000-0000-4000-8000-000000000004', NULL, NULL, 'digital_services', 'Digital services',        true),
    ('a0000000-0000-4000-8000-000000000007', NULL, NULL, 'physical_goods',   'Physical goods',          true),
    ('a0000000-0000-4000-8000-000000000008', NULL, NULL, 'nontaxable',       'Non-taxable / exempt',    true);

-- Add the Tax connector type so a tax provider can be configured per invoicing
-- entity like any other connector (encrypted credentials, nexus, ...). External
-- provider engine impls are added separately (see the add-tax-provider skill).
ALTER TYPE "ConnectorTypeEnum" ADD VALUE IF NOT EXISTS 'TAX';

-- A custom tax may target a tax category instead of being wired product by product:
-- it then applies to every line whose resolved category matches. NULL keeps the
-- existing behaviour (applies only to products explicitly linked via product_custom_tax).
ALTER TABLE custom_tax
    ADD COLUMN tax_category_id UUID REFERENCES tax_category(id) ON DELETE SET NULL;

CREATE INDEX custom_tax_category_idx
    ON custom_tax (invoicing_entity_id, tax_category_id)
    WHERE tax_category_id IS NOT NULL;

-- Party tax status (W6): replace the exemption bool with a tri-state status.
-- TAXABLE: normal taxation. EXEMPT: no tax (charity, treaty exemption, ...).
-- REVERSE_CHARGE: buyer accounts for the tax (additive to the VIES-derived
-- reverse charge the built-in engine already computes).
CREATE TYPE "CustomerTaxStatusEnum" AS ENUM ('TAXABLE', 'EXEMPT', 'REVERSE_CHARGE');

ALTER TABLE customer
  ADD COLUMN tax_status "CustomerTaxStatusEnum" NOT NULL DEFAULT 'TAXABLE',
  -- Free-text legal mention required on EU exempt/reverse-charge invoices.
  ADD COLUMN exemption_reason TEXT;

-- Preserve current behavior: existing tax-exempt customers map to EXEMPT.
UPDATE customer SET tax_status = 'EXEMPT' WHERE is_tax_exempt = true;

ALTER TABLE customer DROP COLUMN is_tax_exempt;

-- Delegation collapses onto one selector (C3): add the External resolver so an
-- external tax provider is selectable. The provider engine itself is added later
-- (see the add-tax-provider skill); External routes to the "no provider" seam.
-- The tax_provider_id-only-under-External invariant is owned by Rust (patch_invoicing_entity).
ALTER TYPE "TaxResolverEnum" ADD VALUE IF NOT EXISTS 'EXTERNAL';

-- W1: the tax rate's `tax_code` is the accounting/reporting code carried onto
-- the invoice tax breakdown. Denormalize tenant_id so the code can be enforced
-- unique per tenant at the database level (multi-instance safe).
ALTER TABLE custom_tax
  ADD COLUMN tenant_id UUID;

UPDATE custom_tax ct
  SET tenant_id = ie.tenant_id
  FROM invoicing_entity ie
  WHERE ie.id = ct.invoicing_entity_id;

ALTER TABLE custom_tax
  ALTER COLUMN tenant_id SET NOT NULL,
  ADD CONSTRAINT custom_tax_tenant_id_fkey
    FOREIGN KEY (tenant_id) REFERENCES tenant (id) ON DELETE CASCADE;

CREATE UNIQUE INDEX custom_tax_tenant_tax_code_key
  ON custom_tax (tenant_id, tax_code);
