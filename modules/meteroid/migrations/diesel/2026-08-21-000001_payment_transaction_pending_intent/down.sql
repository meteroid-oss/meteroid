DROP INDEX IF EXISTS idx_payment_transaction_pending_provider_intent;

ALTER TABLE payment_transaction
    DROP COLUMN IF EXISTS pending_provider_intent_id,
    DROP COLUMN IF EXISTS pending_connection_id;
