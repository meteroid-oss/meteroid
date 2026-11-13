-- Dropping the defaults broke inserts from pods of the previous release during a rolling deploy;
-- they are kept for now (see 2026-11-13-171544_billing_rows_created_at_defaults_kept).
SELECT 1;
