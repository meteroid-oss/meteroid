-- Opt-in strictness (Hyperline-style): when enabled, reverse charge applies only
-- once the customer's VAT number has a VIES VALID status; any other state falls
-- back to the standard rate of the customer's country. Off by default (fail-open).
ALTER TABLE invoicing_entity
  ADD COLUMN require_vies_valid_for_reverse_charge BOOLEAN NOT NULL DEFAULT false;

-- Raw fields of the last definitive VIES answer (request date/identifier,
-- registered name/address): audit evidence for reverse-charge decisions.
ALTER TABLE customer
  ADD COLUMN vat_number_vies_check JSONB;

-- Serves the daily revalidation worker's candidate scan (oldest checked first,
-- never-checked leading).
CREATE INDEX idx_customer_vat_revalidation
  ON customer (vat_number_checked_at ASC NULLS FIRST)
  WHERE vat_number IS NOT NULL AND vat_number_format_valid = true AND archived_at IS NULL;
