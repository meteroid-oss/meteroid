CREATE TYPE "DeclineKindEnum" AS ENUM (
    'INSUFFICIENT_FUNDS', 'DO_NOT_HONOR', 'CARD_EXPIRED', 'AUTHENTICATION_REQUIRED',
    'MANDATE_INACTIVE', 'FRAUD', 'PROCESSING_ERROR', 'OTHER'
);

-- The provider's normalized decline category, so the retry ladder can stop on a hard decline.
ALTER TABLE payment_transaction
    ADD COLUMN decline_kind "DeclineKindEnum";
