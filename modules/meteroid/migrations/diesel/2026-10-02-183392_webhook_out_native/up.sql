CREATE TYPE "WebhookDeliveryStatusEnum" AS ENUM ('PENDING', 'IN_FLIGHT', 'SUCCEEDED', 'FAILED', 'CANCELLED');

CREATE TYPE "WebhookEndpointDisabledReasonEnum" AS ENUM ('MANUAL', 'AUTO_FAILURES', 'GONE');

CREATE TYPE "WebhookOutBackendEnum" AS ENUM ('SVIX', 'NATIVE');

-- Native outbound webhook destinations, one row per tenant endpoint. `secret` and
-- `previous_secret` hold signing keys encrypted with the store crypt key. `max_in_flight`
-- caps concurrent deliveries per endpoint and is defaulted from the tenant environment.
-- `headers` lists the custom headers sent with every delivery: `[{name, value, sensitive,
-- set}]`, where a sensitive entry never carries its value here (`set` says whether one
-- exists); sensitive values live encrypted in `sensitive_header_values` as a JSON map.
-- `svix_endpoint_id` is the Svix endpoint this one was migrated from, if any.
-- `unreachable_streak` counts consecutive attempts that got no usable answer (no
-- response, 429, 502-504); past a threshold the endpoint is paused until `paused_until`
-- and then probed one delivery at a time. `failing_since` starts the auto-disable window
-- at the first failure after a success. `rate_limit_per_sec` caps deliveries started per
-- second.
CREATE TABLE webhook_out_endpoint (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    url TEXT NOT NULL,
    description TEXT,
    event_types TEXT[] NOT NULL DEFAULT '{}',
    secret TEXT NOT NULL,
    previous_secret TEXT,
    previous_secret_expires_at TIMESTAMP,
    disabled BOOLEAN NOT NULL DEFAULT false,
    disabled_reason "WebhookEndpointDisabledReasonEnum",
    disabled_at TIMESTAMP,
    consecutive_failures INT NOT NULL DEFAULT 0,
    last_success_at TIMESTAMP,
    last_failure_at TIMESTAMP,
    created_at TIMESTAMP NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at TIMESTAMP,
    archived_at TIMESTAMP,
    max_in_flight INT NOT NULL DEFAULT 1 CHECK (max_in_flight BETWEEN 1 AND 64),
    headers JSONB NOT NULL DEFAULT '[]',
    sensitive_header_values TEXT,
    svix_endpoint_id TEXT,
    failing_since TIMESTAMP,
    unreachable_streak INT NOT NULL DEFAULT 0,
    paused_until TIMESTAMP,
    rate_limit_per_sec INT CHECK (rate_limit_per_sec > 0)
);

CREATE INDEX idx_webhook_out_endpoint_tenant ON webhook_out_endpoint (tenant_id) WHERE archived_at IS NULL;

CREATE UNIQUE INDEX idx_webhook_out_endpoint_svix ON webhook_out_endpoint (tenant_id, svix_endpoint_id) WHERE svix_endpoint_id IS NOT NULL AND archived_at IS NULL;

-- One row per emitted business event, shared by every endpoint it fans out to.
-- `id` is the outbox EventId so a redelivered outbox message stays idempotent.
CREATE TABLE webhook_out_message (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT (now() AT TIME ZONE 'UTC')
);

CREATE INDEX idx_webhook_out_message_tenant_created ON webhook_out_message (tenant_id, created_at DESC);

CREATE INDEX idx_webhook_out_message_created ON webhook_out_message (created_at);

-- High churn (insert, update, retention delete), hence the tighter autovacuum.
CREATE TABLE webhook_out_delivery (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    message_id UUID NOT NULL REFERENCES webhook_out_message (id) ON DELETE CASCADE,
    endpoint_id UUID NOT NULL REFERENCES webhook_out_endpoint (id) ON DELETE CASCADE,
    status "WebhookDeliveryStatusEnum" NOT NULL DEFAULT 'PENDING',
    attempt_count INT NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMP,
    lease_expires_at TIMESTAMP,
    last_response_status SMALLINT,
    last_error TEXT,
    manual BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMP NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    updated_at TIMESTAMP,
    completed_at TIMESTAMP,
    -- When the delivery was last claimed: what the per-endpoint rate limit counts.
    claimed_at TIMESTAMP,
    UNIQUE (message_id, endpoint_id)
) WITH (autovacuum_vacuum_scale_factor = 0.02, autovacuum_analyze_scale_factor = 0.02);

CREATE INDEX idx_webhook_out_delivery_endpoint_due ON webhook_out_delivery (endpoint_id, next_attempt_at) WHERE status = 'PENDING';

CREATE INDEX idx_webhook_out_delivery_inflight ON webhook_out_delivery (endpoint_id) WHERE status = 'IN_FLIGHT';

CREATE INDEX idx_webhook_out_delivery_endpoint_created ON webhook_out_delivery (endpoint_id, created_at DESC);

CREATE INDEX idx_webhook_out_delivery_tenant ON webhook_out_delivery (tenant_id);

CREATE INDEX idx_webhook_out_delivery_endpoint_claimed ON webhook_out_delivery (endpoint_id, claimed_at) WHERE claimed_at IS NOT NULL;

CREATE TABLE webhook_out_attempt (
    id UUID PRIMARY KEY,
    delivery_id UUID NOT NULL REFERENCES webhook_out_delivery (id) ON DELETE CASCADE,
    attempt_number INT NOT NULL,
    attempted_at TIMESTAMP NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    duration_ms INT NOT NULL,
    response_status SMALLINT,
    response_body TEXT,
    error TEXT,
    succeeded BOOLEAN NOT NULL
) WITH (autovacuum_vacuum_scale_factor = 0.05);

CREATE UNIQUE INDEX idx_webhook_out_attempt_delivery ON webhook_out_attempt (delivery_id, attempt_number);

-- Per-tenant backend during the Svix migration. No row means the instance default
-- (WEBHOOK_OUT_BACKEND). Kept out of `tenant` so tenant insert paths stay untouched.
-- `legacy_svix` marks a migrated tenant that still has endpoints only Svix can serve
-- (e.g. sensitive headers Svix never reveals): its events go to Svix as well, until the
-- tenant has moved them over. `migration_error` is set on a tenant the automatic
-- migration had to leave on SVIX; it is retried on the next start.
CREATE TABLE webhook_out_tenant_backend (
    tenant_id UUID PRIMARY KEY REFERENCES tenant (id) ON DELETE CASCADE,
    backend "WebhookOutBackendEnum" NOT NULL,
    legacy_svix BOOLEAN NOT NULL DEFAULT false,
    updated_at TIMESTAMP NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    migrated_at TIMESTAMP,
    migration_error TEXT
);

-- One row, written once the automatic migration has given every tenant Svix knows a
-- state of its own. Until then a tenant without one keeps delivering through Svix;
-- afterwards (and on an instance without Svix) such a tenant is native.
CREATE TABLE webhook_out_auto_migration (
    id BOOLEAN PRIMARY KEY DEFAULT true CHECK (id),
    completed_at TIMESTAMP NOT NULL
);
