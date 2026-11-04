CREATE TYPE "AlertConditionKindEnum" AS ENUM('THRESHOLD', 'PCT_CHANGE', 'ANOMALY', 'LIMIT');
CREATE TYPE "AlertSeverityEnum" AS ENUM('INFO', 'WARNING', 'CRITICAL');
CREATE TYPE "AlertEventStatusEnum" AS ENUM('FIRING', 'ACKNOWLEDGED', 'RESOLVED');

CREATE TABLE alert_definition (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    metric_key TEXT NOT NULL,
    condition_kind "AlertConditionKindEnum" NOT NULL,
    condition JSONB NOT NULL,
    grain TEXT NOT NULL DEFAULT 'day',
    filters JSONB,
    cohort JSONB,
    channels JSONB NOT NULL DEFAULT '[]',
    severity "AlertSeverityEnum" NOT NULL DEFAULT 'WARNING',
    cooldown_secs INT4 NOT NULL DEFAULT 86400,
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMP NOT NULL DEFAULT now(),
    updated_at TIMESTAMP,
    archived_at TIMESTAMP
);

CREATE INDEX idx_alert_definition_tenant ON alert_definition(tenant_id);

CREATE TABLE alert_event (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    alert_definition_id UUID NOT NULL REFERENCES alert_definition(id) ON DELETE CASCADE,
    fired_at TIMESTAMP NOT NULL DEFAULT now(),
    window_date DATE NOT NULL,
    metric_value DOUBLE PRECISION NOT NULL,
    baseline_value DOUBLE PRECISION,
    score DOUBLE PRECISION,
    severity "AlertSeverityEnum" NOT NULL,
    status "AlertEventStatusEnum" NOT NULL DEFAULT 'FIRING',
    payload JSONB NOT NULL DEFAULT '{}',
    delivered_at TIMESTAMP,
    acknowledged_at TIMESTAMP,
    UNIQUE (alert_definition_id, window_date)
);

CREATE INDEX idx_alert_event_tenant ON alert_event(tenant_id);
