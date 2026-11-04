-- A new milestone needs ALTER TYPE "CustomerMilestoneEnum" ADD VALUE.
CREATE TYPE "CustomerMilestoneEnum" AS ENUM (
    'CUSTOMER_CREATED',
    'QUOTE_CREATED',
    'QUOTE_ACCEPTED',
    'CHECKOUT_COMPLETED',
    'TRIAL_STARTED',
    'SUBSCRIPTION_ACTIVATED',
    'PAYMENT_METHOD_ATTACHED',
    'SEATS_PROVISIONED',
    'FIRST_INVOICE_FINALIZED',
    'FIRST_INVOICE_PAID',
    -- Derived from metering (ClickHouse), not from a Postgres source.
    'FIRST_EVENT_INGESTED'
);

-- First-occurrence milestones per customer. Write-once: the worker inserts
-- ON CONFLICT DO NOTHING, so a recorded milestone is never revised.
CREATE TABLE customer_milestone (
    tenant_id   UUID                      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    customer_id UUID                      NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    milestone   "CustomerMilestoneEnum"   NOT NULL,
    occurred_at TIMESTAMP(3)              NOT NULL,
    created_at  TIMESTAMP(3)              NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, customer_id, milestone)
);

-- The funnel reads every milestone for a tenant's customers.
CREATE INDEX idx_customer_milestone_tenant_occurred
    ON customer_milestone (tenant_id, milestone, occurred_at);
