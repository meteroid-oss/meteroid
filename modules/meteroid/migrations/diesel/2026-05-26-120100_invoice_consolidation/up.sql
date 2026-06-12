-- Points a per-subscription draft to the consolidated invoice it was merged into. Such a
-- child is never finalized/charged on its own. Distinct from parent_invoice_id (corrections).
ALTER TABLE invoice
    ADD COLUMN consolidated_into_invoice_id UUID REFERENCES invoice (id);

CREATE INDEX idx_invoice_consolidated_into_invoice_id
    ON invoice (consolidated_into_invoice_id)
    WHERE consolidated_into_invoice_id IS NOT NULL;
