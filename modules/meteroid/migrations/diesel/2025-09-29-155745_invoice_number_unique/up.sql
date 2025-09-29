DROP INDEX IF EXISTS invoice_invoice_number_key;

ALTER TABLE invoice
ADD COLUMN invoicing_entity_id UUID
REFERENCES invoicing_entity(id)
ON UPDATE CASCADE ON DELETE RESTRICT;

UPDATE invoice i
SET invoicing_entity_id = c.invoicing_entity_id
FROM customer c
WHERE i.customer_id = c.id;

ALTER TABLE invoice
ALTER COLUMN invoicing_entity_id SET NOT NULL;

CREATE UNIQUE INDEX invoice_invoice_number_key
ON invoice (invoice_number, invoicing_entity_id)
WHERE status <> 'DRAFT'::"InvoiceStatusEnum";
