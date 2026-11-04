DROP INDEX IF EXISTS idx_invoice_pending;
CREATE INDEX idx_invoice_pending
    ON invoice (id)
    WHERE status <> 'VOID'::"InvoiceStatusEnum"
      AND status <> 'FINALIZED'::"InvoiceStatusEnum";
