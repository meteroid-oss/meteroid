-- Async processing queue for inbound webhooks.
SELECT pgmq.create('webhook_in');

-- Dedup + processing-state tracking on the inbound webhook audit table.
-- `processed_at` is the single source of truth for "has this been handled"
-- (NULL = not yet); it replaces the redundant `processed` boolean.
ALTER TABLE webhook_in_event
  ADD COLUMN event_id     TEXT,
  ADD COLUMN processed_at TIMESTAMPTZ,
  DROP COLUMN processed;

-- Idempotency on the provider event id (e.g. Stripe `evt_...`).
-- NULLs are distinct in Postgres, so providers/events without an id are unaffected
-- while repeated deliveries of the same event are deduped.
CREATE UNIQUE INDEX webhook_in_event_provider_event_id_uq
  ON webhook_in_event (provider_config_id, event_id);
