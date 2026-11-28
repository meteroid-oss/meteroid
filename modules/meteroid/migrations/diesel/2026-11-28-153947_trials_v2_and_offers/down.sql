SELECT pgmq.unbind_topic('credit_grant.updated', 'credit_recompute');
DROP FUNCTION IF EXISTS offer_wrap_coupons(UUID[]);

ALTER TABLE checkout_session DROP COLUMN IF EXISTS offer_request;
DROP TABLE IF EXISTS quote_offer;
DROP TABLE IF EXISTS offer_application_item;
DROP TABLE IF EXISTS offer_application;

DROP INDEX IF EXISTS idx_retention_offer_offer_id;
ALTER TABLE retention_offer DROP COLUMN IF EXISTS offer_id;
DROP INDEX IF EXISTS idx_coupon_offer_id;
ALTER TABLE coupon DROP COLUMN IF EXISTS offer_id;
DROP TABLE IF EXISTS offer_grant;

-- Offers without a trial (coupon wrappers, plans' discounts) go; a plan or session naming one no
-- longer does.
UPDATE plan SET trial_offer_id = NULL
WHERE trial_offer_id IN (SELECT id FROM trial_offer WHERE duration_days IS NULL);
UPDATE checkout_session SET trial_offer_id = NULL
WHERE trial_offer_id IN (SELECT id FROM trial_offer WHERE duration_days IS NULL);
DELETE FROM trial_offer WHERE duration_days IS NULL;

ALTER TABLE trial_offer DROP CONSTRAINT IF EXISTS trial_offer_trial_plan_needs_trial;
ALTER TABLE trial_offer
    ALTER COLUMN duration_days SET NOT NULL,
    DROP COLUMN IF EXISTS description,
    DROP COLUMN IF EXISTS standalone;

UPDATE coupon SET code = 'coupon-' || id::text WHERE code IS NULL;
ALTER TABLE coupon ALTER COLUMN code SET NOT NULL;

-- Checkout sessions and quotes.
ALTER TABLE quote
    DROP CONSTRAINT IF EXISTS quote_trial_end_action,
    DROP COLUMN IF EXISTS trial_end_action;
ALTER TABLE checkout_session DROP COLUMN IF EXISTS trial_request;

-- A manual end converts once its columns go; trials back to the three end actions.
UPDATE subscription_trial SET end_action = 'CONVERT' WHERE end_action = 'MANUAL';
UPDATE trial_offer SET end_action = 'CONVERT' WHERE end_action = 'MANUAL';

DROP INDEX IF EXISTS idx_subscription_trial_without_card_plan_version_id;
DROP INDEX IF EXISTS idx_trial_offer_without_card_plan_version_id;

ALTER TABLE subscription_trial
    DROP CONSTRAINT IF EXISTS subscription_trial_without_card_plan,
    DROP CONSTRAINT IF EXISTS subscription_trial_without_card,
    DROP CONSTRAINT IF EXISTS subscription_trial_ended,
    DROP CONSTRAINT IF EXISTS subscription_trial_outcome,
    DROP CONSTRAINT IF EXISTS subscription_trial_end_action;
ALTER TABLE subscription_trial
    DROP COLUMN IF EXISTS without_card_plan_version_id,
    DROP COLUMN IF EXISTS without_card,
    DROP COLUMN IF EXISTS ended_at,
    DROP COLUMN IF EXISTS outcome;
ALTER TABLE subscription_trial ADD CONSTRAINT subscription_trial_end_action
    CHECK (end_action IN ('CONVERT', 'CANCEL', 'FALLBACK'));

ALTER TABLE trial_offer
    DROP CONSTRAINT IF EXISTS trial_offer_without_card_plan,
    DROP CONSTRAINT IF EXISTS trial_offer_without_card,
    DROP CONSTRAINT IF EXISTS trial_offer_end_action;
ALTER TABLE trial_offer
    DROP COLUMN IF EXISTS without_card_plan_version_id,
    DROP COLUMN IF EXISTS without_card,
    DROP COLUMN IF EXISTS card_required;
ALTER TABLE trial_offer ADD CONSTRAINT trial_offer_end_action_check
    CHECK (end_action IN ('CONVERT', 'CANCEL', 'FALLBACK'));

-- ── end of trials_v2 ──────────────────────────────────────────────────────────────────────────

UPDATE invoicing_entity_email_settings
SET disabled_kinds = array_remove(array_remove(array_remove(disabled_kinds,
        'trial_started'), 'trial_ending'), 'trial_ended');
