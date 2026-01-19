
DROP TABLE IF EXISTS checkout_session;
DROP TYPE IF EXISTS "CheckoutSessionStatusEnum";
DROP TYPE IF EXISTS "CheckoutTypeEnum";


-- Remove index
DROP INDEX IF EXISTS idx_payment_transaction_checkout_session_id;
ALTER TABLE payment_transaction DROP CONSTRAINT IF EXISTS payment_transaction_invoice_or_checkout;
ALTER TABLE payment_transaction DROP COLUMN IF EXISTS checkout_session_id;
ALTER TABLE payment_transaction ALTER COLUMN invoice_id SET NOT NULL;

