ALTER TABLE invoicing_entity
    ADD COLUMN require_billing_information BOOLEAN NOT NULL DEFAULT FALSE;
