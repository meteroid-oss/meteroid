DROP INDEX IF EXISTS invoice_subscription_recurring_period_unique;

ALTER TABLE checkout_session ADD COLUMN IF NOT EXISTS activation_condition "SubscriptionActivationConditionEnum" NOT NULL DEFAULT 'ON_CHECKOUT';
ALTER TABLE checkout_session ADD COLUMN IF NOT EXISTS payment_strategy JSONB;
-- ALTER TABLE checkout_session ADD CONSTRAINT checkout_session_created_by_fkey FOREIGN KEY (created_by) REFERENCES "user"(id);
