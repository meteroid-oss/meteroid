-- Add the Tax connector type and the TaxJar provider so a tax provider can be
-- configured per invoicing entity like any other connector (encrypted credentials, nexus, ...).
ALTER TYPE "ConnectorTypeEnum" ADD VALUE IF NOT EXISTS 'TAX';
ALTER TYPE "ConnectorProviderEnum" ADD VALUE IF NOT EXISTS 'TAXJAR';
