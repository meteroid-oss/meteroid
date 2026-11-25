-- The pricing model: managed quantities, rate cards, self-serve journeys, eligibility, coupons
-- widened, add-on prices, plan options. One migration for what the branch built in steps.
-- A long transaction of a serving pod fails this run instead of queueing every query behind it;
-- the next start retries.
SET LOCAL lock_timeout = '5s';

-- ── managed quantity ──

-- A quantity a subscription holds over time (seats, licences, projects), shared by the charges,
-- limits and grants that read it. Each subscription holds its own value of it.
-- An increase takes effect at once (billed as its collection says) or at the next renewal; a
-- decrease at the renewal or at once, its unused time given back.
CREATE TYPE "QuantityIncreaseEnum" AS ENUM ('IMMEDIATELY', 'NEXT_PERIOD');
CREATE TYPE "QuantityDecreaseEnum" AS ENUM ('AT_PERIOD_END', 'IMMEDIATELY');

CREATE TABLE managed_quantity (
    id          UUID PRIMARY KEY,
    tenant_id   UUID                   NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    code        TEXT                   NOT NULL,
    name        TEXT                   NOT NULL,
    on_increase "QuantityIncreaseEnum" NOT NULL DEFAULT 'IMMEDIATELY',
    on_decrease "QuantityDecreaseEnum" NOT NULL DEFAULT 'AT_PERIOD_END',
    created_at  TIMESTAMPTZ            NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ            NOT NULL DEFAULT now(),
    archived_at TIMESTAMPTZ,
    UNIQUE (tenant_id, code)
);

-- One quantity per distinct seat unit a tenant used: on a seat ledger, a seat product, or a v1
-- seat component. The unit is the code, so seats keep sharing a count where they did.
INSERT INTO managed_quantity (id, tenant_id, code, name)
SELECT gen_random_uuid(), tenant_id, unit, unit
FROM (
    SELECT s.tenant_id, st.unit
    FROM slot_transaction st
    JOIN subscription s ON s.id = st.subscription_id
    UNION
    SELECT p.tenant_id, p.fee_structure ->> 'unit_name'
    FROM product p
    WHERE p.fee_structure ->> 'type' = 'Slot'
    UNION
    SELECT pv.tenant_id, pc.legacy_fee -> 'Slot' ->> 'slot_unit_name'
    FROM price_component pc
    JOIN plan_version pv ON pv.id = pc.plan_version_id
    WHERE pc.legacy_fee ? 'Slot'
) units
WHERE unit IS NOT NULL;

ALTER TABLE slot_transaction ADD COLUMN managed_quantity_id UUID REFERENCES managed_quantity (id) ON DELETE RESTRICT;

UPDATE slot_transaction st
SET managed_quantity_id = mq.id
FROM subscription s, managed_quantity mq
WHERE s.id = st.subscription_id
  AND mq.tenant_id = s.tenant_id
  AND mq.code = st.unit;

ALTER TABLE slot_transaction ALTER COLUMN managed_quantity_id SET NOT NULL;

CREATE INDEX slot_transaction_managed_quantity_idx ON slot_transaction (managed_quantity_id);

-- A removal waits for its fee's renewal (R-108) unless it was made to take effect at once.
ALTER TABLE slot_transaction ADD COLUMN removal_waits_for_renewal BOOLEAN NOT NULL DEFAULT TRUE;

-- ── rate cards ──

-- Usage no entry's key matches: hold the invoice until it is rated, or leave it unbilled on
-- purpose. A key of `*` only is a fallback rate that still flags the usage it rates.
CREATE TYPE "RateCardUnmatchedEnum" AS ENUM ('HOLD', 'IGNORE');

