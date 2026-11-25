-- The retention job forgets the v2 middleware's replay records (operation `v2`) older than 24
-- hours, oldest first, a bounded batch per run (ADR-0013 §3): it finds them by operation and claim
-- time. v1's keys and the store's own are kept, so the index leads with the operation: a run never
-- walks the kept rows to reach the ones it forgets.
--
-- Built in the transaction, not CONCURRENTLY: the table holds every key v1's credit and invoice
-- writes claimed since it was created (2026-10-08), one row per keyed request. Hundreds of
-- thousands of rows at most at the largest installation's volume, which a b-tree sorts in about a
-- second, ten million rows within the half-minute a deploy can lock for; the deploy stops the old
-- pods first, so the build blocks no write. Should one still serve, its long transaction fails
-- this run instead of queueing every query behind it; the next start retries.
SET LOCAL lock_timeout = '5s';

CREATE INDEX api_idempotency_key_operation_created_at_idx ON api_idempotency_key (operation, created_at);
