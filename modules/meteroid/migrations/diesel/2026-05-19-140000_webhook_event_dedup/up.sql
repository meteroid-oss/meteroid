-- Add idempotency for inbound webhooks. The provider's own event id
-- (e.g. Stripe `evt_…`) is unique per (connector, event); the unique
-- index lets the DB enforce "process each event once" without any
-- application-level locking.
--
-- The column is nullable so existing rows survive the migration with NULL
-- values. PostgreSQL's default `NULLS DISTINCT` treatment means multiple
-- NULL rows are accepted under a unique index — safe on populated tables
-- and no partial WHERE predicate needed.
--
-- Why NOT a partial index (WHERE provider_event_id IS NOT NULL): Diesel's
-- `on_conflict((cols)).do_nothing()` generates an `ON CONFLICT (cols)`
-- clause without a WHERE predicate, which PostgreSQL refuses to match
-- against a partial unique index. The smaller-index optimisation isn't
-- worth blocking the upsert path.
ALTER TABLE webhook_in_event
    ADD COLUMN provider_event_id TEXT;

CREATE UNIQUE INDEX webhook_in_event_dedup_idx
    ON webhook_in_event (provider_config_id, provider_event_id);
