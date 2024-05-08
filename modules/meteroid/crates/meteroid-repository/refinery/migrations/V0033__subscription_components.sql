CREATE TYPE "SubscriptionFeeBillingPeriod" AS ENUM (
    'ONE_TIME', 'MONTHLY', 'QUARTERLY', 'ANNUAL'
    );


CREATE TABLE "subscription_component"
(
    "id"                 UUID                         NOT NULL PRIMARY KEY,
    "name"              TEXT                 NOT NULL,
    "subscription_id"    UUID                         NOT NULL REFERENCES "subscription" ("id") ON DELETE CASCADE,
    "price_component_id" UUID                         NULL REFERENCES "price_component" ("id") ON DELETE CASCADE,
    "product_item_id"    UUID                         NULL REFERENCES "product" ("id") ON DELETE CASCADE,
    "period"             "SubscriptionFeeBillingPeriod" NOT NULL,
    "fee"                JSONB NOT NULL
);


-- Create enum type "TenantEnvironmentEnum"
CREATE TYPE "TenantEnvironmentEnum" AS ENUM (
    'PRODUCTION', 'STAGING', 'QA', 'DEVELOPMENT', 'SANDBOX', 'DEMO'
    );
-- Modify "product" table
ALTER TABLE "product"
    ALTER COLUMN "product_family_id" SET NOT NULL;
-- Modify "tenant" table
ALTER TABLE "tenant"
    ADD COLUMN "environment" "TenantEnvironmentEnum" NOT NULL DEFAULT 'DEVELOPMENT';

ALTER TABLE "subscription"
    ADD COLUMN "currency" VARCHAR(3) NOT NULL DEFAULT 'USD';