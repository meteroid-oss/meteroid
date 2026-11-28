DROP TABLE IF EXISTS payment_fee_capture;
ALTER TABLE payment_transaction
    DROP CONSTRAINT IF EXISTS payment_transaction_fee_complete,
    DROP COLUMN IF EXISTS fee_settled_at,
    DROP COLUMN IF EXISTS fee_source,
    DROP COLUMN IF EXISTS fee_details,
    DROP COLUMN IF EXISTS fee_currency,
    DROP COLUMN IF EXISTS fee_amount;
DROP TYPE IF EXISTS "PaymentFeeSourceEnum";
