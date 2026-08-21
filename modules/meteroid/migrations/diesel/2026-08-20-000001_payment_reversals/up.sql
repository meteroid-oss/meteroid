-- Post-settlement money reversals (refunds, chargebacks, lost disputes) must
-- affect money state: a settled payment can be clawed back, reopening its invoice.

-- Terminal state for a settled payment whose funds were fully reclaimed. Excluded
-- from the settled-payments sum that derives invoice.amount_due, so flipping a row
-- into it reopens the invoice.
ALTER TYPE "PaymentStatusEnum" ADD VALUE IF NOT EXISTS 'REFUNDED';

-- Cumulative amount clawed back while the transaction is still SETTLED (partial
-- refunds). invoice.amount_due nets this out; a full claw-back instead flips the
-- status to REFUNDED. Existing rows backfill to 0 (nothing refunded).
ALTER TABLE payment_transaction
    ADD COLUMN amount_refunded BIGINT NOT NULL DEFAULT 0;
