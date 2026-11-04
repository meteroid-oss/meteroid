-- A subscription ends once: at most one live final invoice, so a retried termination can never
-- bill its last window twice. A voided one leaves room for its regeneration.
--
-- CONCURRENTLY, so writers are never locked out (metadata.toml sets run_in_transaction = false).
-- This file must stay a SINGLE statement. If a run fails it leaves an INVALID index that
-- IF NOT EXISTS will not replace: drop it by hand, then re-run.
CREATE UNIQUE INDEX CONCURRENTLY IF NOT EXISTS invoice_subscription_final_unique
    ON invoice (subscription_id)
    WHERE invoice_type = 'FINAL'::"InvoiceType" AND status <> 'VOID'::"InvoiceStatusEnum";
