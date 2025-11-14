-- Remove status and invoice_id columns
DROP INDEX IF EXISTS idx_slot_transaction_invoice_status;

ALTER TABLE slot_transaction
  DROP COLUMN IF EXISTS invoice_id,
  DROP COLUMN IF EXISTS status;

DROP TYPE IF EXISTS slot_transaction_status;
