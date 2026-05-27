ALTER TABLE payment_transaction DROP COLUMN next_action;

DROP INDEX IF EXISTS idx_payment_transaction_pending_created_at;
ALTER TABLE payment_transaction DROP COLUMN created_at;

DROP INDEX IF EXISTS webhook_in_event_dedup_idx;
ALTER TABLE webhook_in_event DROP COLUMN provider_event_id;

-- ConnectorProviderEnum 'GOCARDLESS' cannot be removed (Postgres has no DROP
-- VALUE); rolling it back requires recreating the type manually.
