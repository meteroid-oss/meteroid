-- The exact provider request of a charge, committed before it is sent, so an attempt whose
-- outcome never came back can be re-sent under the same idempotency key.
ALTER TABLE payment_transaction ADD COLUMN charge_attempt JSONB;

-- An attempt whose provider outcome could not be confirmed: failed without dunning.
ALTER TYPE "DeclineKindEnum" ADD VALUE IF NOT EXISTS 'UNCONFIRMED';
