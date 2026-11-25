-- A charge owns how it bills its product: a plan's price component and an add-on each carry
-- their fee structure, so one product can be a flat fee in one plan and per seat in another. A
-- seat structure names the managed quantity it counts.
-- A long transaction of a serving pod fails this run instead of queueing every query behind it;
-- the next start retries.
SET LOCAL lock_timeout = '5s';

-- ── seat quantities ──

-- Every seat product's unit has its quantity (the first pricing-model migration made one per
-- unit; products since registered theirs): created here when one is missing.
INSERT INTO managed_quantity (id, tenant_id, code, name)
SELECT DISTINCT ON (p.tenant_id, p.fee_structure ->> 'unit_name')
       gen_random_uuid(), p.tenant_id, p.fee_structure ->> 'unit_name', p.fee_structure ->> 'unit_name'
FROM product p
WHERE p.fee_structure ->> 'type' = 'Slot'
  AND p.fee_structure ->> 'unit_name' IS NOT NULL
ON CONFLICT (tenant_id, code) DO NOTHING;

-- ── charge structures ──

-- NULL on a product included with no price, and on a v1 component (its structure is in
-- `legacy_fee`).
ALTER TABLE price_component ADD COLUMN fee_structure JSONB;
ALTER TABLE add_on ADD COLUMN fee_structure JSONB;

-- Each product's structure as a charge holds it: a seat structure names the quantity coded as
-- its unit, which counted its seats so far.
CREATE TEMP TABLE product_charge_structure ON COMMIT DROP AS
SELECT p.id AS product_id,
       CASE
           WHEN p.fee_structure ->> 'type' = 'Slot'
               THEN p.fee_structure || jsonb_build_object('quantity_id', mq.id)
           ELSE p.fee_structure
       END AS fee_structure
FROM product p
         LEFT JOIN managed_quantity mq
                   ON mq.tenant_id = p.tenant_id AND mq.code = p.fee_structure ->> 'unit_name';
-- Joined once per component and add-on below.
CREATE UNIQUE INDEX ON product_charge_structure (product_id);
ANALYZE product_charge_structure;

UPDATE price_component pc
SET fee_structure = s.fee_structure
FROM product_charge_structure s
WHERE s.product_id = pc.product_id
  AND EXISTS (SELECT 1 FROM plan_component_price pcp WHERE pcp.plan_component_id = pc.id);

UPDATE add_on a
SET fee_structure = s.fee_structure
FROM product_charge_structure s
WHERE s.product_id = a.product_id;

ALTER TABLE add_on ALTER COLUMN fee_structure SET NOT NULL;

-- ── product structure ──

-- No longer written for new products nor read by billing. The release before is stopped before
-- this runs (forward-only), so no pod writes a charge without its structure; a later migration
-- drops fee_type and fee_structure.
ALTER TABLE product
    ALTER COLUMN fee_type DROP NOT NULL,
    ALTER COLUMN fee_structure DROP NOT NULL;

-- ── rate card prices ──

-- A price no longer knows its metric: the charge it is linked to does, and linking checks the
-- card's metric against the charge's. The price keeps naming a card of its tenant, in the
-- card's denomination.
CREATE OR REPLACE FUNCTION price_rate_card_is_usable() RETURNS trigger AS
$$
DECLARE
    card rate_card;
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
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
