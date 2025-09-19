-- Quote status enum
CREATE TYPE "QuoteStatusEnum" AS ENUM('DRAFT', 'PENDING', 'ACCEPTED', 'DECLINED', 'EXPIRED', 'CANCELLED');

-- Quote table
CREATE TABLE quote (
    id UUID PRIMARY KEY,
    status "QuoteStatusEnum" NOT NULL DEFAULT 'DRAFT',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ,
    tenant_id UUID NOT NULL REFERENCES tenant(id),
    customer_id UUID NOT NULL REFERENCES customer(id),
    plan_version_id UUID NOT NULL REFERENCES plan_version(id),
    currency VARCHAR NOT NULL,
    quote_number VARCHAR NOT NULL,

    trial_duration_days INTEGER,
    billing_start_date DATE NOT NULL,
    billing_end_date DATE,
    billing_day_anchor INTEGER,
    activation_condition "SubscriptionActivationConditionEnum" NOT NULL DEFAULT 'ON_START',

    valid_until TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,
    accepted_at TIMESTAMPTZ,
    declined_at TIMESTAMPTZ,

    internal_notes TEXT,
    cover_image UUID,
    overview TEXT,
    terms_and_services TEXT,

    net_terms INTEGER NOT NULL DEFAULT 30,

    attachments UUID[] NOT NULL DEFAULT '{}',

    pdf_document_id UUID,
    sharing_key VARCHAR,

    converted_to_invoice_id UUID REFERENCES invoice(id),
    converted_to_subscription_id UUID REFERENCES subscription(id),
    converted_at TIMESTAMPTZ,

    recipients JSONB NOT NULL DEFAULT '[]',

    UNIQUE(tenant_id, quote_number)
);

CREATE TABLE quote_component (
     id UUID PRIMARY KEY,
     name TEXT NOT NULL,
     quote_id UUID NOT NULL REFERENCES quote(id) ON DELETE CASCADE,
     price_component_id UUID  REFERENCES price_component(id) ON DELETE CASCADE,
     product_id UUID   REFERENCES product(id) ON DELETE CASCADE,
     period             "SubscriptionFeeBillingPeriod" NOT NULL,
     fee                JSONB                          NOT NULL,
     is_override        BOOLEAN                        NOT NULL DEFAULT FALSE
);

CREATE TABLE quote_signature (
    id UUID PRIMARY KEY,
    quote_id UUID NOT NULL REFERENCES quote(id) ON DELETE CASCADE,

    signed_by_name VARCHAR NOT NULL,
    signed_by_email VARCHAR NOT NULL,
    signed_by_title VARCHAR,
    signature_data TEXT, -- Base64 encoded signature image or digital signature
    signature_method VARCHAR NOT NULL DEFAULT 'electronic',

    signed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ip_address VARCHAR,
    user_agent TEXT,

    verification_token VARCHAR UNIQUE,
    verified_at TIMESTAMPTZ
);

-- Quote activities/audit log
CREATE TABLE quote_activity (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    quote_id UUID NOT NULL REFERENCES quote(id) ON DELETE CASCADE,

    activity_type VARCHAR NOT NULL,
    description TEXT NOT NULL,

    actor_type VARCHAR NOT NULL,
    actor_id VARCHAR,
    actor_name VARCHAR,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ip_address VARCHAR,
    user_agent TEXT
);

CREATE INDEX idx_quote_tenant_id ON quote(tenant_id);
CREATE INDEX idx_quote_customer_id ON quote(customer_id);
CREATE INDEX idx_quote_status ON quote(status);
CREATE INDEX idx_quote_created_at ON quote(created_at);
CREATE INDEX idx_quote_quote_number ON quote(quote_number);
CREATE INDEX idx_quote_expires_at ON quote(expires_at);

CREATE INDEX idx_quote_signature_quote_id ON quote_signature(quote_id);
CREATE INDEX idx_quote_activity_quote_id ON quote_activity(quote_id);
CREATE INDEX idx_quote_activity_created_at ON quote_activity(created_at);
