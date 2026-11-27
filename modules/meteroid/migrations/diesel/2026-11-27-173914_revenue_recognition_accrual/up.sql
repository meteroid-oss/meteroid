-- The month-end accrual of unbilled usage, and its reversal on the next month's first day (ADR-0014 §4).
ALTER TYPE "JournalEntryKindEnum" ADD VALUE IF NOT EXISTS 'ACCRUAL';
