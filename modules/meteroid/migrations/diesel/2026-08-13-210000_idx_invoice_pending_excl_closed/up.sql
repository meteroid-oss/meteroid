-- The worker scans backed by this partial index (list_to_finalize, list_outdated) now also
-- exclude CLOSED, and a usage-only tenant accretes one CLOSED row per subscription per empty
-- period forever. Keep the index aligned with the queries so those rows stay out of the B-tree.
DROP INDEX IF EXISTS idx_invoice_pending;
CREATE INDEX idx_invoice_pending
    ON invoice (id)
    WHERE status <> 'VOID'::"InvoiceStatusEnum"
      AND status <> 'FINALIZED'::"InvoiceStatusEnum"
      AND status <> 'CLOSED'::"InvoiceStatusEnum";
