DROP INDEX IF EXISTS idx_invoice_consolidated_into_invoice_id;
ALTER TABLE invoice DROP COLUMN IF EXISTS consolidated_into_invoice_id;
