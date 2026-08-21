-- Records who initiated an invoice payment, so a settled payment can be
-- attributed to the paying customer (portal) vs. a system auto-charge.
ALTER TABLE payment_transaction ADD COLUMN initiated_by_customer_id UUID;
