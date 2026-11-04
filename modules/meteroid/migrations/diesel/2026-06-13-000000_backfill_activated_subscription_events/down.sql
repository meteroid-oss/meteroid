-- No-op: a backfilled Activated event is indistinguishable from a genuinely
-- emitted one (same shape), so there is no safe way to delete only the
-- backfilled rows without dropping legitimate lifecycle history. The forward
-- migration is idempotent, so re-applying it after a rollback is harmless.
SELECT 1;
