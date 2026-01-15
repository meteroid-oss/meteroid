-- Simplify trial system:
-- - Remove downgrade_plan_id (not our problem - users subscribe to Free with trial)
-- - Remove action_after_trial (derive from plan type instead)
-- - Keep TrialExpired status (used when paid plan trial ends without payment method)

-- Remove columns from plan_version
ALTER TABLE plan_version
    DROP COLUMN IF EXISTS downgrade_plan_id,
    DROP COLUMN IF EXISTS action_after_trial;

-- Drop the ActionAfterTrialEnum type
DROP TYPE IF EXISTS "ActionAfterTrialEnum";
