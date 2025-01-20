
DROP TABLE IF EXISTS "bank_account" CASCADE;
 
DROP TYPE IF EXISTS "BankAccountFormat";

ALTER TABLE "invoicing_entity"
  DROP COLUMN IF EXISTS "cc_provider_id",
  DROP COLUMN IF EXISTS "bank_account_id";
 