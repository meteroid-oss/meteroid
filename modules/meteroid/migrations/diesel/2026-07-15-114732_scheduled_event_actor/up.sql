-- Persist the initiating actor on scheduled events so the worker that later applies a
-- due lifecycle transition (cancel/pause) can attribute the terminal activity-log event
-- to the real user instead of falling back to System.
ALTER TABLE scheduled_event
    ADD COLUMN actor_type "ActorTypeEnum" NOT NULL DEFAULT 'SYSTEM',
    ADD COLUMN actor_uuid  UUID,
    ADD COLUMN actor_alias TEXT;
