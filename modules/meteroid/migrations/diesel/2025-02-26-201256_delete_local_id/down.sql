ALTER TABLE "invoicing_entity"
  ADD COLUMN "local_id" TEXT NOT NULL DEFAULT gen_random_uuid()::TEXT,
  ADD UNIQUE ("tenant_id", "local_id");

ALTER TABLE "add_on"
  ADD COLUMN "local_id" TEXT NOT NULL DEFAULT gen_random_uuid()::TEXT,
  ADD UNIQUE ("tenant_id", "local_id");

ALTER TABLE "bank_account"
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

ALTER TABLE "outbox_event"
  ADD COLUMN "local_id" TEXT NOT NULL DEFAULT gen_random_uuid()::TEXT,
  ADD UNIQUE ("tenant_id", "local_id");

ALTER TABLE "product_family"
  ADD COLUMN "local_id" TEXT NOT NULL DEFAULT gen_random_uuid()::TEXT,
  ADD UNIQUE ("tenant_id", "local_id");

ALTER TABLE "product"
  ADD COLUMN "local_id" TEXT NOT NULL DEFAULT gen_random_uuid()::TEXT,
  ADD UNIQUE ("tenant_id", "local_id");

ALTER TABLE "price_component"
  ADD COLUMN "local_id" TEXT NOT NULL DEFAULT gen_random_uuid()::TEXT,
  ADD UNIQUE ("plan_version_id", "local_id");

ALTER TABLE "plan"
  ADD COLUMN "local_id" TEXT NOT NULL DEFAULT gen_random_uuid()::TEXT,
  ADD UNIQUE ("tenant_id", "local_id");
