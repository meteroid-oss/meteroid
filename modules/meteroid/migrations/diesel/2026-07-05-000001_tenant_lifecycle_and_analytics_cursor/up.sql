
-- ============================================================================
-- 1. Tenant activity + lifecycle
-- ============================================================================
-- last_active_at is set (throttled) when a user loads a tenant; it drives sandbox
-- usage freshness and the non-prod inactivity lifecycle.
-- status is the lifecycle state: ACTIVE is normal; PAUSED (inactive) is skipped by
-- scheduled billing/lifecycle workers; ARCHIVED / MARKED_FOR_DELETION are terminal
-- states for cleanup. Production tenants are always ACTIVE.
ALTER TABLE tenant ADD COLUMN last_active_at TIMESTAMP;

CREATE TYPE "TenantStatusEnum" AS ENUM ('ACTIVE', 'PAUSED', 'ARCHIVED', 'MARKED_FOR_DELETION');
ALTER TABLE tenant ADD COLUMN status "TenantStatusEnum" NOT NULL DEFAULT 'ACTIVE';
CREATE INDEX idx_tenant_status ON tenant (status) WHERE status <> 'ACTIVE';

-- ============================================================================
-- 2. Analytics projection cursor
-- ============================================================================
-- The Postgres->ClickHouse projector pages every source table with a keyset cursor that
-- must follow *commit* order. Business timestamps (finalized_at, voided_at, processed_at,
-- updated_at, created_at, ...) can be backdated by imports and the sandbox seeder, so a row
-- committed "in the past" would fall below an already-advanced watermark and never be
-- projected -- empty usage charts, a recurring-vs-collected revenue gap, and split-by
-- breakdowns collapsing to a single "Unattributed" series when a customer never reaches
-- dim_customer.
--
-- store_committed_at is a business-meaningless marker: the wall-clock instant our store
-- physically wrote the row, trigger-maintained (clock_timestamp() on INSERT and UPDATE) so
-- no app write path can forget or backdate it, and re-stamped on every edit so a mutation
-- re-enters the projection window. Business timestamps keep their meaning and keep bucketing
-- facts into their correct historical periods (billed_at, paid_at, ...).
--
-- Covers the fact source tables (invoice, credit_note, payment_transaction) and the dimension
-- source tables (customer, plan, product). historical_rates_from_usd is excluded: its
-- updated_at is already insertion-time (rates are fetched, never backdated).

CREATE OR REPLACE FUNCTION set_store_committed_at() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    NEW.store_committed_at := clock_timestamp();
    RETURN NEW;
END $$;

ALTER TABLE invoice ADD COLUMN store_committed_at timestamptz;
UPDATE invoice SET store_committed_at = COALESCE(updated_at, created_at);
ALTER TABLE invoice
    ALTER COLUMN store_committed_at SET NOT NULL,
    ALTER COLUMN store_committed_at SET DEFAULT clock_timestamp();
CREATE TRIGGER trg_invoice_store_committed_at
    BEFORE INSERT OR UPDATE ON invoice
    FOR EACH ROW EXECUTE FUNCTION set_store_committed_at();
CREATE INDEX idx_invoice_store_committed_at ON invoice (store_committed_at, id);

ALTER TABLE credit_note ADD COLUMN store_committed_at timestamptz;
UPDATE credit_note SET store_committed_at = COALESCE(updated_at, created_at);
ALTER TABLE credit_note
    ALTER COLUMN store_committed_at SET NOT NULL,
    ALTER COLUMN store_committed_at SET DEFAULT clock_timestamp();
CREATE TRIGGER trg_credit_note_store_committed_at
    BEFORE INSERT OR UPDATE ON credit_note
    FOR EACH ROW EXECUTE FUNCTION set_store_committed_at();
CREATE INDEX idx_credit_note_store_committed_at ON credit_note (store_committed_at, id);

-- payment_transaction has no created_at/updated_at; settled/refunded times are naive UTC
-- and backdatable, so backfill from them (interpreted as UTC) or "now".
ALTER TABLE payment_transaction ADD COLUMN store_committed_at timestamptz;
UPDATE payment_transaction
SET store_committed_at = COALESCE(GREATEST(processed_at, refunded_at) AT TIME ZONE 'UTC', now());
ALTER TABLE payment_transaction
    ALTER COLUMN store_committed_at SET NOT NULL,
    ALTER COLUMN store_committed_at SET DEFAULT clock_timestamp();
CREATE TRIGGER trg_payment_transaction_store_committed_at
    BEFORE INSERT OR UPDATE ON payment_transaction
    FOR EACH ROW EXECUTE FUNCTION set_store_committed_at();
CREATE INDEX idx_payment_transaction_store_committed_at ON payment_transaction (store_committed_at, id);

ALTER TABLE customer ADD COLUMN store_committed_at timestamptz;
UPDATE customer SET store_committed_at = COALESCE(updated_at, created_at);
ALTER TABLE customer
    ALTER COLUMN store_committed_at SET NOT NULL,
    ALTER COLUMN store_committed_at SET DEFAULT clock_timestamp();
CREATE TRIGGER trg_customer_store_committed_at
    BEFORE INSERT OR UPDATE ON customer
    FOR EACH ROW EXECUTE FUNCTION set_store_committed_at();
CREATE INDEX idx_customer_store_committed_at ON customer (store_committed_at, id);

ALTER TABLE plan ADD COLUMN store_committed_at timestamptz;
UPDATE plan SET store_committed_at = COALESCE(updated_at, created_at);
ALTER TABLE plan
    ALTER COLUMN store_committed_at SET NOT NULL,
    ALTER COLUMN store_committed_at SET DEFAULT clock_timestamp();
CREATE TRIGGER trg_plan_store_committed_at
    BEFORE INSERT OR UPDATE ON plan
    FOR EACH ROW EXECUTE FUNCTION set_store_committed_at();
CREATE INDEX idx_plan_store_committed_at ON plan (store_committed_at, id);

ALTER TABLE product ADD COLUMN store_committed_at timestamptz;
UPDATE product SET store_committed_at = COALESCE(updated_at, created_at);
ALTER TABLE product
    ALTER COLUMN store_committed_at SET NOT NULL,
    ALTER COLUMN store_committed_at SET DEFAULT clock_timestamp();
CREATE TRIGGER trg_product_store_committed_at
    BEFORE INSERT OR UPDATE ON product
    FOR EACH ROW EXECUTE FUNCTION set_store_committed_at();
CREATE INDEX idx_product_store_committed_at ON product (store_committed_at, id);
