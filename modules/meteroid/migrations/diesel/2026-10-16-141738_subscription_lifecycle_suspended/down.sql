DROP INDEX IF EXISTS idx_subscription_lifecycle_claim;
DROP TRIGGER IF EXISTS trg_tenant_sync_subscription_lifecycle_suspended ON tenant;
DROP FUNCTION IF EXISTS sync_subscription_lifecycle_suspended();
ALTER TABLE subscription DROP COLUMN IF EXISTS lifecycle_suspended;

CREATE INDEX IF NOT EXISTS idx_subscription_lifecycle_due
  ON subscription (current_period_end, processing_started_at, next_retry)
  WHERE status IN ('PENDING_CHARGE', 'PENDING_ACTIVATION', 'ACTIVE', 'TRIAL_ACTIVE', 'PAUSED')
    AND current_period_end IS NOT NULL;
