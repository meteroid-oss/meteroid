-- Revert trial simplification

-- Recreate ActionAfterTrialEnum
CREATE TYPE "ActionAfterTrialEnum" AS ENUM ('BLOCK', 'CHARGE', 'DOWNGRADE');

-- Add back columns to plan_version
ALTER TABLE plan_version
    ADD COLUMN downgrade_plan_id UUID REFERENCES plan(id),
    ADD COLUMN action_after_trial "ActionAfterTrialEnum";
