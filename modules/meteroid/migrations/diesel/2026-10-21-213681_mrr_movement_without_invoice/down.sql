-- Movements booked without an invoice cannot be kept once one is required.
DELETE FROM bi_mrr_movement_log WHERE invoice_id IS NULL;

DROP INDEX bi_mrr_movement_log_customer_idx;
DROP INDEX bi_mrr_movement_log_subscription_idx;

ALTER TABLE bi_mrr_movement_log
    DROP COLUMN customer_id,
    DROP COLUMN subscription_id,
    ALTER COLUMN invoice_id SET NOT NULL;
