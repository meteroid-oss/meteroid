-- This file should undo anything in `up.sql`
ALTER TABLE customer_payment_method
  ALTER COLUMN "created_at" DROP DEFAULT,
  ALTER COLUMN "updated_at" DROP DEFAULT;
