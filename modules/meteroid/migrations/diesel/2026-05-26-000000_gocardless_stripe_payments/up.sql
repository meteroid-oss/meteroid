-- GOCARDLESS bank-debit provider (mandate-based, Billing Request Flow).
ALTER TYPE "ConnectorProviderEnum" ADD VALUE IF NOT EXISTS 'GOCARDLESS';

-- Inbound webhook idempotency keyed on the provider's own event id.
-- Nullable so existing rows survive (NULLS DISTINCT keeps them allowed); a full
-- (non-partial) unique index is required because Diesel's on_conflict emits
-- `ON CONFLICT (cols)` with no WHERE, which won't match a partial index.
ALTER TABLE webhook_in_event
    ADD COLUMN provider_event_id TEXT;

CREATE UNIQUE INDEX webhook_in_event_dedup_idx
    ON webhook_in_event (provider_config_id, provider_event_id);

-- Insertion time, distinct from processed_at (set only on terminal states).
-- Lets the reconciliation sweep skip Pending rows whose webhook hasn't landed
-- yet; existing rows backfill to NOW().
ALTER TABLE payment_transaction
    ADD COLUMN created_at TIMESTAMP NOT NULL DEFAULT NOW();

CREATE INDEX idx_payment_transaction_pending_created_at
    ON payment_transaction (created_at)
    WHERE status = 'PENDING';

-- Customer auth step (3DS/SCA) the portal drives via Stripe.js while Pending;
-- cleared on terminal.
ALTER TABLE payment_transaction
    ADD COLUMN next_action JSONB;
