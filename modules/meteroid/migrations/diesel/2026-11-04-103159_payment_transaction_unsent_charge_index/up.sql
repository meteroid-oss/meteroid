-- Feeds the sweep that re-sends unanswered charges. CONCURRENTLY (metadata.toml sets
-- run_in_transaction = false), so this must stay a single statement.
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_payment_transaction_unsent_charge
    ON payment_transaction (created_at)
    WHERE status = 'PENDING' AND provider_transaction_id IS NULL AND charge_attempt IS NOT NULL;
