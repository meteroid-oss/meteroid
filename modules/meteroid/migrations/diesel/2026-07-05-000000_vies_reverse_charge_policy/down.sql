DROP INDEX idx_customer_vat_revalidation;

ALTER TABLE customer
  DROP COLUMN vat_number_vies_check;

ALTER TABLE invoicing_entity
  DROP COLUMN require_vies_valid_for_reverse_charge;
