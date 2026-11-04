-- The Postgres BI rollups are replaced by the analytics plane (ClickHouse):
--   bi_delta_mrr_daily      -> fact_mrr_movement aggregates (QueryMrrTimeseries)
--   bi_revenue_daily        -> fact_revenue_item aggregates (QueryRevenueDaily)
--   bi_customer_ytd_summary -> fact_revenue_item aggregates (QueryTopRevenue)
-- bi_mrr_movement_log stays: it is the transactional movement record the
-- analytics projector reads into ClickHouse fact_mrr_movement.

DROP TABLE IF EXISTS bi_delta_mrr_daily;
DROP TABLE IF EXISTS bi_revenue_daily;
DROP TABLE IF EXISTS bi_customer_ytd_summary;

-- The bi_aggregation pgmq consumer is gone (nothing writes the rollups anymore).
DO $$
BEGIN
  PERFORM pgmq.drop_queue('bi_aggregation');
EXCEPTION WHEN OTHERS THEN
  -- Queue absent (fresh install) — nothing to drop.
  NULL;
END $$;
