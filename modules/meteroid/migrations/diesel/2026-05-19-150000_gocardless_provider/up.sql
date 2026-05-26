-- Add GOCARDLESS variant to ConnectorProviderEnum. Bank-debit provider
-- with mandate-based off-session charging; integrated via Billing Request
-- Flow (hosted redirect).
ALTER TYPE "ConnectorProviderEnum" ADD VALUE IF NOT EXISTS 'GOCARDLESS';
