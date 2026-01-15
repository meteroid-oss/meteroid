-- Revert to the old (buggy) behavior

-- Revert USD conversion trigger to use multiplication
CREATE OR REPLACE FUNCTION update_bi_usd_totals_from_rates() RETURNS TRIGGER
  LANGUAGE plpgsql
AS
$$
BEGIN
  -- Update bi_delta_mrr_daily
  UPDATE bi_delta_mrr_daily
  SET net_mrr_cents_usd = net_mrr_cents * (NEW.rates->>currency)::NUMERIC,
      new_business_cents_usd = new_business_cents * (NEW.rates->>currency)::NUMERIC,
      expansion_cents_usd = expansion_cents * (NEW.rates->>currency)::NUMERIC,
      contraction_cents_usd = contraction_cents * (NEW.rates->>currency)::NUMERIC,
      churn_cents_usd = churn_cents * (NEW.rates->>currency)::NUMERIC,
      reactivation_cents_usd = reactivation_cents * (NEW.rates->>currency)::NUMERIC,
      historical_rate_id = NEW.id
  WHERE date = NEW.date;

  -- Update bi_revenue_daily
  UPDATE bi_revenue_daily
  SET net_revenue_cents_usd = net_revenue_cents * (NEW.rates->>currency)::NUMERIC,
      historical_rate_id = NEW.id
  WHERE revenue_date = NEW.date;

  RETURN NEW;
END;
$$;

-- Revert invoice revenue trigger to old behavior (fires on any update when FINALIZED)
DROP TRIGGER IF EXISTS trg_update_revenue_invoice_insert ON invoice;
DROP TRIGGER IF EXISTS trg_update_revenue_invoice_update ON invoice;
CREATE TRIGGER trg_update_revenue_invoice
  AFTER INSERT OR UPDATE
  ON invoice
  FOR EACH ROW
  WHEN (NEW.status = 'FINALIZED'::"InvoiceStatusEnum")
EXECUTE PROCEDURE fn_update_revenue_invoice();

-- Revert credit note revenue trigger
DROP TRIGGER IF EXISTS trg_update_revenue_credit_note_insert ON credit_note;
DROP TRIGGER IF EXISTS trg_update_revenue_credit_note_update ON credit_note;
CREATE TRIGGER trg_update_revenue_credit_note
  AFTER INSERT OR UPDATE
  ON credit_note
  FOR EACH ROW
  WHEN (NEW.status = 'FINALIZED'::"CreditNoteStatus")
EXECUTE PROCEDURE fn_update_revenue_credit_note();

-- Revert invoice customer YTD trigger
DROP TRIGGER IF EXISTS trg_update_customer_ytd_summary_invoice_insert ON invoice;
DROP TRIGGER IF EXISTS trg_update_customer_ytd_summary_invoice_update ON invoice;
CREATE TRIGGER trg_update_customer_ytd_summary_invoice
  AFTER INSERT OR UPDATE
  ON invoice
  FOR EACH ROW
  WHEN (NEW.status = 'FINALIZED'::"InvoiceStatusEnum")
EXECUTE PROCEDURE fn_update_customer_ytd_summary_invoice();

-- Revert credit note customer YTD trigger
DROP TRIGGER IF EXISTS trg_update_customer_ytd_summary_credit_note_insert ON credit_note;
DROP TRIGGER IF EXISTS trg_update_customer_ytd_summary_credit_note_update ON credit_note;
CREATE TRIGGER trg_update_customer_ytd_summary_credit_note
  AFTER INSERT OR UPDATE
  ON credit_note
  FOR EACH ROW
  WHEN (NEW.status = 'FINALIZED'::"CreditNoteStatus")
EXECUTE PROCEDURE fn_update_customer_ytd_summary_credit_note();
