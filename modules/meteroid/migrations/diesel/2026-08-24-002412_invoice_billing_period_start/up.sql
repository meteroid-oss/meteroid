-- The billing period an invoice settles, decoupled from invoice_date so the
-- latter can become the emission date at finalization. Backfilled from
-- invoice_date, which is what it has meant until now.
--
-- No column DEFAULT on purpose: billing_period_start is a period key (dedup, unique
-- index), and no constant is a correct period. A rolling deploy where old code inserts
-- without this column needs a temporary BEFORE INSERT trigger defaulting it to
-- invoice_date (the period start under old semantics) — not a constant that would
-- silently poison the duplicate-billing backstop. Safe to ship code + migration together.
--
-- Note: the backfill UPDATE rewrites the whole table under lock and the index
-- builds below are non-concurrent. Cheap on a small table; on a large one, split
-- into a non-transactional migration with CREATE INDEX CONCURRENTLY.
--
-- Historical straddle: finalized invoices created before this migration keep
-- invoice_date = period start, while new ones carry the emission date. Analytics
-- or tax reports keyed on invoice_date span both semantics.
ALTER TABLE invoice
    ADD COLUMN billing_period_start DATE;

UPDATE invoice
SET billing_period_start = invoice_date;

ALTER TABLE invoice
    ALTER COLUMN billing_period_start SET NOT NULL;

CREATE INDEX idx_invoice_subscription_billing_period_start
    ON invoice (tenant_id, subscription_id, billing_period_start)
    WHERE subscription_id IS NOT NULL;

-- Serves the tenant-wide invoice list default sort (ORDER BY billing_period_start DESC, id DESC),
-- which the partial index above cannot (it excludes one-off/manual invoices and orders differently).
CREATE INDEX idx_invoice_tenant_billing_period_start
    ON invoice (tenant_id, billing_period_start DESC, id DESC);

-- The duplicate-billing backstop keys on the period, not the emission date:
-- several past periods finalized the same day must not collide.
DROP INDEX invoice_subscription_recurring_period_unique;

CREATE UNIQUE INDEX invoice_subscription_recurring_period_unique
ON invoice (subscription_id, billing_period_start)
WHERE subscription_id IS NOT NULL
  AND invoice_type = 'RECURRING'::"InvoiceType"
  AND status <> 'VOID'::"InvoiceStatusEnum"
  AND created_at >= '2026-01-28T23:00:00Z'::timestamptz;
