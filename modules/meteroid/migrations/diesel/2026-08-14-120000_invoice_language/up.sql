-- Preferred language for rendered invoices / credit notes.
-- Resolution order at render time: customer override -> invoicing entity default
-- -> invoicing entity country locale -> en-US. NULL means "inherit".
ALTER TABLE invoicing_entity
    ADD COLUMN invoicing_language TEXT;

ALTER TABLE customer
    ADD COLUMN invoicing_language TEXT;
