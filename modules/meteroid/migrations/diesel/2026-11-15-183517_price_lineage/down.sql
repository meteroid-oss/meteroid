-- A dated successor holds the rates subscriptions bill from its date: refuse while one exists.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM price WHERE supersedes IS NOT NULL) THEN
        RAISE EXCEPTION 'dated price successors exist; the previous release cannot bill them';
    END IF;
END $$;

DROP INDEX IF EXISTS price_supersedes_idx;
ALTER TABLE price
    DROP CONSTRAINT price_successor_is_dated,
    DROP COLUMN supersedes,
    DROP COLUMN effective_from;
