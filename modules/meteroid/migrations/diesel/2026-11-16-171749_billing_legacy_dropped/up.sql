-- What the facts replace, retired: the statuses no fact gives, the cycle columns and the
-- scheduled_event table. Everything that table held is a fact or an invoice column:
--   FINALIZE_INVOICE, RETRY_PAYMENT: invoice.finalize_after, invoice.next_payment_attempt_at.
--   CANCEL_SUBSCRIPTION: the subscription's CANCEL ending.   END_TRIAL: subscription_trial.
--   APPLY_PLAN_CHANGE, APPLY_AMENDMENT: subscription_change, written as facts just before this
--   (`facts::pending::write_pending_scheduled_changes`).   PAUSE_SUBSCRIPTION: never created.
SET LOCAL lock_timeout = '5s';

-- ── Statuses ─────────────────────────────────────────────────────────────────────────────────
-- A subscription's status is derived from its facts, which give none of PENDING_CHARGE, PAUSED,
-- SUSPENDED, SUPERSEDED or ERRORED. Production held none of the first four, and 36 ERRORED rows,
-- all on sandbox tenants; a failure now stays on the billing controller's row.

-- The customers of the rows moved below run at once, as an input change makes them.
INSERT INTO billing_controller (tenant_id, customer_id, dirty_since, updated_at)
SELECT DISTINCT s.tenant_id, s.customer_id, now(), now()
FROM subscription s
WHERE s.status IN ('ERRORED', 'PAUSED', 'SUSPENDED', 'PENDING_CHARGE', 'SUPERSEDED')
ON CONFLICT (tenant_id, customer_id) DO UPDATE
SET dirty_since = excluded.dirty_since, updated_at = excluded.updated_at;

-- A status the controller then re-derives from the facts, at its next run.
UPDATE subscription s
SET status = CASE
        WHEN s.status = 'SUPERSEDED' THEN 'CANCELLED'::"SubscriptionStatusEnum"
        WHEN s.status = 'PENDING_CHARGE' THEN 'PENDING_ACTIVATION'::"SubscriptionStatusEnum"
        WHEN s.activated_at IS NOT NULL THEN 'ACTIVE'::"SubscriptionStatusEnum"
        ELSE 'PENDING_ACTIVATION'::"SubscriptionStatusEnum"
    END,
    next_wake_at = CASE WHEN s.status = 'SUPERSEDED' THEN s.next_wake_at ELSE now() END
WHERE s.status IN ('ERRORED', 'PAUSED', 'SUSPENDED', 'PENDING_CHARGE', 'SUPERSEDED');

-- Objects whose definition compares the status: rebuilt around the type swap.
DROP TRIGGER trg_subscription_set_activated_at ON subscription;
-- The cycle's claim index goes with its columns, next.
DROP INDEX IF EXISTS idx_subscription_lifecycle_claim;
DROP INDEX IF EXISTS subscription_live_customer_idx;

ALTER TYPE "SubscriptionStatusEnum" RENAME TO "SubscriptionStatusEnum_old";
CREATE TYPE "SubscriptionStatusEnum" AS ENUM (
    'PENDING_ACTIVATION',
    'TRIAL_ACTIVE',
    'ACTIVE',
    'TRIAL_EXPIRED',
    'CANCELLED',
    'COMPLETED',
    'ABORTED'
);
ALTER TABLE subscription
    ALTER COLUMN status TYPE "SubscriptionStatusEnum" USING status::text::"SubscriptionStatusEnum";
DROP TYPE "SubscriptionStatusEnum_old";

CREATE TRIGGER trg_subscription_set_activated_at
    BEFORE INSERT OR UPDATE ON subscription
    FOR EACH ROW
    WHEN (NEW.status = 'ACTIVE' AND NEW.activated_at IS NULL)
    EXECUTE FUNCTION set_activated_at_on_active();

-- Partial: the predicate must match the credit recompute's statuses.
CREATE INDEX subscription_live_customer_idx
    ON subscription (tenant_id, customer_id)
    WHERE status IN ('ACTIVE', 'TRIAL_ACTIVE');

-- ── Cycle columns ────────────────────────────────────────────────────────────────────────────
-- The billing controller derives a subscription's state from its facts: no cycle action, cycle
-- count or per-subscription claim and retry columns are read any more.
DROP INDEX IF EXISTS idx_subscription_lifecycle_claim;
DROP INDEX IF EXISTS idx_subscription_lifecycle_due;

ALTER TABLE subscription
    DROP COLUMN cycle_index,
    DROP COLUMN next_cycle_action,
    DROP COLUMN processing_started_at,
    DROP COLUMN last_error,
    DROP COLUMN error_count,
    DROP COLUMN next_retry;

DROP TYPE "CycleActionEnum";

-- ── Scheduled events ─────────────────────────────────────────────────────────────────────────
-- The migrations write every pending plan change and amendment as facts, or cancel it, before
-- this; one left pending refuses.
DO $$
DECLARE
    stray BIGINT;
