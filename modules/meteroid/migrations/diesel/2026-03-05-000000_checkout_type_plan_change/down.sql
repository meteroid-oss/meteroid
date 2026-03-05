ALTER TABLE payment_transaction DROP COLUMN IF EXISTS pending_plan_version_id;

ALTER TABLE checkout_session DROP COLUMN IF EXISTS change_date;
