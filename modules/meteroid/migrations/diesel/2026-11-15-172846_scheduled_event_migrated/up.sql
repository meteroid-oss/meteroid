-- A pending scheduled change written as the facts it becomes is marked MIGRATED
-- (`facts::pending::write_pending_as_facts_tx`), restorable for one release. Unused until then.
ALTER TYPE "ScheduledEventStatus" ADD VALUE IF NOT EXISTS 'MIGRATED';
