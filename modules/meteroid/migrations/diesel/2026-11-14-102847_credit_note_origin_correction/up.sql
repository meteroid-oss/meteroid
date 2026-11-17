-- An operator's credit note that corrects what was billed (days refunded by hand): reconcile counts
-- it like the engine's, so a later change in the period never refunds those days again.
ALTER TYPE "CreditNoteOriginEnum" ADD VALUE IF NOT EXISTS 'CORRECTION';
