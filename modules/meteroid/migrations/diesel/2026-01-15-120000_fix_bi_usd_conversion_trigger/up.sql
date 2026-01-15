-- Fix tables and triggers related to the non-clickhouse-based BI system. In production, use Clickhouse.

-- Truncate and rebuild BI tables from source data
TRUNCATE bi_revenue_daily;
TRUNCATE bi_customer_ytd_summary;
TRUNCATE bi_delta_mrr_daily;

-- Rebuild bi_revenue_daily from invoices
INSERT INTO bi_revenue_daily (tenant_id, plan_version_id, currency, revenue_date, net_revenue_cents, historical_rate_id, net_revenue_cents_usd)
SELECT
    grouped.tenant_id,
    grouped.plan_version_id,
    grouped.currency,
    grouped.revenue_date,
    grouped.net_revenue_cents,
    hr.id AS historical_rate_id,
    grouped.net_revenue_cents / (hr.rates->>grouped.currency)::NUMERIC AS net_revenue_cents_usd
FROM (
    SELECT
        i.tenant_id,
        i.plan_version_id,
        i.currency,
        DATE_TRUNC('day', i.finalized_at)::date AS revenue_date,
        SUM(i.amount_due) AS net_revenue_cents
    FROM invoice i
    WHERE i.status = 'FINALIZED'::"InvoiceStatusEnum" AND i.finalized_at IS NOT NULL
    GROUP BY i.tenant_id, i.plan_version_id, i.currency, DATE_TRUNC('day', i.finalized_at)
) grouped
JOIN LATERAL (
    SELECT id, rates FROM historical_rates_from_usd
    WHERE date <= grouped.revenue_date
    ORDER BY date DESC
    LIMIT 1
) hr ON true;

-- Subtract credit notes from bi_revenue_daily
INSERT INTO bi_revenue_daily (tenant_id, plan_version_id, currency, revenue_date, net_revenue_cents, historical_rate_id, net_revenue_cents_usd)
SELECT
    grouped.tenant_id,
    grouped.plan_version_id,
    grouped.currency,
    grouped.revenue_date,
    grouped.net_revenue_cents,
    hr.id AS historical_rate_id,
    grouped.net_revenue_cents / (hr.rates->>grouped.currency)::NUMERIC AS net_revenue_cents_usd
FROM (
    SELECT
        cn.tenant_id,
        cn.plan_version_id,
        cn.currency,
        DATE_TRUNC('day', cn.finalized_at)::date AS revenue_date,
        -SUM(cn.refunded_amount_cents) AS net_revenue_cents
    FROM credit_note cn
    WHERE cn.status = 'FINALIZED'::"CreditNoteStatus" AND cn.finalized_at IS NOT NULL
    GROUP BY cn.tenant_id, cn.plan_version_id, cn.currency, DATE_TRUNC('day', cn.finalized_at)
) grouped
JOIN LATERAL (
    SELECT id, rates FROM historical_rates_from_usd
    WHERE date <= grouped.revenue_date
    ORDER BY date DESC
    LIMIT 1
) hr ON true
ON CONFLICT (tenant_id, plan_version_id, currency, revenue_date) DO UPDATE
SET net_revenue_cents = bi_revenue_daily.net_revenue_cents + EXCLUDED.net_revenue_cents,
    net_revenue_cents_usd = bi_revenue_daily.net_revenue_cents_usd + EXCLUDED.net_revenue_cents_usd;

-- Rebuild bi_customer_ytd_summary from invoices
INSERT INTO bi_customer_ytd_summary (tenant_id, customer_id, revenue_year, currency, total_revenue_cents)
SELECT
    i.tenant_id,
    i.customer_id,
    DATE_PART('year', i.finalized_at)::integer AS revenue_year,
    i.currency,
    SUM(i.amount_due) AS total_revenue_cents
FROM invoice i
WHERE i.status = 'FINALIZED'::"InvoiceStatusEnum" AND i.finalized_at IS NOT NULL
GROUP BY i.tenant_id, i.customer_id, DATE_PART('year', i.finalized_at), i.currency;

-- Subtract credit notes from bi_customer_ytd_summary
INSERT INTO bi_customer_ytd_summary (tenant_id, customer_id, revenue_year, currency, total_revenue_cents)
SELECT
    cn.tenant_id,
    cn.customer_id,
    DATE_PART('year', cn.finalized_at)::integer AS revenue_year,
    cn.currency,
    -SUM(cn.refunded_amount_cents) AS total_revenue_cents
FROM credit_note cn
WHERE cn.status = 'FINALIZED'::"CreditNoteStatus" AND cn.finalized_at IS NOT NULL
GROUP BY cn.tenant_id, cn.customer_id, DATE_PART('year', cn.finalized_at), cn.currency
ON CONFLICT (tenant_id, customer_id, currency, revenue_year) DO UPDATE
SET total_revenue_cents = bi_customer_ytd_summary.total_revenue_cents + EXCLUDED.total_revenue_cents;

