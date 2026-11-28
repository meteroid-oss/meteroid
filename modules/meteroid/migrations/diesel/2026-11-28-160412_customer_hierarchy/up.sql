-- An account hierarchy: a customer can sit under a parent customer of the same tenant, e.g. the
-- accounts a contracting entity holds, each billed on its own.
ALTER TABLE "customer"
  ADD COLUMN "parent_customer_id" UUID REFERENCES "customer" ON DELETE SET NULL,
  ADD CONSTRAINT "customer_parent_not_self" CHECK ("parent_customer_id" <> "id");

CREATE INDEX "customer_parent_customer_id_idx" ON "customer" ("tenant_id", "parent_customer_id")
  WHERE "parent_customer_id" IS NOT NULL;
