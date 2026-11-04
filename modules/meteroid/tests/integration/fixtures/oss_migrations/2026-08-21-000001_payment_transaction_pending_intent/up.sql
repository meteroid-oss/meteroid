-- Stancer hosted flows (checkout AND invoice payment) capture the payment
-- IN-FLOW on the hosted page, and Stancer has no webhooks: if the customer
-- pays but never returns, only an intent id stored on the pre-created
-- transaction lets a sweeper recover the captured payment.
ALTER TABLE payment_transaction
    ADD COLUMN pending_provider_intent_id TEXT,
    ADD COLUMN pending_connection_id UUID REFERENCES customer_connection (id);

-- Sweeper scan: transactions that still carry a hosted intent and are not
-- settled. Failed/Cancelled rows stay watched until the abandonment cutoff
-- clears the marker (the hosted page can still capture after a decline).
CREATE INDEX idx_payment_transaction_pending_provider_intent
    ON payment_transaction (created_at, id)
    WHERE pending_provider_intent_id IS NOT NULL
      AND status IN ('PENDING', 'READY', 'FAILED', 'CANCELLED');
