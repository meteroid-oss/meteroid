-- The reconcile sweep backs off per row instead of re-polling every Pending row each minute.
ALTER TABLE payment_transaction
    ADD COLUMN last_reconciled_at TIMESTAMPTZ;
