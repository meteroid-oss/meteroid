-- Trials v2 (ADR 0015 §3, R-153, R-179, R-167): an offer's card default and what its end does
-- without a card, a manual end, and a trial's outcome recorded when it ends.

-- A check on end_action named by Postgres (inline in the table's creation), found by its values.
DO $$
DECLARE c record;
BEGIN
    FOR c IN SELECT conname FROM pg_constraint
             WHERE conrelid = 'trial_offer'::regclass AND contype = 'c'
               AND pg_get_constraintdef(oid) LIKE '%''CONVERT''%'
               AND pg_get_constraintdef(oid) LIKE '%''CANCEL''%'
    LOOP
        EXECUTE format('ALTER TABLE trial_offer DROP CONSTRAINT %I', c.conname);
    END LOOP;
END $$;

-- card_required: the default checkout, sign-up and the pricing page take; the subscription's
-- activation condition stays authoritative. without_card: what a converting trial's end does
-- when the subscription, charged automatically, resolves no payment method.
-- Added bare and IF NOT EXISTS: the pending-changes conversion adds them before it runs.
ALTER TABLE trial_offer
    ADD COLUMN IF NOT EXISTS card_required                BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN IF NOT EXISTS without_card                 TEXT    NOT NULL DEFAULT 'AWAIT_PAYMENT',
    ADD COLUMN IF NOT EXISTS without_card_plan_version_id UUID;
-- Sign-up started trials without a card before this release: the trials of sign-up plans keep it.
UPDATE trial_offer o SET card_required = FALSE
WHERE o.id IN (SELECT p.trial_offer_id FROM plan p
               JOIN product_family_journey j ON j.signup_plan_id = p.id
               WHERE p.trial_offer_id IS NOT NULL);
ALTER TABLE trial_offer
    ADD CONSTRAINT trial_offer_without_card_plan_version_id_fkey
        FOREIGN KEY (without_card_plan_version_id) REFERENCES plan_version (id),
    ADD CONSTRAINT trial_offer_end_action
        CHECK (end_action IN ('CONVERT', 'CANCEL', 'FALLBACK', 'MANUAL')),
    ADD CONSTRAINT trial_offer_without_card
        CHECK (without_card IN ('AWAIT_PAYMENT', 'CANCEL', 'SWITCH')),
    ADD CONSTRAINT trial_offer_without_card_plan
        CHECK ((without_card = 'SWITCH') = (without_card_plan_version_id IS NOT NULL));

-- The fact copies the offer's without_card at its start, as its end action; the outcome is
-- written when the trial ends (CONVERTED, CANCELLED, SWITCHED, AWAITING_PAYMENT,
-- AWAITING_DECISION), an awaiting one resolved later.
ALTER TABLE subscription_trial DROP CONSTRAINT subscription_trial_end_action;
ALTER TABLE subscription_trial
    ADD COLUMN IF NOT EXISTS outcome                      TEXT,
    ADD COLUMN IF NOT EXISTS ended_at                     TIMESTAMP,
    ADD COLUMN IF NOT EXISTS without_card                 TEXT NOT NULL DEFAULT 'AWAIT_PAYMENT',
    ADD COLUMN IF NOT EXISTS without_card_plan_version_id UUID;
ALTER TABLE subscription_trial
    ADD CONSTRAINT subscription_trial_without_card_plan_version_id_fkey
        FOREIGN KEY (without_card_plan_version_id) REFERENCES plan_version (id);
ALTER TABLE subscription_trial
    ADD CONSTRAINT subscription_trial_end_action
        CHECK (end_action IN ('CONVERT', 'CANCEL', 'FALLBACK', 'MANUAL')),
    ADD CONSTRAINT subscription_trial_outcome
        CHECK (outcome IN ('CONVERTED', 'CANCELLED', 'SWITCHED', 'AWAITING_PAYMENT',
                           'AWAITING_DECISION')),
    ADD CONSTRAINT subscription_trial_ended
        CHECK ((outcome IS NULL) = (ended_at IS NULL)),
    ADD CONSTRAINT subscription_trial_without_card
        CHECK (without_card IN ('AWAIT_PAYMENT', 'CANCEL', 'SWITCH')),
    ADD CONSTRAINT subscription_trial_without_card_plan
        CHECK ((without_card = 'SWITCH') = (without_card_plan_version_id IS NOT NULL));
CREATE INDEX idx_subscription_trial_without_card_plan_version_id
    ON subscription_trial (without_card_plan_version_id);
CREATE INDEX idx_trial_offer_without_card_plan_version_id
    ON trial_offer (without_card_plan_version_id);

-- The outcome of the trials that ended before, from their subscription's status: ended on or
-- before the trial's end, cancelled; awaiting its opening invoice's payment; falling back
-- through the phase switching plan on its end, switched; else converted.
WITH ended AS (
    SELECT t.subscription_id,
           s.status::text AS status,
           t.end_action,
           t.until_date,
           (SELECT MIN(e.at) FROM subscription_ending e
            WHERE e.subscription_id = t.subscription_id) AS ending_at
    FROM subscription_trial t
    JOIN subscription s ON s.id = t.subscription_id
    WHERE t.outcome IS NULL
      AND s.status::text IN ('ACTIVE', 'TRIAL_EXPIRED', 'CANCELLED', 'COMPLETED', 'ABORTED')
)
UPDATE subscription_trial t
SET outcome = CASE
        WHEN e.status IN ('CANCELLED', 'COMPLETED', 'ABORTED')
             AND (e.ending_at IS NULL OR e.ending_at <= e.until_date::timestamp) THEN 'CANCELLED'
        WHEN e.status = 'TRIAL_EXPIRED' THEN 'AWAITING_PAYMENT'
        WHEN e.end_action = 'FALLBACK' AND EXISTS (
            SELECT 1 FROM subscription_phase p
            WHERE p.subscription_id = e.subscription_id AND p.start_date = e.until_date
              AND p.withdrawn_at IS NULL) THEN 'SWITCHED'
        ELSE 'CONVERTED'
    END,
    ended_at = CASE
        WHEN e.status IN ('CANCELLED', 'COMPLETED', 'ABORTED') AND e.ending_at IS NOT NULL
            THEN LEAST(e.ending_at, e.until_date::timestamp)
        ELSE e.until_date::timestamp
    END
FROM ended e
WHERE e.subscription_id = t.subscription_id;

-- ── end of trials_v2 ──────────────────────────────────────────────────────────────────────────

-- Trial emails (trial_started, trial_ending, trial_ended): on for new invoicing entities, off for
-- existing ones until their tenant turns them on (announced). An entity without a settings row
-- reads the defaults, so it gets one.
UPDATE invoicing_entity_email_settings
SET disabled_kinds = disabled_kinds
    || ARRAY(SELECT k FROM unnest(ARRAY['trial_started', 'trial_ending', 'trial_ended']) AS k
             WHERE NOT (k = ANY (disabled_kinds)));
INSERT INTO invoicing_entity_email_settings (invoicing_entity_id, tenant_id, disabled_kinds)
SELECT ie.id,
       ie.tenant_id,
       '{payment_reminder,payment_method_expiring,trial_started,trial_ending,trial_ended}'
FROM invoicing_entity ie
WHERE NOT EXISTS (SELECT 1 FROM invoicing_entity_email_settings s
                  WHERE s.invoicing_entity_id = ie.id);

-- ── Checkout sessions and quotes ──────────────────────────────────────────────────────────────

-- The trial a checkout session was sent, read back as sent (plan, none, offer, custom by days or
-- end date, features of); its older columns stay written for the readers of before.
ALTER TABLE checkout_session ADD COLUMN IF NOT EXISTS trial_request JSONB;

-- What a quote's trial does at its end: convert, or wait for an operator's decision.
ALTER TABLE quote ADD COLUMN IF NOT EXISTS trial_end_action TEXT NOT NULL DEFAULT 'CONVERT';
ALTER TABLE quote
    ADD CONSTRAINT quote_trial_end_action CHECK (trial_end_action IN ('CONVERT', 'MANUAL'));

-- Offers (ADR 0019), release 1 (expand): an offer is a `trial_offer` row whose trial is optional,
-- with grants; each application of an offer to a subscription lists the engine rows it wrote.

-- A NULL length: the offer gives no trial. `standalone`: listed in the catalog; an offer made for
-- a plan or wrapping a coupon is not.
-- Added bare and `IF NOT EXISTS`: the release's conversion of pending changes reads them
-- (`COLUMNS_THE_CONVERSION_READS`).
ALTER TABLE trial_offer
    ALTER COLUMN duration_days DROP NOT NULL,
    ADD COLUMN IF NOT EXISTS standalone  BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN IF NOT EXISTS description TEXT;
ALTER TABLE trial_offer
    ADD CONSTRAINT trial_offer_description_length CHECK (length(description) <= 1000),
    ADD CONSTRAINT trial_offer_trial_plan_needs_trial
        CHECK (duration_days IS NOT NULL OR trial_plan_id IS NULL);

-- The offers the plan editor made: named after their plan, without a code, named by it.
UPDATE trial_offer o
SET standalone = FALSE
FROM plan p
WHERE p.trial_offer_id = o.id
  AND o.code IS NULL
  AND o.name = left(p.name, 200) || ' trial';

-- A coupon is a code naming an offer; NULL codes are first written by release 2.
ALTER TABLE coupon
    ALTER COLUMN code DROP NOT NULL,
    ADD COLUMN IF NOT EXISTS offer_id UUID;
ALTER TABLE coupon
    ADD CONSTRAINT coupon_offer_id_fkey FOREIGN KEY (offer_id) REFERENCES trial_offer (id);
CREATE INDEX idx_coupon_offer_id ON coupon (offer_id);

CREATE TABLE offer_grant (
    id             UUID      PRIMARY KEY,
    tenant_id      UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    offer_id       UUID      NOT NULL REFERENCES trial_offer (id) ON DELETE CASCADE,
    seq            INTEGER   NOT NULL CHECK (seq >= 0),
    kind           TEXT      NOT NULL CHECK (kind IN ('DISCOUNT', 'CREDIT', 'ENTITLEMENT', 'ADD_ON')),
    anchor         TEXT      NOT NULL CHECK (anchor IN ('START', 'BILLING_START')),
    duration_kind  TEXT      NOT NULL
        CHECK (duration_kind IN ('FOREVER', 'DAYS', 'MONTHS', 'TRIAL', 'INVOICES')),
    duration_value INTEGER   CHECK (duration_value >= 0),
    -- DISCOUNT and CREDIT: the coupon row the engine reads, its definition.
    coupon_id      UUID      REFERENCES coupon (id) ON DELETE CASCADE,
    feature_id     UUID      REFERENCES feature (id),
    -- ENTITLEMENT: `{value, effect}` as the entitlement row takes them.
    entitlement    JSONB,
    add_on_id      UUID      REFERENCES add_on (id),
    quantity       INTEGER   CHECK (quantity >= 1),
    add_on_end     TEXT      CHECK (add_on_end IN ('REVERT', 'CONVERT')),
    removed_at     TIMESTAMP,
    created_at     TIMESTAMP NOT NULL,
    CONSTRAINT offer_grant_duration_value
        CHECK ((duration_kind IN ('DAYS', 'MONTHS', 'INVOICES')) = (duration_value IS NOT NULL)),
    CONSTRAINT offer_grant_coupon CHECK ((kind IN ('DISCOUNT', 'CREDIT')) = (coupon_id IS NOT NULL)),
    CONSTRAINT offer_grant_entitlement
        CHECK ((kind = 'ENTITLEMENT') = (feature_id IS NOT NULL AND entitlement IS NOT NULL)),
    CONSTRAINT offer_grant_add_on
        CHECK ((kind = 'ADD_ON') = (add_on_id IS NOT NULL AND quantity IS NOT NULL
                                    AND add_on_end IS NOT NULL)),
    CONSTRAINT offer_grant_discount_window
        CHECK (kind <> 'DISCOUNT' OR (anchor = 'BILLING_START'
                                      AND duration_kind IN ('FOREVER', 'MONTHS', 'INVOICES'))),
    CONSTRAINT offer_grant_invoices CHECK (duration_kind <> 'INVOICES' OR kind = 'DISCOUNT')
);
CREATE UNIQUE INDEX idx_offer_grant_live_seq ON offer_grant (offer_id, seq) WHERE removed_at IS NULL;
CREATE INDEX idx_offer_grant_tenant_id ON offer_grant (tenant_id);
CREATE INDEX idx_offer_grant_coupon_id ON offer_grant (coupon_id);
CREATE INDEX idx_offer_grant_feature_id ON offer_grant (feature_id);
CREATE INDEX idx_offer_grant_add_on_id ON offer_grant (add_on_id);

-- An offer applied to a subscription, once (R-173); its id derives from the subscription, the
-- offer and the coupon, so a replay writes nothing.
CREATE TABLE offer_application (
    id               UUID             PRIMARY KEY,
    tenant_id        UUID             NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    offer_id         UUID             NOT NULL REFERENCES trial_offer (id),
    customer_id      UUID             NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    subscription_id  UUID             NOT NULL REFERENCES subscription (id) ON DELETE CASCADE,
    -- A coupon deleted takes its redemptions with it, as its applied coupons.
    coupon_id        UUID             REFERENCES coupon (id) ON DELETE CASCADE,
    source           TEXT             NOT NULL
        CHECK (source IN ('PLAN', 'OPERATOR', 'COUPON', 'RETENTION', 'QUOTE', 'API')),
    start_on         DATE             NOT NULL,
    billing_start_on DATE             NOT NULL,
    applied_at       TIMESTAMP        NOT NULL,
    actor_type       "ActorTypeEnum"  NOT NULL,
    actor_uuid       UUID,
    actor_alias      TEXT,
    -- When its grants written at the subscription's start were written.
    started_at       TIMESTAMP,
    withdrawn_at     TIMESTAMP,
    -- The grants it gives, as the offer had them when applied.
    grant_ids        UUID[]           NOT NULL DEFAULT '{}',
    CONSTRAINT offer_application_once UNIQUE (subscription_id, offer_id),
    CONSTRAINT offer_application_coupon CHECK ((source = 'COUPON') = (coupon_id IS NOT NULL))
);
CREATE INDEX idx_offer_application_tenant_id ON offer_application (tenant_id);
CREATE INDEX idx_offer_application_offer_customer ON offer_application (offer_id, customer_id);
CREATE INDEX idx_offer_application_customer_id ON offer_application (customer_id);
CREATE INDEX idx_offer_application_coupon_id ON offer_application (coupon_id);

-- The engine rows an application wrote, per grant: an applied coupon, a credit grant, an
-- entitlement or a dated add-on.
CREATE TABLE offer_application_item (
    application_id UUID      NOT NULL REFERENCES offer_application (id) ON DELETE CASCADE,
    grant_id       UUID      NOT NULL REFERENCES offer_grant (id) ON DELETE CASCADE,
    tenant_id      UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    kind           TEXT      NOT NULL
        CHECK (kind IN ('APPLIED_COUPON', 'CREDIT_GRANT', 'ENTITLEMENT', 'ADD_ON')),
    target_id      UUID      NOT NULL,
    created_at     TIMESTAMP NOT NULL,
    -- Ended: withdrawn, with its subscription or at its window's end (`grant.ended` sent once).
    ended_at       TIMESTAMP,
    -- When its grant's window ends on its own (R-178); NULL never (forever, counted in invoices).
    ends_at        TIMESTAMP,
    PRIMARY KEY (kind, target_id)
);
CREATE INDEX idx_offer_application_item_application_id ON offer_application_item (application_id);
CREATE INDEX idx_offer_application_item_grant_id ON offer_application_item (grant_id);
CREATE INDEX idx_offer_application_item_tenant_id ON offer_application_item (tenant_id);
CREATE INDEX idx_offer_application_item_window_end ON offer_application_item (ends_at)
    WHERE ended_at IS NULL AND ends_at IS NOT NULL;

ALTER TABLE retention_offer ADD COLUMN offer_id UUID REFERENCES trial_offer (id);
CREATE INDEX idx_retention_offer_offer_id ON retention_offer (offer_id);

-- The offer a quote's subscription gets, frozen when it is published (release 3 of the plan).
CREATE TABLE quote_offer (
    quote_id       UUID        PRIMARY KEY REFERENCES quote (id) ON DELETE CASCADE,
    tenant_id      UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    offer_id       UUID        REFERENCES trial_offer (id),
    offer_snapshot JSONB,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_quote_offer_tenant_id ON quote_offer (tenant_id);
CREATE INDEX idx_quote_offer_offer_id ON quote_offer (offer_id);

-- A checkout session's offer as asked (`{"type": "plan" | "none" | "offer", "id"?}`); NULL asks
-- the plan's.
ALTER TABLE checkout_session ADD COLUMN IF NOT EXISTS offer_request JSONB;

-- Each coupon wrapped in an offer of its own whose one grant is the coupon itself, ids derived
-- from the coupon's, so wrapping again writes nothing. A coupon a grant owns is not wrapped.
CREATE FUNCTION offer_wrap_coupons(coupon_ids UUID[]) RETURNS VOID
    LANGUAGE sql AS
$$
INSERT INTO trial_offer (id, tenant_id, name, code, duration_days, free, trial_plan_id, end_action,
                         fallback_plan_version_id, eligibility_scope, archived_at, created_at,
                         updated_at, standalone)
SELECT md5('coupon-offer:' || c.id::text)::uuid, c.tenant_id,
       COALESCE(c.code, NULLIF(c.description, ''), 'Coupon'), NULL, NULL, TRUE, NULL, 'CONVERT',
       NULL, 'NONE', NULL, c.created_at, c.created_at, FALSE
FROM coupon c
WHERE c.id = ANY (coupon_ids)
  AND c.offer_id IS NULL
  AND NOT EXISTS (SELECT 1 FROM offer_grant g WHERE g.coupon_id = c.id)
ON CONFLICT (id) DO NOTHING;

INSERT INTO offer_grant (id, tenant_id, offer_id, seq, kind, anchor, duration_kind,
                         duration_value, coupon_id, created_at)
SELECT md5('coupon-grant:' || c.id::text)::uuid, c.tenant_id,
       md5('coupon-offer:' || c.id::text)::uuid, 0,
       CASE WHEN c.discount ? 'Credit' THEN 'CREDIT' ELSE 'DISCOUNT' END,
       CASE WHEN c.discount ? 'Credit' THEN 'START' ELSE 'BILLING_START' END,
       CASE
           WHEN c.discount ? 'Credit' THEN
               CASE WHEN c.discount -> 'Credit' ->> 'expires_in_days' IS NULL THEN 'FOREVER'
                    ELSE 'DAYS' END
           WHEN c.duration_months IS NOT NULL THEN 'MONTHS'
           WHEN c.recurring_value IS NOT NULL THEN 'INVOICES'
           ELSE 'FOREVER'
       END,
       CASE
           WHEN c.discount ? 'Credit' THEN (c.discount -> 'Credit' ->> 'expires_in_days')::int
           WHEN c.duration_months IS NOT NULL THEN GREATEST(c.duration_months, 0)
           WHEN c.recurring_value IS NOT NULL THEN GREATEST(c.recurring_value, 0)
       END,
       c.id, c.created_at
FROM coupon c
WHERE c.id = ANY (coupon_ids)
  AND c.offer_id IS NULL
  AND NOT EXISTS (SELECT 1 FROM offer_grant g WHERE g.coupon_id = c.id)
ON CONFLICT (id) DO NOTHING;

UPDATE coupon c
SET offer_id = md5('coupon-offer:' || c.id::text)::uuid
WHERE c.id = ANY (coupon_ids)
  AND c.offer_id IS NULL
  AND EXISTS (SELECT 1 FROM offer_grant g
              WHERE g.id = md5('coupon-grant:' || c.id::text)::uuid AND g.coupon_id = c.id);

UPDATE retention_offer r
SET offer_id = c.offer_id
FROM coupon c
WHERE r.coupon_id = c.id
  AND c.id = ANY (coupon_ids)
  AND c.offer_id IS NOT NULL
  AND r.offer_id IS DISTINCT FROM c.offer_id;
$$;

SELECT offer_wrap_coupons(ARRAY(SELECT id FROM coupon));

-- A pending credit grant dated again (its offer's trial moved) changes what the wallet projects.
SELECT pgmq.bind_topic('credit_grant.updated', 'credit_recompute');
