DROP TABLE IF EXISTS metric_target_snapshot;
ALTER TABLE alert_definition DROP COLUMN IF EXISTS muted_until;
ALTER TABLE alert_definition DROP COLUMN IF EXISTS owner_user_id;
DROP TABLE IF EXISTS metric_target_change;
DROP TYPE IF EXISTS "MetricTargetChangeKindEnum";
DROP TABLE IF EXISTS metric_target;
DROP TYPE IF EXISTS "MetricTargetPeriodGrainEnum";