-- The rates of a metric's usage by its dimensions (model, token type), independent of plans:
-- prices name a card instead of copying its rows. Rates are in one denomination, a currency or
-- a credit unit.
CREATE TABLE rate_card (
    id             UUID PRIMARY KEY,
    tenant_id      UUID                    NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    code           TEXT                    NOT NULL,
    name           TEXT                    NOT NULL,
    metric_id      UUID                    NOT NULL REFERENCES billable_metric (id),
    dimensions     TEXT[]                  NOT NULL CHECK (cardinality(dimensions) BETWEEN 1 AND 4),
    currency       TEXT,
    credit_unit_id UUID REFERENCES credit_unit (id),
    on_unmatched   "RateCardUnmatchedEnum" NOT NULL DEFAULT 'HOLD',
    created_at     TIMESTAMPTZ             NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ             NOT NULL DEFAULT now(),
    archived_at    TIMESTAMPTZ,
    CHECK ((currency IS NULL) <> (credit_unit_id IS NULL)),
    UNIQUE (tenant_id, code),
    UNIQUE (tenant_id, id)
);

-- A rate for the usage its key matches (one pattern per dimension of the card: a value, `p*`
-- or `*`), from its day until a later entry of the same key. Never edited: a change is a new
-- entry from a later day.
CREATE TABLE rate_card_entry (
    id             UUID PRIMARY KEY,
    tenant_id      UUID        NOT NULL,
    rate_card_id   UUID        NOT NULL,
    key            TEXT[]      NOT NULL,
    effective_from DATE        NOT NULL,
    rate           NUMERIC     NOT NULL CHECK (rate >= 0),
    label          TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT rate_card_entry_card_fk
        FOREIGN KEY (tenant_id, rate_card_id) REFERENCES rate_card (tenant_id, id) ON DELETE CASCADE,
    CONSTRAINT rate_card_entry_key_day_key UNIQUE (rate_card_id, key, effective_from)
);

CREATE INDEX rate_card_entry_card_idx ON rate_card_entry (rate_card_id, effective_from);

CREATE FUNCTION rate_card_entry_is_dated() RETURNS trigger AS
$$
BEGIN
    IF OLD.key IS DISTINCT FROM NEW.key
        OR OLD.effective_from IS DISTINCT FROM NEW.effective_from
        OR OLD.rate IS DISTINCT FROM NEW.rate THEN
        RAISE EXCEPTION 'a rate card entry is never edited: add one from a later day';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_rate_card_entry_is_dated
    BEFORE UPDATE ON rate_card_entry
    FOR EACH ROW EXECUTE FUNCTION rate_card_entry_is_dated();

-- A price rated by a card names a card of its tenant, reading its product's metric, in the
-- card's denomination.
CREATE FUNCTION price_rate_card_is_usable() RETURNS trigger AS
$$
DECLARE
    card   rate_card;
    metric UUID;
BEGIN
    IF NOT (NEW.pricing ? 'RateCard') THEN
        RETURN NEW;
    END IF;
    SELECT * INTO card
    FROM rate_card
    WHERE id = (NEW.pricing -> 'RateCard' ->> 'rate_card_id')::uuid
      AND tenant_id = NEW.tenant_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'price % names no rate card of its tenant', NEW.id;
    END IF;
    IF card.credit_unit_id IS DISTINCT FROM NEW.credit_unit_id
        OR (card.currency IS NOT NULL AND card.currency <> NEW.currency) THEN
        RAISE EXCEPTION 'price % is not in the denomination of rate card %', NEW.id, card.code;
    END IF;
    SELECT (fee_structure ->> 'metric_id')::uuid INTO metric FROM product WHERE id = NEW.product_id;
    IF metric IS DISTINCT FROM card.metric_id THEN
        RAISE EXCEPTION 'price % reads another metric than rate card %', NEW.id, card.code;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_price_rate_card_is_usable
    BEFORE INSERT ON price
    FOR EACH ROW EXECUTE FUNCTION price_rate_card_is_usable();

-- ── feature quantity limit ──

