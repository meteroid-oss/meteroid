ALTER TABLE customer DROP COLUMN IF EXISTS legal_number;

DROP INDEX IF EXISTS invoice_einvoicing_failed_idx;

ALTER TABLE credit_note
    DROP COLUMN IF EXISTS einvoicing_status,
    DROP COLUMN IF EXISTS einvoicing_report,
    DROP COLUMN IF EXISTS xml_document_id;

ALTER TABLE invoice
    DROP COLUMN IF EXISTS einvoicing_status,
    DROP COLUMN IF EXISTS einvoicing_report;

DROP TYPE IF EXISTS "EInvoicingStatusEnum";
