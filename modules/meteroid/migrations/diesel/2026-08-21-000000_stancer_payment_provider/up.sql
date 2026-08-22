-- Add STANCER variant to ConnectorProviderEnum for the Stancer card payment provider
ALTER TYPE "ConnectorProviderEnum" ADD VALUE IF NOT EXISTS 'STANCER';
