-- Audit logging & activity timeline system.
--
-- Consolidated migration: creates the entity_activity + sent_email tables in
-- their final shape (actor stored as actor_uuid / actor_alias) and drops the
-- per-row created_by attribution columns now subsumed by entity_activity.

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
    -- actor_uuid for USER / API_TOKEN / CUSTOMER actors, actor_alias for
    -- QUOTE_RECIPIENT (e.g. an email address), both NULL for SYSTEM.
    actor_uuid UUID,
    actor_alias TEXT,

    metadata JSONB,

    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Denormalized rollup refs.
    agg_customer_id UUID,
    agg_subscription_id UUID,

    CONSTRAINT entity_activity_agg_customer_not_self
        CHECK (entity_type != 'customer' OR agg_customer_id IS NULL),
    CONSTRAINT entity_activity_agg_subscription_not_self
        CHECK (entity_type != 'subscription' OR agg_subscription_id IS NULL),

    -- Shape constraint: keeps UUID/alias/null aligned with actor_type.
    CONSTRAINT entity_activity_actor_shape CHECK (
        CASE actor_type
            WHEN 'SYSTEM'          THEN actor_uuid IS NULL     AND actor_alias IS NULL
            WHEN 'QUOTE_RECIPIENT' THEN actor_uuid IS NULL     AND actor_alias IS NOT NULL
            ELSE                        actor_uuid IS NOT NULL AND actor_alias IS NULL
        END
    )
);

-- Entity timeline ("everything that happened to this invoice/customer/...")
CREATE INDEX idx_entity_activity_entity
    ON entity_activity (tenant_id, entity_type, entity_id, occurred_at DESC);

-- Global feed + cursor pagination
CREATE INDEX idx_entity_activity_tenant_time
    ON entity_activity (tenant_id, occurred_at DESC);

-- "Everything actor X did" — partial (SYSTEM and QUOTE_RECIPIENT have no uuid)
CREATE INDEX idx_entity_activity_actor
    ON entity_activity (tenant_id, actor_type, actor_uuid, occurred_at DESC)
    WHERE actor_uuid IS NOT NULL;

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

-- Drop attribution columns now subsumed by entity_activity.
-- Kept: batch_job.created_by (still surfaced in BatchJobDetail UI),
--       customer_balance_tx.created_by + customer_balance_pending_tx.created_by
--       (ledger provenance must outlive a possible audit-table reset).
ALTER TABLE api_token         DROP COLUMN created_by;
ALTER TABLE bank_account      DROP COLUMN created_by;
ALTER TABLE billable_metric   DROP COLUMN created_by;
ALTER TABLE checkout_session  DROP COLUMN created_by;
ALTER TABLE customer          DROP COLUMN created_by,
                              DROP COLUMN updated_by,
                              DROP COLUMN archived_by;
ALTER TABLE entitlement       DROP COLUMN created_by;
ALTER TABLE feature           DROP COLUMN created_by;
ALTER TABLE plan              DROP COLUMN created_by;
ALTER TABLE plan_version      DROP COLUMN created_by;
ALTER TABLE price             DROP COLUMN created_by;
ALTER TABLE product           DROP COLUMN created_by;
ALTER TABLE subscription      DROP COLUMN created_by;
