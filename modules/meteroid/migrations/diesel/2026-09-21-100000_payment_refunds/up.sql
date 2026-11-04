-- Refunds split by nature:
--   * voluntary refund  -> a child payment_transaction row (payment_type = REFUND) pointing at its
--     parent Payment row; it never touches the invoice (the credit note does).
--   * involuntary claw-back (chargeback, recall, lost dispute) -> mutates the parent Payment row
--     (amount_reversed / reversal_kind / reversal_reason) and reopens the invoice.
-- A fresh CREATE TYPE is usable in the same transaction (unlike ALTER TYPE ... ADD VALUE).
CREATE TYPE "ReversalKindEnum" AS ENUM (
    'REFUND', 'CHARGEBACK', 'DEBTOR_RECALL', 'INSUFFICIENT_FUNDS',
    'MANDATE_INVALID', 'RETURNED', 'DISPUTE', 'MIGRATED', 'OTHER'
);
CREATE TYPE "RefundModeEnum" AS ENUM ('ONLINE', 'OFFLINE');

ALTER TABLE payment_transaction
    ADD COLUMN amount_reversed       BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN reversal_kind         "ReversalKindEnum",
    ADD COLUMN reversal_reason       TEXT,
    ADD COLUMN parent_transaction_id UUID REFERENCES payment_transaction(id),
    ADD COLUMN credit_note_id        UUID REFERENCES credit_note(id),
    ADD COLUMN refund_mode           "RefundModeEnum";

-- One-off backfill assumption: before this migration `amount_refunded` was written by BOTH the
-- involuntary path (chargebacks, lost disputes) and provider-observed refunds, with nothing in the
-- row telling them apart. Historical rows are therefore read as involuntary — the conservative
-- reading, since that is the one the invoice math already netted out. The figure MOVES rather than
-- being copied: the two totals are now subtracted separately (refundable amount, Refunded status,
-- the delta an observed refund records), so leaving it in both would count every historical
-- claw-back twice.
-- `MIGRATED` marks exactly these rows: the provider still counts the voluntary part of this
-- figure in the cumulative refunded total it reports, so the observed-refund path must treat it
-- as already known or it would record the same money again on the next refund of this charge.
UPDATE payment_transaction
SET amount_reversed  = amount_refunded,
    amount_refunded  = 0,
    reversal_kind    = 'MIGRATED'
WHERE amount_refunded > 0;

-- Both partial: only REFUND rows carry these, a small slice of the table.
CREATE INDEX idx_payment_transaction_parent
    ON payment_transaction (parent_transaction_id) WHERE parent_transaction_id IS NOT NULL;
CREATE INDEX idx_payment_transaction_credit_note
    ON payment_transaction (credit_note_id) WHERE credit_note_id IS NOT NULL;
