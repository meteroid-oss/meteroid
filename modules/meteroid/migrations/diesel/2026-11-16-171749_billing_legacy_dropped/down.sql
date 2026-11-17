-- Reverses billing_legacy_dropped: the scheduled_event table as the previous release reads it,
-- the events it still runs on rebuilt from the facts and the invoice columns (each live
-- cancellation, each paid trial still running, each draft waiting out its grace, each payment
-- attempt still to come), its cycle columns and its statuses.
SET LOCAL lock_timeout = '5s';

-- ── Scheduled events ─────────────────────────────────────────────────────────────────────────
CREATE TYPE "ScheduledEventTypeEnum" AS ENUM (
    'FINALIZE_INVOICE',
    'RETRY_PAYMENT',
    'CANCEL_SUBSCRIPTION',
    'PAUSE_SUBSCRIPTION',
    'APPLY_PLAN_CHANGE',
    'END_TRIAL',
    'APPLY_AMENDMENT'
    );
CREATE TYPE "ScheduledEventStatus" AS ENUM ('PENDING', 'PROCESSING', 'COMPLETED', 'FAILED', 'CANCELED');

CREATE TABLE scheduled_event
(
    id                  UUID PRIMARY KEY,
    subscription_id     UUID                     NOT NULL REFERENCES subscription (id) ON DELETE CASCADE,
    tenant_id           UUID                     NOT NULL,
    event_type          "ScheduledEventTypeEnum" NOT NULL,
    scheduled_time      TIMESTAMP                NOT NULL,
    priority            INTEGER                  NOT NULL,
    event_data          JSONB                    NOT NULL,
    created_at          TIMESTAMP                NOT NULL,
    updated_at          TIMESTAMP default CURRENT_TIMESTAMP NOT NULL,
    status              "ScheduledEventStatus"   NOT NULL,
    retries             INTEGER                  NOT NULL DEFAULT 0,
    last_retry_at       TIMESTAMP,
    error               TEXT,
    processed_at        TIMESTAMP,
    source              TEXT                     NOT NULL,
    created_by_customer BOOLEAN                  NOT NULL DEFAULT false,
    actor_type          "ActorTypeEnum"          NOT NULL DEFAULT 'SYSTEM',
    actor_uuid          UUID,
    actor_alias         TEXT,
    next_attempt_at     TIMESTAMP
);

CREATE INDEX idx_scheduled_events_due ON scheduled_event (scheduled_time)
    WHERE status = 'PENDING';
CREATE INDEX idx_scheduled_events_subscription_id ON scheduled_event (subscription_id);
CREATE INDEX idx_scheduled_events_tenant_id ON scheduled_event (tenant_id);
CREATE INDEX idx_scheduled_events_lookup
    ON scheduled_event (subscription_id, scheduled_time)
    WHERE status = 'PENDING' AND processed_at IS NULL;

INSERT INTO scheduled_event (id, subscription_id, tenant_id, event_type, scheduled_time, priority,
                             event_data, created_at, status, source, created_by_customer,
                             actor_type, actor_uuid, actor_alias)
SELECT gen_random_uuid(), e.subscription_id, e.tenant_id, 'CANCEL_SUBSCRIPTION',
       date_trunc('day', e.at), 0,
       jsonb_build_object('CancelSubscription', jsonb_build_object(
           'reason', e.reason,
           'usage_cutoff', CASE WHEN e.at > date_trunc('day', e.at)
                                THEN to_char(e.at, 'YYYY-MM-DD"T"HH24:MI:SS.US') END)),
       e.requested_at, 'PENDING', 'rollback', e.actor_type = 'CUSTOMER', e.actor_type,
       e.actor_uuid, e.actor_alias
FROM subscription_ending e
JOIN subscription s ON s.id = e.subscription_id
WHERE e.kind = 'CANCEL' AND s.status NOT IN ('CANCELLED', 'COMPLETED', 'ABORTED');

INSERT INTO scheduled_event (id, subscription_id, tenant_id, event_type, scheduled_time, priority,
                             event_data, created_at, status, source)
SELECT gen_random_uuid(), t.subscription_id, t.tenant_id, 'END_TRIAL',
       t.until_date::timestamp, 0, '"EndTrial"'::jsonb, t.created_at, 'PENDING', 'rollback'
FROM subscription_trial t
JOIN subscription s ON s.id = t.subscription_id
WHERE NOT t.free AND s.status = 'TRIAL_ACTIVE';

INSERT INTO scheduled_event (id, subscription_id, tenant_id, event_type, scheduled_time, priority,
                             event_data, created_at, status, source)
SELECT gen_random_uuid(), i.subscription_id, i.tenant_id, 'FINALIZE_INVOICE', i.finalize_after,
       0, jsonb_build_object('FinalizeInvoice', jsonb_build_object('invoice_id', i.id)),
       i.created_at, 'PENDING', 'rollback'
