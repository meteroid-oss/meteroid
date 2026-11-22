-- What an integration could not sync as it stands: kept until the document syncs, so the
-- merchant sees it and can retry once fixed.
CREATE TABLE connector_sync_issue (
    id            UUID PRIMARY KEY,
    tenant_id     UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    connector_id  UUID NOT NULL REFERENCES connector(id) ON DELETE CASCADE,
    resource_type TEXT NOT NULL CHECK (resource_type IN ('customer', 'invoice', 'credit_note')),
    resource_id   UUID NOT NULL,
    message       TEXT NOT NULL,
    occurrences   INTEGER NOT NULL DEFAULT 1,
    first_seen_at TIMESTAMPTZ NOT NULL,
    last_seen_at  TIMESTAMPTZ NOT NULL,
    UNIQUE (connector_id, resource_type, resource_id)
);
CREATE INDEX connector_sync_issue_tenant_idx
    ON connector_sync_issue (tenant_id, connector_id, last_seen_at DESC);

-- Credit notes only reached Pennylane through a manual sync.
SELECT pgmq.bind_topic('credit_note.finalized', 'pennylane_sync');
