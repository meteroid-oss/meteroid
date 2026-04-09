ALTER TABLE invoice
    ADD COLUMN parent_invoice_id UUID REFERENCES invoice (id);

CREATE INDEX idx_invoice_parent_invoice_id
    ON invoice (parent_invoice_id)
    WHERE parent_invoice_id IS NOT NULL;
