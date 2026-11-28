DROP INDEX "customer_parent_customer_id_idx";
ALTER TABLE "customer"
  DROP CONSTRAINT "customer_parent_not_self",
  DROP COLUMN "parent_customer_id";
