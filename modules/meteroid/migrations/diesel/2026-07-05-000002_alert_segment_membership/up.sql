-- Per-customer segment alerts: an alert_event can now target one customer, and
-- alert_segment_member tracks which customers are currently inside a segment
-- definition so the worker can diff membership (enter -> fire, leave -> resolve).

ALTER TABLE alert_event ADD COLUMN customer_id UUID REFERENCES customer(id) ON DELETE CASCADE;

-- The (definition, window) dedup only holds for aggregate events; per-customer
-- events dedup per (definition, window, customer).
ALTER TABLE alert_event DROP CONSTRAINT alert_event_alert_definition_id_window_date_key;
CREATE UNIQUE INDEX idx_alert_event_def_window
    ON alert_event(alert_definition_id, window_date) WHERE customer_id IS NULL;
CREATE UNIQUE INDEX idx_alert_event_def_window_customer
    ON alert_event(alert_definition_id, window_date, customer_id) WHERE customer_id IS NOT NULL;
CREATE INDEX idx_alert_event_customer ON alert_event(tenant_id, customer_id) WHERE customer_id IS NOT NULL;

CREATE TABLE alert_segment_member (
    alert_definition_id UUID NOT NULL REFERENCES alert_definition(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    customer_id UUID NOT NULL REFERENCES customer(id) ON DELETE CASCADE,
    entered_at TIMESTAMP NOT NULL DEFAULT now(),
    left_at TIMESTAMP,
    PRIMARY KEY (alert_definition_id, customer_id)
);

CREATE INDEX idx_alert_segment_member_customer ON alert_segment_member(tenant_id, customer_id);
