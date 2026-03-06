ALTER TYPE "CheckoutTypeEnum" ADD VALUE IF NOT EXISTS 'PLAN_CHANGE';

ALTER TABLE checkout_session ADD COLUMN change_date DATE;

ALTER TABLE payment_transaction ADD COLUMN pending_plan_version_id UUID REFERENCES plan_version(id);
