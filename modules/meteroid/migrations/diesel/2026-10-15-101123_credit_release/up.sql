-- The credit release's changes to shipped tables after the ledger carry-over, in one transaction. A
-- lock not granted within 5s aborts it unchanged rather than queue billing traffic: retry the deploy.
-- The tables it indexes are small (largest: invoice, ~86k rows in production on 2026-10-15), so no
-- CONCURRENTLY.
SET LOCAL lock_timeout = '5s';

-- A usage or capacity price in credits: `pricing` amounts are credits and `credit_unit_rate` is the
-- invoice-currency major units one credit is worth when it is billed as overage.
ALTER TABLE price
    ADD COLUMN credit_unit_id   UUID,
    ADD COLUMN credit_unit_rate NUMERIC,
    ADD CONSTRAINT price_credit_unit_rate_positive_check CHECK (credit_unit_rate > 0),
    ADD CONSTRAINT price_credit_unit_fk
        FOREIGN KEY (tenant_id, credit_unit_id) REFERENCES credit_unit (tenant_id, id);

ALTER TABLE billable_metric
    ADD COLUMN unit_singular TEXT,
    ADD COLUMN unit_plural   TEXT;

-- One row per seeded sandbox: whether its sample data finished, and why it stopped if not. While
-- it is in progress the lifecycle worker leaves the tenant's subscriptions to the seeder. Tenants
-- seeded before this table have no row and count as completed.
CREATE TYPE "SandboxSeedStatusEnum" AS ENUM ('IN_PROGRESS', 'COMPLETED', 'FAILED');
CREATE TABLE sandbox_seed (
    tenant_id      UUID PRIMARY KEY REFERENCES tenant (id) ON DELETE CASCADE,
    status         "SandboxSeedStatusEnum" NOT NULL,
    failure_reason TEXT,
    started_at     TIMESTAMP NOT NULL DEFAULT NOW(),
    finished_at    TIMESTAMP
);

-- Promotional credit taken off an invoice's charges before tax, net like `discount`.
ALTER TABLE invoice ADD COLUMN promotional_credits BIGINT NOT NULL DEFAULT 0;

-- The note an invoice void issues: the void reverses the invoice's revenue itself, so revenue
-- reporting leaves this note out instead of reversing it twice.
ALTER TABLE credit_note ADD COLUMN voids_invoice BOOLEAN NOT NULL DEFAULT false;
UPDATE credit_note cn
SET voids_invoice = true
FROM invoice i
WHERE i.id = cn.invoice_id
  AND i.tenant_id = cn.tenant_id
  AND i.voided_at IS NOT NULL
  AND cn.reason = 'Invoice voided'
  AND cn.credit_type = 'CREDIT_TO_BALANCE';

-- Voiding a credit note did not record when; the best available moment is its last update.
UPDATE credit_note
SET voided_at = COALESCE(updated_at, finalized_at, created_at)
WHERE status = 'VOIDED' AND voided_at IS NULL;

-- The analytics projector reads revenue sources in transaction order: each row carries the id of
-- the transaction that last wrote it, read only once older than every transaction still open.
-- Nullable, without default: no table rewrite. Rows written before stay NULL, read as 0.
ALTER TABLE invoice ADD COLUMN store_xid xid8;
ALTER TABLE credit_note ADD COLUMN store_xid xid8;

CREATE FUNCTION set_store_xid() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    NEW.store_xid := pg_current_xact_id();
    RETURN NEW;
END $$;

CREATE TRIGGER trg_invoice_store_xid
    BEFORE INSERT OR UPDATE ON invoice
    FOR EACH ROW EXECUTE FUNCTION set_store_xid();
CREATE TRIGGER trg_credit_note_store_xid
    BEFORE INSERT OR UPDATE ON credit_note
    FOR EACH ROW EXECUTE FUNCTION set_store_xid();

-- A store_committed_at stamp is taken once its transaction has an id, so a row stamped before a
-- moment belongs to a transaction the snapshots taken after it count as running or finished.
CREATE OR REPLACE FUNCTION set_store_committed_at() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_current_xact_id();
    NEW.store_committed_at := clock_timestamp();
    RETURN NEW;
