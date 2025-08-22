ALTER TABLE invoice
  DROP COLUMN tax_breakdown,
  ADD COLUMN tax_breakdown JSONB NOT NULL DEFAULT '{}';

ALTER TABLE custom_tax
  DROP COLUMN rules,
  ADD COLUMN rules JSONB NOT NULL DEFAULT '{}';