-- Rebuild bi_delta_mrr_daily from bi_mrr_movement_log
INSERT INTO bi_delta_mrr_daily (
    tenant_id, plan_version_id, currency, date,
    net_mrr_cents, net_mrr_cents_usd,
    new_business_cents, new_business_cents_usd, new_business_count,
    expansion_cents, expansion_cents_usd, expansion_count,
    contraction_cents, contraction_cents_usd, contraction_count,
    churn_cents, churn_cents_usd, churn_count,
    reactivation_cents, reactivation_cents_usd, reactivation_count,
    historical_rate_id
)
SELECT
    grouped.tenant_id,
    grouped.plan_version_id,
    grouped.currency,
    grouped.date,
    grouped.net_mrr_cents,
    grouped.net_mrr_cents / (hr.rates->>grouped.currency)::NUMERIC AS net_mrr_cents_usd,
    grouped.new_business_cents,
    grouped.new_business_cents / (hr.rates->>grouped.currency)::NUMERIC AS new_business_cents_usd,
    grouped.new_business_count,
    grouped.expansion_cents,
    grouped.expansion_cents / (hr.rates->>grouped.currency)::NUMERIC AS expansion_cents_usd,
    grouped.expansion_count,
    grouped.contraction_cents,
    grouped.contraction_cents / (hr.rates->>grouped.currency)::NUMERIC AS contraction_cents_usd,
    grouped.contraction_count,
    grouped.churn_cents,
    grouped.churn_cents / (hr.rates->>grouped.currency)::NUMERIC AS churn_cents_usd,
    grouped.churn_count,
    grouped.reactivation_cents,
    grouped.reactivation_cents / (hr.rates->>grouped.currency)::NUMERIC AS reactivation_cents_usd,
    grouped.reactivation_count,
    hr.id AS historical_rate_id
FROM (
    SELECT
        ml.tenant_id,
        ml.plan_version_id,
        ml.currency,
        ml.applies_to AS date,
        SUM(ml.net_mrr_change) AS net_mrr_cents,
        SUM(CASE WHEN ml.movement_type = 'NEW_BUSINESS' THEN ml.net_mrr_change ELSE 0 END) AS new_business_cents,
        SUM(CASE WHEN ml.movement_type = 'NEW_BUSINESS' THEN 1 ELSE 0 END) AS new_business_count,
        SUM(CASE WHEN ml.movement_type = 'EXPANSION' THEN ml.net_mrr_change ELSE 0 END) AS expansion_cents,
        SUM(CASE WHEN ml.movement_type = 'EXPANSION' THEN 1 ELSE 0 END) AS expansion_count,
        SUM(CASE WHEN ml.movement_type = 'CONTRACTION' THEN ml.net_mrr_change ELSE 0 END) AS contraction_cents,
        SUM(CASE WHEN ml.movement_type = 'CONTRACTION' THEN 1 ELSE 0 END) AS contraction_count,
        SUM(CASE WHEN ml.movement_type = 'CHURN' THEN ml.net_mrr_change ELSE 0 END) AS churn_cents,
        SUM(CASE WHEN ml.movement_type = 'CHURN' THEN 1 ELSE 0 END) AS churn_count,
        SUM(CASE WHEN ml.movement_type = 'REACTIVATION' THEN ml.net_mrr_change ELSE 0 END) AS reactivation_cents,
        SUM(CASE WHEN ml.movement_type = 'REACTIVATION' THEN 1 ELSE 0 END) AS reactivation_count
    FROM bi_mrr_movement_log ml
    GROUP BY ml.tenant_id, ml.plan_version_id, ml.currency, ml.applies_to
) grouped
JOIN LATERAL (
    SELECT id, rates FROM historical_rates_from_usd
    WHERE date <= grouped.date
    ORDER BY date DESC
    LIMIT 1
) hr ON true;

-- Fix the USD conversion trigger
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

-- Fix invoice revenue trigger: split into INSERT and UPDATE to avoid double-counting
DROP TRIGGER IF EXISTS trg_update_revenue_invoice ON invoice;
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

-- Fix credit note revenue trigger: split into INSERT and UPDATE
DROP TRIGGER IF EXISTS trg_update_revenue_credit_note ON credit_note;
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

-- Fix invoice customer YTD trigger: split into INSERT and UPDATE
DROP TRIGGER IF EXISTS trg_update_customer_ytd_summary_invoice ON invoice;
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

-- Fix credit note customer YTD trigger: split into INSERT and UPDATE
DROP TRIGGER IF EXISTS trg_update_customer_ytd_summary_credit_note ON credit_note;
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
