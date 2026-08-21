-- The webhook settlement/reversal path resolves transactions by the provider's
-- own charge id whenever the event carries no `meteroid.transaction_id` — every
-- lifecycle event of a GoCardless checkout-origin payment. Without this index
-- the lookup scans all of the tenant's payment rows per webhook event.
-- Partial: manual payments and pre-charge rows have no provider id.
-- Not UNIQUE: manual payments store a free-form user-supplied reference in
-- this column, so duplicates are legitimate.
CREATE INDEX idx_payment_transaction_provider_tx_id
    ON payment_transaction (tenant_id, provider_transaction_id)
    WHERE provider_transaction_id IS NOT NULL;
