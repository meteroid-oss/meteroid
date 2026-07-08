-- External (VIES) VAT validation state for customers.
-- PENDING: a format-valid, VIES-eligible number awaiting/needing (re)validation.
-- VALID/INVALID: a definitive VIES answer (terminal until the number changes).
-- UNAVAILABLE: VIES could not be reached after retries (fail-open in billing).
CREATE TYPE "CustomerVatValidationStatusEnum" AS ENUM ('PENDING', 'VALID', 'INVALID', 'UNAVAILABLE');

ALTER TABLE customer
  ADD COLUMN vat_number_validation_status "CustomerVatValidationStatusEnum",
  ADD COLUMN vat_number_checked_at TIMESTAMP;

-- Event-driven VIES verification jobs (enqueued from the customer outbox).
SELECT pgmq.create('vat_validation');
