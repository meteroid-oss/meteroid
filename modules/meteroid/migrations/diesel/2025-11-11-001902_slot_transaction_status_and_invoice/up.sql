-- Add status to track pending vs active slot transactions
CREATE TYPE slot_transaction_status AS ENUM ('PENDING', 'ACTIVE');

ALTER TABLE slot_transaction
  ADD COLUMN status slot_transaction_status NOT NULL DEFAULT 'ACTIVE',
  ADD COLUMN invoice_id UUID REFERENCES invoice(id) ON UPDATE CASCADE ON DELETE SET NULL;

-- Index for looking up pending transactions by invoice
CREATE INDEX idx_slot_transaction_invoice_status
  ON slot_transaction(invoice_id, status)
  WHERE invoice_id IS NOT NULL;

COMMENT ON COLUMN slot_transaction.status IS 'pending: awaiting payment, active: slots are live';
COMMENT ON COLUMN slot_transaction.invoice_id IS 'Links to invoice for OnInvoicePaid billing mode';
