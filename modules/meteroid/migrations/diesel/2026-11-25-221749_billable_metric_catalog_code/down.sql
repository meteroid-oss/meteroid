DROP TRIGGER IF EXISTS trg_billable_metric_fill_catalog_code ON billable_metric;

DROP INDEX IF EXISTS billable_metric_tenant_catalog_code_key;
ALTER TABLE billable_metric DROP COLUMN catalog_code;

DROP FUNCTION IF EXISTS fill_metric_catalog_code();
DROP FUNCTION IF EXISTS unique_metric_catalog_code(UUID, TEXT);
DROP FUNCTION IF EXISTS metric_catalog_code_base(TEXT);
