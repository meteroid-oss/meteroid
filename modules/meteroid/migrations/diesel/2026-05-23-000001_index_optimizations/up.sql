-- ============================================================
-- DROP: redundant / duplicate indexes
-- ============================================================

-- Exact duplicate of idx_scheduled_events_due (same table, column, partial predicate)
DROP INDEX IF EXISTS idx_scheduled_event_time;

-- subscription_component: superseded by idx_subscription_component_subscription_id (plain);
--   < 100 rows per subscription makes partial/composite variants pointless
DROP INDEX IF EXISTS idx_sub_component_active;
DROP INDEX IF EXISTS idx_sub_component_period_overlap;

-- billing_day_anchor is never used as a query filter in the Rust query layer
DROP INDEX IF EXISTS subscription_billing_day_idx;

-- start_date is never used as a query filter in the Rust query layer
DROP INDEX IF EXISTS subscription_start_date_idx;

-- Low-cardinality single-column enum index; covered by composite/partial indexes;
-- replaced below by a targeted partial index for the PROCESSING timeout path
DROP INDEX IF EXISTS idx_scheduled_events_status;

-- ============================================================
-- ADD: missing indexes identified from query patterns
-- ============================================================

-- subscription: list_subscriptions(tenant, customer) + find_active_by_customer
CREATE INDEX idx_subscription_tenant_customer
    ON subscription (tenant_id, customer_id);

-- invoice: list_invoices / list_full_invoices filtered by customer
CREATE INDEX idx_invoice_tenant_customer
    ON invoice (tenant_id, customer_id);

-- invoice: find_last_by_subscription_id, find_existing_recurring_invoice,
--          list_invoices filtered by subscription_id
CREATE INDEX idx_invoice_tenant_subscription
    ON invoice (tenant_id, subscription_id)
    WHERE subscription_id IS NOT NULL;

-- invoice: default sort is invoice_date DESC; covers full-tenant paginated lists
CREATE INDEX idx_invoice_tenant_date
    ON invoice (tenant_id, invoice_date DESC, id DESC);

-- invoice: worker scans (list_to_finalize, list_outdated) cursor-paginate by id
--   over non-terminal invoices. Partial index on PK is not redundant — it is a
--   smaller B-tree containing only non-terminal rows, so cursor scans skip the
--   (majority) VOID/FINALIZED rows entirely.
CREATE INDEX idx_invoice_pending
    ON invoice (id)
    WHERE status <> 'VOID'::"InvoiceStatusEnum"
      AND status <> 'FINALIZED'::"InvoiceStatusEnum";

-- payment_transaction: BelongingTo join + list_by_invoice_id + last_settled_by_invoice_id
CREATE INDEX idx_payment_tx_invoice
    ON payment_transaction (invoice_id)
    WHERE invoice_id IS NOT NULL;

-- subscription_event: fetch_by_subscription_id_and_date
--                     fetch_by_subscription_id_and_event_type
CREATE INDEX idx_subscription_event_sub_date
    ON subscription_event (subscription_id, applies_to);

-- plan: list_plans by tenant (no index on plan.tenant_id currently)
CREATE INDEX idx_plan_tenant
    ON plan (tenant_id);

-- product: list_products by tenant
CREATE INDEX idx_product_tenant
    ON product (tenant_id);

-- billable_metric: list_metrics by tenant
CREATE INDEX idx_billable_metric_tenant
    ON billable_metric (tenant_id);

-- product_family: list by tenant
CREATE INDEX idx_product_family_tenant
    ON product_family (tenant_id);

-- price_component: fetch components for a plan_version (no index currently)
CREATE INDEX idx_price_component_plan_version
    ON price_component (plan_version_id);

-- schedule: list_schedules_by_subscription joins via plan_version_id
CREATE INDEX idx_schedule_plan_version
    ON schedule (plan_version_id);

-- checkout_session: composite replaces the standalone idx_checkout_session_tenant
--                  (leading tenant_id covers full-tenant list; both columns cover
--                  the per-customer filter)
DROP INDEX IF EXISTS idx_checkout_session_tenant;
CREATE INDEX idx_checkout_session_tenant_customer
    ON checkout_session (tenant_id, customer_id);


-- ============================================================
-- ADD: tenant_id leading indexes (every table filtered by tenant_id)
-- ============================================================

-- plan_version: all find/list queries filter by tenant_id; plan_id is the
--               next most selective column
CREATE INDEX idx_plan_version_tenant
    ON plan_version (tenant_id, plan_id);

-- invoicing_entity: list_by_tenant_id + exists_any_for_tenant; only existing
--                   index is partial-unique WHERE is_default = true
CREATE INDEX idx_invoicing_entity_tenant
    ON invoicing_entity (tenant_id);

-- payment_transaction: list/find queries filter by (tenant_id, invoice_id);
--                      idx_payment_tx_invoice above covers BelongingTo joins
--                      (invoice_id-only scan); this composite covers tenant path
CREATE INDEX idx_payment_tx_tenant
    ON payment_transaction (tenant_id, invoice_id)
    WHERE invoice_id IS NOT NULL;

-- api_token: find_by_tenant_id; only existing index is unique on hash
CREATE INDEX idx_api_token_tenant
    ON api_token (tenant_id);

-- bank_account: all access scoped to tenant_id; no indexes exist
CREATE INDEX idx_bank_account_tenant
    ON bank_account (tenant_id);

-- customer_payment_method: every query filters by (tenant_id, customer_id)
CREATE INDEX idx_customer_payment_method_tenant_customer
    ON customer_payment_method (tenant_id, customer_id);
