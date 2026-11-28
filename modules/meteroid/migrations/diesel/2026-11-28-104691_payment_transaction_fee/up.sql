-- What a payment cost the tenant in provider fees. In the settlement currency when the provider
-- reports it (PROVIDER), in the charge currency when a tenant's fee rule estimates it
-- (ESTIMATED); NULL while unknown. Core never writes these columns: an Enterprise sweep does,
-- outside the payment path.
CREATE TYPE "PaymentFeeSourceEnum" AS ENUM ('PROVIDER', 'ESTIMATED');

ALTER TABLE payment_transaction
    ADD COLUMN fee_amount     BIGINT,
    ADD COLUMN fee_currency   TEXT,
    ADD COLUMN fee_details    JSONB,
    ADD COLUMN fee_source     "PaymentFeeSourceEnum",
    ADD COLUMN fee_settled_at TIMESTAMPTZ,
    ADD CONSTRAINT payment_transaction_fee_complete
        CHECK ((fee_amount IS NULL) = (fee_source IS NULL) AND (fee_amount IS NULL) = (fee_currency IS NULL));

-- The sweep's queue: a settled payment whose provider fee is still to read, and when to retry.
CREATE TABLE payment_fee_capture
(
    payment_transaction_id UUID PRIMARY KEY REFERENCES payment_transaction (id) ON DELETE CASCADE,
    tenant_id              UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    attempts               SMALLINT    NOT NULL DEFAULT 0,
    next_attempt_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error             TEXT
);
CREATE INDEX payment_fee_capture_due ON payment_fee_capture (next_attempt_at);
