-- Add payment_methods_config JSONB column to subscription table
-- NULL = Inherit from invoicing entity (backwards compatible default)
-- Format: { "type": "inherit" } or { "type": "override", "card_enabled": bool, "direct_debit_enabled": bool, "bank_transfer_enabled": bool }
ALTER TABLE subscription ADD COLUMN IF NOT EXISTS payment_methods_config JSONB NULL;

-- Also add to checkout_session table so checkout sessions can specify payment config
ALTER TABLE checkout_session ADD COLUMN IF NOT EXISTS payment_methods_config JSONB NULL;

-- Remove legacy payment method fields from subscription
-- These are replaced by dynamic resolution via payment_methods_config
ALTER TABLE subscription DROP COLUMN IF EXISTS card_connection_id;
ALTER TABLE subscription DROP COLUMN IF EXISTS direct_debit_connection_id;
ALTER TABLE subscription DROP COLUMN IF EXISTS bank_account_id;
ALTER TABLE subscription DROP COLUMN IF EXISTS payment_method;
ALTER TABLE subscription DROP COLUMN IF EXISTS payment_method_type;

-- Remove payment_strategy from quote table (replaced by payment_methods_config)
ALTER TABLE quote DROP COLUMN IF EXISTS payment_strategy;

-- Add payment_methods_config to quote table
ALTER TABLE quote ADD COLUMN IF NOT EXISTS payment_methods_config JSONB NULL;

-- Drop the legacy SubscriptionPaymentStrategy enum type
DROP TYPE IF EXISTS "SubscriptionPaymentStrategy";

-- Remove legacy provider fields from customer table
-- These are no longer used - payment methods are now resolved from:
-- 1. Subscription's payment_methods_config
-- 2. InvoicingEntity's providers
ALTER TABLE customer DROP COLUMN IF EXISTS card_provider_id;
ALTER TABLE customer DROP COLUMN IF EXISTS direct_debit_provider_id;
ALTER TABLE customer DROP COLUMN IF EXISTS bank_account_id;
