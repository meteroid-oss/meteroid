-- Subscription migration (EE module `migration`): subscriptions imported from another billing
-- system, shadow-billed against its invoices, then cut over to Meteroid.

CREATE TABLE migration_source
(
    id                     UUID PRIMARY KEY,
    tenant_id              UUID        NOT NULL REFERENCES tenant ON DELETE CASCADE,
    kind                   TEXT        NOT NULL CHECK (kind IN ('LEMON_SQUEEZY', 'STRIPE', 'CSV')),
    name                   TEXT        NOT NULL,
    live_mode              BOOLEAN     NOT NULL DEFAULT FALSE,
    -- The source's own account (a Lemon Squeezy store id, a Stripe account id).
    external_account_id    TEXT,
    -- Encrypted (crypt::encrypt): the API key.
    credentials            TEXT,
    default_currency       TEXT        NOT NULL,
    invoicing_entity_id    UUID REFERENCES invoicing_entity ON DELETE SET NULL,
    sync_requested_at      TIMESTAMPTZ,
    sync_started_at        TIMESTAMPTZ,
    sync_finished_at       TIMESTAMPTZ,
    sync_error             TEXT,
    sync_stats             JSONB       NOT NULL DEFAULT '{}',
    -- Policy, applied to every subscription of the source.
    auto_cutover           BOOLEAN     NOT NULL DEFAULT FALSE,
    drift_tolerance_amount BIGINT      NOT NULL DEFAULT 50 CHECK (drift_tolerance_amount >= 0),
    drift_tolerance_bps    INTEGER     NOT NULL DEFAULT 10 CHECK (drift_tolerance_bps >= 0),
    min_clean_periods      INTEGER     NOT NULL DEFAULT 3 CHECK (min_clean_periods >= 1),
    backtest_periods       INTEGER     NOT NULL DEFAULT 3 CHECK (backtest_periods >= 0),
    max_shadow_days        INTEGER CHECK (max_shadow_days > 0),
    compare_tax            BOOLEAN     NOT NULL DEFAULT FALSE,
    require_payment_method BOOLEAN     NOT NULL DEFAULT TRUE,
    cutover_lead_hours     INTEGER     NOT NULL DEFAULT 24 CHECK (cutover_lead_hours >= 1),
    -- Per bucket ({"tax": {"amount": 100, "bps": 50}}): a period is clean when each bucket is too.
    bucket_tolerances      JSONB       NOT NULL DEFAULT '{}',
    -- [{"plan_id", "auto_cutover", "drift_tolerance_amount", "min_clean_periods"}], over the defaults above.
    plan_overrides         JSONB       NOT NULL DEFAULT '[]',
    -- What a period over tolerance does: SUSPEND (DRIFT, waits for review) or BLOCK.
    breach_action          TEXT        NOT NULL DEFAULT 'SUSPEND' CHECK (breach_action IN ('SUSPEND', 'BLOCK')),
    -- The kill-switch: nothing is scheduled nor cut over while set.
    paused                 BOOLEAN     NOT NULL DEFAULT FALSE,
    -- Every subscription is cut over before this day: the source's sunset, or the merchant's own.
    cutover_deadline       DATE,
    -- An operator confirmed the hand-over checklist (tax, payments, the app's access source).
    readiness_confirmed_at TIMESTAMPTZ,
    readiness_confirmed_by UUID,
    -- The records an operator uploaded, for sources without an API.
    upload                 JSONB,
    archived_at            TIMESTAMPTZ,
    created_by             UUID        NOT NULL,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX migration_source_tenant_idx ON migration_source (tenant_id) WHERE archived_at IS NULL;

CREATE TABLE migration_customer
(
    id          UUID PRIMARY KEY,
    tenant_id   UUID        NOT NULL REFERENCES tenant ON DELETE CASCADE,
    source_id   UUID        NOT NULL REFERENCES migration_source ON DELETE CASCADE,
    external_id TEXT        NOT NULL,
    name        TEXT        NOT NULL,
    email       TEXT,
    country     TEXT,
    address     JSONB       NOT NULL DEFAULT '{}',
    tax_id      TEXT,
    is_company  BOOLEAN     NOT NULL DEFAULT FALSE,
    -- A payment method Meteroid can charge: {"provider", "customer_ref", "payment_method_ref"}.
    payment_method JSONB,
    customer_id UUID REFERENCES customer ON DELETE SET NULL,
    data        JSONB       NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (source_id, external_id)
);

-- One source price, and the Meteroid plan component that bills it.
CREATE TABLE migration_price_mapping
(
    id                    UUID PRIMARY KEY,
    tenant_id             UUID        NOT NULL REFERENCES tenant ON DELETE CASCADE,
    source_id             UUID        NOT NULL REFERENCES migration_source ON DELETE CASCADE,
    external_product_id   TEXT        NOT NULL,
    external_product_name TEXT        NOT NULL,
    external_price_id     TEXT        NOT NULL,
    external_price_name   TEXT        NOT NULL,
    -- Minor units per unit and interval; NULL for tiered prices.
    unit_amount           BIGINT,
    currency              TEXT        NOT NULL,
    billing_interval      TEXT        NOT NULL CHECK (billing_interval IN ('DAY', 'WEEK', 'MONTH', 'YEAR', 'ONE_TIME')),
    interval_count        INTEGER     NOT NULL DEFAULT 1,
    usage_based           BOOLEAN     NOT NULL DEFAULT FALSE,
    pricing_scheme        TEXT        NOT NULL DEFAULT 'STANDARD',
    data                  JSONB       NOT NULL DEFAULT '{}',
    status                TEXT        NOT NULL DEFAULT 'UNMAPPED'
        CHECK (status IN ('SUGGESTED', 'CONFIRMED', 'SKIPPED', 'UNMAPPED')),
    plan_id               UUID REFERENCES plan ON DELETE SET NULL,
    plan_version_id       UUID REFERENCES plan_version ON DELETE SET NULL,
    price_component_id    UUID REFERENCES price_component ON DELETE SET NULL,
    -- SOURCE: the component's price is overridden with the source's, as grandfathered.
    price_mode            TEXT        NOT NULL DEFAULT 'SOURCE' CHECK (price_mode IN ('PLAN', 'SOURCE')),
    confidence            REAL        NOT NULL DEFAULT 0,
    note                  TEXT,
    confirmed_by          UUID,
    confirmed_at          TIMESTAMPTZ,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (source_id, external_price_id)
);

CREATE TABLE migration_subscription
(
    id                          UUID PRIMARY KEY,
    tenant_id                   UUID        NOT NULL REFERENCES tenant ON DELETE CASCADE,
    source_id                   UUID        NOT NULL REFERENCES migration_source ON DELETE CASCADE,
    external_id                 TEXT        NOT NULL,
    external_customer_id        TEXT        NOT NULL,
    migration_customer_id       UUID REFERENCES migration_customer ON DELETE SET NULL,
    external_price_id           TEXT,
    external_item_id            TEXT,
    quantity                    INTEGER     NOT NULL DEFAULT 1,
    currency                    TEXT        NOT NULL,
    source_status               TEXT        NOT NULL,
    product_name                TEXT        NOT NULL,
    price_name                  TEXT        NOT NULL,
    -- Minor units per month, in `currency`.
    mrr                         BIGINT      NOT NULL DEFAULT 0,
    billing_anchor              INTEGER,
    started_at                  TIMESTAMPTZ NOT NULL,
    current_period_start        TIMESTAMPTZ,
    current_period_end          TIMESTAMPTZ,
    trial_ends_at               TIMESTAMPTZ,
    ends_at                     TIMESTAMPTZ,
    usage_based                 BOOLEAN     NOT NULL DEFAULT FALSE,
    card_brand                  TEXT,
    card_last_four              TEXT,
    payment_processor           TEXT,
    -- [{"external_id", "price_external_id", "quantity", "usage_based"}]: every price it bills.
    items                       JSONB       NOT NULL DEFAULT '[]',
    discount                    JSONB,
    data                        JSONB       NOT NULL DEFAULT '{}',
    state                       TEXT        NOT NULL DEFAULT 'IMPORTED'
        CHECK (state IN ('IMPORTED', 'SHADOW', 'READY', 'SCHEDULED', 'ACTIVE', 'DRIFT', 'BLOCKED')),
    state_reason                TEXT,
    state_changed_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    clean_periods               INTEGER     NOT NULL DEFAULT 0,
    compared_periods            INTEGER     NOT NULL DEFAULT 0,
    last_drift                  BIGINT,
    shadow_stale                BOOLEAN     NOT NULL DEFAULT TRUE,
    -- Operator facts the state is derived from.
    hold_reason                 TEXT,
    held_by                     UUID,
    held_at                     TIMESTAMPTZ,
    drift_accepted_through      DATE,
    exclude_auto_cutover        BOOLEAN     NOT NULL DEFAULT FALSE,
    drift_tolerance_override    BIGINT CHECK (drift_tolerance_override >= 0),
    approved_by                 UUID,
    approved_at                 TIMESTAMPTZ,
    -- Cutover.
    cutover_at                  TIMESTAMPTZ,
    cutover_started_at          TIMESTAMPTZ,
    -- Set when the cutover itself cancelled the source subscription, before creating Meteroid's.
    source_cancelled_at         TIMESTAMPTZ,
    -- For a source cancelled by hand: an operator confirmed it no longer renews.
    manual_cancel_confirmed_at  TIMESTAMPTZ,
    cutover_completed_at        TIMESTAMPTZ,
    cutover_error               TEXT,
    cutover_attempts            INTEGER     NOT NULL DEFAULT 0,
    subscription_id             UUID REFERENCES subscription ON DELETE SET NULL,
    payment_method_strategy     TEXT        NOT NULL DEFAULT 'RECOLLECT'
        CHECK (payment_method_strategy IN ('REUSE', 'RECOLLECT')),
    payment_method_requested_at TIMESTAMPTZ,
    -- Customers emailed for a payment method are reminded until one is added.
    payment_method_reminders    BOOLEAN     NOT NULL DEFAULT FALSE,
    payment_method_emails       INTEGER     NOT NULL DEFAULT 0,
    payment_method_emailed_at   TIMESTAMPTZ,
    imported_at                 TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at                TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (source_id, external_id)
);

CREATE INDEX migration_subscription_state_idx ON migration_subscription (tenant_id, source_id, state);
CREATE INDEX migration_subscription_due_idx ON migration_subscription (cutover_at) WHERE state = 'SCHEDULED';
CREATE INDEX migration_subscription_stale_idx ON migration_subscription (source_id) WHERE shadow_stale;

CREATE TABLE migration_source_invoice
(
    id                        UUID PRIMARY KEY,
    tenant_id                 UUID        NOT NULL REFERENCES tenant ON DELETE CASCADE,
    source_id                 UUID        NOT NULL REFERENCES migration_source ON DELETE CASCADE,
    migration_subscription_id UUID        NOT NULL REFERENCES migration_subscription ON DELETE CASCADE,
    external_id               TEXT        NOT NULL,
    billing_reason            TEXT        NOT NULL CHECK (billing_reason IN ('INITIAL', 'RENEWAL', 'UPDATED', 'OTHER')),
    status                    TEXT        NOT NULL,
    currency                  TEXT        NOT NULL,
    -- Minor units, before discount and tax.
    subtotal                  BIGINT      NOT NULL,
    discount                  BIGINT      NOT NULL DEFAULT 0,
    tax                       BIGINT      NOT NULL DEFAULT 0,
    total                     BIGINT      NOT NULL,
    refunded                  BIGINT      NOT NULL DEFAULT 0,
    issued_at                 TIMESTAMPTZ NOT NULL,
    period_start              DATE        NOT NULL,
    period_end                DATE        NOT NULL,
    lines                     JSONB       NOT NULL DEFAULT '[]',
    data                      JSONB       NOT NULL DEFAULT '{}',
    created_at                TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (source_id, external_id)
);

CREATE INDEX migration_source_invoice_sub_idx ON migration_source_invoice (migration_subscription_id, period_start);

-- The source's usage of a usage-based item over one period, captured before the period renews.
CREATE TABLE migration_usage_snapshot
(
    id                        UUID PRIMARY KEY,
    tenant_id                 UUID        NOT NULL REFERENCES tenant ON DELETE CASCADE,
    migration_subscription_id UUID        NOT NULL REFERENCES migration_subscription ON DELETE CASCADE,
    period_start              DATE        NOT NULL,
    period_end                DATE        NOT NULL,
    quantity                  NUMERIC     NOT NULL,
    captured_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (migration_subscription_id, period_start)
);

-- One source invoice against what Meteroid bills for the same period.
CREATE TABLE migration_shadow_period
(
    id                        UUID PRIMARY KEY,
    tenant_id                 UUID        NOT NULL REFERENCES tenant ON DELETE CASCADE,
    migration_subscription_id UUID        NOT NULL REFERENCES migration_subscription ON DELETE CASCADE,
    source_invoice_id         UUID        NOT NULL REFERENCES migration_source_invoice ON DELETE CASCADE,
    period_start              DATE        NOT NULL,
    period_end                DATE        NOT NULL,
    currency                  TEXT        NOT NULL,
    source_subtotal           BIGINT      NOT NULL,
    source_discount           BIGINT      NOT NULL,
    source_tax                BIGINT      NOT NULL,
    source_total              BIGINT      NOT NULL,
    meteroid_subtotal         BIGINT      NOT NULL,
    meteroid_discount         BIGINT      NOT NULL,
    meteroid_tax              BIGINT      NOT NULL,
    meteroid_total            BIGINT      NOT NULL,
    -- Meteroid minus the source, on the compared amount (with or without tax, per the policy).
    drift                     BIGINT      NOT NULL,
    buckets                   JSONB       NOT NULL DEFAULT '{}',
    lines                     JSONB       NOT NULL DEFAULT '[]',
    outcome                   TEXT        NOT NULL CHECK (outcome IN ('CLEAN', 'DRIFT', 'INCOMPLETE', 'ERROR')),
    detail                    TEXT,
    computed_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (migration_subscription_id, source_invoice_id)
);

CREATE TABLE migration_event
(
    id                        UUID PRIMARY KEY,
    tenant_id                 UUID        NOT NULL REFERENCES tenant ON DELETE CASCADE,
    source_id                 UUID        NOT NULL REFERENCES migration_source ON DELETE CASCADE,
    migration_subscription_id UUID REFERENCES migration_subscription ON DELETE CASCADE,
    kind                      TEXT        NOT NULL,
    message                   TEXT        NOT NULL,
    actor_kind                TEXT        NOT NULL CHECK (actor_kind IN ('SYSTEM', 'POLICY', 'USER')),
    actor_id                  UUID,
    metadata                  JSONB       NOT NULL DEFAULT '{}',
    created_at                TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX migration_event_subscription_idx ON migration_event (migration_subscription_id, created_at DESC);
CREATE INDEX migration_event_source_idx ON migration_event (source_id, created_at DESC);
