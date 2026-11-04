-- The activation derivation asks, per milestone source, "which rows of this
-- table did a real user create?" — entity_activity rows whose actor is not the
-- seeder. The planner hashes that subquery once rather than probing per row, so
-- without a covering index it seq-scans the whole table for every source.
--
-- Partial on actor_type keeps this at a fraction of the table (roughly a
-- quarter of rows are non-SYSTEM), and the four columns cover the subquery
-- outright. activity_type leads because the subquery's only scan-time
-- constants are activity_type and entity_type: tenant_id and entity_id
-- correlate to the outer query and become hash keys, not index conditions.
--
-- CONCURRENTLY, so writers are never locked out (metadata.toml sets
-- run_in_transaction = false). This file must stay a SINGLE statement:
-- Postgres wraps a multi-statement simple query in an implicit transaction,
-- which CONCURRENTLY rejects. If a run fails it leaves an INVALID index that
-- IF NOT EXISTS will not replace — drop it by hand, then re-run.
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_entity_activity_user_created
    ON entity_activity (activity_type, entity_type, tenant_id, entity_id)
    WHERE actor_type <> 'SYSTEM';
