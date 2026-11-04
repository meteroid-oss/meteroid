DROP INDEX idx_payment_transaction_credit_note;
DROP INDEX idx_payment_transaction_parent;

-- Refund rows have no representation before this migration; their amounts live on the parent's
-- `amount_refunded` already, so dropping them keeps the invoice math consistent.
DELETE FROM payment_transaction WHERE payment_type = 'REFUND';

ALTER TABLE payment_transaction
    DROP COLUMN refund_mode,
    DROP COLUMN credit_note_id,
    DROP COLUMN parent_transaction_id,
    DROP COLUMN reversal_reason,
    DROP COLUMN reversal_kind,
    DROP COLUMN amount_reversed;

DROP TYPE "RefundModeEnum";
DROP TYPE "ReversalKindEnum";
