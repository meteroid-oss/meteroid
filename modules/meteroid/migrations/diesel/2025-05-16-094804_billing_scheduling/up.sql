-- Migration to add billing system tables

-- Create enum types using your format
create type "SubscriptionStatusEnum" as enum (
  'PENDING_ACTIVATION',
  'PENDING_CHARGE',
  'TRIAL_ACTIVE',
  'ACTIVE',
  'TRIAL_EXPIRED',
  'PAUSED',
  'SUSPENDED',
  'CANCELLED',
  'COMPLETED',
  'SUPERSEDED'
  );
create type "CycleActionEnum" as enum (
  'ACTIVATE_SUBSCRIPTION',
  'RENEW_SUBSCRIPTION',
  'END_TRIAL',
  'END_SUBSCRIPTION'
  );
create type "ScheduledEventTypeEnum" as enum (
  'FINALIZE_INVOICE',
  'RETRY_PAYMENT',
  'CANCEL_SUBSCRIPTION',
  'PAUSE_SUBSCRIPTION',
  'APPLY_PLAN_CHANGE'
  );
create type "ScheduledEventStatus" as enum ('PENDING', 'PROCESSING', 'COMPLETED', 'FAILED', 'CANCELED');

-- Add new fields to subscriptions table
-- Including both pending plan fields and cycle tracking fields
ALTER TABLE subscription
--   ADD COLUMN pending_plan_version_id  UUID,
--   ADD COLUMN pending_plan_change_date DATE,
-- Merged cycle fields
  ADD COLUMN cycle_index              INTEGER           NULL,
  ADD COLUMN status              "SubscriptionStatusEnum"  NOT NULL DEFAULT 'PENDING_ACTIVATION',
  ADD COLUMN current_period_start     DATE              NOT NULL DEFAULT CURRENT_DATE,
  ADD COLUMN current_period_end       DATE              NULL,
  ADD COLUMN next_cycle_action        "CycleActionEnum" NULL,
  -- processing
  ADD COLUMN last_error      TEXT NULL,
  ADD COLUMN error_count     INTEGER NOT NULL DEFAULT 0,
  ADD COLUMN next_retry      TIMESTAMP NULL;
;

-- drop all defaults
ALTER TABLE subscription
  ALTER COLUMN status DROP DEFAULT,
  ALTER COLUMN current_period_start DROP DEFAULT,
  ALTER COLUMN current_period_end DROP DEFAULT,
  ALTER COLUMN next_cycle_action DROP DEFAULT;

ALTER TABLE subscription
  DROP COLUMN canceled_at,
  DROP COLUMN cancellation_reason;


CREATE INDEX idx_subscriptions_current_period_end ON subscription (current_period_end)
  WHERE current_period_end IS NOT NULL;

-- Create scheduled events table
CREATE TABLE scheduled_event
(
  id              UUID PRIMARY KEY,
  subscription_id UUID                     NOT NULL REFERENCES subscription (id) ON DELETE CASCADE,
  tenant_id       UUID                     NOT NULL,
  event_type      "ScheduledEventTypeEnum" NOT NULL,
  scheduled_time  TIMESTAMP                NOT NULL,
  priority        INTEGER                  NOT NULL,
  event_data      JSONB                    NOT NULL,
  created_at      TIMESTAMP default CURRENT_TIMESTAMP                NOT NULL,
  updated_at      TIMESTAMP default CURRENT_TIMESTAMP                NOT NULL,
  status          "ScheduledEventStatus"   NOT NULL,
  retries         INTEGER                  NOT NULL DEFAULT 0,
  last_retry_at   TIMESTAMP,
  error           TEXT,
  processed_at    TIMESTAMP,
  source          TEXT                     NOT NULL
);

-- Create indexes for efficient lookup and processing
CREATE INDEX idx_scheduled_events_due ON scheduled_event (scheduled_time)
  WHERE status = 'PENDING';
CREATE INDEX idx_scheduled_events_subscription_id ON scheduled_event (subscription_id);
CREATE INDEX idx_scheduled_events_tenant_id ON scheduled_event (tenant_id);
CREATE INDEX idx_scheduled_events_status ON scheduled_event (status);

ALTER TABLE historical_rates_from_usd
  ADD COLUMN updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP;

ALTER TABLE customer
  ALTER COLUMN balance_value_cents TYPE BIGINT;

ALTER TABLE customer_balance_tx
  ALTER COLUMN amount_cents TYPE BIGINT,
  ALTER COLUMN balance_cents_after TYPE BIGINT;

ALTER TABLE customer_balance_pending_tx
  ALTER COLUMN amount_cents TYPE BIGINT;
