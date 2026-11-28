CREATE OR REPLACE FUNCTION journal_posted_is_final() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' AND pg_trigger_depth() > 1 THEN
        RETURN OLD;
    END IF;
    IF TG_TABLE_NAME = 'journal_entry' THEN
        IF OLD.status = 'POSTED' THEN
            RAISE EXCEPTION 'posted journal entry % cannot change', OLD.id USING ERRCODE = 'check_violation';
        END IF;
    ELSIF EXISTS (SELECT 1 FROM journal_entry
                  WHERE id = coalesce(OLD.entry_id, NEW.entry_id) AND status = 'POSTED') THEN
        RAISE EXCEPTION 'lines of a posted journal entry cannot change' USING ERRCODE = 'check_violation';
    END IF;
    RETURN coalesce(NEW, OLD);
END $$;

ALTER TABLE journal_entry DROP COLUMN exported_at;

DROP TABLE revrec_contract_adjustment;