FROM invoice i
WHERE i.finalize_after IS NOT NULL AND i.status = 'DRAFT' AND i.subscription_id IS NOT NULL;

INSERT INTO scheduled_event (id, subscription_id, tenant_id, event_type, scheduled_time, priority,
                             event_data, created_at, status, source)
SELECT gen_random_uuid(), i.subscription_id, i.tenant_id, 'RETRY_PAYMENT',
       i.next_payment_attempt_at, 0,
       jsonb_build_object('RetryPayment', jsonb_build_object('invoice_id', i.id)),
       i.created_at, 'PENDING', 'dunning'
FROM invoice i
WHERE i.next_payment_attempt_at IS NOT NULL AND i.status = 'FINALIZED'
  AND i.subscription_id IS NOT NULL;

-- The change history the previous release left, back as its events.
INSERT INTO scheduled_event (id, subscription_id, tenant_id, event_type, scheduled_time, priority,
                             event_data, created_at, updated_at, status, error, source,
                             created_by_customer, actor_type, actor_uuid, actor_alias)
SELECT c.id, c.subscription_id, c.tenant_id,
       CASE c.kind
           WHEN 'PLAN_CHANGE' THEN 'APPLY_PLAN_CHANGE'::"ScheduledEventTypeEnum"
           ELSE 'APPLY_AMENDMENT'::"ScheduledEventTypeEnum"
       END,
       c.effective_at, 0, c.change, c.created_at, c.updated_at,
       ((regexp_match(c.withdrawn_reason, '^previous release (\w+)'))[1])::"ScheduledEventStatus",
       (regexp_match(c.withdrawn_reason, '^previous release \w+: (.*)$'))[1],
       c.source, c.created_by_customer, c.actor_type, c.actor_uuid, c.actor_alias
FROM subscription_change c
WHERE c.withdrawn_reason ~ '^previous release (COMPLETED|CANCELED|FAILED)';
DELETE FROM subscription_change
WHERE withdrawn_reason ~ '^previous release (COMPLETED|CANCELED|FAILED)';

-- ── Cycle columns ────────────────────────────────────────────────────────────────────────────
CREATE TYPE "CycleActionEnum" AS ENUM (
    'ACTIVATE_SUBSCRIPTION',
    'RENEW_SUBSCRIPTION',
    'END_TRIAL',
    'END_SUBSCRIPTION'
);

ALTER TABLE subscription
    ADD COLUMN cycle_index           INTEGER           NULL,
    ADD COLUMN next_cycle_action     "CycleActionEnum" NULL,
    ADD COLUMN processing_started_at TIMESTAMP         NULL,
    ADD COLUMN last_error            TEXT              NULL,
    ADD COLUMN error_count           INTEGER           NOT NULL DEFAULT 0,
    ADD COLUMN next_retry            TIMESTAMP         NULL;

-- ── Statuses ─────────────────────────────────────────────────────────────────────────────────
-- The statuses come back empty: rows moved by the up migration keep their new status.

DROP TRIGGER trg_subscription_set_activated_at ON subscription;
DROP INDEX IF EXISTS idx_subscription_lifecycle_claim;
DROP INDEX IF EXISTS subscription_live_customer_idx;

ALTER TYPE "SubscriptionStatusEnum" RENAME TO "SubscriptionStatusEnum_new";
CREATE TYPE "SubscriptionStatusEnum" AS ENUM (
    'PENDING_ACTIVATION',
    'PENDING_CHARGE',
    'TRIAL_ACTIVE',
    'ACTIVE',
    'TRIAL_EXPIRED',
    'PAUSED',
    'SUSPENDED',
    'CANCELLED',
    'COMPLETED',
    'SUPERSEDED',
    'ERRORED',
    'ABORTED'
);
ALTER TABLE subscription
    ALTER COLUMN status TYPE "SubscriptionStatusEnum" USING status::text::"SubscriptionStatusEnum";
DROP TYPE "SubscriptionStatusEnum_new";

CREATE TRIGGER trg_subscription_set_activated_at
    BEFORE INSERT OR UPDATE ON subscription
    FOR EACH ROW
    WHEN (NEW.status = 'ACTIVE' AND NEW.activated_at IS NULL)
    EXECUTE FUNCTION set_activated_at_on_active();

CREATE INDEX idx_subscription_lifecycle_claim
    ON subscription (current_period_end, processing_started_at, next_retry)
    WHERE status IN ('PENDING_ACTIVATION', 'PENDING_CHARGE', 'TRIAL_ACTIVE', 'ACTIVE', 'PAUSED')
      AND current_period_end IS NOT NULL
      AND NOT lifecycle_suspended;

CREATE INDEX subscription_live_customer_idx
    ON subscription (tenant_id, customer_id)
    WHERE status IN ('ACTIVE', 'TRIAL_ACTIVE', 'PENDING_CHARGE');
