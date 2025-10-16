alter table billable_metric add column synced_at timestamp;
alter table billable_metric add column sync_error text;
