--: Schedule()
--! create_schedule : Schedule
INSERT INTO schedule (id, billing_period, plan_version_id, ramps)
SELECT :id, :billing_period, :plan_version_id, :ramps
FROM plan_version
WHERE plan_version.id = :plan_version_id
  AND plan_version.tenant_id = :tenant_id
RETURNING id, billing_period, ramps;

--! update_schedule : Schedule
UPDATE schedule
SET ramps = :ramps
FROM plan_version
WHERE schedule.plan_version_id = plan_version.id
  AND plan_version.tenant_id = :tenant_id
  AND plan_version.is_draft_version = true
  AND schedule.id = :id
RETURNING schedule.id, schedule.billing_period, schedule.ramps;

--! list_schedules : Schedule
SELECT s.id, s.billing_period, s.ramps
FROM schedule s
         JOIN plan_version pv ON s.plan_version_id = pv.id
WHERE pv.tenant_id = :tenant_id
  AND pv.id = :plan_version_id;

--! list_schedules_by_subscription : Schedule
SELECT s.id, s.billing_period, s.ramps
FROM schedule s
         JOIN subscription ss ON s.plan_version_id = ss.plan_version_id
         JOIN plan_version pv ON s.plan_version_id = pv.id
WHERE ss.id = :subscription_id;

--! delete_schedule
DELETE
FROM schedule s
    USING plan_version pv
WHERE s.id = :id
  AND s.plan_version_id = pv.id
  AND pv.tenant_id = :tenant_id
  AND pv.is_draft_version = true;
