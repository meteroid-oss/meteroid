-- ── Subscription phases ──────────────────────────────────────────────────────────────────────
-- A phase is a dated stretch of a subscription with its own terms (a ramp's year). It is authoring
-- only: its boundary is one subscription_change of kind PHASE whose written facts are the rows it
-- opens and closes; billing reads those rows, never the phase.
ALTER TYPE "SubscriptionChangeKindEnum" ADD VALUE IF NOT EXISTS 'PHASE';

CREATE TABLE subscription_phase (
    id              UUID      PRIMARY KEY,
    tenant_id       UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    subscription_id UUID      NOT NULL REFERENCES subscription (id) ON DELETE CASCADE,
    seq             INT4      NOT NULL,
    start_date      DATE      NOT NULL,
    label           TEXT,
    -- The phase's absolute terms (`domain::subscription_phases::PhaseTerms`), never a delta.
    terms           JSONB     NOT NULL,
    -- The change writing its boundary; withdrawn with the phase.
    change_id       UUID      NOT NULL REFERENCES subscription_change (id),
    created_by      UUID,
    created_at      TIMESTAMP NOT NULL,
    withdrawn_at    TIMESTAMP
);
CREATE UNIQUE INDEX idx_subscription_phase_live_seq ON subscription_phase (subscription_id, seq)
    WHERE withdrawn_at IS NULL;
CREATE UNIQUE INDEX idx_subscription_phase_live_start ON subscription_phase (subscription_id, start_date)
    WHERE withdrawn_at IS NULL;
CREATE INDEX idx_subscription_phase_tenant_id ON subscription_phase (tenant_id);
CREATE INDEX idx_subscription_phase_change_id ON subscription_phase (change_id);

-- Plan ramps were never billed nor shown: subscription phases (Enterprise) replace them.
DROP TABLE IF EXISTS schedule;

-- A dated add-on of a subscription (an add-on for three months, a trial of one converting to
-- paid) or a dated discount of one of its fees: its start and end are timeline changes (kind
-- PHASE) on renewals, written ahead like a phase's boundary; billing reads the rows they write.
CREATE TABLE subscription_timeline_item (
    id              UUID      PRIMARY KEY,
    tenant_id       UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    subscription_id UUID      NOT NULL REFERENCES subscription (id) ON DELETE CASCADE,
    kind            TEXT      NOT NULL CHECK (kind IN ('ADD_ON', 'DISCOUNT')),
    add_on_id       UUID      REFERENCES add_on (id),
    -- ADD_ON: the add-on row it starts, or the one already on the subscription it ends.
    -- DISCOUNT: the lineage of the component it discounts.
    lineage_id      UUID      NOT NULL,
    start_date      DATE      NOT NULL,
    until_date      DATE      NOT NULL,
    -- `TimelineItemTerms` (free or not, and what happens at the end) or `DiscountTerms`.
    terms           JSONB     NOT NULL,
    label           TEXT,
    reason          TEXT,
    start_change_id UUID      REFERENCES subscription_change (id),
    end_change_id   UUID      NOT NULL REFERENCES subscription_change (id),
    created_by      UUID,
    created_at      TIMESTAMP NOT NULL,
    withdrawn_at    TIMESTAMP,
    CHECK (until_date > start_date),
    CHECK ((kind = 'ADD_ON') = (add_on_id IS NOT NULL))
);
CREATE INDEX idx_subscription_timeline_item_subscription ON subscription_timeline_item (subscription_id);
CREATE INDEX idx_subscription_timeline_item_tenant_id ON subscription_timeline_item (tenant_id);

-- A subscription's contract term: where it ends, and whether it renews. Authoring only: an end
-- without renewal is the subscription's COMPLETE ending, a renewal is a phase at the term's end
-- (ADR 0015, R-152); billing reads those, never this.
CREATE TABLE subscription_term (
    subscription_id  UUID      PRIMARY KEY REFERENCES subscription (id) ON DELETE CASCADE,
    tenant_id        UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    term_end         DATE      NOT NULL,
    auto_renew       BOOLEAN   NOT NULL,
    renewal_months   INTEGER   NOT NULL CHECK (renewal_months > 0),
    -- Percent added to each fee priced by one amount at each renewal; none keeps the prices.
    uplift_percent   NUMERIC,
    notice_days      INTEGER   NOT NULL DEFAULT 0 CHECK (notice_days >= 0),
    -- The phase that renews the term on `term_end`, while it auto-renews.
    renewal_phase_id UUID      REFERENCES subscription_phase (id),
    created_by       UUID,
    updated_at       TIMESTAMP NOT NULL
);
CREATE INDEX idx_subscription_term_tenant_id ON subscription_term (tenant_id);

-- Renewal notices (R-157): one policy per tenant for every renewal, a term's or a period's.
-- Checkpoints count days to the opt-out deadline of a term with notice, else to the date itself.
CREATE TABLE renewal_settings (
    tenant_id              UUID PRIMARY KEY REFERENCES tenant (id) ON DELETE CASCADE,
    checkpoints            INTEGER[]   NOT NULL DEFAULT '{90,60,30}' CHECK (
        cardinality(checkpoints) <= 6
        AND array_position(checkpoints, NULL) IS NULL
        AND 1 <= ALL (checkpoints) AND 365 >= ALL (checkpoints)
    ),
    -- Days before the same date the customer is emailed (R-161); NULL emails no one.
    customer_reminder_days INTEGER CHECK (customer_reminder_days BETWEEN 1 AND 365),
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO renewal_settings (tenant_id, checkpoints, updated_at)
SELECT tenant_id, renewal_checkpoints, updated_at FROM retention_settings;
ALTER TABLE retention_settings DROP COLUMN renewal_checkpoints;

-- The retention module's claims become every renewal's: the notices already sent keep deduping.
ALTER TABLE retention_renewal_notice RENAME TO subscription_renewal_notice;
ALTER TABLE subscription_renewal_notice RENAME CONSTRAINT retention_renewal_notice_pkey
    TO subscription_renewal_notice_pkey;
ALTER TABLE subscription_renewal_notice
    ADD COLUMN audience TEXT NOT NULL DEFAULT 'TEAM' CHECK (audience IN ('TEAM', 'CUSTOMER')),
    ADD CONSTRAINT subscription_renewal_notice_mode CHECK (mode IN ('RENEWING', 'ENDING', 'CANCELLING')),
    DROP CONSTRAINT subscription_renewal_notice_pkey,
    ADD PRIMARY KEY (subscription_id, renewal_date, mode, audience, days_before);
CREATE INDEX idx_subscription_renewal_notice_tenant_id ON subscription_renewal_notice (tenant_id);

-- Trials (ADR 0015 §3, R-153, R-159): one definition, the trial offer. A plan's trial is the
-- offer it names (`plan.trial_offer_id`), the same on every version; the catalog adds offers of
-- their own. Billing reads the subscription's trial fact, never this.
CREATE TABLE trial_offer (
    id                       UUID      PRIMARY KEY,
    tenant_id                UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    name                     TEXT      NOT NULL,
    -- How the API names it beside its id, unique in the tenant.
    code                     TEXT,
    duration_days            INTEGER   NOT NULL CHECK (duration_days BETWEEN 1 AND 3650),
    free                     BOOLEAN   NOT NULL,
    -- The plan whose features the trial gives over the subscription's own (a reverse trial).
    trial_plan_id            UUID      REFERENCES plan (id) ON DELETE SET NULL,
    -- CONVERT: the subscription goes on; CANCEL: it ends with the trial; FALLBACK: it switches to
    -- `fallback_plan_version_id` at the trial's end (free trials only).
    end_action               TEXT      NOT NULL DEFAULT 'CONVERT'
        CHECK (end_action IN ('CONVERT', 'CANCEL', 'FALLBACK')),
    fallback_plan_version_id UUID      REFERENCES plan_version (id),
    -- OFFER, PLAN or PRODUCT_FAMILY: what a customer is given once; NONE: no limit.
    eligibility_scope        TEXT      NOT NULL DEFAULT 'NONE'
        CHECK (eligibility_scope IN ('OFFER', 'PLAN', 'PRODUCT_FAMILY', 'NONE')),
    archived_at              TIMESTAMP,
    created_at               TIMESTAMP NOT NULL,
    updated_at               TIMESTAMP NOT NULL,
    CHECK ((end_action = 'FALLBACK') = (fallback_plan_version_id IS NOT NULL)),
    CHECK (end_action <> 'FALLBACK' OR free)
);
CREATE INDEX idx_trial_offer_tenant_id ON trial_offer (tenant_id);
CREATE UNIQUE INDEX idx_trial_offer_code ON trial_offer (tenant_id, code) WHERE code IS NOT NULL;

-- Added bare before the release's conversion (`COLUMNS_THE_CONVERSION_READS`).
ALTER TABLE plan ADD COLUMN IF NOT EXISTS trial_offer_id UUID;
ALTER TABLE plan ADD CONSTRAINT plan_trial_offer_id_fkey
    FOREIGN KEY (trial_offer_id) REFERENCES trial_offer (id);
CREATE INDEX idx_plan_trial_offer_id ON plan (trial_offer_id);

-- Each plan's trial becomes its offer: its active version's, else its draft's (a plan never
-- published). Older versions' and a draft's differing trial go (preflight trials_to_offers).
CREATE TEMP TABLE moved_trial ON COMMIT DROP AS
SELECT p.id AS plan_id, p.tenant_id, md5('plan-trial:' || p.id::text)::uuid AS offer_id,
       left(p.name, 200) || ' trial' AS name, LEAST(pv.trial_duration_days, 3650) AS days,
       pv.trial_is_free AS free, NULLIF(pv.trialing_plan_id, p.id) AS trial_plan_id
FROM plan p
JOIN plan_version pv ON pv.id = COALESCE(p.active_version_id, p.draft_version_id)
WHERE pv.trial_duration_days > 0;
INSERT INTO trial_offer (id, tenant_id, name, duration_days, free, trial_plan_id, created_at,
                         updated_at)
SELECT offer_id, tenant_id, name, days, free, trial_plan_id, now() AT TIME ZONE 'UTC',
       now() AT TIME ZONE 'UTC'
FROM moved_trial;
UPDATE plan p SET trial_offer_id = m.offer_id FROM moved_trial m WHERE m.plan_id = p.id;

-- A trial's own record, fixed at its start: the plan it gives, the offer and the end action.
-- Running trials take the trialing plan of the version they started on (read live until now).
ALTER TABLE subscription_trial
    ADD COLUMN IF NOT EXISTS trial_plan_id UUID,
    ADD COLUMN IF NOT EXISTS trial_offer_id UUID,
    ADD COLUMN IF NOT EXISTS end_action TEXT NOT NULL DEFAULT 'CONVERT';
ALTER TABLE subscription_trial
    ADD CONSTRAINT subscription_trial_trial_plan_id_fkey
        FOREIGN KEY (trial_plan_id) REFERENCES plan (id) ON DELETE SET NULL,
    ADD CONSTRAINT subscription_trial_trial_offer_id_fkey
        FOREIGN KEY (trial_offer_id) REFERENCES trial_offer (id),
    ADD CONSTRAINT subscription_trial_end_action
        CHECK (end_action IN ('CONVERT', 'CANCEL', 'FALLBACK'));
CREATE TEMP TABLE trial_start_version ON COMMIT DROP AS
SELECT DISTINCT ON (w.subscription_id) w.subscription_id, w.plan_version_id
FROM subscription_plan_window w
ORDER BY w.subscription_id, w.from_date, w.id;
UPDATE subscription_trial t SET trial_plan_id = NULLIF(pv.trialing_plan_id, pv.plan_id)
FROM trial_start_version w
JOIN plan_version pv ON pv.id = w.plan_version_id
WHERE w.subscription_id = t.subscription_id AND pv.trialing_plan_id IS NOT NULL;

-- Every trial given at a creation, with the plan and family it started on, so an offer given once
-- per plan or family counts the trials before it (R-159).
CREATE TABLE trial_redemption (
    subscription_id   UUID      PRIMARY KEY REFERENCES subscription (id) ON DELETE CASCADE,
    tenant_id         UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    customer_id       UUID      NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    trial_offer_id    UUID      REFERENCES trial_offer (id),
    plan_id           UUID      NOT NULL REFERENCES plan (id) ON DELETE CASCADE,
    product_family_id UUID      NOT NULL REFERENCES product_family (id) ON DELETE CASCADE,
    created_at        TIMESTAMP NOT NULL
);
CREATE INDEX idx_trial_redemption_customer ON trial_redemption (tenant_id, customer_id);
INSERT INTO trial_redemption (subscription_id, tenant_id, customer_id, plan_id, product_family_id,
                              created_at)
SELECT t.subscription_id, t.tenant_id, s.customer_id, pl.id, pl.product_family_id, t.created_at
FROM subscription_trial t
JOIN subscription s ON s.id = t.subscription_id
JOIN trial_start_version w ON w.subscription_id = t.subscription_id
JOIN plan_version pv ON pv.id = w.plan_version_id
JOIN plan pl ON pl.id = pv.plan_id;

-- A quote and a checkout session state their trial whole: its freeness, a quote its trial plan,
-- a session the offer it names. Added bare before the release's conversion as well.
ALTER TABLE quote ADD COLUMN IF NOT EXISTS trial_free BOOLEAN,
    ADD COLUMN IF NOT EXISTS trial_plan_id UUID;
ALTER TABLE quote ADD CONSTRAINT quote_trial_plan_id_fkey
    FOREIGN KEY (trial_plan_id) REFERENCES plan (id) ON DELETE SET NULL;
ALTER TABLE checkout_session ADD COLUMN IF NOT EXISTS trial_free BOOLEAN,
    ADD COLUMN IF NOT EXISTS trial_offer_id UUID;
ALTER TABLE checkout_session ADD CONSTRAINT checkout_session_trial_offer_id_fkey
    FOREIGN KEY (trial_offer_id) REFERENCES trial_offer (id);

-- The trial fact is a subscription's only record of its trial (I29); versions carry none.
ALTER TABLE plan_version DROP COLUMN trial_duration_days, DROP COLUMN trial_is_free,
    DROP COLUMN trialing_plan_id;
ALTER TABLE subscription DROP COLUMN trial_duration;

-- A plan version's phase template: its phases at offsets in months from a subscription's billing
-- start, written as the subscription's own phases when it is created (ADR 0015, R-154). The
-- first, at offset 0, prices the fees the subscription starts with.
CREATE TABLE plan_version_phase (
    id              UUID      PRIMARY KEY,
    tenant_id       UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    plan_version_id UUID      NOT NULL REFERENCES plan_version (id) ON DELETE CASCADE,
    seq             INTEGER   NOT NULL,
    offset_months   INTEGER   NOT NULL CHECK (offset_months >= 0),
    label           TEXT,
    -- `domain::subscription_phases::PhaseTerms`, its fees named by plan component.
    terms           JSONB     NOT NULL,
    created_at      TIMESTAMP NOT NULL,
    UNIQUE (plan_version_id, seq),
    UNIQUE (plan_version_id, offset_months)
);
CREATE INDEX idx_plan_version_phase_tenant_id ON plan_version_phase (tenant_id);

-- A quote's phases, at offsets in months from the subscription's billing start, written as the
-- subscription's own phases when the quote converts (ADR 0015, R-155): a ramp, an escalator,
-- free months, a pilot switching plan.
CREATE TABLE quote_phase (
    id            UUID      PRIMARY KEY,
    tenant_id     UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    quote_id      UUID      NOT NULL REFERENCES quote (id) ON DELETE CASCADE,
    seq           INTEGER   NOT NULL,
    offset_months INTEGER   NOT NULL CHECK (offset_months >= 0),
    label         TEXT,
    -- `domain::subscription_phases::PhaseTerms`, its fees named by plan component.
    terms         JSONB     NOT NULL,
    created_at    TIMESTAMP NOT NULL,
    UNIQUE (quote_id, seq),
    UNIQUE (quote_id, offset_months)
);
CREATE INDEX idx_quote_phase_tenant_id ON quote_phase (tenant_id);

-- Notices published once each: an override's end per lineage and kind (ending soon, ended), so an
-- edit moving its end re-arms them (R-148); a trial's end, re-armed when the trial moves (R-158).
CREATE TABLE entitlement_override_notice (
    lineage_id  UUID        NOT NULL,
    valid_until TIMESTAMPTZ NOT NULL,
    ended       BOOLEAN     NOT NULL,
    tenant_id   UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    noticed_at  TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (lineage_id, valid_until, ended)
);
CREATE INDEX idx_entitlement_override_notice_tenant_id ON entitlement_override_notice (tenant_id);

CREATE TABLE subscription_trial_notice (
    subscription_id UUID        NOT NULL REFERENCES subscription (id) ON DELETE CASCADE,
    trial_end       DATE        NOT NULL,
    tenant_id       UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    noticed_at      TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (subscription_id, trial_end)
);
CREATE INDEX idx_subscription_trial_notice_tenant_id ON subscription_trial_notice (tenant_id);

-- What a quote is worth as billing bills it (R-165), valued after each write; a revision keeps
-- the value it was published with.
CREATE TABLE quote_contract_value (
    quote_id    UUID      PRIMARY KEY REFERENCES quote (id) ON DELETE CASCADE,
    tenant_id   UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    value       JSONB     NOT NULL,
    computed_at TIMESTAMP NOT NULL
);
CREATE INDEX idx_quote_contract_value_tenant_id ON quote_contract_value (tenant_id);
ALTER TABLE quote_revision ADD COLUMN contract_value JSONB;
