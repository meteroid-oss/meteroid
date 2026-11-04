
CREATE TYPE "AdjustmentKindEnum" AS ENUM ('MINIMUM');

-- ── Plan version ──────────────────────────────────────────────────────────────────────────
CREATE TABLE if not exists  plan_version_adjustment (
    plan_version_id         UUID    NOT NULL REFERENCES plan_version(id) ON DELETE CASCADE,
    kind                    "AdjustmentKindEnum" NOT NULL,
    amount_subunit          BIGINT  NOT NULL CHECK (amount_subunit > 0),
    includes_all_components BOOLEAN NOT NULL DEFAULT TRUE,
    created_at              TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (plan_version_id, kind)
);

CREATE TABLE if not exists  plan_version_adjustment_component (
    plan_version_id    UUID NOT NULL,
    kind               "AdjustmentKindEnum" NOT NULL,

    product_id         UUID NOT NULL REFERENCES product(id) ON DELETE CASCADE,
    PRIMARY KEY (plan_version_id, kind, product_id),
    FOREIGN KEY (plan_version_id, kind)
        REFERENCES plan_version_adjustment(plan_version_id, kind) ON DELETE CASCADE
);

CREATE INDEX if not exists  idx_plan_version_adjustment_component_product
    ON plan_version_adjustment_component(product_id);

CREATE TABLE if not exists  subscription_adjustment (
    subscription_id         UUID    NOT NULL REFERENCES subscription(id) ON DELETE CASCADE,
    kind                    "AdjustmentKindEnum" NOT NULL,
    amount_subunit          BIGINT  NOT NULL CHECK (amount_subunit > 0),
    includes_all_components BOOLEAN NOT NULL,
    -- TRUE when set by operator override OR sourced from a quote_adjustment at conversion time.
    -- Plan-change must NOT touch overrides; only plan-snapshots (is_override = FALSE) get
    -- re-snapped to the new plan version's adjustment.
    is_override             BOOLEAN NOT NULL DEFAULT FALSE,
    created_at              TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (subscription_id, kind)
);

CREATE TABLE if not exists  subscription_adjustment_component (
    subscription_id    UUID NOT NULL,
    kind               "AdjustmentKindEnum" NOT NULL,
    -- Scope keyed by product (see plan_version_adjustment_component). No FK: the id is a stable
    -- product reference kept across the snapshot boundary.
    product_id         UUID NOT NULL,
    PRIMARY KEY (subscription_id, kind, product_id),
    FOREIGN KEY (subscription_id, kind)
        REFERENCES subscription_adjustment(subscription_id, kind) ON DELETE CASCADE
);

-- ── Quote-level override ──────────────────────────────────────────────────────────────────
CREATE TABLE if not exists  quote_adjustment (
    quote_id                UUID    NOT NULL REFERENCES quote(id) ON DELETE CASCADE,
    kind                    "AdjustmentKindEnum" NOT NULL,
    amount_subunit          BIGINT  NOT NULL CHECK (amount_subunit > 0),
    includes_all_components BOOLEAN NOT NULL DEFAULT TRUE,
    -- Names of inline new-product extras included in the scope whose products don't exist yet
    -- (a quote doesn't mint them until conversion). Resolved to product ids at conversion time;
    -- see apply_quote_minimum_override. Empty for AllComponents / id-only scopes.
    pending_extra_product_names TEXT[] NOT NULL DEFAULT '{}',
    created_at              TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (quote_id, kind)
);

CREATE TABLE if not exists  quote_adjustment_component (
    quote_id           UUID NOT NULL,
    kind               "AdjustmentKindEnum" NOT NULL,
    -- Same convention as subscription_adjustment_component: stable product id, no FK.
    product_id         UUID NOT NULL,
    PRIMARY KEY (quote_id, kind, product_id),
    FOREIGN KEY (quote_id, kind)
        REFERENCES quote_adjustment(quote_id, kind) ON DELETE CASCADE
);
