-- One row per viewing session: opens within 30 minutes of the session's last open extend it.
CREATE TABLE quote_view (
    id           UUID PRIMARY KEY,
    tenant_id    UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    quote_id     UUID NOT NULL REFERENCES quote(id) ON DELETE CASCADE,
    email        TEXT NOT NULL,
    revision     INTEGER NOT NULL,
    viewed_at    TIMESTAMPTZ NOT NULL,
    last_seen_at TIMESTAMPTZ NOT NULL,
    ip_address   TEXT,
    user_agent   TEXT
);
CREATE INDEX quote_view_quote_idx ON quote_view (quote_id, viewed_at DESC);

-- Sessions before this table existed: the evidence keeps the first open per person and revision.
INSERT INTO quote_view
    (id, tenant_id, quote_id, email, revision, viewed_at, last_seen_at, ip_address, user_agent)
SELECT gen_random_uuid(), v.tenant_id, v.quote_id, trim(v.actor_email), v.revision,
       v.occurred_at, v.occurred_at, v.ip_address, v.user_agent
FROM quote_evidence_event v
WHERE v.kind = 'viewed' AND v.actor_email IS NOT NULL;
