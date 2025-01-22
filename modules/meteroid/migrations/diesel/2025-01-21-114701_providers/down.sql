ALTER TABLE "connector"
  RENAME TO "provider_config";


create type "InvoicingProviderEnum" as enum ('STRIPE', 'MANUAL');

ALTER TABLE "invoice"
  ADD COLUMN "invoicing_provider" "InvoicingProviderEnum" NOT NULL DEFAULT 'MANUAL';

ALTER TABLE "provider_config"
  DROP COLUMN "alias",
  DROP COLUMN "connector_type",
  DROP COLUMN "provider",
  DROP COLUMN "data",
  DROP COLUMN "sensitive",
  ADD COLUMN invoicing_provider "InvoicingProviderEnum" not null,
  ADD COLUMN enabled            boolean default true    not null,
  ADD COLUMN webhook_security   jsonb                   not null,
  ADD COLUMN api_security       jsonb                   not null;

drop type "ConnectorProviderEnum";
drop type "ConnectorTypeEnum";

