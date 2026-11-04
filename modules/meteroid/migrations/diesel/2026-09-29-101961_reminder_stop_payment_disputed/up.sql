-- Reminders reopened by a chargeback or dispute are held for an operator rather than restarted.
ALTER TYPE "ReminderStopReasonEnum" ADD VALUE IF NOT EXISTS 'PAYMENT_DISPUTED';
