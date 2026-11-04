-- B2C person layer for customers (additive, non-breaking).
-- COMPANY is the default so every existing row keeps its current behavior.
-- INDIVIDUAL carries a natural person (first/last name).
CREATE TYPE "CustomerTypeEnum" AS ENUM ('COMPANY', 'INDIVIDUAL');

ALTER TABLE customer
  ADD COLUMN customer_type "CustomerTypeEnum" NOT NULL DEFAULT 'COMPANY',
  ADD COLUMN first_name TEXT,
  ADD COLUMN last_name TEXT;