END $$;

-- The analytics projector's keysets: transaction order, rows from before store_xid as 0.
CREATE INDEX idx_invoice_store_xid ON invoice ((COALESCE(store_xid, '0'::xid8)), id);
CREATE INDEX idx_credit_note_store_xid ON credit_note ((COALESCE(store_xid, '0'::xid8)), id);

-- The sweep settling plan switches and immediate cancellations whose period invoices were issued
-- without it checks each one with a doubling backoff; an issued period invoice makes it due again.
-- Nullable: rows written by earlier releases are due at once.
ALTER TABLE subscription_event
    ADD COLUMN outgoing_checked_at TIMESTAMP,
    ADD COLUMN outgoing_next_check_at TIMESTAMP;
-- Serve `SubscriptionEventRow::list_incomplete_settling_endings`, keyset ordered on
-- (applies_to, subscription_id). Partial: each predicate must match the query's.
CREATE INDEX idx_subscription_event_incomplete_switch
    ON subscription_event (applies_to, subscription_id)
    WHERE event_type = 'SWITCH'
      AND details ->> 'outgoing' = 'settled'
      AND NOT (details ? 'outgoing_complete');
CREATE INDEX idx_subscription_event_incomplete_cancellation
    ON subscription_event (applies_to, subscription_id)
    WHERE event_type = 'CANCELLED'
      AND details ->> 'outgoing' = 'settled'
      AND NOT (details ? 'outgoing_complete');

-- When a failed scheduled event is tried again. `scheduled_time` stays the date the event takes
-- effect on; NULL means it is due at `scheduled_time`.
ALTER TABLE scheduled_event ADD COLUMN next_attempt_at TIMESTAMP;

-- A subscription's usage filter: it bills only its customer's events whose property has the
-- filter's value; without one it bills the events no filtered sibling claims. The columns hold the
-- current filter; `subscription_usage_filter` keeps every version with the instant it applies from
-- (a version with no property bills without a filter from `effective_from`).
ALTER TABLE subscription
    ADD COLUMN usage_filter_property TEXT,
    ADD COLUMN usage_filter_value TEXT,
    ADD COLUMN usage_filter_since TIMESTAMP;

CREATE TABLE subscription_usage_filter (
    id              UUID PRIMARY KEY,
    tenant_id       UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    subscription_id UUID      NOT NULL REFERENCES subscription (id) ON DELETE CASCADE,
    property        TEXT,
    value           TEXT,
    effective_from  TIMESTAMP NOT NULL,
    effective_until TIMESTAMP,
    created_at      TIMESTAMP NOT NULL DEFAULT now()
);
CREATE INDEX idx_subscription_usage_filter_subscription
    ON subscription_usage_filter (subscription_id, effective_from);

-- A payment lost to a dispute keeps its invoice paid: the disputed part of `amount_reversed` is
-- recorded apart, and what the provider took above what the payment still held is kept as excess.
-- `refund_excess`, on a REFUND row without a credit note: the part of it that gave back money
-- collected above the invoice when it was issued (what a later reversal must not count again).
-- Constant defaults: metadata-only, no rewrite.
ALTER TABLE payment_transaction
    ADD COLUMN amount_disputed        BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN amount_reversed_excess BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN refund_excess          BIGINT NOT NULL DEFAULT 0,
    ADD CONSTRAINT payment_transaction_reversal_split
        CHECK (amount_disputed >= 0 AND amount_disputed <= amount_reversed AND amount_reversed_excess >= 0
               AND refund_excess >= 0 AND refund_excess <= amount);

ALTER TABLE invoice ADD COLUMN payment_disputed_at TIMESTAMP;

