-- How often each person opened a quote's portal link. Not evidence: the evidence keeps one view per revision.
CREATE TABLE quote_recipient_engagement (
    id                   UUID PRIMARY KEY,
    tenant_id            UUID NOT NULL REFERENCES tenant(id) ON DELETE CASCADE,
    quote_id             UUID NOT NULL REFERENCES quote(id) ON DELETE CASCADE,
    email                TEXT NOT NULL,
    view_count           INTEGER NOT NULL CHECK (view_count >= 1),
    first_viewed_at      TIMESTAMPTZ NOT NULL,
    last_viewed_at       TIMESTAMPTZ NOT NULL,
    last_viewed_revision INTEGER NOT NULL
);
CREATE UNIQUE INDEX quote_recipient_engagement_email_idx ON quote_recipient_engagement (quote_id, lower(email));

-- Opens before this table existed: the evidence holds one view per person and revision, a lower bound.
INSERT INTO quote_recipient_engagement
    (id, tenant_id, quote_id, email, view_count, first_viewed_at, last_viewed_at, last_viewed_revision)
SELECT gen_random_uuid(), v.tenant_id, v.quote_id,
       (array_agg(trim(v.actor_email) ORDER BY v.occurred_at))[1],
       count(*), min(v.occurred_at), max(v.occurred_at),
       (array_agg(v.revision ORDER BY v.occurred_at DESC))[1]
FROM quote_evidence_event v
WHERE v.kind = 'viewed' AND v.actor_email IS NOT NULL
GROUP BY v.tenant_id, v.quote_id, lower(trim(v.actor_email));
