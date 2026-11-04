CREATE TABLE analytics_saved_view (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    metric_key TEXT NOT NULL,
    visualization TEXT NOT NULL,
    config JSONB NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT now(),
    updated_at TIMESTAMP,
    archived_at TIMESTAMP
);

CREATE INDEX idx_analytics_saved_view_tenant ON analytics_saved_view(tenant_id);
