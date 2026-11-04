
DROP TABLE IF EXISTS quote_activity CASCADE;

CREATE TYPE "ActorTypeEnum" AS ENUM (
    'SYSTEM',
    'USER',
    'API_TOKEN',
    'CUSTOMER',
    'QUOTE_RECIPIENT'
);

CREATE TABLE entity_activity (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,

    entity_type TEXT NOT NULL,
    entity_id UUID NOT NULL,

    activity_type TEXT NOT NULL,

    actor_type "ActorTypeEnum" NOT NULL,
    actor_id TEXT,

    metadata JSONB,

    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Denormalized rollup refs.
    agg_customer_id UUID,
    agg_subscription_id UUID,

    CONSTRAINT entity_activity_agg_customer_not_self
        CHECK (entity_type != 'customer' OR agg_customer_id IS NULL),
    CONSTRAINT entity_activity_agg_subscription_not_self
        CHECK (entity_type != 'subscription' OR agg_subscription_id IS NULL)
);

-- Entity timeline ("everything that happened to this invoice/customer/...")
CREATE INDEX idx_entity_activity_entity
    ON entity_activity (tenant_id, entity_type, entity_id, occurred_at DESC);

-- Global feed + cursor pagination
CREATE INDEX idx_entity_activity_tenant_time
    ON entity_activity (tenant_id, occurred_at DESC);

-- "Everything actor X did" — partial to keep it cheap (actor_id is null for SYSTEM)
CREATE INDEX idx_entity_activity_actor
    ON entity_activity (tenant_id, actor_type, actor_id, occurred_at DESC)
    WHERE actor_id IS NOT NULL;

-- Partial indexes for agg rollups — most rows have NULL agg refs (plan/product/etc
-- events don't roll up), so a partial index keeps the index small and the
-- customer/subscription rollup query selective.
CREATE INDEX idx_entity_activity_agg_customer
    ON entity_activity (tenant_id, agg_customer_id, occurred_at DESC)
    WHERE agg_customer_id IS NOT NULL;

CREATE INDEX idx_entity_activity_agg_subscription
    ON entity_activity (tenant_id, agg_subscription_id, occurred_at DESC)
    WHERE agg_subscription_id IS NOT NULL;

-- Sent email log: one row per delivered email. Body is TEXT (TOAST-compressed)
-- — no S3 hop on preview, no extra service to back up. Shares its PK with the
-- `entity.email_sent` audit row that records the same delivery, so the audit
-- row IS the receipt by id.
CREATE TABLE sent_email (
    id UUID PRIMARY KEY REFERENCES entity_activity(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    subject TEXT NOT NULL,
    from_addr TEXT NOT NULL,
    reply_to TEXT,
    recipients TEXT[] NOT NULL,
    body_html TEXT NOT NULL,
    attachments JSONB
);

CREATE INDEX idx_sent_email_tenant_time
    ON sent_email (tenant_id, sent_at DESC);
