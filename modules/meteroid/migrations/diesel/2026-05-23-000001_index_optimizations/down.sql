-- ============================================================
-- RESTORE: re-create dropped indexes
-- ============================================================

CREATE INDEX subscription_billing_day_idx
    ON subscription (billing_day_anchor);

CREATE INDEX subscription_start_date_idx
    ON subscription (start_date);

CREATE INDEX idx_scheduled_event_time
    ON scheduled_event (scheduled_time)
    WHERE status = 'PENDING'::"ScheduledEventStatus";

CREATE INDEX idx_scheduled_events_status
    ON scheduled_event (status);

CREATE INDEX idx_sub_component_active
    ON subscription_component (subscription_id)
    WHERE effective_to IS NULL;

CREATE INDEX idx_sub_component_period_overlap
    ON subscription_component (subscription_id, effective_from, effective_to);

-- ============================================================
-- REMOVE: indexes added by this migration
-- ============================================================

DROP INDEX IF EXISTS idx_subscription_tenant_customer;
DROP INDEX IF EXISTS idx_invoice_tenant_customer;
DROP INDEX IF EXISTS idx_invoice_tenant_subscription;
DROP INDEX IF EXISTS idx_invoice_tenant_date;
DROP INDEX IF EXISTS idx_invoice_pending;
DROP INDEX IF EXISTS idx_payment_tx_invoice;
DROP INDEX IF EXISTS idx_subscription_event_sub_date;
DROP INDEX IF EXISTS idx_plan_tenant;
DROP INDEX IF EXISTS idx_product_tenant;
DROP INDEX IF EXISTS idx_billable_metric_tenant;
DROP INDEX IF EXISTS idx_product_family_tenant;
DROP INDEX IF EXISTS idx_price_component_plan_version;
DROP INDEX IF EXISTS idx_schedule_plan_version;
CREATE INDEX idx_checkout_session_tenant
    ON checkout_session (tenant_id);
DROP INDEX IF EXISTS idx_checkout_session_tenant_customer;
DROP INDEX IF EXISTS idx_plan_version_tenant;
DROP INDEX IF EXISTS idx_invoicing_entity_tenant;
DROP INDEX IF EXISTS idx_payment_tx_tenant;
DROP INDEX IF EXISTS idx_api_token_tenant;
DROP INDEX IF EXISTS idx_bank_account_tenant;
DROP INDEX IF EXISTS idx_customer_payment_method_tenant_customer;