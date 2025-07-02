ALTER TABLE invoice
  ALTER COLUMN pdf_document_id TYPE text USING pdf_document_id::text,
  ALTER COLUMN xml_document_id TYPE text USING xml_document_id::text
;

ALTER TABLE invoicing_entity
  ALTER COLUMN logo_attachment_id TYPE text USING logo_attachment_id::text;

create type "InvoiceExternalStatusEnum" as enum ('DELETED', 'DRAFT', 'FINALIZED', 'PAID', 'PAYMENT_FAILED', 'UNCOLLECTIBLE', 'VOID');

ALTER TABLE invoice
  ADD COLUMN external_invoice_id TEXT,
  ADD COLUMN external_status "InvoiceExternalStatusEnum",
  ADD COLUMN issued bool NOT NULL default false,
  ADD COLUMN issue_attempts integer NOT NULL default 0,
  ADD COLUMN last_issue_attempt_at TIMESTAMPTZ,
  ADD COLUMN last_issue_error TEXT
;

ALTER TYPE "InvoiceStatusEnum" RENAME VALUE 'UNCOLLECTIBLE' TO 'PENDING';


ALTER TABLE invoice
  DROP COLUMN auto_advance,
  DROP COLUMN issued_at,
  DROP COLUMN payment_status,
  DROP COLUMN paid_at;

ALTER TABLE subscription
  DROP COLUMN auto_advance_invoices,
  DROP COLUMN charge_automatically;

DROP TYPE "InvoicePaymentStatus";


ALTER TABLE payment_transaction
  DROP COLUMN receipt_pdf_id;

SELECT pgmq.drop_queue('invoice_orchestration');
SELECT pgmq.drop_queue('payment_request');
SELECT pgmq.drop_queue('send_email_request');
