CREATE TABLE historical_rates_from_usd (
   id UUID PRIMARY KEY,
   date DATE NOT NULL UNIQUE,
   rates JSONB NOT NULL
);


CREATE OR REPLACE FUNCTION convert_currency(
    amount NUMERIC,
    source_rate_from_usd NUMERIC,
    target_rate_from_usd NUMERIC
)
    RETURNS NUMERIC AS $$
DECLARE
    conversion_rate NUMERIC;
BEGIN
    conversion_rate := target_rate_from_usd / source_rate_from_usd;
    RETURN amount * conversion_rate;
END;
$$ LANGUAGE plpgsql;


ALTER TABLE bi_delta_mrr_daily
    ADD COLUMN historical_rate_id UUID NOT NULL references historical_rates_from_usd on update cascade on delete restrict,
    ADD COLUMN net_mrr_cents_usd BIGINT NOT NULL,
    ADD COLUMN new_business_cents_usd BIGINT NOT NULL,
    ADD COLUMN expansion_cents_usd BIGINT NOT NULL,
    ADD COLUMN contraction_cents_usd BIGINT NOT NULL,
    ADD COLUMN churn_cents_usd BIGINT NOT NULL,
    ADD COLUMN reactivation_cents_usd BIGINT NOT NULL;

CREATE OR REPLACE FUNCTION fn_update_mrr()
    RETURNS TRIGGER AS $$
DECLARE
    net_mrr_change_usd BIGINT;
    historical_rate_record RECORD;
BEGIN
    SELECT id, (rates->>NEW.currency)::NUMERIC as rate INTO historical_rate_record
    FROM historical_rates_from_usd
    WHERE date <= NEW.applies_to
    ORDER BY date DESC
    LIMIT 1;

    net_mrr_change_usd := NEW.net_mrr_change / historical_rate_record.rate;

    INSERT INTO bi_delta_mrr_daily (
        tenant_id,
        plan_version_id,
        currency,
        date,
        net_mrr_cents,
        net_mrr_cents_usd,
        new_business_cents,
        new_business_cents_usd,
        new_business_count,
        expansion_cents,
        expansion_cents_usd,
        expansion_count,
        contraction_cents,
        contraction_cents_usd,
        contraction_count,
        churn_cents,
        churn_cents_usd,
        churn_count,
        reactivation_cents,
        reactivation_cents_usd,
        reactivation_count,
        historical_rate_id
    )
    VALUES (
               NEW.tenant_id,
               NEW.plan_version_id,
               NEW.currency,
               NEW.applies_to,
               NEW.net_mrr_change,
               net_mrr_change_usd,
               CASE WHEN NEW.movement_type = 'NEW_BUSINESS' THEN NEW.net_mrr_change ELSE 0 END,
               CASE WHEN NEW.movement_type = 'NEW_BUSINESS' THEN net_mrr_change_usd ELSE 0 END,
               CASE WHEN NEW.movement_type = 'NEW_BUSINESS' THEN 1 ELSE 0 END,
               CASE WHEN NEW.movement_type = 'EXPANSION' THEN NEW.net_mrr_change ELSE 0 END,
               CASE WHEN NEW.movement_type = 'EXPANSION' THEN net_mrr_change_usd ELSE 0 END,
               CASE WHEN NEW.movement_type = 'EXPANSION' THEN 1 ELSE 0 END,
               CASE WHEN NEW.movement_type = 'CONTRACTION' THEN NEW.net_mrr_change ELSE 0 END,
               CASE WHEN NEW.movement_type = 'CONTRACTION' THEN net_mrr_change_usd ELSE 0 END,
               CASE WHEN NEW.movement_type = 'CONTRACTION' THEN 1 ELSE 0 END,
               CASE WHEN NEW.movement_type = 'CHURN' THEN NEW.net_mrr_change ELSE 0 END,
               CASE WHEN NEW.movement_type = 'CHURN' THEN net_mrr_change_usd ELSE 0 END,
               CASE WHEN NEW.movement_type = 'CHURN' THEN 1 ELSE 0 END,
               CASE WHEN NEW.movement_type = 'REACTIVATION' THEN NEW.net_mrr_change ELSE 0 END,
               CASE WHEN NEW.movement_type = 'REACTIVATION' THEN net_mrr_change_usd ELSE 0 END,
               CASE WHEN NEW.movement_type = 'REACTIVATION' THEN 1 ELSE 0 END,
               historical_rate_record.id
           )
    ON CONFLICT (tenant_id, plan_version_id, currency, date) DO UPDATE
        SET
            net_mrr_cents = bi_delta_mrr_daily.net_mrr_cents + EXCLUDED.net_mrr_cents,
            net_mrr_cents_usd = bi_delta_mrr_daily.net_mrr_cents_usd + EXCLUDED.net_mrr_cents_usd,
            new_business_cents = bi_delta_mrr_daily.new_business_cents + EXCLUDED.new_business_cents,
            new_business_cents_usd = bi_delta_mrr_daily.new_business_cents_usd + EXCLUDED.new_business_cents_usd,
            new_business_count = bi_delta_mrr_daily.new_business_count + EXCLUDED.new_business_count,
            expansion_cents = bi_delta_mrr_daily.expansion_cents + EXCLUDED.expansion_cents,
            expansion_cents_usd = bi_delta_mrr_daily.expansion_cents_usd + EXCLUDED.expansion_cents_usd,
            expansion_count = bi_delta_mrr_daily.expansion_count + EXCLUDED.expansion_count,
            contraction_cents = bi_delta_mrr_daily.contraction_cents + EXCLUDED.contraction_cents,
            contraction_cents_usd = bi_delta_mrr_daily.contraction_cents_usd + EXCLUDED.contraction_cents_usd,
            contraction_count = bi_delta_mrr_daily.contraction_count + EXCLUDED.contraction_count,
            churn_cents = bi_delta_mrr_daily.churn_cents + EXCLUDED.churn_cents,
            churn_cents_usd = bi_delta_mrr_daily.churn_cents_usd + EXCLUDED.churn_cents_usd,
            churn_count = bi_delta_mrr_daily.churn_count + EXCLUDED.churn_count,
            reactivation_cents = bi_delta_mrr_daily.reactivation_cents + EXCLUDED.reactivation_cents,
            reactivation_cents_usd = bi_delta_mrr_daily.reactivation_cents_usd + EXCLUDED.reactivation_cents_usd,
            reactivation_count = bi_delta_mrr_daily.reactivation_count + EXCLUDED.reactivation_count,
            historical_rate_id = EXCLUDED.historical_rate_id;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;


