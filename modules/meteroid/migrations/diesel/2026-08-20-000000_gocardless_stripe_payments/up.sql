-- GOCARDLESS bank-debit provider (mandate-based, Billing Request Flow).
ALTER TYPE "ConnectorProviderEnum" ADD VALUE IF NOT EXISTS 'GOCARDLESS';

-- Insertion time, distinct from processed_at (set only on terminal states).
-- Lets the reconciliation sweep skip Pending rows whose webhook hasn't landed
-- yet; existing rows backfill to NOW(). TIMESTAMPTZ (not naive) so the age math
-- in the reconciliation worker is unambiguous UTC, matching sibling columns.
ALTER TABLE payment_transaction
    ADD COLUMN created_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

CREATE INDEX idx_payment_transaction_pending_created_at
    ON payment_transaction (created_at)
    WHERE status = 'PENDING';

-- Customer auth step (3DS/SCA) the portal drives via Stripe.js while Pending;
-- cleared on terminal.
ALTER TABLE payment_transaction
    ADD COLUMN next_action JSONB;
