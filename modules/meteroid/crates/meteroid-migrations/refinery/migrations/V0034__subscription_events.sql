CREATE TYPE "SubscriptionEventType" AS ENUM (
    'CREATED',
    'ACTIVATED',
    'SWITCH',
    'CANCELLED',
    'REACTIVATED',
    'UPDATED'
    );

CREATE TABLE "subscription_event"
(
    "id"                     UUID                                   NOT NULL PRIMARY KEY,
    "mrr_delta"              bigint                                NULL,
    "event_type"             "SubscriptionEventType"                NOT NULL,
    "created_at"             timestamp(3) default CURRENT_TIMESTAMP NOT NULL,
    "applies_to"             date                                   NOT NULL,
    "subscription_id"        UUID                                   NOT NULL REFERENCES "subscription" ("id") ON DELETE CASCADE,
    "bi_mrr_movement_log_id" UUID                                   NULL REFERENCES "bi_mrr_movement_log" ("id") ON DELETE CASCADE,
    "details"                JSONB                                  NULL
);