-- One row per applied reversal or reinstatement of a payment, dated at the provider's event.
CREATE TABLE payment_reversal_movement (
    id                     UUID        NOT NULL PRIMARY KEY,
    tenant_id              UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    payment_transaction_id UUID        NOT NULL REFERENCES payment_transaction (id) ON DELETE CASCADE,
    invoice_id             UUID REFERENCES invoice (id) ON DELETE CASCADE,
    customer_id            UUID        NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    disputed               BOOLEAN     NOT NULL,
    amount                 BIGINT      NOT NULL,
    excess                 BIGINT      NOT NULL,
    reversal_kind          "ReversalKindEnum",
    provider_ref           TEXT,
    occurred_at            TIMESTAMP   NOT NULL,
    created_at             TIMESTAMP   NOT NULL DEFAULT now(),
    store_xid              XID8        NOT NULL DEFAULT pg_current_xact_id()
);
CREATE INDEX payment_reversal_movement_payment_idx
    ON payment_reversal_movement (tenant_id, payment_transaction_id);
CREATE INDEX payment_reversal_movement_xid_idx ON payment_reversal_movement (store_xid, id);
CREATE INDEX payment_reversal_movement_invoice_idx
    ON payment_reversal_movement (tenant_id, invoice_id) WHERE invoice_id IS NOT NULL;
-- A provider reference (dispute, chargeback or event id) applies once per direction and nature,
-- so a redelivered webhook never applies twice.
CREATE UNIQUE INDEX payment_reversal_movement_provider_ref
    ON payment_reversal_movement (payment_transaction_id, provider_ref, disputed, (amount + excess > 0))
    WHERE provider_ref IS NOT NULL;

-- An invoice a sandbox seed replayed from past billing: never collected nor sent to the customer.
ALTER TABLE invoice ADD COLUMN delivery_held_at TIMESTAMPTZ;

-- The coupons a checkout session held before a payment attempt changed them: restored when the
-- attempt fails, so a code typed for that attempt does not stay on the session.
-- Nullable and unread by older pods: safe under mixed versions.
ALTER TABLE checkout_session
    ADD COLUMN pre_payment_coupon_code VARCHAR,
    ADD COLUMN pre_payment_coupon_ids UUID[];

-- Set when the customer saves a method to pay with right away; its first settled payment clears it.
ALTER TABLE customer_payment_method ADD COLUMN saved_for_payment_at TIMESTAMPTZ;

-- One promotional code tried by a customer while the shared rate limiter (Redis) is unavailable;
-- a code that proves usable deletes its row. Rows outside the window are pruned on the next try.
CREATE TABLE promo_code_attempt (
    id           UUID PRIMARY KEY,
    tenant_id    UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    customer_id  UUID        NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    attempted_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX promo_code_attempt_customer_idx
    ON promo_code_attempt (tenant_id, customer_id, attempted_at);

-- The live subscriptions the credit recompute pages in (tenant, customer) order. Partial: the
-- predicate must match the query's statuses.
CREATE INDEX subscription_live_customer_idx
    ON subscription (tenant_id, customer_id)
    WHERE status IN ('ACTIVE', 'TRIAL_ACTIVE', 'PENDING_CHARGE');

-- The tenants with a metered entitlement under a limit, which the credit recompute reads every
-- minute and on each sweep page. Partial: the predicate must match the query's.
CREATE INDEX entitlement_metered_limit_tenant_idx
    ON entitlement (tenant_id, feature_id)
    WHERE value ->> 'type' = 'metered' AND value ? 'limit';

-- Limit terms that act on nothing are refused on write since this release; rows saved before would
-- block copies (plan versions, quote conversions). Without a limit: Block, no grace, no warning. A
-- sliding window sends no alerts: no warning. After the index, whose SHARE lock keeps writers out.
UPDATE entitlement
SET value = (value - 'warning_threshold_pct') || '{"overage_behavior": {"type": "block"}}'::jsonb
WHERE value ->> 'type' = 'metered'
  AND (value -> 'limit' IS NULL OR value -> 'limit' = 'null'::jsonb)
  AND (value -> 'overage_behavior' IS DISTINCT FROM '{"type": "block"}'::jsonb
    OR value ? 'warning_threshold_pct');

UPDATE entitlement
SET value = value - 'warning_threshold_pct'
WHERE value ->> 'type' = 'metered'
  AND value -> 'reset_period' ->> 'type' = 'sliding_window'
  AND value ? 'warning_threshold_pct';
