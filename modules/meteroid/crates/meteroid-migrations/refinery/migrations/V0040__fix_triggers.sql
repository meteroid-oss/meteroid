alter table bi_revenue_daily
    drop constraint "bi_revenue_daily_pkey";
alter table bi_revenue_daily
    alter column plan_version_id drop not null;
alter table bi_revenue_daily
    add column "id" uuid not null primary key default gen_random_uuid();

create unique index bi_revenue_daily_uniqueness on bi_revenue_daily (tenant_id, plan_version_id, currency, revenue_date);

-- invoice.amount_cents got renamed earlier to total
CREATE OR REPLACE FUNCTION fn_update_customer_ytd_summary_invoice()
    RETURNS TRIGGER AS
$$
BEGIN
    INSERT INTO bi_customer_ytd_summary (tenant_id, customer_id, revenue_year, currency, total_revenue_cents)
    VALUES (NEW.tenant_id, NEW.customer_id, DATE_PART('year', NEW.finalized_at)::integer, NEW.currency, NEW.amount_due)
    ON CONFLICT (tenant_id, customer_id, currency, revenue_year) DO UPDATE
        SET total_revenue_cents = bi_customer_ytd_summary.total_revenue_cents + EXCLUDED.total_revenue_cents;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- invoice.amount_cents got renamed earlier to total
CREATE OR REPLACE FUNCTION fn_update_revenue_invoice()
    RETURNS TRIGGER AS
$$
DECLARE
    net_revenue_cents_usd  BIGINT;
    historical_rate_record RECORD;
BEGIN

    SELECT id, (rates ->> NEW.currency)::NUMERIC as rate
    INTO historical_rate_record
    FROM historical_rates_from_usd
    WHERE date <= NEW.finalized_at
    ORDER BY date DESC
    LIMIT 1;

    net_revenue_cents_usd := NEW.amount_due / historical_rate_record.rate;

    INSERT INTO bi_revenue_daily (tenant_id, plan_version_id, currency, revenue_date, net_revenue_cents,
                                  historical_rate_id, net_revenue_cents_usd)
    VALUES (NEW.tenant_id, NEW.plan_version_id, NEW.currency, DATE_TRUNC('day', NEW.finalized_at), NEW.amount_due,
            historical_rate_record.id, net_revenue_cents_usd)
    ON CONFLICT (tenant_id, plan_version_id, currency, revenue_date) DO UPDATE
        SET net_revenue_cents     = bi_revenue_daily.net_revenue_cents + EXCLUDED.net_revenue_cents,
            net_revenue_cents_usd = bi_revenue_daily.net_revenue_cents_usd + EXCLUDED.net_revenue_cents_usd,
            historical_rate_id    = EXCLUDED.historical_rate_id;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
