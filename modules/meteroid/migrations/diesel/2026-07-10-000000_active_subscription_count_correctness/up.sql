-- Active-subscriptions analytics correctness: the DB owns both the stamping of activated_at and
-- the emission of the ACTIVATED lifecycle event, so the "counted iff activated_at set" invariant
-- can't be violated from any write path.

-- Terminal status for a subscription cancelled before it ever activated (never counted, no -1).
ALTER TYPE "SubscriptionStatusEnum" ADD VALUE IF NOT EXISTS 'ABORTED';

-- Lifecycle event for a counted (activated) subscription reaching its natural end (Completed).
-- Drives the -1 in the active-subscriptions running total, mirroring CANCELLED.
ALTER TYPE "SubscriptionEventType" ADD VALUE IF NOT EXISTS 'ENDED';

-- An Active subscription must always carry an activated_at. The WHEN guard keeps the trigger
-- function off subscription's hot write path: it fires only on the write that first activates.
CREATE OR REPLACE FUNCTION set_activated_at_on_active() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    -- naive-UTC, matching the Rust writers' Utc::now().naive_utc()
    NEW.activated_at := (now() AT TIME ZONE 'UTC');
    RETURN NEW;
END $$;

CREATE OR REPLACE TRIGGER trg_subscription_set_activated_at
    BEFORE INSERT OR UPDATE ON subscription
    FOR EACH ROW
    WHEN (NEW.status = 'ACTIVE' AND NEW.activated_at IS NULL)
    EXECUTE FUNCTION set_activated_at_on_active();

-- Emit an ACTIVATED subscription_event exactly when activated_at transitions to non-null. Split by
-- operation with a WHEN guard so the function only runs on that transition, not on ordinary writes.
CREATE OR REPLACE FUNCTION emit_subscription_activated_event() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO subscription_event (id, event_type, created_at, applies_to, subscription_id, mrr_delta, bi_mrr_movement_log_id, details)
    VALUES (uuidv7(), 'ACTIVATED', (now() AT TIME ZONE 'UTC'), NEW.activated_at::date, NEW.id, NULL, NULL, NULL);
    RETURN NULL;
END $$;

CREATE OR REPLACE TRIGGER trg_subscription_emit_activated_event_ins
    AFTER INSERT ON subscription
    FOR EACH ROW
    WHEN (NEW.activated_at IS NOT NULL)
    EXECUTE FUNCTION emit_subscription_activated_event();

CREATE OR REPLACE TRIGGER trg_subscription_emit_activated_event_upd
    AFTER UPDATE ON subscription
    FOR EACH ROW
    WHEN (NEW.activated_at IS NOT NULL AND OLD.activated_at IS NULL)
    EXECUTE FUNCTION emit_subscription_activated_event();

-- The analytics projector cursors subscription_event on (created_at, id::text); index it so each
-- projection tick is an index range scan instead of a full-table scan + sort.
CREATE INDEX IF NOT EXISTS idx_subscription_event_created_at_id
    ON subscription_event (created_at, (id::text));
