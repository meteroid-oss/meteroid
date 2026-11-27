-- Before the release with migration 2026-11-27-163847 (ADR 0015 §3): what moving plan versions'
-- trials into each plan's trial offer drops or changes. Read-only; run against production.

-- 1. A draft whose trial differs from its plan's active version: the draft's trial is dropped.
SELECT 'draft trial dropped' AS finding, p.tenant_id, p.id AS plan_id, p.name,
       a.trial_duration_days AS active_days, d.trial_duration_days AS draft_days,
       a.trial_is_free AS active_free, d.trial_is_free AS draft_free
FROM plan p
JOIN plan_version a ON a.id = p.active_version_id
JOIN plan_version d ON d.id = p.draft_version_id
WHERE (coalesce(a.trial_duration_days, 0), a.trial_is_free, a.trialing_plan_id)
      IS DISTINCT FROM (coalesce(d.trial_duration_days, 0), d.trial_is_free, d.trialing_plan_id);

-- 2. An older version whose trial differs from the active one's: subscriptions created on it
--    afterwards get the plan's trial (informational).
SELECT 'older version trial differs' AS finding, p.tenant_id, p.id AS plan_id, v.version,
       v.trial_duration_days, v.trial_is_free
FROM plan p
JOIN plan_version a ON a.id = p.active_version_id
JOIN plan_version v ON v.plan_id = p.id AND v.id <> a.id AND NOT v.is_draft_version
WHERE (coalesce(v.trial_duration_days, 0), v.trial_is_free, v.trialing_plan_id)
      IS DISTINCT FROM (coalesce(a.trial_duration_days, 0), a.trial_is_free, a.trialing_plan_id);

-- 3. A version naming a trialing plan with no trial days: the setting is dropped.
SELECT 'trialing plan without days' AS finding, tenant_id, id AS plan_version_id, trialing_plan_id
FROM plan_version
WHERE trialing_plan_id IS NOT NULL AND coalesce(trial_duration_days, 0) = 0;

-- 4. A running trial on a version whose trialing plan is not the one it started on: its
--    entitlements resolve the version it started on, recorded at the release.
SELECT 'running trial, version moved' AS finding, s.tenant_id, s.id AS subscription_id,
       s.plan_version_id, w.plan_version_id AS started_on
FROM subscription s
JOIN subscription_trial t ON t.subscription_id = s.id
JOIN LATERAL (SELECT plan_version_id FROM subscription_plan_window
              WHERE subscription_id = s.id ORDER BY from_date, id LIMIT 1) w ON true
JOIN plan_version now ON now.id = s.plan_version_id
JOIN plan_version started ON started.id = w.plan_version_id
WHERE s.status = 'TRIAL_ACTIVE'
  AND now.trialing_plan_id IS DISTINCT FROM started.trialing_plan_id;

-- 5. Live quotes stating a trial: their freeness is read from the plan's trial at conversion.
SELECT 'quote trial freeness unsigned' AS finding, q.tenant_id, q.id AS quote_id, q.status,
       q.trial_duration_days
FROM quote q
WHERE coalesce(q.trial_duration_days, 0) > 0
  AND q.converted_at IS NULL AND q.status::text IN ('DRAFT', 'PENDING', 'ACCEPTED');
