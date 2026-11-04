ALTER TABLE alert_event      ADD COLUMN driver_label TEXT;
ALTER TABLE alert_definition ADD COLUMN last_impact_cents BIGINT;
ALTER TABLE alert_definition ADD COLUMN last_preview JSONB;
