-- Public API v2 (ADR-0013 §10): when a subscription last changed, which `GET /api/v2/subscriptions`
-- filters on (`updated_after`). Its own row's changes, bookkeeping aside, set
-- `subscription.updated_at`; those and the changes to what the API reads beside it (its charges,
-- add-ons, coupons, seats, and the changes and cancellation scheduled on it) set
-- `subscription_activity.updated_at` as their transaction commits. A subscription changed last
-- when the later of the two says.
-- A long transaction of a serving pod fails this run instead of queueing every query behind it;
-- the next start retries.
SET LOCAL lock_timeout = '5s';

-- No backfill: a subscription unchanged since reads as changed when it was created. Adding a
-- column without a default does not rewrite the table.
ALTER TABLE subscription ADD COLUMN updated_at TIMESTAMPTZ;
ALTER TABLE subscription ALTER COLUMN updated_at SET DEFAULT now();

-- Kept apart from `subscription`, which the billing engine locks and writes all the time: a write
-- to a subscription's rows never waits on the subscription's lock. No foreign key, whose check
-- would lock the subscription's row: a trigger removes the row with its subscription.
CREATE TABLE subscription_activity
(
    subscription_id UUID PRIMARY KEY,
    updated_at      TIMESTAMPTZ NOT NULL
);

CREATE FUNCTION subscription_activity_forget() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    DELETE FROM subscription_activity WHERE subscription_id = OLD.id;
    RETURN NULL;
END
$$;

-- The one write of `subscription_activity` before a commit: a subscription deleted (with its
-- tenant's or its sandbox's data).
CREATE TRIGGER trg_subscription_activity_forget
    AFTER DELETE ON subscription
    FOR EACH ROW EXECUTE FUNCTION subscription_activity_forget();

-- The subscriptions a running transaction changed, noted as it writes and moved to
-- `subscription_activity` as it commits. A row lives only inside its transaction (the commit
-- deletes it, an abort takes it), so unlogged: nothing of it is to survive a crash. Its key leads
-- with the transaction: a transaction never meets another's rows, so noting never waits.
CREATE UNLOGGED TABLE subscription_activity_pending
(
    xact            XID8 NOT NULL,
    subscription_id UUID NOT NULL,
    PRIMARY KEY (xact, subscription_id)
);

