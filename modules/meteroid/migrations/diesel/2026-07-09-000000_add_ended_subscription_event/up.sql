-- A fixed-term subscription that reaches its end date completes naturally. Recording
-- that as an ENDED lifecycle event lets the MRR pipeline churn the recurring revenue
-- (movement log + is_active), the same way CANCELLED already does. Without it, a
-- completed subscription's MRR is never reversed and it stays counted as active.
ALTER TYPE "SubscriptionEventType" ADD VALUE IF NOT EXISTS 'ENDED';
