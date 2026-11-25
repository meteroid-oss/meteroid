-- ── card price order and hidden charges ──

ALTER TABLE plan
    DROP COLUMN pricing_hidden_product_ids,
    DROP COLUMN pricing_product_order;

-- ── hidden options ──

ALTER TABLE plan_version_option
    DROP COLUMN hidden;

-- ── sales-led plans on the pricing page ──

ALTER TABLE plan
    DROP COLUMN pricing_contact_url,
    DROP COLUMN pricing_hide_price,
    DROP COLUMN pricing_sales_led;

-- ── product appearance ──

ALTER TABLE product
    DROP COLUMN icon,
    DROP COLUMN color;

-- ── rate card currencies ──

-- Back to one currency per card: its first currency kept (the tenant's when it has no rate),
-- the rates in others and the removals dropped.
DROP TRIGGER trg_rate_card_entry_in_denomination ON rate_card_entry;
DROP FUNCTION rate_card_entry_in_denomination();

ALTER TABLE rate_card
    ADD COLUMN currency TEXT;

UPDATE rate_card c
SET currency = coalesce((SELECT min(e.currency) FROM rate_card_entry e WHERE e.rate_card_id = c.id),
                        (SELECT t.reporting_currency FROM tenant t WHERE t.id = c.tenant_id))
WHERE c.credit_unit_id IS NULL;

DELETE
FROM rate_card_entry e
USING rate_card c
WHERE c.id = e.rate_card_id
  AND (e.rate IS NULL OR e.currency IS DISTINCT FROM c.currency);

ALTER TABLE rate_card
    ADD CHECK ((currency IS NULL) <> (credit_unit_id IS NULL));

ALTER TABLE rate_card_entry
    DROP CONSTRAINT rate_card_entry_key_day_key;
ALTER TABLE rate_card_entry
    DROP COLUMN currency;
ALTER TABLE rate_card_entry
    ADD CONSTRAINT rate_card_entry_key_day_key UNIQUE (rate_card_id, key, effective_from);
ALTER TABLE rate_card_entry
    ALTER COLUMN rate SET NOT NULL;

ALTER TABLE rate_card
    DROP COLUMN unit_size;

CREATE OR REPLACE FUNCTION rate_card_entry_is_dated() RETURNS trigger AS
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

-- ── plan options ──

ALTER TABLE quote DROP COLUMN plan_option;
ALTER TABLE subscription DROP COLUMN plan_option_id;

-- One link per (charge, price), whatever the options that shared it.
DELETE FROM plan_component_price a
    USING plan_component_price b
WHERE a.plan_component_id = b.plan_component_id
  AND a.price_id = b.price_id
  AND a.option_id > b.option_id;

DROP INDEX idx_plan_component_price_option;
ALTER TABLE plan_component_price DROP CONSTRAINT plan_component_price_pkey;
ALTER TABLE plan_component_price DROP COLUMN option_id;
ALTER TABLE plan_component_price ADD PRIMARY KEY (plan_component_id, price_id);

DROP TABLE plan_version_option;

-- ── applied coupon duration months ──

ALTER TABLE applied_coupon
    DROP COLUMN duration_months;

-- ── coupon duration months ──

ALTER TABLE coupon
    DROP CONSTRAINT coupon_duration_months_check,
    DROP COLUMN duration_months;

-- ── applied coupon terms ──

ALTER TABLE applied_coupon
    DROP COLUMN add_on_ids,
    DROP COLUMN fixed_currency,
    DROP COLUMN fixed_amount;

-- ── add on price backfill ──

-- A backfill: the rows are the add-ons' own prices, kept on the way down.
SELECT 1;

-- ── purchase link promotion code ──

ALTER TABLE purchase_link DROP COLUMN promotion_code;

-- ── add on prices ──

DROP TABLE add_on_price;

-- ── eligibility promotions ──

DROP TABLE coupon_add_on;
ALTER TABLE coupon
    DROP COLUMN fixed_amounts,
    DROP COLUMN valid_from,
    DROP COLUMN eligibility;
ALTER TABLE plan
    DROP COLUMN eligibility;

-- ── plan pricing page ──

DROP INDEX plan_pricing_popular_per_family_idx;
ALTER TABLE plan
    DROP COLUMN pricing_highlights,
    DROP COLUMN pricing_cta_label,
    DROP COLUMN pricing_popular,
    DROP COLUMN pricing_tagline;

-- ── self serve journey ──

ALTER TABLE purchase_link DROP COLUMN plan_option;
ALTER TABLE checkout_session DROP COLUMN plan_option;
DROP TABLE product_family_journey;

-- ── subscription excluded component ──

DROP TABLE subscription_excluded_component;

-- ── feature quantity limit ──

DROP TABLE feature_quantity_limit;

-- ── rate cards ──

DROP TRIGGER trg_price_rate_card_is_usable ON price;
DROP FUNCTION price_rate_card_is_usable();
DROP TABLE rate_card_entry;
DROP FUNCTION rate_card_entry_is_dated();
DROP TABLE rate_card;
DROP TYPE "RateCardUnmatchedEnum";

-- ── managed quantity ──

ALTER TABLE slot_transaction DROP COLUMN removal_waits_for_renewal;
ALTER TABLE slot_transaction DROP COLUMN managed_quantity_id;
DROP TABLE managed_quantity;
DROP TYPE "QuantityDecreaseEnum";
DROP TYPE "QuantityIncreaseEnum";
