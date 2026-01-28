-- Revert to old constraint (per tenant)
DROP INDEX IF EXISTS credit_note_number_key;

CREATE UNIQUE INDEX credit_note_number_key
    ON credit_note(credit_note_number, tenant_id)
    WHERE (status != 'DRAFT'::"CreditNoteStatus");
