ALTER TABLE "plan_version"
  RENAME COLUMN "downgrade_plan_id" TO "trial_fallback_plan_id";

ALTER TABLE "plan_version"
  DROP COLUMN "trialing_plan_id",
  DROP COLUMN "action_after_trial",
  DROP COLUMN "trial_is_free"
;

DROP TYPE "ActionAfterTrialEnum";


ALTER TABLE "plan_version"
  ADD CONSTRAINT "plan_version_check" CHECK (((trial_duration_days IS NULL) AND (trial_fallback_plan_id IS NULL)) OR
                                             ((trial_duration_days IS NOT NULL) AND
                                              (trial_fallback_plan_id IS NOT NULL)));
