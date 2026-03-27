SELECT pgmq.drop_queue('billable_metric_sync');
ALTER TABLE billable_metric DROP COLUMN IF EXISTS synced_at;
ALTER TABLE billable_metric DROP COLUMN IF EXISTS sync_error;
