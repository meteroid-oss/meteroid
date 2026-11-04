-- Archived subscription budgets lose their scope; a live one has no place in the schema before.
DO
$$
    BEGIN
        IF EXISTS (SELECT 1 FROM customer_budget WHERE subscription_id IS NOT NULL AND archived_at IS NULL) THEN
            RAISE EXCEPTION 'live subscription budgets exist; archive them before rollback';
        END IF;
    END
$$;
DROP INDEX customer_budget_subscription_idx;
DROP INDEX customer_budget_live_currency_idx;
ALTER TABLE customer_budget DROP COLUMN subscription_id;
CREATE UNIQUE INDEX customer_budget_live_currency_idx ON customer_budget (customer_id, currency)
    WHERE archived_at IS NULL;

CREATE TYPE "UsageLimitBehaviorEnum" AS ENUM ('TRACK', 'SOFT', 'HARD');
CREATE TABLE usage_limit (
    id                 UUID PRIMARY KEY,
    tenant_id          UUID                     NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    customer_id        UUID                     NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    billable_metric_id UUID                     NOT NULL REFERENCES billable_metric (id) ON DELETE CASCADE,
    name               TEXT                     NOT NULL,
    limit_value        NUMERIC                  NOT NULL,
    "window"           JSONB                    NOT NULL,
    group_by           TEXT,
    behavior           "UsageLimitBehaviorEnum" NOT NULL,
    grace_pct          INTEGER,
    thresholds         INTEGER[]                NOT NULL DEFAULT '{}',
    version            INTEGER                  NOT NULL DEFAULT 1,
    archived_at        TIMESTAMPTZ,
    created_at         TIMESTAMPTZ              NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ              NOT NULL DEFAULT now()
);
CREATE INDEX usage_limit_customer_idx ON usage_limit (tenant_id, customer_id) WHERE archived_at IS NULL;
CREATE INDEX usage_limit_customer_fk_idx ON usage_limit (customer_id);
CREATE INDEX usage_limit_metric_idx ON usage_limit (billable_metric_id);

ALTER TABLE usage_monitor_state
    DROP CONSTRAINT usage_monitor_state_monitor_kind_check,
    ADD CONSTRAINT usage_monitor_state_monitor_kind_check
        CHECK (monitor_kind IN ('entitlement', 'usage_limit', 'budget', 'heartbeat'));
