SELECT pgmq.create('quote_conversion');

CREATE TYPE "SubscriptionPaymentStrategy" AS ENUM ('AUTO', 'BANK', 'EXTERNAL');

-- Add payment configuration fields to quote table
ALTER TABLE quote ADD COLUMN payment_strategy "SubscriptionPaymentStrategy" NOT NULL DEFAULT 'AUTO';
ALTER TABLE quote ADD COLUMN auto_advance_invoices BOOLEAN NOT NULL DEFAULT true;
ALTER TABLE quote ADD COLUMN charge_automatically BOOLEAN NOT NULL DEFAULT true;
ALTER TABLE quote ADD COLUMN invoice_memo TEXT;
ALTER TABLE quote ADD COLUMN invoice_threshold NUMERIC;
ALTER TABLE quote ADD COLUMN create_subscription_on_acceptance BOOLEAN NOT NULL DEFAULT false;

-- Make billing_start_date optional to support dynamic start dates
ALTER TABLE quote ALTER COLUMN billing_start_date DROP NOT NULL;

-- Create quote_add_on table (mirrors subscription_add_on structure)
CREATE TABLE quote_add_on (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    quote_id UUID NOT NULL REFERENCES quote(id) ON DELETE CASCADE,
    add_on_id UUID NOT NULL REFERENCES add_on(id) ON DELETE CASCADE,
    period "SubscriptionFeeBillingPeriod" NOT NULL,
    fee JSONB NOT NULL
);

CREATE INDEX idx_quote_add_on_quote_id ON quote_add_on(quote_id);
CREATE INDEX idx_quote_add_on_add_on_id ON quote_add_on(add_on_id);

-- Create quote_coupon table
CREATE TABLE quote_coupon (
    id UUID PRIMARY KEY,
    quote_id UUID NOT NULL REFERENCES quote(id) ON DELETE CASCADE,
    coupon_id UUID NOT NULL REFERENCES coupon(id) ON DELETE CASCADE,
    UNIQUE(quote_id, coupon_id)
);

CREATE INDEX idx_quote_coupon_quote_id ON quote_coupon(quote_id);
CREATE INDEX idx_quote_coupon_coupon_id ON quote_coupon(coupon_id);


ALTER TABLE subscription ADD COLUMN quote_id UUID REFERENCES quote(id) ON DELETE SET NULL;

CREATE INDEX idx_subscription_quote_id ON subscription(quote_id) WHERE quote_id IS NOT NULL;

