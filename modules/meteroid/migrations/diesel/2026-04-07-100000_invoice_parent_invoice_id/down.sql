DROP INDEX IF EXISTS idx_invoice_parent_invoice_id;
ALTER TABLE invoice DROP COLUMN IF EXISTS parent_invoice_id;
