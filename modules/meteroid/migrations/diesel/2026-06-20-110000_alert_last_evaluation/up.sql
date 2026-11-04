-- Cache each definition's most recent evaluation so a Signal shows its current
-- value/status even when not firing (the worker writes these every run).
ALTER TABLE alert_definition ADD COLUMN last_evaluated_at TIMESTAMP;
ALTER TABLE alert_definition ADD COLUMN last_active BOOLEAN;
ALTER TABLE alert_definition ADD COLUMN last_value DOUBLE PRECISION;
