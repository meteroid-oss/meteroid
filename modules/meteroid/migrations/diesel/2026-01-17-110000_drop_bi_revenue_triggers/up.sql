-- Create the bi_aggregation queue for processing BI table updates via Rust
SELECT pgmq.create('bi_aggregation');



-- Drop ALL BI triggers that are now handled by Rust
-- Revenue aggregation is handled via the BiAggregation worker (PGMQ)
-- MRR aggregation is handled inline in invoice processing and subscription termination
-- USD recalculation is handled by the currency rates worker

-- ============================================================
-- REVENUE TRIGGERS (invoice/credit_note -> bi_revenue_daily, bi_customer_ytd_summary)
-- ============================================================

-- Drop invoice revenue triggers
DROP TRIGGER IF EXISTS trg_update_revenue_invoice_insert ON invoice;
DROP TRIGGER IF EXISTS trg_update_revenue_invoice_update ON invoice;

-- Drop credit note revenue triggers
DROP TRIGGER IF EXISTS trg_update_revenue_credit_note_insert ON credit_note;
DROP TRIGGER IF EXISTS trg_update_revenue_credit_note_update ON credit_note;

-- Drop invoice customer YTD triggers
DROP TRIGGER IF EXISTS trg_update_customer_ytd_summary_invoice_insert ON invoice;
DROP TRIGGER IF EXISTS trg_update_customer_ytd_summary_invoice_update ON invoice;

-- Drop credit note customer YTD triggers
DROP TRIGGER IF EXISTS trg_update_customer_ytd_summary_credit_note_insert ON credit_note;
DROP TRIGGER IF EXISTS trg_update_customer_ytd_summary_credit_note_update ON credit_note;

-- ============================================================
-- MRR AGGREGATION TRIGGER (bi_mrr_movement_log -> bi_delta_mrr_daily)
-- ============================================================

DROP TRIGGER IF EXISTS tr_after_insert_bi_mrr_movement_log ON bi_mrr_movement_log;

-- ============================================================
-- USD RECALCULATION TRIGGER (historical_rates_from_usd -> bi_delta_mrr_daily, bi_revenue_daily)
-- ============================================================

DROP TRIGGER IF EXISTS update_usd_totals_trigger ON historical_rates_from_usd;

-- ============================================================
-- DROP TRIGGER FUNCTIONS (no longer needed)
-- ============================================================

DROP FUNCTION IF EXISTS fn_update_revenue_invoice();
DROP FUNCTION IF EXISTS fn_update_revenue_credit_note();
DROP FUNCTION IF EXISTS fn_update_customer_ytd_summary_invoice();
DROP FUNCTION IF EXISTS fn_update_customer_ytd_summary_credit_note();
DROP FUNCTION IF EXISTS fn_update_mrr();
DROP FUNCTION IF EXISTS update_bi_usd_totals_from_rates();


-- Convert USD columns from BIGINT to NUMERIC(20,4) for better precision
-- NUMERIC(20,4) provides 4 decimal places of sub-cent precision

-- Update bi_revenue_daily
ALTER TABLE bi_revenue_daily
  ALTER COLUMN net_revenue_cents_usd TYPE NUMERIC(20,4) USING net_revenue_cents_usd::NUMERIC(20,4);

-- Update bi_delta_mrr_daily
ALTER TABLE bi_delta_mrr_daily
  ALTER COLUMN net_mrr_cents_usd TYPE NUMERIC(20,4) USING net_mrr_cents_usd::NUMERIC(20,4),
  ALTER COLUMN new_business_cents_usd TYPE NUMERIC(20,4) USING new_business_cents_usd::NUMERIC(20,4),
  ALTER COLUMN expansion_cents_usd TYPE NUMERIC(20,4) USING expansion_cents_usd::NUMERIC(20,4),
  ALTER COLUMN contraction_cents_usd TYPE NUMERIC(20,4) USING contraction_cents_usd::NUMERIC(20,4),
  ALTER COLUMN churn_cents_usd TYPE NUMERIC(20,4) USING churn_cents_usd::NUMERIC(20,4),
  ALTER COLUMN reactivation_cents_usd TYPE NUMERIC(20,4) USING reactivation_cents_usd::NUMERIC(20,4);
