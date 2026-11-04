-- Drop attribution columns now subsumed by entity_activity.
-- Kept: batch_job.created_by (still surfaced in BatchJobDetail UI),
--       customer_balance_tx.created_by + customer_balance_pending_tx.created_by
--       (ledger provenance must outlive a possible audit-table reset).

ALTER TABLE api_token         DROP COLUMN created_by;
ALTER TABLE bank_account      DROP COLUMN created_by;
ALTER TABLE billable_metric   DROP COLUMN created_by;
ALTER TABLE checkout_session  DROP COLUMN created_by;
ALTER TABLE customer          DROP COLUMN created_by,
                              DROP COLUMN updated_by,
                              DROP COLUMN archived_by;
ALTER TABLE entitlement       DROP COLUMN created_by;
ALTER TABLE feature           DROP COLUMN created_by;
ALTER TABLE plan              DROP COLUMN created_by;
ALTER TABLE plan_version      DROP COLUMN created_by;
ALTER TABLE price             DROP COLUMN created_by;
ALTER TABLE product           DROP COLUMN created_by;
ALTER TABLE subscription      DROP COLUMN created_by;
