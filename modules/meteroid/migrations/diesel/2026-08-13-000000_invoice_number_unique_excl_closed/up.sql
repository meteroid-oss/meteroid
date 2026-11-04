-- Every draft carries the literal placeholder invoice_number 'draft' until finalization assigns a
-- real one. The partial index previously excluded only DRAFT, which held because the sole exit from
-- DRAFT was finalize (VOID is reachable only from FINALIZED). CLOSED breaks that: it leaves the row
-- unnumbered, so the second closed invoice of an invoicing entity collided on ('draft', entity).
DROP INDEX IF EXISTS invoice_invoice_number_key;

CREATE UNIQUE INDEX invoice_invoice_number_key
ON invoice (invoice_number, invoicing_entity_id)
WHERE status NOT IN ('DRAFT'::"InvoiceStatusEnum", 'CLOSED'::"InvoiceStatusEnum");
