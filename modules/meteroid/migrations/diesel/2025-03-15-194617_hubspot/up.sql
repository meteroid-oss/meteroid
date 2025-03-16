ALTER TYPE "ConnectorProviderEnum" ADD VALUE 'HUBSPOT';
ALTER TYPE "ConnectorTypeEnum" ADD VALUE 'CRM';

truncate table oauth_verifier;
alter table oauth_verifier drop column is_signup;
alter table oauth_verifier drop column invite_key;
alter table oauth_verifier add column data jsonb;
