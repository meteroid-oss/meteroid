DROP TABLE IF EXISTS alert_segment_member;

DROP INDEX IF EXISTS idx_alert_event_customer;
DROP INDEX IF EXISTS idx_alert_event_def_window_customer;
DROP INDEX IF EXISTS idx_alert_event_def_window;
-- Per-customer events belong to the feature being rolled back and would violate
-- the restored (definition, window) uniqueness.
DELETE FROM alert_event WHERE customer_id IS NOT NULL;
ALTER TABLE alert_event DROP COLUMN customer_id;
ALTER TABLE alert_event ADD CONSTRAINT alert_event_alert_definition_id_window_date_key
    UNIQUE (alert_definition_id, window_date);
