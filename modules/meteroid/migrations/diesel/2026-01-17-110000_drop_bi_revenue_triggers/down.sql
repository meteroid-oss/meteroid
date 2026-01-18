-- Drop the bi_aggregation queue
SELECT pgmq.drop_queue('bi_aggregation');


-- Recreate ALL BI triggers if rolling back
-- NOTE: This rollback should rarely be used as Rust-based aggregation is preferred

-- ============================================================
-- TRIGGER FUNCTIONS
-- ============================================================

-- fn_update_revenue_invoice
CREATE OR REPLACE FUNCTION fn_update_revenue_invoice() RETURNS TRIGGER
  LANGUAGE plpgsql
AS
$$
DECLARE
  net_revenue_cents_usd BIGINT;
  historical_rate_record RECORD;
BEGIN
  SELECT id, (rates->>NEW.currency)::NUMERIC as rate INTO historical_rate_record
  FROM historical_rates_from_usd
  WHERE date <= NEW.finalized_at
  ORDER BY date DESC
  LIMIT 1;

  net_revenue_cents_usd := NEW.amount_due / historical_rate_record.rate;

  INSERT INTO bi_revenue_daily (tenant_id, plan_version_id, currency, revenue_date, net_revenue_cents, historical_rate_id, net_revenue_cents_usd)
  VALUES (NEW.tenant_id, NEW.plan_version_id, NEW.currency, DATE_TRUNC('day', NEW.finalized_at), NEW.amount_due, historical_rate_record.id, net_revenue_cents_usd)
  ON CONFLICT (tenant_id, plan_version_id, currency, revenue_date) DO UPDATE
    SET net_revenue_cents = bi_revenue_daily.net_revenue_cents + EXCLUDED.net_revenue_cents,
        net_revenue_cents_usd = bi_revenue_daily.net_revenue_cents_usd + EXCLUDED.net_revenue_cents_usd,
        historical_rate_id = EXCLUDED.historical_rate_id;
  RETURN NEW;
END;
$$;

-- fn_update_revenue_credit_note
CREATE OR REPLACE FUNCTION fn_update_revenue_credit_note() RETURNS TRIGGER
  LANGUAGE plpgsql
AS
$$
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
$$;

-- fn_update_customer_ytd_summary_invoice
CREATE OR REPLACE FUNCTION fn_update_customer_ytd_summary_invoice() RETURNS TRIGGER
  LANGUAGE plpgsql
AS
$$
BEGIN
  INSERT INTO bi_customer_ytd_summary (tenant_id, customer_id, revenue_year, currency, total_revenue_cents)
  VALUES (NEW.tenant_id, NEW.customer_id, DATE_PART('year', NEW.finalized_at)::integer, NEW.currency, NEW.amount_due)
  ON CONFLICT (tenant_id, customer_id, currency, revenue_year) DO UPDATE
    SET total_revenue_cents = bi_customer_ytd_summary.total_revenue_cents + EXCLUDED.total_revenue_cents;
  RETURN NEW;
END;
$$;

-- fn_update_customer_ytd_summary_credit_note
CREATE OR REPLACE FUNCTION fn_update_customer_ytd_summary_credit_note() RETURNS TRIGGER
  LANGUAGE plpgsql
AS
$$
BEGIN
  INSERT INTO bi_customer_ytd_summary (tenant_id, customer_id, revenue_year, currency, total_revenue_cents)
  VALUES (NEW.tenant_id, NEW.customer_id, DATE_PART('year', NEW.finalized_at)::integer, NEW.currency, -NEW.refunded_amount_cents)
  ON CONFLICT (tenant_id, customer_id, currency, revenue_year) DO UPDATE
    SET total_revenue_cents = bi_customer_ytd_summary.total_revenue_cents + EXCLUDED.total_revenue_cents;
  RETURN NEW;
END;
$$;

-- fn_update_mrr
CREATE OR REPLACE FUNCTION fn_update_mrr() RETURNS TRIGGER
  LANGUAGE plpgsql
AS
$$
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
$$;

-- update_bi_usd_totals_from_rates
CREATE OR REPLACE FUNCTION update_bi_usd_totals_from_rates() RETURNS TRIGGER
  LANGUAGE plpgsql
AS
$$
BEGIN
  -- Update bi_delta_mrr_daily
  UPDATE bi_delta_mrr_daily
  SET net_mrr_cents_usd = net_mrr_cents / (NEW.rates->>currency)::NUMERIC,
      new_business_cents_usd = new_business_cents / (NEW.rates->>currency)::NUMERIC,
      expansion_cents_usd = expansion_cents / (NEW.rates->>currency)::NUMERIC,
      contraction_cents_usd = contraction_cents / (NEW.rates->>currency)::NUMERIC,
      churn_cents_usd = churn_cents / (NEW.rates->>currency)::NUMERIC,
      reactivation_cents_usd = reactivation_cents / (NEW.rates->>currency)::NUMERIC,
      historical_rate_id = NEW.id
  WHERE date = NEW.date;

  -- Update bi_revenue_daily
  UPDATE bi_revenue_daily
  SET net_revenue_cents_usd = net_revenue_cents / (NEW.rates->>currency)::NUMERIC,
      historical_rate_id = NEW.id
  WHERE revenue_date = NEW.date;

  RETURN NEW;
