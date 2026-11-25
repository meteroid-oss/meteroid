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

-- Products created without a structure take their first charge's, else a flat fee's.
UPDATE product p
SET fee_structure = COALESCE(
        (SELECT pc.fee_structure - 'quantity_id' FROM price_component pc
         WHERE pc.product_id = p.id AND pc.fee_structure IS NOT NULL LIMIT 1),
        (SELECT a.fee_structure - 'quantity_id' FROM add_on a WHERE a.product_id = p.id LIMIT 1),
        '{"type": "Rate"}'::jsonb)
WHERE p.fee_structure IS NULL;

UPDATE product p
SET fee_type = CASE p.fee_structure ->> 'type'
                   WHEN 'Rate' THEN 'RATE'
                   WHEN 'Slot' THEN 'SLOT'
                   WHEN 'Capacity' THEN 'CAPACITY'
                   WHEN 'Usage' THEN 'USAGE'
                   WHEN 'ExtraRecurring' THEN 'EXTRA_RECURRING'
                   WHEN 'OneTime' THEN 'ONE_TIME'
                   ELSE 'CREDIT' END::"FeeTypeEnum"
WHERE p.fee_type IS NULL;

ALTER TABLE product
    ALTER COLUMN fee_type SET NOT NULL,
    ALTER COLUMN fee_structure SET NOT NULL;

ALTER TABLE add_on DROP COLUMN fee_structure;
ALTER TABLE price_component DROP COLUMN fee_structure;
