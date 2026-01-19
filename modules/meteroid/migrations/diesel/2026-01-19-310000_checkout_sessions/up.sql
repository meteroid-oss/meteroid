CREATE TYPE "CheckoutSessionStatusEnum" AS ENUM('CREATED', 'COMPLETED', 'EXPIRED', 'CANCELLED', 'AWAITING_PAYMENT');
CREATE TYPE "CheckoutTypeEnum" AS ENUM('SELF_SERVE', 'SUBSCRIPTION_ACTIVATION');

CREATE TABLE checkout_session (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenant(id),
    customer_id UUID NOT NULL REFERENCES customer(id),
    plan_version_id UUID NOT NULL REFERENCES plan_version(id),
    created_by UUID NOT NULL REFERENCES "user"(id),

    -- Basic subscription parameters
    billing_start_date DATE,
    billing_day_anchor SMALLINT,
    net_terms INTEGER,
    trial_duration_days INTEGER,
    end_date DATE,

    -- Billing options
    activation_condition "SubscriptionActivationConditionEnum" NOT NULL DEFAULT 'ON_START',
    auto_advance_invoices BOOLEAN NOT NULL DEFAULT true,
    charge_automatically BOOLEAN NOT NULL DEFAULT true,
    invoice_memo TEXT,
    invoice_threshold NUMERIC,
    purchase_order TEXT,

    -- Complex parameters (JSONB)
    payment_strategy JSONB,
    components JSONB,
    add_ons JSONB,

    -- Coupons (supports both code lookup and direct IDs)
    coupon_code VARCHAR,
    coupon_ids UUID[] NOT NULL DEFAULT '{}',

    -- Session state
    status "CheckoutSessionStatusEnum" NOT NULL DEFAULT 'CREATED',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    subscription_id UUID REFERENCES subscription(id),
    metadata JSONB,
    checkout_type "CheckoutTypeEnum" NOT NULL DEFAULT 'SELF_SERVE'
);

CREATE INDEX idx_checkout_session_tenant ON checkout_session(tenant_id);
CREATE INDEX idx_checkout_session_subscription ON checkout_session(subscription_id) WHERE subscription_id IS NOT NULL;
CREATE INDEX idx_checkout_session_status_expires ON checkout_session(status, expires_at)
  WHERE status = 'CREATED' AND expires_at IS NOT NULL;


-- Make invoice_id nullable on payment_transaction (for checkout payments without invoice yet)
ALTER TABLE payment_transaction ALTER COLUMN invoice_id DROP NOT NULL;

-- Add checkout_session_id to payment_transaction for linking checkout payments
ALTER TABLE payment_transaction ADD COLUMN checkout_session_id UUID REFERENCES checkout_session(id) ON DELETE SET NULL;

-- Add constraint: either invoice_id or checkout_session_id must be set
ALTER TABLE payment_transaction ADD CONSTRAINT payment_transaction_invoice_or_checkout
  CHECK (invoice_id IS NOT NULL OR checkout_session_id IS NOT NULL);

-- Index for looking up transactions by checkout_session_id
CREATE INDEX idx_payment_transaction_checkout_session_id ON payment_transaction(checkout_session_id) WHERE checkout_session_id IS NOT NULL;
