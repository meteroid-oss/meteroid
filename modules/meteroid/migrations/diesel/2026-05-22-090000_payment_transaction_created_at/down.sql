DROP INDEX IF EXISTS idx_payment_transaction_pending_created_at;
ALTER TABLE payment_transaction DROP COLUMN created_at;
