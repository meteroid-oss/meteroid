CREATE TYPE "ActionAfterTrialEnum" AS ENUM ('BLOCK', 'CHARGE', 'DOWNGRADE');

ALTER TABLE "plan_version"
  RENAME COLUMN "trial_fallback_plan_id" TO "downgrade_plan_id";

ALTER TABLE "plan_version"
  ADD COLUMN "trialing_plan_id"   UUID REFERENCES "plan" ("id"),
  ADD COLUMN "action_after_trial" "ActionAfterTrialEnum" NULL,
  ADD COLUMN "trial_is_free"      BOOLEAN                NOT NULL DEFAULT TRUE;
;

ALTER TABLE "plan_version"
  DROP CONSTRAINT "plan_version_check";