END;
$$;

-- ============================================================
-- REVENUE TRIGGERS
-- ============================================================

-- Invoice revenue triggers (split into INSERT and UPDATE per migration 2026-01-15)
CREATE TRIGGER trg_update_revenue_invoice_insert
  AFTER INSERT
  ON invoice
  FOR EACH ROW
  WHEN (NEW.status = 'FINALIZED'::"InvoiceStatusEnum")
EXECUTE PROCEDURE fn_update_revenue_invoice();

CREATE TRIGGER trg_update_revenue_invoice_update
  AFTER UPDATE
  ON invoice
  FOR EACH ROW
  WHEN (NEW.status = 'FINALIZED'::"InvoiceStatusEnum" AND OLD.status != 'FINALIZED'::"InvoiceStatusEnum")
EXECUTE PROCEDURE fn_update_revenue_invoice();

-- Credit note revenue triggers
CREATE TRIGGER trg_update_revenue_credit_note_insert
  AFTER INSERT
  ON credit_note
  FOR EACH ROW
  WHEN (NEW.status = 'FINALIZED'::"CreditNoteStatus")
EXECUTE PROCEDURE fn_update_revenue_credit_note();

CREATE TRIGGER trg_update_revenue_credit_note_update
  AFTER UPDATE
  ON credit_note
  FOR EACH ROW
  WHEN (NEW.status = 'FINALIZED'::"CreditNoteStatus" AND OLD.status != 'FINALIZED'::"CreditNoteStatus")
EXECUTE PROCEDURE fn_update_revenue_credit_note();

-- ============================================================
-- CUSTOMER YTD SUMMARY TRIGGERS
-- ============================================================

-- Invoice customer YTD triggers
CREATE TRIGGER trg_update_customer_ytd_summary_invoice_insert
  AFTER INSERT
  ON invoice
  FOR EACH ROW
  WHEN (NEW.status = 'FINALIZED'::"InvoiceStatusEnum")
EXECUTE PROCEDURE fn_update_customer_ytd_summary_invoice();

CREATE TRIGGER trg_update_customer_ytd_summary_invoice_update
  AFTER UPDATE
  ON invoice
  FOR EACH ROW
  WHEN (NEW.status = 'FINALIZED'::"InvoiceStatusEnum" AND OLD.status != 'FINALIZED'::"InvoiceStatusEnum")
EXECUTE PROCEDURE fn_update_customer_ytd_summary_invoice();

-- Credit note customer YTD triggers
CREATE TRIGGER trg_update_customer_ytd_summary_credit_note_insert
  AFTER INSERT
  ON credit_note
  FOR EACH ROW
  WHEN (NEW.status = 'FINALIZED'::"CreditNoteStatus")
EXECUTE PROCEDURE fn_update_customer_ytd_summary_credit_note();

CREATE TRIGGER trg_update_customer_ytd_summary_credit_note_update
  AFTER UPDATE
  ON credit_note
  FOR EACH ROW
  WHEN (NEW.status = 'FINALIZED'::"CreditNoteStatus" AND OLD.status != 'FINALIZED'::"CreditNoteStatus")
EXECUTE PROCEDURE fn_update_customer_ytd_summary_credit_note();

-- ============================================================
-- MRR AGGREGATION TRIGGER
-- ============================================================

CREATE TRIGGER tr_after_insert_bi_mrr_movement_log
  AFTER INSERT
  ON bi_mrr_movement_log
  FOR EACH ROW
EXECUTE PROCEDURE fn_update_mrr();

-- ============================================================
-- USD RECALCULATION TRIGGER
-- ============================================================

CREATE TRIGGER update_usd_totals_trigger
  AFTER INSERT OR UPDATE
  ON historical_rates_from_usd
  FOR EACH ROW
EXECUTE PROCEDURE update_bi_usd_totals_from_rates();



-- Revert USD columns from NUMERIC(20,4) back to BIGINT
-- Warning: This will truncate decimal precision

-- Update bi_revenue_daily
ALTER TABLE bi_revenue_daily
  ALTER COLUMN net_revenue_cents_usd TYPE BIGINT USING ROUND(net_revenue_cents_usd)::BIGINT;

-- Update bi_delta_mrr_daily
ALTER TABLE bi_delta_mrr_daily
  ALTER COLUMN net_mrr_cents_usd TYPE BIGINT USING ROUND(net_mrr_cents_usd)::BIGINT,
  ALTER COLUMN new_business_cents_usd TYPE BIGINT USING ROUND(new_business_cents_usd)::BIGINT,
  ALTER COLUMN expansion_cents_usd TYPE BIGINT USING ROUND(expansion_cents_usd)::BIGINT,
  ALTER COLUMN contraction_cents_usd TYPE BIGINT USING ROUND(contraction_cents_usd)::BIGINT,
  ALTER COLUMN churn_cents_usd TYPE BIGINT USING ROUND(churn_cents_usd)::BIGINT,
  ALTER COLUMN reactivation_cents_usd TYPE BIGINT USING ROUND(reactivation_cents_usd)::BIGINT;
