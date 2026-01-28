-- Prevent duplicate recurring invoices for the same subscription and billing period.
-- This constraint allows:
--   - Multiple OneOff invoices on the same day (e.g., slot purchases)
--   - Multiple UsageThreshold invoices on the same day
--   - Adjustment invoices anytime
-- But blocks:
--   - Two Recurring invoices for the same subscription + invoice_date (unless one is voided)
-- (on failure clean existing or update tz)
CREATE UNIQUE INDEX invoice_subscription_recurring_period_unique
ON invoice (subscription_id, invoice_date)
WHERE subscription_id IS NOT NULL
  AND invoice_type = 'RECURRING'::"InvoiceType"
  AND status <> 'VOID'::"InvoiceStatusEnum"
  AND created_at >= '2026-01-28T20:00:00Z'::timestamptz;

-- Drop activation_condition and payment_strategy from checkout_session table.
ALTER TABLE checkout_session DROP COLUMN IF EXISTS activation_condition;
ALTER TABLE checkout_session DROP COLUMN IF EXISTS payment_strategy;
ALTER TABLE checkout_session DROP CONSTRAINT checkout_session_created_by_fkey;
