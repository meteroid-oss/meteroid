-- Remove payment_methods_config column from subscription table
ALTER TABLE subscription DROP COLUMN IF EXISTS payment_methods_config;

-- Remove payment_methods_config column from checkout_session table
ALTER TABLE checkout_session DROP COLUMN IF EXISTS payment_methods_config;

-- Remove payment_methods_config column from quote table
ALTER TABLE quote DROP COLUMN IF EXISTS payment_methods_config;

-- Recreate SubscriptionPaymentStrategy enum type
CREATE TYPE "SubscriptionPaymentStrategy" AS ENUM ('AUTO', 'BANK', 'EXTERNAL');

-- Restore payment_strategy column on quote table
ALTER TABLE quote ADD COLUMN payment_strategy "SubscriptionPaymentStrategy" NOT NULL DEFAULT 'AUTO';

-- Restore legacy payment method fields on subscription
ALTER TABLE subscription ADD COLUMN card_connection_id UUID REFERENCES customer_connection(id);
ALTER TABLE subscription ADD COLUMN direct_debit_connection_id UUID REFERENCES customer_connection(id);
ALTER TABLE subscription ADD COLUMN bank_account_id UUID REFERENCES bank_account(id);
ALTER TABLE subscription ADD COLUMN payment_method UUID REFERENCES customer_payment_method(id);
ALTER TABLE subscription ADD COLUMN payment_method_type "PaymentMethodTypeEnum";

-- Restore legacy provider fields on customer table
ALTER TABLE customer ADD COLUMN card_provider_id UUID REFERENCES connector(id) ON DELETE SET NULL;
ALTER TABLE customer ADD COLUMN direct_debit_provider_id UUID REFERENCES connector(id) ON DELETE SET NULL;
ALTER TABLE customer ADD COLUMN bank_account_id UUID REFERENCES bank_account(id) ON DELETE SET NULL;
