-- Signals scaling (Phase 0+1).
--
-- 1. Bound the per-tick membership read. The worker only needs currently-inside
--    members plus exits still inside their re-entry cooldown; this index lets the
--    read seek (definition, left_at) instead of scanning every lifetime row.
CREATE INDEX idx_alert_segment_member_def_left
    ON alert_segment_member (alert_definition_id, left_at);

-- 2. Support the tombstone purge: a partial index over exited rows so the periodic
--    `DELETE ... WHERE left_at IS NOT NULL AND left_at < horizon` stays cheap.
CREATE INDEX idx_alert_segment_member_left_at
    ON alert_segment_member (left_at) WHERE left_at IS NOT NULL;

-- 3. Configurable evaluation cadence per tenant. NULL = grain-derived default
--    (minute/hour grains evaluate every tick; day grain evaluates hourly). A value
--    is the minimum seconds between evaluations of a tenant's definitions, so a
--    tenant can choose e.g. hourly (3600) or daily (86400) incident batching.
ALTER TABLE tenant ADD COLUMN alert_evaluation_interval_secs INT4;
