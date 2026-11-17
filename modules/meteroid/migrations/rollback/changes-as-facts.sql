-- Rollback of "changes are fact edits", for a same-day redeploy of the previous
-- release (decision 10). Every scheduled change written as facts since the deploy and not applied
-- yet goes back to a PENDING event, and the rows it wrote are withdrawn: the previous release
-- then applies it as an event at its date. Changes the state applied stay applied, COMPLETED.
--   usage: psql -v deployed_at="'2026-11-20 09:00:00'" -f changes-as-facts.sql
-- Tested by `subscription::pending_facts` (the rows a pending change leaves, then its migration).
BEGIN;

CREATE TEMP TABLE rollback_change ON COMMIT DROP AS
SELECT e.id, e.subscription_id, e.written_facts AS w, (e.written_facts ->> 'on')::date AS on_date
FROM scheduled_event e
WHERE e.status = 'MIGRATED'
  AND e.written_facts IS NOT NULL
  AND e.processed_at >= :deployed_at::timestamp
  AND NOT EXISTS (SELECT 1 FROM subscription_event ev
                  WHERE ev.subscription_id = e.subscription_id AND ev.event_type = 'SWITCH'
                    AND ev.details ->> 'event' = e.id::text);

-- The plan window it opened goes, then the one it replaced that day comes back, else the one
-- before it reopens (one open window at a time).
WITH gone AS (DELETE FROM subscription_plan_window w
              USING rollback_change c
              WHERE w.id = (c.w ->> 'plan_window')::uuid
              RETURNING w.subscription_id, w.from_date, c.w -> 'replaced_plan_window' AS replaced)
, restored AS (INSERT INTO subscription_plan_window
                   (id, tenant_id, subscription_id, plan_version_id, from_date, cadence,
                    cadence_origin, created_at)
               SELECT (r ->> 'id')::uuid, s.tenant_id, gone.subscription_id,
                      (r ->> 'plan_version_id')::uuid, (r ->> 'from_date')::date,
                      (r ->> 'cadence')::"BillingPeriodEnum", (r ->> 'cadence_origin')::date,
                      now() AT TIME ZONE 'UTC'
               FROM gone
               JOIN subscription s ON s.id = gone.subscription_id
               CROSS JOIN LATERAL (SELECT gone.replaced AS r) x
               WHERE jsonb_typeof(gone.replaced) = 'object')
UPDATE subscription_plan_window p SET to_date = NULL
FROM gone
WHERE p.subscription_id = gone.subscription_id AND p.to_date = gone.from_date
  AND jsonb_typeof(gone.replaced) IS DISTINCT FROM 'object';

-- Fee rows: those it opened go, those it closed on its date reopen.
DELETE FROM subscription_component sc
USING rollback_change c
WHERE sc.subscription_id = c.subscription_id AND sc.effective_from = c.on_date
  AND sc.id IN (SELECT jsonb_array_elements_text(c.w -> 'opened_components')::uuid);
UPDATE subscription_component sc SET effective_to = NULL
FROM rollback_change c
WHERE sc.subscription_id = c.subscription_id AND sc.effective_to = c.on_date
  AND sc.id IN (SELECT jsonb_array_elements_text(c.w -> 'closed_components')::uuid);
DELETE FROM subscription_add_on sa
USING rollback_change c
WHERE sa.subscription_id = c.subscription_id AND sa.effective_from = c.on_date
  AND sa.id IN (SELECT jsonb_array_elements_text(c.w -> 'opened_add_ons')::uuid);
UPDATE subscription_add_on sa SET effective_to = NULL
FROM rollback_change c
WHERE sa.subscription_id = c.subscription_id AND sa.effective_to = c.on_date
  AND sa.id IN (SELECT jsonb_array_elements_text(c.w -> 'closed_add_ons')::uuid);

-- Seat seeds of the fees it opened.
DELETE FROM slot_transaction st
USING rollback_change c
WHERE st.subscription_id = c.subscription_id
  AND st.id IN (SELECT jsonb_array_elements_text(c.w -> 'seat_seeds')::uuid);

-- The minimum it replaced or ended on its date.
DELETE FROM subscription_adjustment_component m
USING rollback_change c
WHERE (c.w ->> 'minimum')::boolean AND m.subscription_id = c.subscription_id
  AND m.kind = 'MINIMUM' AND m.effective_from = c.on_date;
DELETE FROM subscription_adjustment m
USING rollback_change c
WHERE (c.w ->> 'minimum')::boolean AND m.subscription_id = c.subscription_id
  AND m.kind = 'MINIMUM' AND m.effective_from = c.on_date;
UPDATE subscription_adjustment m SET effective_to = NULL
FROM rollback_change c
WHERE (c.w ->> 'minimum')::boolean AND m.subscription_id = c.subscription_id
  AND m.kind = 'MINIMUM' AND m.effective_to = c.on_date;

-- The trial it cut, and the trial end it moved to its date.
UPDATE subscription_trial t SET until_date = (c.w ->> 'trial_until')::date
FROM rollback_change c
WHERE c.w ->> 'trial_until' IS NOT NULL AND t.subscription_id = c.subscription_id;
UPDATE scheduled_event t SET scheduled_time = (c.w -> 'end_trial' ->> 1)::timestamp
FROM rollback_change c
WHERE jsonb_typeof(c.w -> 'end_trial') = 'array' AND t.id = (c.w -> 'end_trial' ->> 0)::uuid
  AND t.status = 'PENDING';

-- The change pending again, as the previous release scheduled it.
UPDATE scheduled_event e SET status = 'PENDING', written_facts = NULL, processed_at = NULL
FROM rollback_change c
WHERE e.id = c.id;

-- A change the state applied is done: COMPLETED, which the previous release reads.
UPDATE scheduled_event SET status = 'COMPLETED' WHERE status = 'MIGRATED';

COMMIT;
