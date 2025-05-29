-- Drop tables
DROP TABLE IF EXISTS scheduled_event;

-- Remove columns from subscriptions table
ALTER TABLE subscription
--   DROP COLUMN IF EXISTS pending_plan_version_id,
--   DROP COLUMN IF EXISTS pending_plan_change_date,
  DROP COLUMN IF EXISTS cycle_index,
  DROP COLUMN IF EXISTS status,
  DROP COLUMN IF EXISTS current_period_start,
  DROP COLUMN IF EXISTS current_period_end,
  DROP COLUMN IF EXISTS next_cycle_action,
  DROP COLUMN IF EXISTS last_error,
  DROP COLUMN IF EXISTS error_count,
  DROP COLUMN IF EXISTS next_retry;

ALTER TABLE subscription
  ADD COLUMN IF NOT EXISTS canceled_at         TIMESTAMP,
ADD COLUMN IF NOT EXISTS cancellation_reason         TEXT;


-- Drop enum types
DROP TYPE IF EXISTS "ScheduledEventStatus";
DROP TYPE IF EXISTS "ScheduledEventTypeEnum";
DROP TYPE IF EXISTS "CycleActionEnum";
DROP TYPE IF EXISTS "SubscriptionStatusEnum";

ALTER TABLE historical_rates_from_usd
  DROP COLUMN updated_at;

ALTER TABLE customer
  ALTER COLUMN balance_value_cents TYPE integer;

ALTER TABLE customer_balance_tx
  ALTER COLUMN amount_cents TYPE integer,
  ALTER COLUMN balance_cents_after TYPE integer;

ALTER TABLE customer_balance_pending_tx
  ALTER COLUMN amount_cents TYPE integer;

SELECT pgmq.drop_queue('billable_metric_sync');
