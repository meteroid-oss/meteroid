-- Settings > Customer emails > Activity > Skipped: a tenant's skip entries, newest first,
-- keyset-paginated. Partial, as skips are a sliver of entity_activity; the literal list must
-- match the query's (EntityActivityRow::list_skipped_emails) for the planner to use it.
--
-- CONCURRENTLY, so writers are never locked out (metadata.toml sets
-- run_in_transaction = false). This file must stay a SINGLE statement. If a run fails it
-- leaves an INVALID index that IF NOT EXISTS will not replace: drop it by hand, then re-run.
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_entity_activity_email_skipped
    ON entity_activity (tenant_id, occurred_at DESC, id DESC)
    WHERE activity_type IN ('entity.email_skipped', 'payment_reminder.skipped');
