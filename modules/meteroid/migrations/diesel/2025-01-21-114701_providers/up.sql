create type "ConnectorProviderEnum" as enum ('STRIPE');
create type "ConnectorTypeEnum" as enum ('PAYMENT_PROVIDER');


ALTER TABLE "provider_config"
  RENAME TO "connector";


ALTER TABLE "connector"
  ADD COLUMN "alias"          TEXT                    NOT NULL DEFAULT gen_random_uuid()::TEXT,
  ADD COLUMN "connector_type" "ConnectorTypeEnum"     NOT NULL DEFAULT 'PAYMENT_PROVIDER',
  ADD COLUMN "provider"       "ConnectorProviderEnum" NOT NULL DEFAULT 'STRIPE',
  ADD COLUMN "data"           jsonb,
  ADD COLUMN "sensitive"      TEXT,
  DROP COLUMN "enabled",
  DROP COLUMN "webhook_security",
  DROP COLUMN "api_security",
  DROP COLUMN "invoicing_provider";
ALTER TABLE "connector"
  ADD UNIQUE ("tenant_id", "alias");

ALTER TABLE "invoice"
  DROP COLUMN "invoicing_provider";

DROP TYPE "InvoicingProviderEnum";