ALTER TABLE bi_revenue_daily
    ADD COLUMN historical_rate_id UUID NOT NULL references historical_rates_from_usd on update cascade on delete restrict,
    ADD COLUMN net_revenue_cents_usd BIGINT NOT NULL
;

CREATE OR REPLACE FUNCTION fn_update_revenue_credit_note()
    RETURNS TRIGGER AS $$
DECLARE
    net_revenue_cents_usd BIGINT;
    historical_rate_record RECORD;
BEGIN

    SELECT id, (rates->>NEW.currency)::NUMERIC as rate INTO historical_rate_record
    FROM historical_rates_from_usd
    WHERE date <= NEW.finalized_at
    ORDER BY date DESC
    LIMIT 1;

    net_revenue_cents_usd := -NEW.refunded_amount_cents / historical_rate_record.rate;

    INSERT INTO bi_revenue_daily (tenant_id, plan_version_id, currency, revenue_date, net_revenue_cents, historical_rate_id, net_revenue_cents_usd)
    VALUES (NEW.tenant_id, NEW.plan_version_id, NEW.currency, DATE_TRUNC('day', NEW.finalized_at), -NEW.refunded_amount_cents, historical_rate_record.id, net_revenue_cents_usd)
    ON CONFLICT (tenant_id, plan_version_id, currency, revenue_date) DO UPDATE
        SET
            net_revenue_cents = bi_revenue_daily.net_revenue_cents + EXCLUDED.net_revenue_cents,
            net_revenue_cents_usd = bi_revenue_daily.net_revenue_cents_usd + EXCLUDED.net_revenue_cents_usd,
            historical_rate_id = EXCLUDED.historical_rate_id;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;


CREATE OR REPLACE FUNCTION fn_update_revenue_invoice()
    RETURNS TRIGGER AS $$
DECLARE
    net_revenue_cents_usd BIGINT;
    historical_rate_record RECORD;
BEGIN

    SELECT id, (rates->>NEW.currency)::NUMERIC as rate INTO historical_rate_record
    FROM historical_rates_from_usd
    WHERE date <= NEW.finalized_at
    ORDER BY date DESC
    LIMIT 1;

    net_revenue_cents_usd := NEW.amount_cents / historical_rate_record.rate;

    INSERT INTO bi_revenue_daily (tenant_id, plan_version_id, currency, revenue_date, net_revenue_cents, historical_rate_id, net_revenue_cents_usd)
    VALUES (NEW.tenant_id, NEW.plan_version_id, NEW.currency, DATE_TRUNC('day', NEW.finalized_at), NEW.amount_cents, historical_rate_record.id, net_revenue_cents_usd)
    ON CONFLICT (tenant_id, plan_version_id, currency, revenue_date) DO UPDATE
        SET net_revenue_cents = bi_revenue_daily.net_revenue_cents + EXCLUDED.net_revenue_cents,
            net_revenue_cents_usd = bi_revenue_daily.net_revenue_cents_usd + EXCLUDED.net_revenue_cents_usd,
            historical_rate_id = EXCLUDED.historical_rate_id;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;


CREATE OR REPLACE FUNCTION update_bi_usd_totals_from_rates()
    RETURNS TRIGGER AS $$
BEGIN
    -- Update bi_delta_mrr_daily
    UPDATE bi_delta_mrr_daily
    SET net_mrr_cents_usd = net_mrr_cents * NEW.rates->>currency::NUMERIC,
        new_business_cents_usd = new_business_cents * NEW.rates->>currency::NUMERIC,
        expansion_cents_usd = expansion_cents * NEW.rates->>currency::NUMERIC,
        contraction_cents_usd = contraction_cents * NEW.rates->>currency::NUMERIC,
        churn_cents_usd = churn_cents * NEW.rates->>currency::NUMERIC,
        reactivation_cents_usd = reactivation_cents * NEW.rates->>currency::NUMERIC,
        historical_rate_id = NEW.id
    WHERE date = NEW.date;

    -- Update bi_revenue_daily
    UPDATE bi_revenue_daily
    SET net_revenue_cents_usd = net_revenue_cents * NEW.rates->>currency::NUMERIC,
        historical_rate_id = NEW.id
    WHERE revenue_date = NEW.date;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER update_usd_totals_trigger
    AFTER INSERT OR UPDATE ON historical_rates_from_usd
    FOR EACH ROW EXECUTE FUNCTION update_bi_usd_totals_from_rates();
