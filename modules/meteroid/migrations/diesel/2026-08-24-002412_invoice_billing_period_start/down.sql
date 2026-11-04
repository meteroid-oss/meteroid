DROP INDEX invoice_subscription_recurring_period_unique;

-- Recreated NON-unique on purpose: once the feature has been live, catch-up billing
-- can finalize several periods of one subscription on the same day, so (subscription_id,
-- invoice_date) is no longer unique and a UNIQUE build would fail — wedging the rollback.
-- The index still serves the same lookups; uniqueness cannot be restored after the fact.
CREATE INDEX invoice_subscription_recurring_period_unique
ON invoice (subscription_id, invoice_date)
WHERE subscription_id IS NOT NULL
  AND invoice_type = 'RECURRING'::"InvoiceType"
  AND status <> 'VOID'::"InvoiceStatusEnum"
  AND created_at >= '2026-01-28T23:00:00Z'::timestamptz;

DROP INDEX IF EXISTS idx_invoice_subscription_billing_period_start;
DROP INDEX IF EXISTS idx_invoice_tenant_billing_period_start;

ALTER TABLE invoice
    DROP COLUMN billing_period_start;