-- As the transaction commits (the schema's only deferred trigger, so its last write), when each
-- subscription it noted changed:
-- - in subscription id order, in one statement: two committing transactions take the rows' locks
--   in the same order, so they never deadlock, and a transaction already holding every other lock
--   it takes only ever waits here on another committing one;
-- - stamped by the clock once the row is held (`clock_timestamp()`, not `now()`, the
--   transaction's start): a change becomes visible moments after its stamp, however long its
--   transaction ran, and a subscription's stamp never goes back.
-- A subscription deleted in the same transaction is left out. The first of the transaction's rows
-- to fire writes them all; the others find nothing left.
CREATE FUNCTION subscription_activity_flush() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    WITH noted AS (
        DELETE FROM subscription_activity_pending
            WHERE xact = pg_current_xact_id()
            RETURNING subscription_id)
    INSERT
    INTO subscription_activity AS a (subscription_id, updated_at)
    SELECT noted.subscription_id, clock_timestamp()
    FROM noted
    WHERE EXISTS (SELECT 1 FROM subscription s WHERE s.id = noted.subscription_id)
    ORDER BY noted.subscription_id
    ON CONFLICT (subscription_id) DO UPDATE SET updated_at = greatest(a.updated_at, clock_timestamp());
    RETURN NULL;
END
$$;

CREATE CONSTRAINT TRIGGER trg_subscription_activity_flush
    AFTER INSERT ON subscription_activity_pending
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION subscription_activity_flush();

-- The subscription's own row, created or changed. The columns the billing engine keeps for itself:
-- writing them changes nothing a reader sees. A BEFORE trigger on the row being written takes no
-- lock of its own.
CREATE FUNCTION subscription_touch() RETURNS trigger
    LANGUAGE plpgsql AS
$$
DECLARE
    bookkeeping TEXT[] := ARRAY [
        'updated_at', 'conn_meta', 'lifecycle_suspended', 'reconcile_from', 'next_wake_at',
        'reconcile_arrears_from', 'billing_failures', 'billing_first_failed_at',
        'billing_last_failed_at', 'billing_last_error', 'billing_retry_at'];
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF (to_jsonb(NEW) - bookkeeping) IS NOT DISTINCT FROM (to_jsonb(OLD) - bookkeeping) THEN
            RETURN NEW;
        END IF;
        NEW.updated_at := clock_timestamp();
    END IF;
    INSERT INTO subscription_activity_pending (xact, subscription_id)
    VALUES (pg_current_xact_id(), NEW.id)
    ON CONFLICT DO NOTHING;
    RETURN NEW;
END
$$;

CREATE TRIGGER trg_subscription_touch
    BEFORE INSERT OR UPDATE ON subscription
    FOR EACH ROW EXECUTE FUNCTION subscription_touch();

-- A row of a subscription's own changed: inserted, deleted, or updated in anything but the columns
-- the trigger's arguments name, those the billing engine keeps for itself:
--   applied_coupon       applied_count, applied_amount, last_applied_at: every invoice a coupon
--                        discounts counts itself there, and the invoice is the change readers see
--                        (`GET /invoices?updated_after=`); the coupon running out (`is_active`)
--                        still counts;
--   slot_transaction     invoice_id: the invoice billing the seats, re-pointed when drafts are
--                        consolidated;
--   subscription_change  updated_at: the column itself (a deferral or a withdrawal changes the
--                        date or the state beside it).
-- An update writing back what was there counts for nothing either.
CREATE FUNCTION subscription_note_activity() RETURNS trigger
    LANGUAGE plpgsql AS
$$
DECLARE
    bookkeeping TEXT[] := coalesce(TG_ARGV, '{}');
BEGIN
    IF TG_OP = 'DELETE' THEN
        INSERT INTO subscription_activity_pending (xact, subscription_id)
        VALUES (pg_current_xact_id(), OLD.subscription_id)
        ON CONFLICT DO NOTHING;
        RETURN NULL;
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF (to_jsonb(NEW) - bookkeeping) IS NOT DISTINCT FROM (to_jsonb(OLD) - bookkeeping) THEN
            RETURN NULL;
        END IF;
        IF OLD.subscription_id IS DISTINCT FROM NEW.subscription_id THEN
            INSERT INTO subscription_activity_pending (xact, subscription_id)
            VALUES (pg_current_xact_id(), OLD.subscription_id)
            ON CONFLICT DO NOTHING;
        END IF;
    END IF;
    INSERT INTO subscription_activity_pending (xact, subscription_id)
    VALUES (pg_current_xact_id(), NEW.subscription_id)
    ON CONFLICT DO NOTHING;
    RETURN NULL;
END
$$;

CREATE TRIGGER trg_subscription_component_touch
    AFTER INSERT OR UPDATE OR DELETE ON subscription_component
    FOR EACH ROW EXECUTE FUNCTION subscription_note_activity();
CREATE TRIGGER trg_subscription_add_on_touch
    AFTER INSERT OR UPDATE OR DELETE ON subscription_add_on
    FOR EACH ROW EXECUTE FUNCTION subscription_note_activity();
CREATE TRIGGER trg_applied_coupon_touch
    AFTER INSERT OR UPDATE OR DELETE ON applied_coupon
    FOR EACH ROW EXECUTE FUNCTION subscription_note_activity('applied_count', 'applied_amount',
                                                             'last_applied_at');
CREATE TRIGGER trg_slot_transaction_touch
    AFTER INSERT OR UPDATE OR DELETE ON slot_transaction
    FOR EACH ROW EXECUTE FUNCTION subscription_note_activity('invoice_id');
CREATE TRIGGER trg_subscription_change_touch
    AFTER INSERT OR UPDATE OR DELETE ON subscription_change
    FOR EACH ROW EXECUTE FUNCTION subscription_note_activity('updated_at');
CREATE TRIGGER trg_subscription_ending_touch
    AFTER INSERT OR UPDATE OR DELETE ON subscription_ending
    FOR EACH ROW EXECUTE FUNCTION subscription_note_activity();
