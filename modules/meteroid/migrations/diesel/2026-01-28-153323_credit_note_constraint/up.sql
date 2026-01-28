-- Fix credit note unique constraint to be per invoicing entity (not per tenant)
-- This matches how invoice numbers work and how credit note numbers are generated

DROP INDEX IF EXISTS credit_note_number_key;

CREATE UNIQUE INDEX credit_note_number_key
    ON credit_note(credit_note_number, invoicing_entity_id)
    WHERE (status != 'DRAFT'::"CreditNoteStatus");
