ALTER TABLE payment_transaction DROP COLUMN IF EXISTS charge_attempt;
-- PostgreSQL cannot remove enum values; UNCONFIRMED remains in the enum unused.
