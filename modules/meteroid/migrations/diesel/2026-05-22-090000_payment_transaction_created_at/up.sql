-- Track when a payment_transaction row was first inserted, separate from
-- `processed_at` (which only gets set when the transaction transitions to a
-- terminal state).
--
-- The reconciliation worker uses this to filter out *fresh* Pending rows
-- whose webhook simply hasn't arrived yet — without it, every Pending row
-- gets polled against the provider on every sweep, wasting API rate limit.
--
-- Backfill: existing rows get `created_at = NOW()`. That's a slight
-- inaccuracy for already-Pending rows (they get reconciled one sweep later
-- than they "should"), but the alternative (extracting from the v7 UUID id)
-- adds complexity for marginal benefit. After the first reconciliation
-- cycle the value is irrelevant for those rows anyway.
ALTER TABLE payment_transaction
    ADD COLUMN created_at TIMESTAMP NOT NULL DEFAULT NOW();

-- Speeds up the worker's "stale pending" sweep — filtered by status +
-- created_at, ordered by created_at to process oldest first.
CREATE INDEX idx_payment_transaction_pending_created_at
    ON payment_transaction (created_at)
    WHERE status = 'PENDING';
