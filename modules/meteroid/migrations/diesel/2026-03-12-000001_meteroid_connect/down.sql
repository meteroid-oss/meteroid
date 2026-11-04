-- Rollback Meteroid Connect migration
-- Drop columns and tables in reverse dependency order

-- Drop bidirectional FK columns first to remove cross-dependencies
ALTER TABLE customer
  DROP COLUMN IF EXISTS connected_account_id;
ALTER TABLE connect_connected_account
  DROP COLUMN IF EXISTS platform_customer_id;

-- Drop tables in reverse order of dependencies
DROP TABLE IF EXISTS connect_onboarding_link;
DROP TABLE IF EXISTS connect_platform_settings;
DROP TABLE IF EXISTS connect_connected_account;
DROP TABLE IF EXISTS connect_refresh_token;
DROP TABLE IF EXISTS connect_access_token;
DROP TABLE IF EXISTS connect_authorization_code;
DROP TABLE IF EXISTS connect_oauth_app;

ALTER TABLE organization DROP COLUMN IF EXISTS is_express;

DROP TYPE IF EXISTS "ConnectOnboardingModeEnum";
DROP TYPE IF EXISTS "ConnectConnectionStatusEnum";
DROP TYPE IF EXISTS "ConnectConnectionTypeEnum";
