-- Modify "tenant" table
ALTER TABLE "tenant" DROP COLUMN "billing_config";
CREATE UNIQUE INDEX "provider_config_uniqueness_idx"
  ON "provider_config" (tenant_id, invoicing_provider)
  WHERE enabled = true;
