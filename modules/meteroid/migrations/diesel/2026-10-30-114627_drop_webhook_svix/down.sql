CREATE TYPE "WebhookOutBackendEnum" AS ENUM ('SVIX', 'NATIVE');

CREATE TABLE webhook_out_tenant_backend (
    tenant_id UUID PRIMARY KEY REFERENCES tenant (id) ON DELETE CASCADE,
    backend "WebhookOutBackendEnum" NOT NULL,
    legacy_svix BOOLEAN NOT NULL DEFAULT false,
    updated_at TIMESTAMP NOT NULL DEFAULT (now() AT TIME ZONE 'UTC'),
    migrated_at TIMESTAMP,
    migration_error TEXT
);

CREATE TABLE webhook_out_auto_migration (
    id BOOLEAN PRIMARY KEY DEFAULT true CHECK (id),
    completed_at TIMESTAMP NOT NULL
);

-- Endpoints that sent `svix-*` headers get a placeholder, so code reading this column keeps
-- sending them.
ALTER TABLE webhook_out_endpoint ADD COLUMN svix_endpoint_id TEXT;
UPDATE webhook_out_endpoint SET svix_endpoint_id = 'legacy_' || id WHERE legacy_svix_headers;
CREATE UNIQUE INDEX idx_webhook_out_endpoint_svix ON webhook_out_endpoint (tenant_id, svix_endpoint_id) WHERE svix_endpoint_id IS NOT NULL AND archived_at IS NULL;
ALTER TABLE webhook_out_endpoint DROP COLUMN legacy_svix_headers;
