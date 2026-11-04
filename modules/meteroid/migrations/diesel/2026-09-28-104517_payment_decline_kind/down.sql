ALTER TABLE payment_transaction
    DROP COLUMN decline_kind;

DROP TYPE "DeclineKindEnum";
