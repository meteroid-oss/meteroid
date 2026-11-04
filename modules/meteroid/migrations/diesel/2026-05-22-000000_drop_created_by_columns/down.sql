-- Re-add columns as NULLABLE. Original NOT NULL constraint cannot be
-- restored without backfill data, which is irrecoverable once the up
-- migration runs. Attribution for pre-down rows is permanently lost.

ALTER TABLE api_token         ADD COLUMN created_by UUID;
ALTER TABLE bank_account      ADD COLUMN created_by UUID;
ALTER TABLE billable_metric   ADD COLUMN created_by UUID;
ALTER TABLE checkout_session  ADD COLUMN created_by UUID;
ALTER TABLE customer          ADD COLUMN created_by UUID,
                              ADD COLUMN updated_by UUID,
                              ADD COLUMN archived_by UUID;
ALTER TABLE entitlement       ADD COLUMN created_by UUID;
ALTER TABLE feature           ADD COLUMN created_by UUID;
ALTER TABLE plan              ADD COLUMN created_by UUID;
ALTER TABLE plan_version      ADD COLUMN created_by UUID;
ALTER TABLE price             ADD COLUMN created_by UUID;
ALTER TABLE product           ADD COLUMN created_by UUID;
ALTER TABLE subscription      ADD COLUMN created_by UUID;
