create table if not exists outbox_event (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    aggregate_id TEXT NOT NULL,
    aggregate_type TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT (now() at time zone 'utc')
);
