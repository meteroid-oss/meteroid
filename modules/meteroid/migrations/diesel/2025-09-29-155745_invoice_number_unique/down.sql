DROP INDEX IF EXISTS invoice_invoice_number_key;

ALTER TABLE invoice
DROP COLUMN IF EXISTS invoicing_entity_id;

CREATE UNIQUE INDEX invoice_invoice_number_key
ON invoice (invoice_number, tenant_id)
WHERE status <> 'DRAFT'::"InvoiceStatusEnum";
