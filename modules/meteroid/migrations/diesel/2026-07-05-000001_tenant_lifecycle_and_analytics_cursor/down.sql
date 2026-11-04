-- Reverse of up.sql: analytics projection cursor first, then tenant lifecycle.

-- ---- Analytics projection cursor ----
DROP TRIGGER IF EXISTS trg_product_store_committed_at ON product;
DROP INDEX IF EXISTS idx_product_store_committed_at;
ALTER TABLE product DROP COLUMN IF EXISTS store_committed_at;

DROP TRIGGER IF EXISTS trg_plan_store_committed_at ON plan;
DROP INDEX IF EXISTS idx_plan_store_committed_at;
ALTER TABLE plan DROP COLUMN IF EXISTS store_committed_at;

DROP TRIGGER IF EXISTS trg_customer_store_committed_at ON customer;
DROP INDEX IF EXISTS idx_customer_store_committed_at;
ALTER TABLE customer DROP COLUMN IF EXISTS store_committed_at;

DROP TRIGGER IF EXISTS trg_payment_transaction_store_committed_at ON payment_transaction;
DROP INDEX IF EXISTS idx_payment_transaction_store_committed_at;
ALTER TABLE payment_transaction DROP COLUMN IF EXISTS store_committed_at;

DROP TRIGGER IF EXISTS trg_credit_note_store_committed_at ON credit_note;
DROP INDEX IF EXISTS idx_credit_note_store_committed_at;
ALTER TABLE credit_note DROP COLUMN IF EXISTS store_committed_at;

DROP TRIGGER IF EXISTS trg_invoice_store_committed_at ON invoice;
DROP INDEX IF EXISTS idx_invoice_store_committed_at;
ALTER TABLE invoice DROP COLUMN IF EXISTS store_committed_at;

DROP FUNCTION IF EXISTS set_store_committed_at();

-- ---- Tenant activity + lifecycle ----
DROP INDEX IF EXISTS idx_tenant_status;
ALTER TABLE tenant DROP COLUMN status;
DROP TYPE "TenantStatusEnum";
ALTER TABLE tenant DROP COLUMN last_active_at;
