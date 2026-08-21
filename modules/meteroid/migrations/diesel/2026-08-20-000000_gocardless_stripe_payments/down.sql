ALTER TABLE payment_transaction DROP COLUMN next_action;

DROP INDEX IF EXISTS idx_payment_transaction_pending_created_at;
ALTER TABLE payment_transaction DROP COLUMN created_at;

-- ConnectorProviderEnum 'GOCARDLESS' cannot be removed (Postgres has no DROP
-- VALUE); rolling it back requires recreating the type manually.
