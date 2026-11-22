-- When an operator deleted a subscription's recurring draft, closing it: billing from facts reads
-- its lines as voided, billed and forgiven (R-124). Other closed invoices billed nothing.
ALTER TABLE invoice ADD COLUMN waived_at TIMESTAMP;
