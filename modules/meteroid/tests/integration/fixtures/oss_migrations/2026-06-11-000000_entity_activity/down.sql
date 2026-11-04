-- Re-add attribution columns as NULLABLE. The original NOT NULL constraint
-- cannot be restored without backfill data, which is irrecoverable once the up
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

DROP TABLE IF EXISTS sent_email;
DROP TABLE IF EXISTS entity_activity;
DROP TYPE IF EXISTS "ActorTypeEnum";

CREATE TABLE IF NOT EXISTS quote_activity (
    id uuid PRIMARY KEY,
    quote_id uuid NOT NULL REFERENCES quote(id) ON DELETE CASCADE,
    activity_type varchar NOT NULL,
    description text NOT NULL,
    actor_type varchar NOT NULL,
    actor_id varchar,
    actor_name varchar,
    created_at timestamptz NOT NULL DEFAULT now(),
    ip_address varchar,
    user_agent text
);

CREATE INDEX IF NOT EXISTS idx_quote_activity_quote_id ON quote_activity (quote_id, created_at DESC);
