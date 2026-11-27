ALTER TABLE quote_revision DROP COLUMN IF EXISTS contract_value;
DROP TABLE IF EXISTS quote_contract_value;

DROP TABLE IF EXISTS subscription_trial_notice;
DROP TABLE IF EXISTS entitlement_override_notice;

DROP TABLE IF EXISTS quote_phase;

DROP TABLE IF EXISTS plan_version_phase;

-- Trials back onto plan versions and subscriptions: each plan's trial on its versions, each
-- subscription's length from its trial fact.
ALTER TABLE subscription ADD COLUMN trial_duration INTEGER;
UPDATE subscription s SET trial_duration = t.until_date - t.from_date
FROM subscription_trial t WHERE t.subscription_id = s.id;
ALTER TABLE plan_version
    ADD COLUMN trial_duration_days INTEGER,
    ADD COLUMN trialing_plan_id UUID REFERENCES plan (id),
    ADD COLUMN trial_is_free BOOLEAN NOT NULL DEFAULT TRUE;
UPDATE plan_version pv SET trial_duration_days = o.duration_days, trial_is_free = o.free,
                           trialing_plan_id = o.trial_plan_id
FROM plan p JOIN trial_offer o ON o.id = p.trial_offer_id
WHERE pv.plan_id = p.id;
ALTER TABLE checkout_session DROP COLUMN IF EXISTS trial_offer_id, DROP COLUMN IF EXISTS trial_free;
ALTER TABLE quote DROP COLUMN IF EXISTS trial_plan_id, DROP COLUMN IF EXISTS trial_free;
DROP TABLE IF EXISTS trial_redemption;
ALTER TABLE subscription_trial DROP COLUMN IF EXISTS end_action,
    DROP COLUMN IF EXISTS trial_offer_id, DROP COLUMN IF EXISTS trial_plan_id;
ALTER TABLE plan DROP COLUMN IF EXISTS trial_offer_id;
DROP TABLE IF EXISTS trial_offer;

DROP INDEX IF EXISTS idx_subscription_renewal_notice_tenant_id;
DELETE FROM subscription_renewal_notice WHERE audience <> 'TEAM';
ALTER TABLE subscription_renewal_notice
    DROP CONSTRAINT subscription_renewal_notice_pkey,
    DROP CONSTRAINT subscription_renewal_notice_mode,
    DROP COLUMN audience,
    ADD CONSTRAINT retention_renewal_notice_pkey
        PRIMARY KEY (subscription_id, renewal_date, days_before, mode);
ALTER TABLE subscription_renewal_notice RENAME TO retention_renewal_notice;

ALTER TABLE retention_settings ADD COLUMN renewal_checkpoints INTEGER[] NOT NULL DEFAULT '{90,60,30}' CHECK (
    cardinality(renewal_checkpoints) <= 6
    AND array_position(renewal_checkpoints, NULL) IS NULL
    AND 1 <= ALL (renewal_checkpoints) AND 365 >= ALL (renewal_checkpoints)
);
UPDATE retention_settings r SET renewal_checkpoints = s.checkpoints
FROM renewal_settings s WHERE s.tenant_id = r.tenant_id;
DROP TABLE IF EXISTS renewal_settings;

DROP TABLE IF EXISTS subscription_term;

DROP TABLE IF EXISTS subscription_timeline_item;

CREATE TABLE schedule (
    id              UUID                NOT NULL PRIMARY KEY,
    billing_period  "BillingPeriodEnum" NOT NULL,
    plan_version_id UUID                NOT NULL REFERENCES plan_version (id) ON UPDATE CASCADE ON DELETE CASCADE,
    ramps           JSONB               NOT NULL
);
CREATE INDEX idx_schedule_plan_version ON schedule (plan_version_id);

DROP TABLE IF EXISTS subscription_phase;
-- Postgres cannot drop an enum value in place. The rows phase changes wrote stay and keep
-- billing: withdraw the phases still to start through the API before rolling this back.
UPDATE subscription_change SET withdrawn_at = now(), withdrawn_reason = 'Phases removed'
    WHERE kind::text = 'PHASE' AND withdrawn_at IS NULL;
