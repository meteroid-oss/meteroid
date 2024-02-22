
CREATE TABLE bi_customer_ytd_summary (
      tenant_id UUID NOT NULL,
      customer_id UUID NOT NULL,
      revenue_year INT NOT NULL,
      currency TEXT NOT NULL,
      total_revenue_cents BIGINT NOT NULL,
      PRIMARY KEY (tenant_id, customer_id, revenue_year)
);


CREATE OR REPLACE FUNCTION fn_update_customer_ytd_summary_credit_note()
    RETURNS TRIGGER AS $$
BEGIN
    INSERT INTO bi_revenue_daily (tenant_id, plan_version_id, currency, revenue_date, net_revenue_cents, gross_revenue_cents)
    VALUES (NEW.tenant_id, NEW.plan_version_id, NEW.currency, DATE_TRUNC(NEW.finalized_at, 'day'), -NEW.refunded_amount_cents, 0)
    ON CONFLICT (tenant_id, plan_version_id, currency, revenue_date) DO UPDATE
        SET net_revenue_cents = bi_revenue_daily.net_revenue_cents + EXCLUDED.net_revenue_cents;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_update_customer_ytd_summary_credit_note
    AFTER UPDATE ON credit_note
    FOR EACH ROW
    WHEN (NEW.status = 'FINALIZED')
EXECUTE FUNCTION fn_update_customer_ytd_summary_credit_note();


CREATE OR REPLACE FUNCTION fn_update_customer_ytd_summary_invoice()
    RETURNS TRIGGER AS $$
BEGIN
    -- Update or insert into bi_revenue_daily
    INSERT INTO bi_revenue_daily (tenant_id, plan_version_id, currency, revenue_date, net_revenue_cents, gross_revenue_cents)
    VALUES (NEW.tenant_id, NEW.plan_version_id, NEW.currency, DATE_TRUNC(NEW.finalized_at, 'day'), NEW.net_total_cents, NEW.gross_total_cents)
    ON CONFLICT (tenant_id, plan_version_id, currency, revenue_date) DO UPDATE
        SET net_revenue_cents = bi_revenue_daily.net_revenue_cents + EXCLUDED.net_revenue_cents,
            gross_revenue_cents = bi_revenue_daily.gross_revenue_cents + EXCLUDED.gross_revenue_cents;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_update_customer_ytd_summary_invoice
    AFTER INSERT OR UPDATE ON invoice
    FOR EACH ROW
    WHEN (NEW.status = 'FINALIZED')
EXECUTE FUNCTION fn_update_customer_ytd_summary_invoice();



CREATE TABLE bi_revenue_daily (
       tenant_id UUID NOT NULL,
       plan_version_id UUID NOT NULL,
       currency TEXT NOT NULL,
       revenue_date DATE NOT NULL,
       net_revenue_cents BIGINT NOT NULL,
       gross_revenue_cents BIGINT NOT NULL,
       PRIMARY KEY (tenant_id, plan_version_id, currency, revenue_date)
);



CREATE OR REPLACE FUNCTION fn_update_revenue_credit_note()
    RETURNS TRIGGER AS $$
BEGIN
    INSERT INTO bi_revenue_daily (tenant_id, plan_version_id, currency, revenue_date, net_revenue_cents, gross_revenue_cents)
    VALUES (NEW.tenant_id, NEW.plan_version_id, NEW.currency, DATE_TRUNC(NEW.finalized_at, 'day'), -NEW.refunded_amount_cents, 0)
    ON CONFLICT (tenant_id, plan_version_id, currency, revenue_date) DO UPDATE
        SET
            net_revenue_cents = bi_revenue_daily.net_revenue_cents + EXCLUDED.net_revenue_cents;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_update_revenue_credit_note
    AFTER UPDATE ON credit_note
    FOR EACH ROW
    WHEN (NEW.status = 'FINALIZED')
EXECUTE FUNCTION fn_update_revenue_credit_note();

CREATE OR REPLACE FUNCTION fn_update_revenue_invoice()
    RETURNS TRIGGER AS $$
BEGIN
    INSERT INTO bi_revenue_daily (tenant_id, plan_version_id, currency, revenue_date, net_revenue_cents, gross_revenue_cents)
    VALUES (NEW.tenant_id, NEW.plan_version_id, NEW.currency, DATE_TRUNC(NEW.finalized_at, 'day'), NEW.net_total_cents, NEW.gross_total_cents)
    ON CONFLICT (tenant_id, plan_version_id, currency, revenue_date) DO UPDATE
        SET net_revenue_cents = bi_revenue_daily.net_revenue_cents + EXCLUDED.net_revenue_cents,
            gross_revenue_cents = bi_revenue_daily.gross_revenue_cents + EXCLUDED.gross_revenue_cents;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_update_revenue_invoice
    AFTER INSERT OR UPDATE ON invoice
    FOR EACH ROW
    WHEN (NEW.status = 'FINALIZED')
EXECUTE FUNCTION fn_update_revenue_invoice();
