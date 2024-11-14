-- Your SQL goes here


ALTER TABLE "billable_metric"
  ADD COLUMN "product_id" UUID REFERENCES "product" ("id");

ALTER TABLE "plan"
  ADD COLUMN "active_version_id" UUID REFERENCES "plan_version" ("id");
ALTER TABLE "plan"
  ADD COLUMN "draft_version_id" UUID REFERENCES "plan_version" ("id");

ALTER TABLE "plan_version"
  DROP COLUMN "billing_periods";

ALTER TABLE "price_component"
  RENAME COLUMN "product_item_id" TO "product_id";

ALTER TABLE "subscription_component"
  RENAME COLUMN "product_item_id" TO "product_id";

ALTER TABLE "add_on"
  ADD COLUMN "local_id" TEXT NOT NULL DEFAULT gen_random_uuid()::TEXT,
  ADD UNIQUE ("tenant_id", "local_id");


ALTER TABLE "billable_metric"
  ADD COLUMN "local_id" TEXT NOT NULL DEFAULT gen_random_uuid()::TEXT,
  ADD UNIQUE ("tenant_id", "local_id");


ALTER TABLE "coupon"
  ADD COLUMN "local_id" TEXT NOT NULL DEFAULT gen_random_uuid()::TEXT,
  ADD UNIQUE ("tenant_id", "local_id");

ALTER TABLE "credit_note"
  ADD COLUMN "local_id" TEXT NOT NULL DEFAULT gen_random_uuid()::TEXT,
  ADD UNIQUE ("tenant_id", "local_id");

-- ALTER TABLE "customer"
--   RENAME COLUMN "alias" TO "external_id";
ALTER TABLE "customer"
  ADD COLUMN "local_id" TEXT NOT NULL DEFAULT gen_random_uuid()::TEXT,
  ADD UNIQUE ("tenant_id", "local_id");

ALTER TABLE "plan"
  RENAME COLUMN "external_id" TO "local_id";
ALTER INDEX "plan_tenant_id_external_id_key" RENAME TO "plan_tenant_id_local_id_key";


-- TODO what is the unicity we want here ?
-- same id when upgrading a plan ? probably
ALTER TABLE "price_component"
  ADD COLUMN "local_id" TEXT NOT NULL DEFAULT gen_random_uuid()::TEXT,
  ADD UNIQUE ("plan_version_id", "local_id");

ALTER TABLE "product"
  ADD COLUMN "local_id" TEXT NOT NULL DEFAULT gen_random_uuid()::TEXT,
  ADD UNIQUE ("tenant_id", "local_id");

ALTER TABLE "product_family"
  RENAME COLUMN "external_id" TO "local_id";
ALTER INDEX "product_family_external_id_tenant_id_key" RENAME TO "product_family_tenant_id_local_id_key";
DROP INDEX if exists "product_family_api_name_tenant_id_key";

ALTER TABLE "subscription"
  ADD COLUMN "local_id" TEXT NOT NULL DEFAULT gen_random_uuid()::TEXT,
  ADD UNIQUE ("tenant_id", "local_id");


-- ALTER TABLE "subscription_add_on"
--   ADD COLUMN "local_id" TEXT NOT NULL DEFAULT gen_random_uuid()::TEXT,
--   ADD UNIQUE ("tenant_id", "local_id");
--
-- ALTER TABLE "subscription_component"
--   ADD COLUMN "local_id" TEXT NOT NULL DEFAULT gen_random_uuid()::TEXT,
--   ADD UNIQUE ("tenant_id", "local_id");


