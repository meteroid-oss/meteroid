-- The application stamps when billing rows are written (common_domain::clock::now), so a test
-- that moves the clock sees them at its date. No default: an insert that forgets fails.
SET LOCAL lock_timeout = '5s';
ALTER TABLE applied_coupon ALTER COLUMN created_at DROP DEFAULT;
ALTER TABLE checkout_session ALTER COLUMN created_at DROP DEFAULT;
ALTER TABLE credit_note ALTER COLUMN created_at DROP DEFAULT;
ALTER TABLE credit_usage_settlement ALTER COLUMN created_at DROP DEFAULT;
ALTER TABLE customer_credit_entry ALTER COLUMN created_at DROP DEFAULT;
ALTER TABLE customer_credit_grant ALTER COLUMN created_at DROP DEFAULT;
ALTER TABLE customer_credit_line_allocation ALTER COLUMN created_at DROP DEFAULT;
ALTER TABLE customer_credit_operation ALTER COLUMN created_at DROP DEFAULT;
ALTER TABLE customer_credit_purchase ALTER COLUMN created_at DROP DEFAULT;
ALTER TABLE invoice ALTER COLUMN created_at DROP DEFAULT;
ALTER TABLE payment_reversal_movement ALTER COLUMN created_at DROP DEFAULT;
ALTER TABLE payment_transaction ALTER COLUMN created_at DROP DEFAULT;
ALTER TABLE scheduled_event ALTER COLUMN created_at DROP DEFAULT;
ALTER TABLE subscription ALTER COLUMN created_at DROP DEFAULT;
ALTER TABLE subscription_add_on ALTER COLUMN created_at DROP DEFAULT;
ALTER TABLE subscription_adjustment ALTER COLUMN created_at DROP DEFAULT;
ALTER TABLE subscription_event ALTER COLUMN created_at DROP DEFAULT;
