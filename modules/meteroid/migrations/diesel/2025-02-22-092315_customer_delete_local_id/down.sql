ALTER TABLE "customer"
  ADD COLUMN "local_id" TEXT NOT NULL DEFAULT gen_random_uuid()::TEXT,
  ADD UNIQUE ("tenant_id", "local_id");