-- A metered feature whose limit counts per unit of a managed quantity: "1 000 calls per seat".
-- Resolved for a subscription, the limit is the entitlement's times the units it holds.
CREATE TABLE feature_quantity_limit (
    feature_id          UUID PRIMARY KEY REFERENCES feature (id) ON DELETE CASCADE,
    managed_quantity_id UUID NOT NULL REFERENCES managed_quantity (id) ON DELETE CASCADE
);

-- ── subscription excluded component ──

-- A product the plan includes with no price, left out of a subscription created without it.
CREATE TABLE subscription_excluded_component (
    subscription_id    UUID NOT NULL REFERENCES subscription (id) ON DELETE CASCADE,
    price_component_id UUID NOT NULL REFERENCES price_component (id) ON DELETE CASCADE,
    PRIMARY KEY (subscription_id, price_component_id)
);

-- ── self serve journey ──

-- How customers move through a product family's self-serve offers: the plan (and option, by its
-- alias) a sign-up asked for starts on, whether they switch option or downgrade themselves.
-- Timing and currency rules are fixed by the engine and only shown.
CREATE TABLE product_family_journey (
    product_family_id    UUID PRIMARY KEY REFERENCES product_family (id) ON DELETE CASCADE,
    tenant_id            UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    signup_plan_id       UUID REFERENCES plan (id) ON DELETE SET NULL,
    signup_option        TEXT,
    period_switch        BOOLEAN     NOT NULL DEFAULT TRUE,
    self_serve_downgrade BOOLEAN     NOT NULL DEFAULT TRUE,
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The plan option a checkout or purchase link sells, by its alias (kept across versions); its
-- currency is the customer's.
ALTER TABLE checkout_session ADD COLUMN plan_option TEXT;
ALTER TABLE purchase_link ADD COLUMN plan_option TEXT;

-- ── plan pricing page ──

-- What a plan shows on a pricing page: marketing copy, not billing, so it lives on the plan and
-- never makes a new version.
ALTER TABLE plan
    ADD COLUMN pricing_tagline    TEXT,
    ADD COLUMN pricing_popular    BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN pricing_cta_label  TEXT,
    ADD COLUMN pricing_highlights JSONB   NOT NULL DEFAULT '[]';

-- One "popular" plan per product line.
CREATE UNIQUE INDEX plan_pricing_popular_per_family_idx
    ON plan (tenant_id, product_family_id)
    WHERE pricing_popular;

-- ── eligibility promotions ──

-- Who may see or get a plan, or redeem a coupon: live, never part of a plan version.
ALTER TABLE plan
    ADD COLUMN eligibility JSONB;

-- A coupon redeemable from `valid_from`; a fixed discount priced in other currencies than the
-- one of its discount ({"EUR": "10.00"}).
ALTER TABLE coupon
    ADD COLUMN eligibility   JSONB,
    ADD COLUMN valid_from    TIMESTAMP,
    ADD COLUMN fixed_amounts JSONB;

-- Add-ons a coupon is redeemable with, like coupon_plan; neither listed: any subscription.
CREATE TABLE coupon_add_on (
    coupon_id UUID NOT NULL REFERENCES coupon (id) ON DELETE CASCADE,
    add_on_id UUID NOT NULL REFERENCES add_on (id) ON DELETE CASCADE,
    PRIMARY KEY (coupon_id, add_on_id)
);

CREATE INDEX idx_coupon_add_on_add_on_id ON coupon_add_on (add_on_id);

-- ── add on prices ──

-- Every price of an add-on, one per currency and billing period, as a plan charge has.
-- add_on.price_id stays its first price, for readers of a single price.
CREATE TABLE add_on_price (
    add_on_id UUID NOT NULL REFERENCES add_on (id) ON DELETE CASCADE,
    price_id  UUID NOT NULL REFERENCES price (id) ON DELETE CASCADE,
    PRIMARY KEY (add_on_id, price_id)
);

CREATE INDEX idx_add_on_price_price_id ON add_on_price (price_id);

INSERT INTO add_on_price (add_on_id, price_id)
SELECT id, price_id
FROM add_on;

-- ── purchase link promotion code ──

-- The promotion code a purchase link applies when the buyer enters none.
ALTER TABLE purchase_link ADD COLUMN promotion_code TEXT;

-- ── add on price backfill ──

-- Add-ons created by pods still on the single-price code during the rollout have no
-- add_on_price row: backfill them again, idempotently.
INSERT INTO add_on_price (add_on_id, price_id)
SELECT id, price_id
FROM add_on
WHERE price_id IS NOT NULL
ON CONFLICT DO NOTHING;

-- ── applied coupon terms ──

-- What a redemption bills, as the coupon read when it was redeemed: a fixed discount's amount
-- in the subscription's currency, and the add-ons it alone discounts (NULL: the whole invoice).
ALTER TABLE applied_coupon
    ADD COLUMN fixed_amount   NUMERIC,
    ADD COLUMN fixed_currency TEXT,
    ADD COLUMN add_on_ids     UUID[];

-- Backfill from the coupon as it is now, as `AppliedCouponRow::snapshot_terms` records them.
UPDATE applied_coupon ac
SET fixed_amount   = t.amount,
    fixed_currency = CASE WHEN t.amount IS NULL THEN NULL ELSE t.currency END,
    add_on_ids     = t.add_on_ids
FROM (
    SELECT a.id,
           s.currency,
           CASE
               WHEN c.discount -> 'Fixed' IS NULL THEN NULL
               WHEN c.discount -> 'Fixed' ->> 'currency' = s.currency
                   THEN (c.discount -> 'Fixed' ->> 'amount')::numeric
               ELSE (c.fixed_amounts ->> s.currency)::numeric
           END AS amount,
           CASE
               WHEN EXISTS (SELECT 1 FROM coupon_add_on ca WHERE ca.coupon_id = c.id)
                   AND NOT EXISTS (
                       SELECT 1 FROM coupon_plan cp
                       JOIN plan_version pv ON pv.plan_id = cp.plan_id
                       WHERE cp.coupon_id = c.id AND pv.id = s.plan_version_id
                   )
                   THEN ARRAY(
                       SELECT ca.add_on_id FROM coupon_add_on ca
                       WHERE ca.coupon_id = c.id ORDER BY ca.add_on_id
                   )
           END AS add_on_ids
    FROM applied_coupon a
    JOIN coupon c ON c.id = a.coupon_id
    JOIN subscription s ON s.id = a.subscription_id
) t
WHERE ac.id = t.id;

-- ── coupon duration months ──

-- A coupon lasting N months: it discounts the invoices whose period starts before its
-- redemption plus N months. `recurring_value` (a count of invoices) stays for older coupons.
ALTER TABLE coupon
    ADD COLUMN duration_months INTEGER,
    ADD CONSTRAINT coupon_duration_months_check CHECK (
        duration_months IS NULL OR (duration_months > 0 AND recurring_value IS NULL)
    );

-- ── applied coupon duration months ──

-- The months a redemption lasts, as the coupon read when it was redeemed: editing the coupon's
-- duration reaches only later redemptions. NULL: it does not last months.
ALTER TABLE applied_coupon
    ADD COLUMN duration_months INTEGER;

-- Backfill from the coupon as it is now, as `AppliedCouponRow::snapshot_terms` records it.
UPDATE applied_coupon ac
SET duration_months = c.duration_months
FROM coupon c
WHERE c.id = ac.coupon_id
  AND c.duration_months IS NOT NULL;

-- ── plan options ──

-- What a buyer picks when buying a plan version ("Annual", "Startup"): each prices every charge
-- in its own cells. The alias is the API handle, kept across versions; the first is the default.
CREATE TABLE plan_version_option (
    id              UUID PRIMARY KEY,
    tenant_id       UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    plan_version_id UUID      NOT NULL REFERENCES plan_version (id) ON DELETE CASCADE,
    name            TEXT      NOT NULL,
    alias           TEXT      NOT NULL,
    position        INTEGER   NOT NULL,
    created_at      TIMESTAMP NOT NULL DEFAULT now(),
    UNIQUE (plan_version_id, alias)
);

-- Backfill: the choice was derived from the fixed fees priced on several cadences. Each such
-- cadence of a version becomes an option; a version without one gets a single option.
CREATE TEMP TABLE plan_option_fixed_link ON COMMIT DROP AS
SELECT pc.plan_version_id, pc.id AS component_id, p.id AS price_id, p.cadence
FROM price_component pc
         JOIN plan_component_price pcp ON pcp.plan_component_id = pc.id
         JOIN price p ON p.id = pcp.price_id
WHERE COALESCE(p.pricing ->> 'type', '') NOT IN ('Usage', 'OneTime');

CREATE TEMP TABLE plan_option_switching ON COMMIT DROP AS
SELECT component_id
FROM plan_option_fixed_link
GROUP BY component_id
HAVING count(DISTINCT cadence) > 1;

CREATE TEMP TABLE plan_option_cadence ON COMMIT DROP AS
SELECT uuidv7() AS option_id, plan_version_id, cadence
FROM (SELECT DISTINCT f.plan_version_id, f.cadence
      FROM plan_option_fixed_link f
               JOIN plan_option_switching s USING (component_id)) switching;
-- Read once per subscription below: unindexed, that read is quadratic under the subscription lock.
CREATE INDEX ON plan_option_cadence (plan_version_id, cadence);
ANALYZE plan_option_cadence;

INSERT INTO plan_version_option (id, tenant_id, plan_version_id, name, alias, position)
SELECT oc.option_id,
       pv.tenant_id,
       oc.plan_version_id,
       CASE oc.cadence
           WHEN 'MONTHLY' THEN 'Monthly'
           WHEN 'QUARTERLY' THEN 'Quarterly'
           WHEN 'SEMIANNUAL' THEN 'Semiannual'
           ELSE 'Annual' END,
       CASE oc.cadence
           WHEN 'MONTHLY' THEN 'monthly'
           WHEN 'QUARTERLY' THEN 'quarterly'
           WHEN 'SEMIANNUAL' THEN 'semiannual'
           ELSE 'annual' END,
       -- The enum is declared by period length.
       (row_number() OVER (PARTITION BY oc.plan_version_id ORDER BY oc.cadence) - 1)::INTEGER
FROM plan_option_cadence oc
         JOIN plan_version pv ON pv.id = oc.plan_version_id;

INSERT INTO plan_version_option (id, tenant_id, plan_version_id, name, alias, position)
SELECT uuidv7(), pv.tenant_id, pv.id, 'Standard', 'standard', 0
FROM plan_version pv
WHERE NOT EXISTS (SELECT 1 FROM plan_option_cadence oc WHERE oc.plan_version_id = pv.id);

-- A link is one option's: every link is copied into each option of its version, but a switching
-- charge's fixed price only into the option of its cadence.
ALTER TABLE plan_component_price
    ADD COLUMN option_id UUID REFERENCES plan_version_option (id) ON DELETE CASCADE;
ALTER TABLE plan_component_price DROP CONSTRAINT plan_component_price_pkey;

INSERT INTO plan_component_price (plan_component_id, price_id, option_id)
SELECT pcp.plan_component_id, pcp.price_id, o.id
FROM plan_component_price pcp
         JOIN price_component pc ON pc.id = pcp.plan_component_id
         JOIN plan_version_option o ON o.plan_version_id = pc.plan_version_id
         LEFT JOIN plan_option_cadence oc ON oc.option_id = o.id
WHERE pcp.option_id IS NULL
  AND NOT EXISTS (SELECT 1
                  FROM plan_option_fixed_link f
                           JOIN plan_option_switching s USING (component_id)
                  WHERE f.component_id = pcp.plan_component_id
                    AND f.price_id = pcp.price_id
                    AND oc.cadence IS NOT NULL
                    AND f.cadence <> oc.cadence);

DELETE FROM plan_component_price WHERE option_id IS NULL;

ALTER TABLE plan_component_price ALTER COLUMN option_id SET NOT NULL;
ALTER TABLE plan_component_price ADD PRIMARY KEY (plan_component_id, option_id, price_id);
CREATE INDEX idx_plan_component_price_option ON plan_component_price (option_id);

-- The option a subscription bought; NULL only if its option is gone.
ALTER TABLE subscription
    ADD COLUMN plan_option_id UUID REFERENCES plan_version_option (id) ON DELETE SET NULL;

-- The option holding most of the prices a subscription bills now: its period is the shortest of
-- its fees, usage included, so an annual fee beside monthly usage would read as monthly.
CREATE TEMP TABLE subscription_held_option ON COMMIT DROP AS
SELECT DISTINCT ON (sc.subscription_id) sc.subscription_id, pcp.option_id
FROM subscription_component sc
         JOIN subscription s ON s.id = sc.subscription_id
         JOIN plan_component_price pcp
              ON pcp.plan_component_id = sc.price_component_id AND pcp.price_id = sc.price_id
         JOIN plan_version_option o ON o.id = pcp.option_id AND o.plan_version_id = s.plan_version_id
WHERE sc.effective_to IS NULL
   OR sc.effective_to > CURRENT_DATE
GROUP BY sc.subscription_id, pcp.option_id, o.position
ORDER BY sc.subscription_id, count(*) DESC, o.position;
CREATE INDEX ON subscription_held_option (subscription_id);
ANALYZE subscription_held_option;

UPDATE subscription s
SET plan_option_id = COALESCE(
        (SELECT h.option_id
         FROM subscription_held_option h
         WHERE h.subscription_id = s.id),
        (SELECT oc.option_id
         FROM plan_option_cadence oc
         WHERE oc.plan_version_id = s.plan_version_id
           AND oc.cadence = s.period),
        (SELECT o.id
         FROM plan_version_option o
         WHERE o.plan_version_id = s.plan_version_id
         ORDER BY o.position
         LIMIT 1));

DROP TABLE subscription_held_option;
DROP TABLE plan_option_cadence;
DROP TABLE plan_option_switching;
DROP TABLE plan_option_fixed_link;

-- The option a quote sells, by alias, recorded on the subscription it converts to.
ALTER TABLE quote ADD COLUMN plan_option TEXT;

-- ── rate card currencies ──

-- A plan is priced per currency, so a card holds a rate per currency for each key: a money card
-- (no credit unit) rates in each entry's currency, a credit card's entries carry none. Rates are
-- per `unit_size` units of the metric (1000000: per 1M tokens).
ALTER TABLE rate_card
    ADD COLUMN unit_size NUMERIC NOT NULL DEFAULT 1 CHECK (unit_size > 0);

ALTER TABLE rate_card_entry
    ADD COLUMN currency TEXT;

UPDATE rate_card_entry e
SET currency = c.currency
FROM rate_card c
WHERE c.id = e.rate_card_id
  AND c.currency IS NOT NULL;

-- Drops the currency/credit unit CHECK with it: no credit unit now means money.
ALTER TABLE rate_card
    DROP COLUMN currency;

-- A NULL rate removes the key in its currency from its day: entries stay never edited, a removal
-- is one more dated entry, and the key's earlier days keep their rates.
ALTER TABLE rate_card_entry
    ALTER COLUMN rate DROP NOT NULL;

ALTER TABLE rate_card_entry
    DROP CONSTRAINT rate_card_entry_key_day_key;
ALTER TABLE rate_card_entry
    ADD CONSTRAINT rate_card_entry_key_day_key
        UNIQUE NULLS NOT DISTINCT (rate_card_id, key, currency, effective_from);

CREATE OR REPLACE FUNCTION rate_card_entry_is_dated() RETURNS trigger AS
$$
BEGIN
    IF OLD.key IS DISTINCT FROM NEW.key
        OR OLD.currency IS DISTINCT FROM NEW.currency
        OR OLD.effective_from IS DISTINCT FROM NEW.effective_from
        OR OLD.rate IS DISTINCT FROM NEW.rate THEN
        RAISE EXCEPTION 'a rate card entry is never edited: add one from a later day';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION rate_card_entry_in_denomination() RETURNS trigger AS
$$
BEGIN
    IF (NEW.currency IS NULL) <> EXISTS (SELECT 1
                                         FROM rate_card
                                         WHERE id = NEW.rate_card_id
                                           AND credit_unit_id IS NOT NULL) THEN
        RAISE EXCEPTION 'a money card''s rate has a currency, a credit card''s none';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_rate_card_entry_in_denomination
    BEFORE INSERT ON rate_card_entry
    FOR EACH ROW EXECUTE FUNCTION rate_card_entry_in_denomination();

-- A money card rates a price in any currency (a key without a rate in it is unmatched); a credit
-- card only prices in its unit.
CREATE OR REPLACE FUNCTION price_rate_card_is_usable() RETURNS trigger AS
$$
DECLARE
    card   rate_card;
    metric UUID;
BEGIN
    IF NOT (NEW.pricing ? 'RateCard') THEN
        RETURN NEW;
    END IF;
    SELECT * INTO card
    FROM rate_card
    WHERE id = (NEW.pricing -> 'RateCard' ->> 'rate_card_id')::uuid
      AND tenant_id = NEW.tenant_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'price % names no rate card of its tenant', NEW.id;
    END IF;
    IF card.credit_unit_id IS DISTINCT FROM NEW.credit_unit_id THEN
        RAISE EXCEPTION 'price % is not in the denomination of rate card %', NEW.id, card.code;
    END IF;
    SELECT (fee_structure ->> 'metric_id')::uuid INTO metric FROM product WHERE id = NEW.product_id;
    IF metric IS DISTINCT FROM card.metric_id THEN
        RAISE EXCEPTION 'price % reads another metric than rate card %', NEW.id, card.code;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- ── product appearance ──

-- The icon and color the dashboard draws a product with; null is the default box, neutral.
ALTER TABLE product
    ADD COLUMN icon  TEXT,
    ADD COLUMN color TEXT;

-- ── sales-led plans on the pricing page ──

-- A plan on the pricing page is sold self-serve or by contacting sales: a sales-led one is shown
-- (its price, unless hidden) with a contact link, and never offered in the portal or checkout.
ALTER TABLE plan
    ADD COLUMN pricing_sales_led   BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN pricing_hide_price  BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN pricing_contact_url TEXT;

-- ── ranks without ties ──

-- Older plans could share a rank (the rank's first migration gave every standard plan 1): each
-- product line's ladder is renumbered 1..n in the order it was read, rank then creation.
UPDATE plan p
SET self_service_rank = ranked.rank
FROM (SELECT id,
             ROW_NUMBER() OVER (PARTITION BY tenant_id, product_family_id
                                ORDER BY self_service_rank, created_at, id)::INT AS rank
      FROM plan
      WHERE self_service_rank IS NOT NULL) ranked
WHERE p.id = ranked.id
  AND p.self_service_rank IS DISTINCT FROM ranked.rank;

-- ── hidden options ──

-- An option sold when an operator or integrator names it, never advertised: off the pricing page
-- and the portal.
ALTER TABLE plan_version_option
    ADD COLUMN hidden BOOLEAN NOT NULL DEFAULT FALSE;

-- ── card price order and hidden charges ──

-- The order a plan's card reads its charges in, the first that can lead being its price, and
-- which usage it leaves off (read as "+ usage"), by product: copy kept across versions, whose
-- charges are new each time. Products not listed follow in the default order; empty: the card
-- reads as computed.
ALTER TABLE plan
    ADD COLUMN pricing_product_order      UUID[] NOT NULL DEFAULT '{}',
    ADD COLUMN pricing_hidden_product_ids UUID[] NOT NULL DEFAULT '{}';
