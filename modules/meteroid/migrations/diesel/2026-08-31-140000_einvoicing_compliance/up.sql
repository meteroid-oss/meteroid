-- Structured e-invoicing, round two: report rather than block, and the two
-- fields France asks for that EN 16931's core model does not.

-- Whether the e-invoice was produced for a document, and why not when it was
-- not. Billing must never stop because a document cannot be made compliant:
-- the invoice is still issued, and the failure is recorded against it.
CREATE TYPE "EInvoicingStatusEnum" AS ENUM ('GENERATED', 'FAILED');

ALTER TABLE invoice
    ADD COLUMN einvoicing_status "EInvoicingStatusEnum",
    -- The findings, in the standard's own vocabulary: rule, business term,
    -- message. Structured rather than a string so the UI can point at the field.
    ADD COLUMN einvoicing_report JSONB;

ALTER TABLE credit_note
    ADD COLUMN einvoicing_status "EInvoicingStatusEnum",
    ADD COLUMN einvoicing_report JSONB,
    -- Credit notes are e-invoices too (BT-3 = 381); they had nowhere to store one.
    ADD COLUMN xml_document_id UUID;

-- Non-compliant documents are what an operator goes looking for.
CREATE INDEX invoice_einvoicing_failed_idx
    ON invoice (tenant_id, invoice_date DESC)
    WHERE einvoicing_status = 'FAILED';

-- BT-47: the buyer's national register identifier (SIREN/SIRET, HRB). The
-- French mandate requires it for both parties; EN 16931's core model does not,
-- which is why it is nullable and unblocking.
ALTER TABLE customer
    ADD COLUMN legal_number TEXT;
