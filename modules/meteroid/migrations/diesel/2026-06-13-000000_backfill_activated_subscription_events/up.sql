-- Backfill an Activated lifecycle event for subscriptions that were activated
-- before the event started being emitted, so the analytics active_subscriptions
-- running total sees the +1. Idempotent (WHERE NOT EXISTS), re-runnable.
--
-- created_at = now() (NOT the historical date): the analytics projector cursors
-- on subscription_event.created_at, so backfilled rows need a created_at greater
-- than the current watermark to be picked up on the next tick. applies_to is the
-- historical activation date so the fact lands in the right period.
--
-- mrr_delta is NULL: these rows are MRR-neutral. invoices.rs::process_mrr only
-- emits an MRR movement when mrr_delta is non-null and non-zero, so this never
-- creates duplicate MRR.
INSERT INTO subscription_event (id, mrr_delta, event_type, created_at, applies_to, subscription_id, bi_mrr_movement_log_id, details)
SELECT gen_random_uuid(),
       NULL,
       'ACTIVATED'::"SubscriptionEventType",
       now(),
       s.activated_at::date,
       s.id,
       NULL,
       NULL
FROM subscription s
WHERE s.activated_at IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM subscription_event e
      WHERE e.subscription_id = s.id
        AND e.event_type = 'ACTIVATED'::"SubscriptionEventType"
  );
