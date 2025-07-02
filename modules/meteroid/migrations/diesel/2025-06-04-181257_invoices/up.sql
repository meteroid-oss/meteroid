ALTER TABLE invoice
  ALTER COLUMN pdf_document_id TYPE uuid USING pdf_document_id::uuid,
  ALTER COLUMN xml_document_id TYPE uuid USING xml_document_id::uuid
;

ALTER TABLE invoicing_entity
  ALTER COLUMN logo_attachment_id TYPE uuid USING logo_attachment_id::uuid;

ALTER TABLE invoice
    DROP COLUMN external_invoice_id,
    DROP COLUMN external_status,
    DROP COLUMN issued,
    DROP COLUMN issue_attempts,
    DROP COLUMN last_issue_attempt_at,
    DROP COLUMN last_issue_error;

DROP TYPE "InvoiceExternalStatusEnum";


ALTER TYPE "InvoiceStatusEnum" RENAME VALUE 'PENDING' TO 'UNCOLLECTIBLE';

CREATE TYPE "InvoicePaymentStatus" AS ENUM('UNPAID', 'PARTIALLY_PAID', 'PAID', 'ERRORED'); -- we may add 'REFUNDED' & 'VOIDED'

ALTER TABLE invoice
  ADD COLUMN auto_advance bool NOT NULL default true,
  ADD COLUMN issued_at TIMESTAMPTZ,
  ADD COLUMN payment_status "InvoicePaymentStatus" NOT NULL default 'UNPAID',
  ADD COLUMN paid_at TIMESTAMPTZ;

ALTER TABLE payment_transaction
  ADD COLUMN receipt_pdf_id uuid;

ALTER TABLE subscription
  ADD COLUMN auto_advance_invoices bool NOT NULL default true,
  ADD COLUMN charge_automatically bool NOT NULL default true;

SELECT pgmq.create('invoice_orchestration');
SELECT pgmq.create('payment_request');
SELECT pgmq.create('send_email_request');
