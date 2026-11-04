-- Invoice-time customer credit ledger: per-currency or per-credit-unit wallets, grants drawn by
-- priority and expiry, an append-only entry per movement, top-ups, usage limits and budgets. The
-- shipped scalar balance is carried into the ledger; any mismatch aborts. The shipped balance tables
-- stay as they were but frozen, so instances still on the previous release can no longer move a
-- balance. A lock not granted within 5s aborts the migration unchanged: retry the deploy.
SET LOCAL lock_timeout = '5s';

-- 1. Enum values. The new values are not usable before commit, so later checks compare as text.
ALTER TYPE "PaymentTypeEnum" ADD VALUE IF NOT EXISTS 'TOP_UP';
ALTER TYPE "PaymentTypeEnum" ADD VALUE IF NOT EXISTS 'AUTO_TOPUP';
ALTER TYPE "FeeTypeEnum" ADD VALUE IF NOT EXISTS 'CREDIT';
ALTER TYPE "CheckoutTypeEnum" ADD VALUE IF NOT EXISTS 'TOP_UP';
CREATE TYPE "UsageLimitBehaviorEnum" AS ENUM ('TRACK', 'SOFT', 'HARD');
CREATE TYPE "BudgetAuthorEnum" AS ENUM ('TENANT', 'CUSTOMER');
CREATE TYPE "BudgetBehaviorEnum" AS ENUM ('SOFT', 'HARD');

-- 2. Old writers must not move a balance while it is carried over. Main's credit purchases
-- (`customer_balance_pending_tx`) were never used in production: the release carries none.
LOCK TABLE customer, customer_balance_tx, customer_balance_pending_tx IN SHARE ROW EXCLUSIVE MODE;

DO
$$
    BEGIN
        IF EXISTS (SELECT 1 FROM customer WHERE balance_value_cents < 0) THEN
            RAISE EXCEPTION 'negative customer balances exist; reconcile before upgrading';
        END IF;
        IF EXISTS (SELECT 1 FROM customer_balance_pending_tx) THEN
            RAISE EXCEPTION 'credit purchases exist; this release carries none over, reconcile before upgrading';
        END IF;
    END
$$;

-- 3. A credit unit is a denomination of its own: wallets, prices and budgets name it by `code`, in
-- minor units of 10^-precision. `product_id`: the product unit sales are booked under.
CREATE TABLE credit_unit (
    id          UUID PRIMARY KEY,
    tenant_id   UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    code        TEXT        NOT NULL,
    name        TEXT        NOT NULL,
    plural_name TEXT        NOT NULL,
    symbol      TEXT,
    precision   SMALLINT    NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    archived_at TIMESTAMPTZ,
    product_id  UUID REFERENCES product (id),
    UNIQUE (tenant_id, code),
    UNIQUE (tenant_id, id),
    UNIQUE (tenant_id, id, code)
);

