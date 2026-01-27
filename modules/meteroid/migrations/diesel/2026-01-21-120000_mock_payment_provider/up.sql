-- Add MOCK variant to ConnectorProviderEnum for testing payment flows
ALTER TYPE "ConnectorProviderEnum" ADD VALUE IF NOT EXISTS 'MOCK';
