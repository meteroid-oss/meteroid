-- Delayed-notification rails (SEPA/ACH/BACS direct debit) accept a debit days before
-- it settles. Without a distinct state, such an invoice is indistinguishable from one
-- that was never paid: dunning chases it, and the auto-charge orchestration re-charges it.
ALTER TYPE "InvoicePaymentStatus" ADD VALUE IF NOT EXISTS 'PROCESSING';
