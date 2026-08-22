-- Tax categories: a provider-agnostic classification of what is sold.
-- Built-in rows are global (tenant_id NULL); tenant-custom categories (future) carry a tenant_id.
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

-- Seed a minimal built-in catalog (global, flat; hierarchy is supported via parent_id for later).
INSERT INTO tax_category (id, tenant_id, parent_id, key, name, is_builtin) VALUES
    ('a0000000-0000-4000-8000-000000000001', NULL, NULL, 'standard',              'Standard — fully taxable', true),
    ('a0000000-0000-4000-8000-000000000002', NULL, NULL, 'saas',                  'Software as a Service',    true),
    ('a0000000-0000-4000-8000-000000000003', NULL, NULL, 'downloadable_software', 'Downloadable software',    true),
    ('a0000000-0000-4000-8000-000000000004', NULL, NULL, 'digital_services',      'Digital services',         true),
    ('a0000000-0000-4000-8000-000000000005', NULL, NULL, 'ebook',                 'E-book',                   true),
    ('a0000000-0000-4000-8000-000000000006', NULL, NULL, 'professional_services', 'Professional services',    true),
    ('a0000000-0000-4000-8000-000000000007', NULL, NULL, 'physical_goods',        'Physical goods',           true),
    ('a0000000-0000-4000-8000-000000000008', NULL, NULL, 'nontaxable',            'Non-taxable / exempt',     true);
