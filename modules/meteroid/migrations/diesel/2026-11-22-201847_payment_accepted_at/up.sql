-- When a provider accepted a collection still in flight (a delayed-notification debit): money is
-- moving, so the invoice is Processing, neither charged again nor chased.
ALTER TABLE payment_transaction ADD COLUMN accepted_at TIMESTAMPTZ;

UPDATE payment_transaction pt
SET accepted_at = pt.created_at
FROM invoice i
WHERE i.id = pt.invoice_id
  AND i.tenant_id = pt.tenant_id
  AND i.payment_status = 'PROCESSING'
  AND pt.status = 'PENDING'
  AND pt.payment_type IN ('PAYMENT', 'TOP_UP', 'AUTO_TOPUP');
