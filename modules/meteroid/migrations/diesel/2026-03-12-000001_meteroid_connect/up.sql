-- Meteroid Connect: Enterprise placeholder

-- ============================================================================
ALTER TABLE organization
    ADD COLUMN is_express BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE customer
    ADD COLUMN connected_account_id UUID UNIQUE;

