-- Add the Tax connector type and the Kintsugi tax provider, so a tax provider can
-- be configured per invoicing entity like any other connector (encrypted
-- credentials, nexus, ...). The provider engine impl is added separately.
ALTER TYPE "ConnectorTypeEnum" ADD VALUE IF NOT EXISTS 'TAX';
ALTER TYPE "ConnectorProviderEnum" ADD VALUE IF NOT EXISTS 'KINTSUGI';
