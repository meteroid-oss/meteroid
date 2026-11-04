ALTER TABLE customer
  DROP COLUMN customer_type,
  DROP COLUMN first_name,
  DROP COLUMN last_name;

DROP TYPE "CustomerTypeEnum";
