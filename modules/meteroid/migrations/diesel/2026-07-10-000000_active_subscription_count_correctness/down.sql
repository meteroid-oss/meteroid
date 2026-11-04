DROP INDEX IF EXISTS idx_subscription_event_created_at_id;
DROP TRIGGER IF EXISTS trg_subscription_emit_activated_event_ins ON subscription;
DROP TRIGGER IF EXISTS trg_subscription_emit_activated_event_upd ON subscription;
DROP TRIGGER IF EXISTS trg_subscription_emit_activated_event ON subscription;
DROP TRIGGER IF EXISTS trg_subscription_set_activated_at ON subscription;
DROP FUNCTION IF EXISTS emit_subscription_activated_event();
DROP FUNCTION IF EXISTS set_activated_at_on_active();
-- Note: PostgreSQL cannot remove enum values, so 'ABORTED' (SubscriptionStatusEnum) and
-- 'ENDED' (SubscriptionEventType) remain after a revert. They are inert unless emitted.
