SELECT pgmq.drop_queue('vat_validation');

ALTER TABLE customer
  DROP COLUMN vat_number_validation_status,
  DROP COLUMN vat_number_checked_at;

DROP TYPE "CustomerVatValidationStatusEnum";
