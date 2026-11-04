-- A method is marked saved-for-payment only while it never paid: that check reads its payments.
--
-- CONCURRENTLY, so writers are never locked out (metadata.toml sets
-- run_in_transaction = false). This file must stay a SINGLE statement. If a run fails it
-- leaves an INVALID index that IF NOT EXISTS will not replace: drop it by hand, then re-run.
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_payment_transaction_payment_method_id
    ON payment_transaction (payment_method_id)
    WHERE payment_method_id IS NOT NULL;
