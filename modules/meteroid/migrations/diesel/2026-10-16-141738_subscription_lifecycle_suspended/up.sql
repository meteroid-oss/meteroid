-- Subscriptions of a non-ACTIVE tenant stay non-terminal forever (archived tenants are never
-- deleted), so they'd sit at the head of the claim index and be walked by every
-- lifecycle poll. This flag lets the index exclude them; the poll's tenant anti-join stays the
-- authority, so a stale flag only costs one skipped row.
ALTER TABLE subscription ADD COLUMN lifecycle_suspended BOOLEAN NOT NULL DEFAULT false;

UPDATE subscription s SET lifecycle_suspended = true
FROM tenant t
WHERE t.id = s.tenant_id AND t.status <> 'ACTIVE';

CREATE OR REPLACE FUNCTION sync_subscription_lifecycle_suspended() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE subscription SET lifecycle_suspended = (NEW.status <> 'ACTIVE')
    WHERE tenant_id = NEW.id AND lifecycle_suspended <> (NEW.status <> 'ACTIVE');
    RETURN NULL;
END $$;

CREATE TRIGGER trg_tenant_sync_subscription_lifecycle_suspended
    AFTER UPDATE OF status ON tenant
    FOR EACH ROW
    WHEN (OLD.status IS DISTINCT FROM NEW.status)
    EXECUTE FUNCTION sync_subscription_lifecycle_suspended();

-- The status list must match NOT_TERMINAL_STATUS_SQL in subscriptions_lifecycle.rs for the planner
-- to use it.
CREATE INDEX idx_subscription_lifecycle_claim
    ON subscription (current_period_end, processing_started_at, next_retry)
    WHERE status IN ('PENDING_ACTIVATION', 'PENDING_CHARGE', 'TRIAL_ACTIVE', 'ACTIVE', 'PAUSED')
      AND current_period_end IS NOT NULL
      AND NOT lifecycle_suspended;

DROP INDEX idx_subscription_lifecycle_due;
