-- Every tenant is on native webhooks. Endpoints migrated from Svix keep sending the
-- `svix-*` signature headers their receivers may verify.
ALTER TABLE webhook_out_endpoint ADD COLUMN legacy_svix_headers BOOLEAN NOT NULL DEFAULT false;
UPDATE webhook_out_endpoint SET legacy_svix_headers = true WHERE svix_endpoint_id IS NOT NULL;

DROP INDEX idx_webhook_out_endpoint_svix;
ALTER TABLE webhook_out_endpoint DROP COLUMN svix_endpoint_id;

DROP TABLE webhook_out_auto_migration;
DROP TABLE webhook_out_tenant_backend;
DROP TYPE "WebhookOutBackendEnum";
