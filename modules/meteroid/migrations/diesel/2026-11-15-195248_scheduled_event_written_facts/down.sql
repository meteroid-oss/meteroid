-- A migrated change is restored by the rollback script, which reads its written facts: refuse
-- before dropping them while one exists.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM scheduled_event WHERE status::text = 'MIGRATED') THEN
        RAISE EXCEPTION 'migrated scheduled events exist; run migrations/rollback/changes-as-facts.sql first';
    END IF;
END $$;

ALTER TABLE scheduled_event DROP COLUMN written_facts;
