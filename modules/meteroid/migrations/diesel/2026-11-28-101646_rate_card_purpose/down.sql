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

DROP INDEX rate_card_live_cost_per_metric;
ALTER TABLE rate_card DROP COLUMN purpose;
DROP TYPE "RateCardPurposeEnum";
