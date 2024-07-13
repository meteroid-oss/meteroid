CREATE TABLE IF NOT EXISTS "alert_rule"
(
  "id"                     UUID                                   NOT NULL PRIMARY KEY,
  "tenant_id"              UUID                                   NOT NULL REFERENCES "tenant" ("id") ON DELETE CASCADE,
  "billable_metric_id"     UUID                                   NOT NULL REFERENCES "billable_metric" ("id") ON DELETE CASCADE,
  "created_at"             timestamp(3) default CURRENT_TIMESTAMP NOT NULL,
  "updated_at"             timestamp(3) default CURRENT_TIMESTAMP NOT NULL,
  "description"            TEXT                                   NOT NULL,
  "enabled"                BOOLEAN default true                   NOT NULL,
  "threshold"              numeric                                NOT NULL
);

CREATE INDEX IF NOT EXISTS "alert_rule_tenant_id" ON alert_rule(tenant_id, created_at desc);

CREATE TABLE IF NOT EXISTS "alert"
(
  "id"                     UUID                                   NOT NULL PRIMARY KEY,
  "tenant_id"              UUID                                   NOT NULL REFERENCES "tenant" ("id") ON DELETE CASCADE,
  "alert_rule_id"          UUID                                   NOT NULL REFERENCES "alert_rule" ("id") ON DELETE CASCADE,
  "customer_id"            UUID                                   NOT NULL REFERENCES "customer" ("id") ON DELETE CASCADE,
  "subscription_id"        UUID                                   NOT NULL REFERENCES "subscription" ("id") ON DELETE CASCADE,
  "created_at"             timestamp(3) default CURRENT_TIMESTAMP NOT NULL,
  "updated_at"             timestamp(3) default CURRENT_TIMESTAMP NOT NULL,
  "is_open"                BOOLEAN default true                   NOT NULL,
  "last_metric_value"      numeric                                NOT NULL
);

CREATE INDEX IF NOT EXISTS "alert_tenant_id" ON alert_rule(tenant_id, created_at desc);
