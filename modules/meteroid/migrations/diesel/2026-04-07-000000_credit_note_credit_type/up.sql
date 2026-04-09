CREATE TYPE "CreditTypeEnum" AS ENUM (
    'CREDIT_TO_BALANCE',
    'REFUND',
    'DEBT_CANCELLATION'
);

ALTER TABLE credit_note
    ADD COLUMN credit_type "CreditTypeEnum";

UPDATE credit_note
SET credit_type = CASE
    WHEN refunded_amount_cents > 0 THEN 'REFUND'::"CreditTypeEnum"
    ELSE 'CREDIT_TO_BALANCE'::"CreditTypeEnum"
END;

ALTER TABLE credit_note
    ALTER COLUMN credit_type SET NOT NULL;
