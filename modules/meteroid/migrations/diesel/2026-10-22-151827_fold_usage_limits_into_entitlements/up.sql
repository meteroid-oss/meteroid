-- A customer's own limit is now an entitlement override: usage limits go, with what their monitors
-- recorded.
DELETE FROM usage_alert_state WHERE monitor_id IN (SELECT id FROM usage_limit);
DELETE FROM usage_monitor_state WHERE monitor_kind = 'usage_limit';
ALTER TABLE usage_monitor_state
    DROP CONSTRAINT usage_monitor_state_monitor_kind_check,
    ADD CONSTRAINT usage_monitor_state_monitor_kind_check
        CHECK (monitor_kind IN ('entitlement', 'budget', 'heartbeat'));
DROP TABLE usage_limit;
DROP TYPE "UsageLimitBehaviorEnum";

-- A budget counts one subscription's spend when `subscription_id` is set, else the customer's.
-- One live budget per denomination and scope.
ALTER TABLE customer_budget
    ADD COLUMN subscription_id UUID REFERENCES subscription (id) ON DELETE CASCADE;
DROP INDEX customer_budget_live_currency_idx;
CREATE UNIQUE INDEX customer_budget_live_currency_idx
    ON customer_budget (customer_id, currency, subscription_id) NULLS NOT DISTINCT
    WHERE archived_at IS NULL;
CREATE INDEX customer_budget_subscription_idx ON customer_budget (subscription_id)
    WHERE subscription_id IS NOT NULL;