-- A credit unit sold for money: one active price per currency, in major units of that currency per
-- credit. A price is never edited: a new one replaces it, so a top-up that pinned it stays exact.
CREATE TABLE credit_unit_price (
    id             UUID PRIMARY KEY,
    tenant_id      UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    credit_unit_id UUID        NOT NULL,
    currency       TEXT        NOT NULL,
    unit_price     NUMERIC     NOT NULL CHECK (unit_price > 0),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    archived_at    TIMESTAMPTZ,
    CONSTRAINT credit_unit_price_unit_fk
        FOREIGN KEY (tenant_id, credit_unit_id) REFERENCES credit_unit (tenant_id, id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX credit_unit_price_active_currency_idx
    ON credit_unit_price (credit_unit_id, currency) WHERE archived_at IS NULL;

-- 4. One wallet per customer and denomination: an ISO currency, or a credit unit (the foreign key
-- pins `currency` to the unit code). Amounts are minor units of that denomination.
-- `unit_price`: the price this customer agreed for a credit of the wallet's unit, in major units of
-- `unit_price_currency`; purchases and automatic top-ups in that currency default to it.
-- `ledger_version`: bumped on every wallet of the customer with each ledger movement, in its
-- transaction; a live projection read at another version predates a movement.
CREATE TABLE customer_credit_account (
    id                           UUID PRIMARY KEY,
    tenant_id                    UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    customer_id                  UUID        NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    currency                     TEXT        NOT NULL CHECK (currency ~ '^([A-Z]{3}|[A-Z][A-Z0-9_]{3,15})$'),
    created_at                   TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- A prepaid wallet refuses the usage it would pay for once it is empty.
    block_when_exhausted         BOOLEAN     NOT NULL DEFAULT FALSE,
    -- Available balances below which `wallet.balance_low` fires once, re-armed when it recovers.
    low_balance_thresholds_minor BIGINT[]    NOT NULL DEFAULT '{}',
    credit_unit_id               UUID,
    unit_price                   NUMERIC CHECK (unit_price > 0),
    unit_price_currency          TEXT,
    ledger_version               BIGINT      NOT NULL DEFAULT 0,
    UNIQUE (tenant_id, customer_id, currency),
    UNIQUE (tenant_id, id),
    CONSTRAINT customer_credit_account_credit_unit_fk
        FOREIGN KEY (tenant_id, credit_unit_id, currency) REFERENCES credit_unit (tenant_id, id, code)
);
CREATE INDEX customer_credit_account_customer_idx ON customer_credit_account (customer_id);
CREATE INDEX customer_credit_account_credit_unit_idx ON customer_credit_account (credit_unit_id)
    WHERE credit_unit_id IS NOT NULL;

-- `remaining` is always the sum of the grant's entries; the bound below is the last guard against
-- crediting a grant beyond what it holds.
CREATE TABLE customer_credit_grant (
    id              UUID PRIMARY KEY,
    tenant_id       UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    customer_id     UUID        NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    account_id      UUID        NOT NULL,
    amount          BIGINT      NOT NULL CHECK (amount > 0),
    remaining       BIGINT      NOT NULL CHECK (remaining >= 0 AND remaining <= amount),
    priority        INTEGER     NOT NULL,
    classification  TEXT        NOT NULL CHECK (classification IN ('PAID', 'PROMOTIONAL')),
    effective_at    TIMESTAMPTZ NOT NULL,
    expires_at      TIMESTAMPTZ,
    source_kind     TEXT        NOT NULL,
    source_id       UUID        NOT NULL,
    -- The invoice (and line) whose payment bought this credit, or whose credit note returned it.
    invoice_id      UUID,
    line_local_id   TEXT,
    credit_note_id  UUID,
    reason          TEXT        NOT NULL,
    -- Operator reference of a manual adjustment, e.g. the invoice it compensates.
    reference       TEXT,
    created_by      UUID,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- The subscription whose charges alone it pays; NULL pays any of the customer's. No cascade:
    -- a subscription is never deleted alone, and a customer delete removes the grants first.
    subscription_id UUID REFERENCES subscription (id),
    -- USAGE pays usage charges only; INVOICE any charge.
    application     TEXT        NOT NULL CHECK (application IN ('INVOICE', 'USAGE')),
    -- When its subscription ended or its plan changed; it lapses then if it has not expired before.
    ends_at         TIMESTAMPTZ,
    end_reason      TEXT CHECK (end_reason IN ('SUBSCRIPTION_ENDED', 'PLAN_CHANGE')),
    lapses_at       TIMESTAMPTZ GENERATED ALWAYS AS (LEAST(expires_at, ends_at)) STORED,
    -- What the grant cost in total, net of tax, so "10 for 3 credits" stays exact.
    cost_minor      BIGINT CHECK (cost_minor >= 0),
    cost_currency   TEXT CHECK (cost_currency ~ '^[A-Z]{3}$'),
    -- Copied from the included credit pack at mint; a rolled grant never rolls again.
    rollover        JSONB,
    -- The start of the usage it may pay when it is not effective_at: a rollover's parent's, an
    -- automatic top-up's trigger, a plan pack's period start. Usage ending by then it never pays.
    pays_usage_from TIMESTAMPTZ,
    UNIQUE (tenant_id, source_kind, source_id),
    FOREIGN KEY (tenant_id, account_id) REFERENCES customer_credit_account (tenant_id, id) ON DELETE CASCADE
);
CREATE INDEX customer_credit_grant_available_idx ON customer_credit_grant (account_id) WHERE remaining > 0;
CREATE INDEX customer_credit_grant_customer_idx ON customer_credit_grant (tenant_id, customer_id);
CREATE INDEX customer_credit_grant_subscription_idx ON customer_credit_grant (tenant_id, subscription_id)
    WHERE subscription_id IS NOT NULL;
CREATE INDEX customer_credit_grant_lapse_idx ON customer_credit_grant (lapses_at)
    WHERE remaining > 0 AND lapses_at IS NOT NULL;
CREATE INDEX customer_credit_grant_invoice_idx ON customer_credit_grant (tenant_id, invoice_id)
    WHERE invoice_id IS NOT NULL;

-- The products a grant pays for, written with the grant; no row means every product.
CREATE TABLE customer_credit_grant_product (
    tenant_id  UUID NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    grant_id   UUID NOT NULL REFERENCES customer_credit_grant (id) ON DELETE CASCADE,
    product_id UUID NOT NULL REFERENCES product (id) ON DELETE RESTRICT,
    PRIMARY KEY (grant_id, product_id)
);

-- FUND, DRAW_REVERSAL and REINSTATEMENT (a reversed payment recovered) add to a grant; DRAW,
-- REVERSAL (credit note, operator removal or payment reversal), EXPIRY and ROLLOVER take from it.
-- `source_id` is the document or request that moved the value; a DRAW or DRAW_REVERSAL also names
-- what it settles in `source_kind`: an invoice, the usage settled ahead of one, or a consumed request.
CREATE TABLE customer_credit_entry (
    id                  UUID PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    customer_id         UUID        NOT NULL,
    grant_id            UUID        NOT NULL REFERENCES customer_credit_grant (id) ON DELETE CASCADE,
    kind                TEXT        NOT NULL
        CHECK (kind IN ('FUND', 'DRAW', 'DRAW_REVERSAL', 'REVERSAL', 'REINSTATEMENT', 'EXPIRY', 'ROLLOVER')),
    amount              BIGINT      NOT NULL CHECK (amount <> 0 AND (amount > 0) = (kind IN ('FUND', 'DRAW_REVERSAL', 'REINSTATEMENT'))),
    source_id           UUID        NOT NULL,
    source_kind         TEXT CHECK (source_kind IN ('INVOICE', 'USAGE_SETTLEMENT', 'CONSUME')),
    invoice_id          UUID,
    credit_note_id      UUID,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    -- Business time of the movement (payment, invoice issue, expiry), which statements order by;
    -- `created_at` is when it was recorded. The application always sets it.
    occurred_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    -- Trigger-stamped store-commit marker; never business time.
    store_committed_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    -- The transaction that wrote it: the analytics projector reads the ledger in that order.
    store_xid           xid8        NOT NULL DEFAULT pg_current_xact_id(),
    -- Operator reason and author of a removal; document-driven movements carry neither.
    reason              TEXT,
    created_by          UUID,
    -- The money value the movement carried, same sign as `amount` (a rounding step can be 0).
    cost_minor          BIGINT,
    -- The payment reversal or reinstatement a payment-driven movement follows, so dispute
    -- analytics leave out what the ledger books itself.
    payment_movement_id UUID,
    UNIQUE (grant_id, kind, source_id),
    CONSTRAINT customer_credit_entry_cost_sign_check
        CHECK (cost_minor IS NULL OR cost_minor = 0 OR (cost_minor > 0) = (amount > 0))
);
CREATE INDEX customer_credit_entry_invoice_idx ON customer_credit_entry (tenant_id, invoice_id)
    WHERE invoice_id IS NOT NULL;
CREATE INDEX customer_credit_entry_note_idx ON customer_credit_entry (tenant_id, credit_note_id)
    WHERE credit_note_id IS NOT NULL;
CREATE INDEX customer_credit_entry_customer_idx ON customer_credit_entry (tenant_id, customer_id, occurred_at, id);
CREATE TRIGGER trg_customer_credit_entry_store_committed_at
    BEFORE INSERT ON customer_credit_entry
    FOR EACH ROW EXECUTE FUNCTION set_store_committed_at();
CREATE INDEX customer_credit_entry_store_xid_idx ON customer_credit_entry (store_xid, id);
-- Settled usage an invoice took over, re-projected with it.
CREATE INDEX customer_credit_entry_settlement_idx ON customer_credit_entry (source_id)
    WHERE source_kind = 'USAGE_SETTLEMENT';
-- The reversal sweep pages through these alone, not through every draw an invoice took.
CREATE INDEX customer_credit_entry_reversal_idx ON customer_credit_entry (tenant_id, invoice_id)
    WHERE kind IN ('REVERSAL', 'REINSTATEMENT') AND invoice_id IS NOT NULL;
-- Covers last_recorded_at_by_grant's GROUP BY grant_id / MAX(created_at), index-only.
CREATE INDEX customer_credit_entry_grant_recency_idx
    ON customer_credit_entry (tenant_id, customer_id, grant_id, created_at);
CREATE INDEX customer_credit_entry_payment_movement_idx
    ON customer_credit_entry (payment_movement_id) WHERE payment_movement_id IS NOT NULL;
CREATE INDEX customer_credit_entry_payment_movement_xid_idx
    ON customer_credit_entry (store_xid, id) WHERE payment_movement_id IS NOT NULL;

-- What each grant paid of each invoice line, as the allocator placed it: when the invoice drew its
-- paid credit (no credit note), and when a credit note gave paid credit back. A null line is what
-- no line names (the rounding between the total and its lines). Revenue facts follow it.
CREATE TABLE customer_credit_line_allocation (
    id             UUID PRIMARY KEY,
    tenant_id      UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    invoice_id     UUID        NOT NULL,
    credit_note_id UUID,
    grant_id       UUID        NOT NULL REFERENCES customer_credit_grant (id) ON DELETE CASCADE,
    line_local_id  TEXT,
    amount         BIGINT      NOT NULL CHECK (amount > 0),
    created_at     TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    UNIQUE NULLS NOT DISTINCT (invoice_id, credit_note_id, grant_id, line_local_id)
);
CREATE INDEX customer_credit_line_allocation_invoice_idx
    ON customer_credit_line_allocation (tenant_id, invoice_id);

-- Source rows an analytics revenue stream left out for breaking an invariant: retried every
-- projector tick until they project, and open (holding the revenue facts' cutover) until then.
CREATE TABLE analytics_projection_quarantine (
    stream        TEXT        NOT NULL,
    source_id     UUID        NOT NULL,
    tenant_id     UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    reason        TEXT        NOT NULL,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_tried_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at   TIMESTAMPTZ,
    -- How far a retry too large for one batch got: the last settled draw it gave back.
    resume_after  UUID,
    PRIMARY KEY (stream, source_id)
);
CREATE INDEX analytics_projection_quarantine_open_idx
    ON analytics_projection_quarantine (stream, last_tried_at) WHERE resolved_at IS NULL;
CREATE INDEX analytics_projection_quarantine_tenant_idx ON analytics_projection_quarantine (tenant_id);

-- 5. A top-up purchase: the payment side of funding a wallet, keyed by the caller's idempotency key
-- within its customer. `settled_at`: when it settled, which `updated_at` does not keep.
CREATE TABLE customer_credit_operation (
    id                           UUID PRIMARY KEY,
    tenant_id                    UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    customer_id                  UUID        NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    account_id                   UUID        NOT NULL,
    state                        TEXT        NOT NULL DEFAULT 'CREATED'
        CHECK (state IN ('CREATED', 'PROCESSING', 'SETTLED', 'FAILED', 'CANCELLED')),
    namespace                    TEXT        NOT NULL,
    idempotency_key              TEXT        NOT NULL,
    request_fingerprint          BYTEA       NOT NULL,
    -- Credit the operation funds, in minor units of the wallet's denomination.
    amount_minor                 BIGINT      NOT NULL CHECK (amount_minor > 0),
    expected_consideration_minor BIGINT CHECK (expected_consideration_minor >= 0),
    consideration_currency       TEXT,
    payment_transaction_id       UUID REFERENCES payment_transaction (id) ON DELETE SET NULL,
    invoice_id                   UUID REFERENCES invoice (id) ON DELETE SET NULL,
    reason                       TEXT        NOT NULL,
    metadata                     JSONB       NOT NULL DEFAULT '{}'::jsonb,
    created_at                   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at                   TIMESTAMPTZ NOT NULL DEFAULT now(),
    settled_at                   TIMESTAMPTZ,
    UNIQUE (tenant_id, customer_id, namespace, idempotency_key),
    FOREIGN KEY (tenant_id, account_id) REFERENCES customer_credit_account (tenant_id, id)
);
-- Single-flight automatic top-up per wallet: the race backstop that prevents a double charge.
CREATE UNIQUE INDEX customer_credit_operation_automatic_inflight_uq ON customer_credit_operation (account_id)
    WHERE namespace = 'AUTOMATIC_TOP_UP' AND state IN ('CREATED', 'PROCESSING');
-- Stuck-attempt polling of every namespace, oldest first.
CREATE INDEX customer_credit_operation_processing_idx ON customer_credit_operation (updated_at)
    WHERE state = 'PROCESSING';
CREATE INDEX customer_credit_operation_account_idx ON customer_credit_operation (account_id);
CREATE INDEX customer_credit_operation_customer_idx ON customer_credit_operation (customer_id);
CREATE INDEX customer_credit_operation_payment_transaction_idx ON customer_credit_operation (payment_transaction_id)
    WHERE payment_transaction_id IS NOT NULL;
CREATE INDEX customer_credit_operation_invoice_idx ON customer_credit_operation (invoice_id)
    WHERE invoice_id IS NOT NULL;
-- Late-capture lookup of a re-armed top-up by `metadata @> {"superseded_payment_transaction_ids": [..]}`.
CREATE INDEX customer_credit_operation_metadata_idx ON customer_credit_operation USING gin (metadata jsonb_path_ops);

-- 6. A wallet's automatic top-up: the rule, who authorized it, and the executor's state. A rule the
-- customer turned off is kept with `parked_at` so turning it back on restores its guardrails.
-- `agreed_*`: the price, and its currency, a credit unit wallet's top-up was authorized at; a new
-- price or currency needs consent.
CREATE TABLE customer_credit_auto_top_up (
    account_id                UUID PRIMARY KEY,
    tenant_id                 UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    method                    TEXT        NOT NULL CHECK (method IN ('FIXED', 'TO_TARGET')),
    threshold_minor           BIGINT      NOT NULL CHECK (threshold_minor >= 0),
    amount_minor              BIGINT CHECK (amount_minor > 0),
    target_minor              BIGINT CHECK (target_minor > 0),
    minimum_minor             BIGINT CHECK (minimum_minor > 0),
    maximum_minor             BIGINT CHECK (maximum_minor > 0),
    daily_cap_minor           BIGINT CHECK (daily_cap_minor > 0),
    monthly_cap_minor         BIGINT CHECK (monthly_cap_minor > 0),
    -- NULL charges the customer's default payment method.
    payment_method_id         UUID REFERENCES customer_payment_method (id),
    parked_at                 TIMESTAMPTZ,
    consecutive_failures      INTEGER     NOT NULL DEFAULT 0,
    last_attempted_at         TIMESTAMPTZ,
    last_succeeded_at         TIMESTAMPTZ,
    suspended_at              TIMESTAMPTZ,
    last_error                TEXT,
    -- When a deferred top-up (backoff, cooldown, daily cap) is due to be checked again.
    retry_due_at              TIMESTAMPTZ,
    authorization_revision    TEXT,
    authorization_actor       TEXT,
    authorization_recorded_at TIMESTAMPTZ,
    created_at                TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at                TIMESTAMPTZ NOT NULL DEFAULT now(),
    agreed_price_id           UUID REFERENCES credit_unit_price (id),
    agreed_unit_price         NUMERIC CHECK (agreed_unit_price > 0),
    agreed_currency           TEXT,
    -- The daily cap and the maximum per top-up came from the customer's own self-service terms, not
    -- the operator: the customer's next edit recomputes them instead of keeping them.
    daily_cap_is_default      BOOLEAN     NOT NULL DEFAULT FALSE,
    maximum_is_default        BOOLEAN     NOT NULL DEFAULT FALSE,
    -- An operator's statement that the customer agreed to automatic charges on terms the customer did
    -- not confirm in their portal: who made it (`user:<id>`, `api_token:<id>`, `oauth_app:<id>`),
    -- when, and the revision of the terms it covers.
    consent_attested_by       TEXT,
    consent_attested_at       TIMESTAMPTZ,
    consent_attested_revision TEXT,
    FOREIGN KEY (tenant_id, account_id) REFERENCES customer_credit_account (tenant_id, id) ON DELETE CASCADE
);
CREATE INDEX customer_credit_auto_top_up_tenant_idx ON customer_credit_auto_top_up (tenant_id);
CREATE INDEX customer_credit_auto_top_up_retry_due_idx ON customer_credit_auto_top_up (retry_due_at)
    WHERE retry_due_at IS NOT NULL;
CREATE INDEX customer_credit_auto_top_up_payment_method_idx ON customer_credit_auto_top_up (payment_method_id)
    WHERE payment_method_id IS NOT NULL;

-- 7. The Idempotency-Key of a public API write and the request it first named, so a retry replays
-- the write and a reused key with a different request is refused.
CREATE TABLE api_idempotency_key (
    tenant_id           UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    operation           TEXT        NOT NULL,
    idempotency_key     TEXT        NOT NULL,
    request_fingerprint BYTEA       NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, operation, idempotency_key)
);

-- 8. A credit purchase, sold on a one-off invoice. A purchase sold through the API has no acting
-- user. `amount_cents` is money (the price before tax); for a credit unit, `amount_credits` is what
-- it sold in the unit's minor units, and `unit_price_source` where its price came from.
CREATE TABLE customer_credit_purchase (
    id                    UUID PRIMARY KEY,
    created_at            TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at            TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    amount_cents          BIGINT       NOT NULL CHECK (amount_cents > 0),
    note                  TEXT,
    expires_at            TIMESTAMPTZ,
    invoice_id            UUID         NOT NULL REFERENCES invoice (id) ON UPDATE CASCADE ON DELETE CASCADE,
    tenant_id             UUID         NOT NULL REFERENCES tenant (id) ON UPDATE CASCADE ON DELETE CASCADE,
    customer_id           UUID         NOT NULL REFERENCES customer (id) ON UPDATE CASCADE ON DELETE CASCADE,
    created_by            UUID,
    amount_credits        BIGINT CHECK (amount_credits > 0),
    negotiated_unit_price NUMERIC CHECK (negotiated_unit_price > 0),
    unit_price_source     TEXT CHECK (unit_price_source IN ('LIST', 'CUSTOMER', 'OVERRIDE'))
);
CREATE UNIQUE INDEX customer_credit_purchase_invoice_id ON customer_credit_purchase (invoice_id);
CREATE INDEX customer_credit_purchase_customer_idx ON customer_credit_purchase (tenant_id, customer_id);

-- Purchased credit already spent when its money came back, as reported so far per wallet: for an
-- invoice's payments (`source_id` = the invoice), one refund note (`source_id` = the note), or a
-- payment no invoice took (`source_id` = its flow key, `invoice_id` NULL).
-- Each notice moves `reported` to the current loss; `notices` numbers them.
CREATE TABLE customer_credit_unrecovered (
    tenant_id  UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    account_id UUID        NOT NULL,
    invoice_id UUID,
    payment_id UUID,
    source_id  UUID        NOT NULL,
    reported   BIGINT      NOT NULL CHECK (reported >= 0),
    notices    INTEGER     NOT NULL CHECK (notices > 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (invoice_id IS NOT NULL OR payment_id IS NOT NULL),
    PRIMARY KEY (tenant_id, account_id, source_id),
    FOREIGN KEY (tenant_id, account_id) REFERENCES customer_credit_account (tenant_id, id) ON DELETE CASCADE
);

-- 9. Live usage credit. The projection is derived and rebuilt from the ledger and ClickHouse, so it
-- is unlogged: a crash empties it and the recompute refills it without touching any money.
-- `metric_ids`: the metrics the wallet's denomination prices usage for. `per_subscription`:
-- [{subscription_id, pending_minor, available_minor, uncovered_minor}]. `charged`: the denominations
-- each metric is charged in, with the plan inputs they were derived from. `pending_by_product`:
-- [{product_id, owed_minor, uncovered_minor}], empty for a credit unit. `ledger_version`: the
-- wallet's version read before the ledger, so a later movement shows as a different version.
CREATE UNLOGGED TABLE credit_balance_projection (
    account_id           UUID PRIMARY KEY,
    tenant_id            UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    customer_id          UUID        NOT NULL,
    computed_at          TIMESTAMPTZ NOT NULL,
    usage_as_of          TIMESTAMPTZ NOT NULL,
    pending_minor        BIGINT      NOT NULL CHECK (pending_minor >= 0),
    live_available_minor BIGINT      NOT NULL,
    overage_minor        BIGINT      NOT NULL CHECK (overage_minor >= 0),
    per_grant            JSONB       NOT NULL DEFAULT '[]'::jsonb,
    metric_ids           UUID[]      NOT NULL DEFAULT '{}',
    per_subscription     JSONB       NOT NULL DEFAULT '[]',
    charged              JSONB,
    pending_by_product   JSONB       NOT NULL DEFAULT '[]',
    ledger_version       BIGINT      NOT NULL DEFAULT 0,
    FOREIGN KEY (tenant_id, account_id) REFERENCES customer_credit_account (tenant_id, id) ON DELETE CASCADE
);
CREATE INDEX credit_balance_projection_customer_idx ON credit_balance_projection (tenant_id, customer_id);

-- The exact deduplicated quantity of a metric over one window up to `fold_at`, so a live projection
-- only reads the events after it. Keyed by the grouping it was folded under (`grouping`: the sorted,
-- comma-joined dimension names, '' = ungrouped) and by the instant its window starts, so the billing
-- matrix, a usage limit grouped by another property and an hour window never sum into one another.
-- `group_key` is the canonical form of the grouping values.
CREATE TABLE usage_fold (
    customer_id        UUID        NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    billable_metric_id UUID        NOT NULL REFERENCES billable_metric (id) ON DELETE CASCADE,
    group_key          TEXT        NOT NULL DEFAULT '',
    tenant_id          UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    fold_at            TIMESTAMPTZ NOT NULL,
    quantity           NUMERIC     NOT NULL,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    metric_version     BIGINT      NOT NULL,
    rebuilt_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    grouping           TEXT        NOT NULL DEFAULT '',
    window_start       TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (customer_id, billable_metric_id, window_start, grouping, group_key)
);
CREATE INDEX usage_fold_metric_idx ON usage_fold (billable_metric_id);
CREATE INDEX usage_fold_tenant_idx ON usage_fold (tenant_id);

-- Usage paid from credit ahead of its invoice, at credit expiry or before. `invoice_id` is set when
-- the invoice that finalizes the period takes it over; `returned_at` when it is given back to the
-- grants it drew from, which frees its key for a later settlement of the same period. Usage dated
-- before a settled bound but ingested later is settled by another row at the bound: `ordinal`
-- numbers them. `tax_minor`: the exact tax its paid credit settled, frozen for revenue reporting.
CREATE TABLE credit_usage_settlement (
    id                  UUID PRIMARY KEY,
    tenant_id           UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    customer_id         UUID        NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    account_id          UUID        NOT NULL,
    subscription_id     UUID        NOT NULL REFERENCES subscription (id) ON DELETE CASCADE,
    period_start        DATE        NOT NULL,
    event_time_bound    TIMESTAMPTZ NOT NULL,
    amount              BIGINT      NOT NULL CHECK (amount > 0),
    -- The subscription's tax-inclusive rate when the usage was settled, frozen.
    tax_inclusive_rate  NUMERIC,
    -- The source of the entries giving it back unused, derived from `id`.
    give_back_source_id UUID        NOT NULL,
    invoice_id          UUID REFERENCES invoice (id) ON DELETE SET NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    returned_at         TIMESTAMPTZ,
    tax_minor           BIGINT      NOT NULL,
    ordinal             INTEGER     NOT NULL DEFAULT 0,
    -- Never both invoiced and given back: that would pay the usage twice.
    CONSTRAINT credit_usage_settlement_taken_or_returned_ck CHECK (invoice_id IS NULL OR returned_at IS NULL),
    CONSTRAINT credit_usage_settlement_tax_ck CHECK (tax_minor >= 0 AND tax_minor <= amount),
    FOREIGN KEY (tenant_id, account_id) REFERENCES customer_credit_account (tenant_id, id) ON DELETE CASCADE
);
CREATE INDEX credit_usage_settlement_customer_idx ON credit_usage_settlement (customer_id);
CREATE INDEX credit_usage_settlement_open_idx ON credit_usage_settlement (subscription_id, period_start)
    WHERE invoice_id IS NULL AND returned_at IS NULL;
CREATE INDEX credit_usage_settlement_invoice_idx ON credit_usage_settlement (invoice_id)
    WHERE invoice_id IS NOT NULL;
CREATE UNIQUE INDEX credit_usage_settlement_give_back_uq
    ON credit_usage_settlement (tenant_id, give_back_source_id);
-- `CreditUsageSettlementRow::insert_new`'s ON CONFLICT target; keep its columns and predicate identical.
CREATE UNIQUE INDEX credit_usage_settlement_bound_uq
    ON credit_usage_settlement (account_id, subscription_id, period_start, event_time_bound, ordinal)
    WHERE returned_at IS NULL;

-- A threshold crossing already announced for a monitor (wallet, usage limit or budget) in one
-- window. Inserting it and its outbox event in one transaction delivers each crossing once. A
-- re-armed level keeps its row: `rearmed` marks it as not announced and `generation` counts the
-- re-arms. Logged, unlike `usage_monitor_state`: the announcements of open windows survive a crash,
-- and a window's crossings are pruned once `window_end` has passed.
CREATE TABLE usage_alert_state (
    monitor_id   UUID        NOT NULL,
    window_key   TEXT        NOT NULL,
    level        INTEGER     NOT NULL,
    tenant_id    UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    fired_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    generation   INTEGER     NOT NULL DEFAULT 0,
    rearmed      BOOLEAN     NOT NULL DEFAULT FALSE,
    window_start TIMESTAMPTZ,
    window_end   TIMESTAMPTZ,
    PRIMARY KEY (monitor_id, window_key, level)
);
CREATE INDEX usage_alert_state_tenant_idx ON usage_alert_state (tenant_id);

-- 10. Lifecycle changes (funding, expiry, finalization, subscription changes) that make a customer's
-- live credit projection stale, written with the change. Every replica polls this table: the
-- recompute leader recomputes the customer, the others drop what they cached of it. Rows are transient.
CREATE TABLE credit_recompute_mark (
    tenant_id   UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    customer_id UUID        NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    marked_at   TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, customer_id)
);
-- The poller orders and pages on all three, so a full page of tied marks can be walked past.
CREATE INDEX credit_recompute_mark_marked_idx ON credit_recompute_mark (marked_at, tenant_id, customer_id);

SELECT pgmq.create('credit_recompute');
SELECT pgmq.bind_topic('subscription.created', 'credit_recompute');
SELECT pgmq.bind_topic('subscription.updated', 'credit_recompute');
SELECT pgmq.bind_topic('subscription.cancelled', 'credit_recompute');
SELECT pgmq.bind_topic('subscription.ended', 'credit_recompute');
SELECT pgmq.bind_topic('invoice.finalized', 'credit_recompute');
SELECT pgmq.bind_topic('invoice.voided', 'credit_recompute');
SELECT pgmq.bind_topic('credit_note.finalized', 'credit_recompute');
SELECT pgmq.bind_topic('credit_note.voided', 'credit_recompute');
SELECT pgmq.bind_topic('wallet.funded', 'credit_recompute');
SELECT pgmq.bind_topic('wallet.debited', 'credit_recompute');
SELECT pgmq.bind_topic('credit_grant.expired', 'credit_recompute');

-- Version tokens of a customer's and a tenant's live-credit inputs, and the feed of their changes.
-- A bump writes a new random token, so equal tokens mean identical inputs even after a restore.
CREATE FUNCTION credit_input_token() RETURNS BIGINT
    LANGUAGE sql VOLATILE PARALLEL SAFE
AS $$ SELECT ('x' || substr(md5(gen_random_uuid()::text), 1, 16))::bit(64)::bigint $$;

-- The feed shard of a tenant, 0..15. The only definition: every writer calls it.
CREATE FUNCTION credit_input_shard(tenant_id UUID) RETURNS SMALLINT
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
AS $$ SELECT (get_byte(decode(md5(tenant_id::text), 'hex'), 0) % 16)::smallint $$;

-- Only the primary key is indexed and no bump changes it, so every bump is a HOT update.
CREATE TABLE credit_customer_inputs (
    tenant_id    UUID   NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    customer_id  UUID   NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    inputs_token BIGINT NOT NULL,
    PRIMARY KEY (tenant_id, customer_id)
) WITH (fillfactor = 80);

CREATE TABLE credit_tenant_inputs (
    tenant_id    UUID   NOT NULL PRIMARY KEY REFERENCES tenant (id) ON DELETE CASCADE,
    inputs_token BIGINT NOT NULL
) WITH (fillfactor = 80);

-- TENANT rows have no customer: the whole tenant's inputs changed.
CREATE TYPE "CreditInputChangeKindEnum" AS ENUM ('INPUTS', 'TENANT', 'LEDGER', 'TOP_UP_RUNTIME');

-- created_on is the transaction's UTC start date, so one transaction's rows share a partition.
CREATE TABLE credit_input_change (
    id          BIGINT GENERATED ALWAYS AS IDENTITY,
    store_xid   XID8     NOT NULL DEFAULT pg_current_xact_id(),
    shard       SMALLINT NOT NULL,
    tenant_id   UUID     NOT NULL,
    customer_id UUID,
    kind        "CreditInputChangeKindEnum" NOT NULL,
    created_on  DATE     NOT NULL DEFAULT (now() AT TIME ZONE 'UTC')::date,
    CHECK ((kind = 'TENANT') = (customer_id IS NULL)),
    -- The poll's keyset index; a key of a partitioned table must hold the partition column.
    PRIMARY KEY (shard, store_xid, id, created_on)
) PARTITION BY RANGE (created_on);

-- One row per transaction, tenant, customer and kind: writers insert with ON CONFLICT DO NOTHING.
CREATE UNIQUE INDEX credit_input_change_once_idx
    ON credit_input_change (created_on, store_xid, tenant_id, customer_id, kind) NULLS NOT DISTINCT;

CREATE TABLE credit_input_change_default PARTITION OF credit_input_change DEFAULT;

DO $$
DECLARE
    day DATE;
BEGIN
    FOR day IN
        SELECT d::date
        FROM generate_series((now() AT TIME ZONE 'UTC')::date,
                             (now() AT TIME ZONE 'UTC')::date + 14, interval '1 day') d
    LOOP
        EXECUTE format(
            'CREATE TABLE %I PARTITION OF credit_input_change FOR VALUES FROM (%L) TO (%L)',
            'credit_input_change_p' || to_char(day, 'YYYYMMDD'), day, day + 1);
    END LOOP;
END
$$;

-- The last (store_xid, id) of each dropped day: a feed cursor below it missed rows.
-- Both are NULL when the day held no row.
CREATE TABLE credit_input_change_retention (
    partition_day DATE        NOT NULL PRIMARY KEY,
    max_store_xid XID8,
    max_id        BIGINT,
    dropped_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((max_store_xid IS NULL) = (max_id IS NULL))
);

-- 11. Per-customer usage limits.
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

-- 12. Per-customer spend budgets, one live budget per denomination: a currency or a credit unit.
CREATE TABLE customer_budget (
    id             UUID PRIMARY KEY,
    tenant_id      UUID                 NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    customer_id    UUID                 NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    currency       TEXT                 NOT NULL CHECK (currency ~ '^([A-Z]{3}|[A-Z][A-Z0-9_]{3,15})$'),
    amount_minor   BIGINT               NOT NULL CHECK (amount_minor > 0),
    thresholds     INTEGER[]            NOT NULL DEFAULT '{50,80,100}',
    behavior       "BudgetBehaviorEnum" NOT NULL DEFAULT 'SOFT',
    created_by     "BudgetAuthorEnum"   NOT NULL,
    version        INTEGER              NOT NULL DEFAULT 1,
    archived_at    TIMESTAMPTZ,
    created_at     TIMESTAMPTZ          NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ          NOT NULL DEFAULT now(),
    credit_unit_id UUID,
    CONSTRAINT customer_budget_credit_unit_fk
        FOREIGN KEY (tenant_id, credit_unit_id, currency) REFERENCES credit_unit (tenant_id, id, code)
);
CREATE UNIQUE INDEX customer_budget_live_currency_idx ON customer_budget (customer_id, currency)
    WHERE archived_at IS NULL;
CREATE INDEX customer_budget_tenant_idx ON customer_budget (tenant_id, customer_id);

-- 13. What each monitor (entitlement, usage limit or budget) has consumed in each of its windows,
-- plus one heartbeat row per monitored customer telling the sweep when it was last computed.
-- Derived like the credit projection: unlogged, emptied by a crash and refilled by the recompute.
-- Only the key and immutable columns are indexed and the fillfactor leaves room, so an upsert stays
-- a heap-only update. `metric_ids`: the metrics a budget's spend is made of.
CREATE UNLOGGED TABLE usage_monitor_state (
    monitor_id   UUID        NOT NULL,
    window_start TIMESTAMPTZ NOT NULL,
    group_key    TEXT        NOT NULL DEFAULT '',
    tenant_id    UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    customer_id  UUID        NOT NULL,
    monitor_kind TEXT        NOT NULL CHECK (monitor_kind IN ('entitlement', 'usage_limit', 'budget', 'heartbeat')),
    window_end   TIMESTAMPTZ NOT NULL,
    used         NUMERIC     NOT NULL,
    limit_value  NUMERIC,
    usage_as_of  TIMESTAMPTZ NOT NULL,
    computed_at  TIMESTAMPTZ NOT NULL,
    metric_ids   UUID[],
    PRIMARY KEY (monitor_id, window_start, group_key)
) WITH (fillfactor = 80);
CREATE INDEX usage_monitor_state_customer_idx ON usage_monitor_state (tenant_id, customer_id);
-- Each monitored customer's heartbeat row (HEARTBEAT_GROUP, HEARTBEAT_KIND at the epoch), which the
-- recompute sweep pages in (tenant, customer) order.
CREATE INDEX usage_monitor_state_heartbeat_idx
    ON usage_monitor_state (tenant_id, customer_id)
    WHERE window_start = 'epoch'::timestamptz AND group_key = '!' AND monitor_kind = 'heartbeat';
CREATE INDEX usage_monitor_state_window_end_idx ON usage_monitor_state (window_end);

-- 14. Shipped tables.
-- The invoice debt a credit note cancelled, apart from the cash it refunds and the credit it
-- returns; shipped credit-to-balance notes never cancelled debt. `credited_retired_balance`: main
-- added the note's credited amount to the scalar now carried into the CUSTOMER_BALANCE grant, so
-- voiding it takes that value back from there.
ALTER TABLE credit_note
    ADD COLUMN debt_cancelled_amount_cents BIGINT NOT NULL DEFAULT 0
        CHECK (debt_cancelled_amount_cents >= 0 AND debt_cancelled_amount_cents::numeric <= abs(total::numeric)),
    ADD COLUMN credited_retired_balance BOOLEAN NOT NULL DEFAULT false;
UPDATE credit_note
SET debt_cancelled_amount_cents = abs(total::numeric)::bigint
WHERE credit_type = 'DEBT_CANCELLATION';
UPDATE credit_note
SET credited_retired_balance = true
WHERE status = 'FINALIZED'
  AND credited_amount_cents > 0;
-- A pod still on the previous release writes debt cancellations without this column during the
-- rollout; a debt cancellation always cancels its whole total, so the row states it itself.
CREATE FUNCTION credit_note_debt_cancelled_amount() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    IF NEW.credit_type = 'DEBT_CANCELLATION' THEN
        NEW.debt_cancelled_amount_cents := abs(NEW.total::numeric)::bigint;
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER credit_note_debt_cancelled_amount
    BEFORE INSERT OR UPDATE OF credit_type, total ON credit_note
    FOR EACH ROW EXECUTE FUNCTION credit_note_debt_cancelled_amount();

-- Portal self-service is opt-in so an upgrade never starts charging cards from the portal.
ALTER TABLE invoicing_entity
    ADD COLUMN portal_credit_top_up_enabled BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN portal_auto_top_up_enabled   BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN portal_budgets_enabled       BOOLEAN NOT NULL DEFAULT false;

-- A failed top-up keeps its charge as evidence after its purchase document is discarded, and can
-- belong to a self-service checkout. A late capture of such a charge is refunded as a child of it.
ALTER TABLE payment_transaction
    DROP CONSTRAINT payment_transaction_invoice_or_checkout,
    ADD CONSTRAINT payment_transaction_invoice_or_checkout CHECK (
        (payment_type::text = 'TOP_UP') OR (payment_type::text = 'AUTO_TOPUP' AND checkout_session_id IS NULL)
        OR (payment_type::text = 'REFUND' AND parent_transaction_id IS NOT NULL)
        OR (payment_type::text NOT IN ('AUTO_TOPUP', 'TOP_UP') AND (invoice_id IS NOT NULL OR checkout_session_id IS NOT NULL))
    );

-- A top-up checkout has no plan.
ALTER TABLE checkout_session ALTER COLUMN plan_version_id DROP NOT NULL;

-- The previous release reverses payments and finalizes cash refunds without the ledger; a worker
-- reconciles them from these indexes. Dropped with the guard triggers once no pre-ledger instance can run.
CREATE INDEX payment_transaction_reversed_idx ON payment_transaction (tenant_id, invoice_id)
    WHERE amount_reversed > 0 AND invoice_id IS NOT NULL;
CREATE INDEX credit_note_finalized_refund_idx ON credit_note (tenant_id, invoice_id)
    WHERE status = 'FINALIZED' AND credit_type = 'REFUND';

-- 15. Carry the scalar balance into the ledger. It had no currency of its own: it was kept in the
-- customer's currency, so is its wallet. Customer money costs its face and pays any charge of any
-- subscription.
INSERT INTO customer_credit_account (id, tenant_id, customer_id, currency)
SELECT uuidv7(), tenant_id, id, upper(currency)
FROM customer
WHERE balance_value_cents > 0;

INSERT INTO customer_credit_grant (id, tenant_id, customer_id, account_id, amount, remaining, priority, classification,
                                   effective_at, source_kind, source_id, reason, application, cost_minor,
                                   cost_currency)
SELECT uuidv7(), c.tenant_id, c.id, a.id, c.balance_value_cents, c.balance_value_cents, 100, 'PAID', now(),
       'CUSTOMER_BALANCE', c.id, 'Customer balance carried over', 'INVOICE', c.balance_value_cents, a.currency
FROM customer c
JOIN customer_credit_account a ON a.tenant_id = c.tenant_id AND a.customer_id = c.id
WHERE c.balance_value_cents > 0;

INSERT INTO customer_credit_entry (id, tenant_id, customer_id, grant_id, kind, amount, source_id, occurred_at,
                                   cost_minor)
SELECT uuidv7(), g.tenant_id, g.customer_id, g.id, 'FUND', g.amount, g.source_id, g.effective_at, g.amount
FROM customer_credit_grant g;

DO
$$
    DECLARE
        mismatch RECORD;
    BEGIN
        FOR mismatch IN
            WITH expected AS (SELECT upper(currency) AS currency, sum(balance_value_cents) AS total
                              FROM customer
                              WHERE balance_value_cents > 0
                              GROUP BY 1),
                 moved AS (SELECT a.currency, sum(g.amount) AS total
                           FROM customer_credit_grant g
                           JOIN customer_credit_account a ON a.id = g.account_id
                           GROUP BY 1)
            SELECT coalesce(e.currency, m.currency) AS currency, e.total AS expected, m.total AS moved
            FROM expected e
            FULL JOIN moved m ON m.currency = e.currency
            WHERE e.total IS DISTINCT FROM m.total
            LOOP
                RAISE EXCEPTION 'customer credit carry-over mismatch for %: expected %, accounted %',
                    mismatch.currency, mismatch.expected, mismatch.moved;
            END LOOP;

        RAISE NOTICE 'customer credit carry-over: % balance grants (% minor units)',
            (SELECT count(*) FROM customer_credit_grant),
            (SELECT coalesce(sum(amount), 0) FROM customer_credit_grant);
    END
$$;

-- main recorded every amount in the customer's currency (converted at finalize before calling
-- CustomerBalance::update), so the ledger stamps the customer's current currency.
ALTER TABLE customer_balance_tx ADD COLUMN currency TEXT;
UPDATE customer_balance_tx t
SET currency = upper(c.currency)
FROM customer c
WHERE c.tenant_id = t.tenant_id AND c.id = t.customer_id;
ALTER TABLE customer_balance_tx ALTER COLUMN currency SET NOT NULL;
COMMENT ON TABLE customer_balance_tx IS
    'Frozen history of the retired customer.balance_value_cents scalar; customer_credit_entry is the live ledger.';
COMMENT ON COLUMN customer.balance_value_cents IS
    'Frozen at the value carried into customer_credit_grant; the ledger holds the live balance.';

-- 16. Instances of the previous release still running during a rolling deploy read the frozen
-- balance as spendable. Refuse their writes, so no balance moves outside the ledger; the caller
-- retries once its instance has been replaced. A cascading tenant delete stays allowed.
CREATE FUNCTION customer_balance_frozen() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    IF TG_OP = 'DELETE' AND NOT EXISTS (SELECT 1 FROM tenant WHERE id = OLD.tenant_id) THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'customer balance moved to the credit ledger; retry on an upgraded instance'
        USING ERRCODE = 'serialization_failure';
END;
$$;

CREATE TRIGGER customer_balance_value_frozen
    BEFORE UPDATE OF balance_value_cents ON customer
    FOR EACH ROW EXECUTE FUNCTION customer_balance_frozen();
CREATE TRIGGER customer_balance_tx_frozen
    BEFORE INSERT OR UPDATE OR DELETE ON customer_balance_tx
    FOR EACH ROW EXECUTE FUNCTION customer_balance_frozen();
CREATE TRIGGER customer_balance_pending_tx_frozen
    BEFORE INSERT OR UPDATE OR DELETE ON customer_balance_pending_tx
    FOR EACH ROW EXECUTE FUNCTION customer_balance_frozen();
-- A customer cannot be created with a balance either (INSERT, COPY or an upsert's insert branch).
CREATE TRIGGER customer_balance_value_seeded
    BEFORE INSERT ON customer
    FOR EACH ROW WHEN (NEW.balance_value_cents <> 0) EXECUTE FUNCTION customer_balance_frozen();
