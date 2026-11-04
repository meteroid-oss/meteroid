-- Rollback Meteroid Connect migration

ALTER TABLE customer
  DROP COLUMN IF EXISTS connected_account_id;
ALTER TABLE organization DROP COLUMN IF EXISTS is_express;
