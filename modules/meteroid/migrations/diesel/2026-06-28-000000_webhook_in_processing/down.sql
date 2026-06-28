DROP INDEX IF EXISTS webhook_in_event_provider_event_id_uq;

ALTER TABLE webhook_in_event
  ADD COLUMN processed BOOLEAN NOT NULL DEFAULT false,
  DROP COLUMN IF EXISTS processed_at,
  DROP COLUMN IF EXISTS event_id;

SELECT pgmq.drop_queue('webhook_in');
