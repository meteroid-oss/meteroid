-- Add claim column for parallel processing
ALTER TABLE subscription ADD COLUMN processing_started_at TIMESTAMP;

-- Index for scheduled events lookup (replaces DATE() function query)
CREATE INDEX idx_scheduled_events_lookup
  ON scheduled_event (subscription_id, scheduled_time)
  WHERE status = 'PENDING' AND processed_at IS NULL;

-- Index for subscription lifecycle worker claims
-- Includes processing_started_at for efficient claim queries
CREATE INDEX idx_subscription_lifecycle_due
  ON subscription (current_period_end, processing_started_at, next_retry)
  WHERE status IN ('PENDING_CHARGE', 'PENDING_ACTIVATION', 'ACTIVE', 'TRIAL_ACTIVE', 'PAUSED')
    AND current_period_end IS NOT NULL;

-- Index for scheduled_event time-based queries
CREATE INDEX idx_scheduled_event_time
  ON scheduled_event (scheduled_time)
  WHERE status = 'PENDING';

-- Indexes for bi_revenue_daily queries
CREATE INDEX idx_bi_revenue_daily_plan_version
  ON bi_revenue_daily (plan_version_id);

CREATE INDEX idx_bi_revenue_daily_date
  ON bi_revenue_daily (revenue_date);
