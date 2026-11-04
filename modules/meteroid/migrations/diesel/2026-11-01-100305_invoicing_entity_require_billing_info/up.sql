-- Ported from the public repository (its 2026-07-08-000000_invoicing_entity_require_billing_info).
-- IF NOT EXISTS: databases migrated by the public repository already have the column.
ALTER TABLE invoicing_entity
    ADD COLUMN IF NOT EXISTS require_billing_information BOOLEAN NOT NULL DEFAULT FALSE;
