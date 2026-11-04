CREATE TABLE segment (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    definition JSONB NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT now(),
    updated_at TIMESTAMP,
    archived_at TIMESTAMP
);

CREATE INDEX idx_segment_tenant ON segment(tenant_id);

CREATE UNIQUE INDEX idx_segment_tenant_name ON segment (tenant_id, name) WHERE archived_at IS NULL;
