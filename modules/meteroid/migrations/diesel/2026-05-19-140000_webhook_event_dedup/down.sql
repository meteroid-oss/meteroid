DROP INDEX IF EXISTS webhook_in_event_dedup_idx;
ALTER TABLE webhook_in_event DROP COLUMN provider_event_id;
