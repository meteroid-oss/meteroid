DROP INDEX IF EXISTS idx_scheduled_events_lookup;
DROP INDEX IF EXISTS idx_subscription_lifecycle_due;
DROP INDEX IF EXISTS idx_scheduled_event_time;
DROP INDEX IF EXISTS idx_bi_revenue_daily_plan_version;
DROP INDEX IF EXISTS idx_bi_revenue_daily_date;
ALTER TABLE subscription DROP COLUMN IF EXISTS processing_started_at;
