
ALTER TABLE provider_config DROP COLUMN wh_endpoint_uid;

CREATE INDEX "provider_config_tenant_id" ON "provider_config"("tenant_id");