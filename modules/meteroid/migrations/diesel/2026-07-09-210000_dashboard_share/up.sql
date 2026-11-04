CREATE TABLE IF NOT EXISTS analytics_dashboard_share (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    saved_view_id UUID NOT NULL REFERENCES analytics_saved_view(id) ON DELETE CASCADE,
    token TEXT NOT NULL,
    snapshot JSONB NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT now(),
    updated_at TIMESTAMP,
    UNIQUE (saved_view_id)
);

CREATE INDEX IF NOT EXISTS idx_analytics_dashboard_share_tenant ON analytics_dashboard_share(tenant_id);
