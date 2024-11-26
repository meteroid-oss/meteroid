-- This file should undo anything in `up.sql`


ALTER TABLE "billable_metric"
  DROP COLUMN "product_id";

ALTER TABLE "plan"
  DROP COLUMN "active_version_id";

ALTER TABLE "plan"
  DROP COLUMN "draft_version_id";

ALTER TABLE "plan_version"
  ADD COLUMN "billing_periods" "BillingPeriodEnum"[] NOT NULL DEFAULT '{}';

ALTER TABLE "price_component"
  RENAME COLUMN "product_id" TO "product_item_id";

ALTER TABLE "subscription_component"
  RENAME COLUMN "product_id" TO "product_item_id";

ALTER TABLE "add_on"
  DROP COLUMN "local_id";

ALTER TABLE "billable_metric"
  DROP COLUMN "local_id";

ALTER TABLE "coupon"
  DROP COLUMN "local_id";

ALTER TABLE "credit_note"
  DROP COLUMN "local_id";

ALTER TABLE "customer"
  DROP COLUMN "local_id";
-- ALTER TABLE "customer"
--   RENAME COLUMN "external_id" TO "alias";

ALTER TABLE "plan"
  RENAME COLUMN "local_id" TO "external_id";

ALTER INDEX "plan_tenant_id_local_id_key" RENAME TO "plan_tenant_id_external_id_key";


ALTER TABLE "price_component"
  DROP COLUMN "local_id";

ALTER TABLE "product"
  DROP COLUMN "local_id";

ALTER INDEX "product_family_tenant_id_local_id_key" RENAME TO "product_family_external_id_tenant_id_key";
ALTER TABLE "product_family"
  RENAME COLUMN "local_id" TO "external_id";

ALTER TABLE "subscription"
  DROP COLUMN "local_id";
