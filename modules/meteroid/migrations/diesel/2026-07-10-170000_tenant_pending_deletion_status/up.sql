-- Dedicated status for user-requested tenant deletion, distinct from the
-- lifecycle-owned MARKED_FOR_DELETION. The weekly cleanup worker hard-deletes
-- tenants in this state.
ALTER TYPE "TenantStatusEnum" ADD VALUE IF NOT EXISTS 'PENDING_DELETION';
