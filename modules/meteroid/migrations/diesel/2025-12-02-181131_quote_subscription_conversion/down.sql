SELECT pgmq.drop_queue('quote_conversion');

DROP TABLE IF EXISTS quote_coupon;
DROP TABLE IF EXISTS quote_add_on;

UPDATE quote SET billing_start_date = created_at::date WHERE billing_start_date IS NULL;
ALTER TABLE quote ALTER COLUMN billing_start_date SET NOT NULL;

ALTER TABLE quote DROP COLUMN IF EXISTS create_subscription_on_acceptance;
ALTER TABLE quote DROP COLUMN IF EXISTS invoice_threshold;
ALTER TABLE quote DROP COLUMN IF EXISTS invoice_memo;
ALTER TABLE quote DROP COLUMN IF EXISTS charge_automatically;
ALTER TABLE quote DROP COLUMN IF EXISTS auto_advance_invoices;
ALTER TABLE quote DROP COLUMN IF EXISTS payment_strategy;

DROP INDEX IF EXISTS idx_subscription_quote_id;
ALTER TABLE subscription DROP COLUMN IF EXISTS quote_id;

DROP TYPE IF EXISTS "SubscriptionPaymentStrategy";
