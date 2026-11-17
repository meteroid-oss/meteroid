-- The rows a scheduled change wrote as facts (`facts::pending::WrittenFacts`), on
-- its MIGRATED record: cancelling the change deletes them and reopens what it closed, and the
-- rollback script does the same for every change written since the deploy.
ALTER TABLE scheduled_event ADD COLUMN written_facts JSONB;