BEGIN
    SELECT count(*) INTO stray
    FROM scheduled_event
    WHERE status IN ('PENDING', 'PROCESSING')
      AND event_type NOT IN ('FINALIZE_INVOICE', 'RETRY_PAYMENT', 'END_TRIAL', 'CANCEL_SUBSCRIPTION');
    IF stray > 0 THEN
        RAISE EXCEPTION '% pending scheduled events (plan changes, amendments or pauses) were never written as facts: run the release through meteroid''s migrations, which write them first', stray;
    END IF;
END $$;

-- An invoice deadline still pending as an event, carried by its column (already mirrored when the
-- event was scheduled; a draft or open invoice only).
UPDATE invoice i
SET finalize_after = e.scheduled_time
FROM (SELECT DISTINCT ON ((event_data -> 'FinalizeInvoice' ->> 'invoice_id')::uuid)
             (event_data -> 'FinalizeInvoice' ->> 'invoice_id')::uuid AS invoice_id,
             COALESCE(next_attempt_at, scheduled_time) AS scheduled_time
      FROM scheduled_event
      WHERE event_type = 'FINALIZE_INVOICE' AND status IN ('PENDING', 'PROCESSING')
      ORDER BY (event_data -> 'FinalizeInvoice' ->> 'invoice_id')::uuid, scheduled_time DESC) e
WHERE i.id = e.invoice_id AND i.status = 'DRAFT' AND i.finalize_after IS NULL;

UPDATE invoice i
SET next_payment_attempt_at = e.scheduled_time
FROM (SELECT DISTINCT ON ((event_data -> 'RetryPayment' ->> 'invoice_id')::uuid)
             (event_data -> 'RetryPayment' ->> 'invoice_id')::uuid AS invoice_id,
             COALESCE(next_attempt_at, scheduled_time) AS scheduled_time
      FROM scheduled_event
      WHERE event_type = 'RETRY_PAYMENT' AND status IN ('PENDING', 'PROCESSING')
      ORDER BY (event_data -> 'RetryPayment' ->> 'invoice_id')::uuid, scheduled_time DESC) e
WHERE i.id = e.invoice_id AND i.status = 'FINALIZED' AND i.payment_status <> 'PAID'
  AND i.next_payment_attempt_at IS NULL;

-- A cancellation still pending as an event is its ending (recorded with the event since the facts
-- migration): one missing is recorded, one recorded is kept.
INSERT INTO subscription_ending (subscription_id, tenant_id, kind, at, reason, actor_type,
                                 actor_uuid, actor_alias, created_at, requested_at)
SELECT DISTINCT ON (e.subscription_id)
       e.subscription_id, e.tenant_id, 'CANCEL',
       COALESCE((e.event_data -> 'CancelSubscription' ->> 'usage_cutoff')::timestamp,
                e.scheduled_time),
       e.event_data -> 'CancelSubscription' ->> 'reason',
       e.actor_type, e.actor_uuid, e.actor_alias, e.created_at, e.created_at
FROM scheduled_event e
JOIN subscription s ON s.id = e.subscription_id
WHERE e.event_type = 'CANCEL_SUBSCRIPTION'
  AND e.status IN ('PENDING', 'PROCESSING')
  AND s.status NOT IN ('CANCELLED', 'COMPLETED', 'ABORTED')
ORDER BY e.subscription_id, e.created_at DESC
ON CONFLICT (subscription_id, kind) DO NOTHING;

-- The plan changes and amendments the previous release applied, cancelled or gave up on (and those
-- of tenants not live the conversion cancelled): kept as the change history, withdrawn, so no
-- read takes them for a change still to come; the reason names what became of each.
INSERT INTO subscription_change (id, tenant_id, subscription_id, kind, effective_at, change,
                                 withdrawn_at, withdrawn_reason, source, created_by_customer,
                                 actor_type, actor_uuid, actor_alias, created_at, updated_at)
SELECT e.id, e.tenant_id, e.subscription_id,
       CASE e.event_type
           WHEN 'APPLY_PLAN_CHANGE' THEN 'PLAN_CHANGE'::"SubscriptionChangeKindEnum"
           ELSE 'AMENDMENT'::"SubscriptionChangeKindEnum"
       END,
       e.scheduled_time, e.event_data, e.updated_at,
       'previous release ' || e.status::text || COALESCE(': ' || e.error, ''),
       e.source, e.created_by_customer, e.actor_type, e.actor_uuid, e.actor_alias,
       e.created_at, e.updated_at
FROM scheduled_event e
WHERE e.event_type IN ('APPLY_PLAN_CHANGE', 'APPLY_AMENDMENT')
  AND e.status IN ('COMPLETED', 'CANCELED', 'FAILED')
ON CONFLICT (id) DO NOTHING;

DROP TABLE scheduled_event;
DROP TYPE "ScheduledEventTypeEnum";
DROP TYPE "ScheduledEventStatus";
