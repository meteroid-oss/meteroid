-- Who issued a credit note: an operator, or the engine for a change it settled. Reconcile counts the
-- engine's notes as corrections of what a charge billed; an operator's only caps what is left.
CREATE TYPE "CreditNoteOriginEnum" AS ENUM (
    'MANUAL',
    'AMENDMENT',
    'PLAN_CHANGE',
    'CANCELLATION',
    'LATE_PACK',
    'UNAPPLIED_CHANGE',
    'VOID'
);

ALTER TABLE credit_note
    ADD COLUMN origin "CreditNoteOriginEnum" NOT NULL DEFAULT 'MANUAL';

-- Notes from before: the engine's by the reason it always wrote. An operator's note reusing one of
-- these counts as a correction, so a later change gives back less, never twice.
UPDATE credit_note
SET origin = CASE reason
    WHEN 'Subscription amendment' THEN 'AMENDMENT'::"CreditNoteOriginEnum"
    WHEN 'Plan change' THEN 'PLAN_CHANGE'::"CreditNoteOriginEnum"
    WHEN 'Subscription cancellation' THEN 'CANCELLATION'::"CreditNoteOriginEnum"
    WHEN 'Credit pack paid after its period was invoiced' THEN 'LATE_PACK'::"CreditNoteOriginEnum"
    WHEN 'Plan change could not apply' THEN 'UNAPPLIED_CHANGE'::"CreditNoteOriginEnum"
    WHEN 'Invoice voided' THEN 'VOID'::"CreditNoteOriginEnum"
END
WHERE reason IN ('Subscription amendment',
                 'Plan change',
                 'Subscription cancellation',
                 'Credit pack paid after its period was invoiced',
                 'Plan change could not apply',
                 'Invoice voided');
