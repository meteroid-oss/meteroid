-- Unit economics (ADR-0018): what only cost cards need, the fallbacks when no card costs a
-- metric (a ratio of revenue) or no provider reports a fee (a fee rule), and where the cost
-- projection of each card stands. Enterprise tables: core never reads or writes them.

CREATE TABLE cost_card_settings
(
    rate_card_id                UUID PRIMARY KEY REFERENCES rate_card (id) ON DELETE CASCADE,
    tenant_id                   UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    -- The one currency the card's costs are in; entries in other currencies are ignored.
    cost_currency               TEXT        NOT NULL,
    -- Usage the card does not price is costed from what its events report, net of the book.
    reported_amount_property    TEXT,
    reported_currency_property  TEXT,
    reported_default_currency   TEXT,
    reported_group_by           TEXT[]      NOT NULL DEFAULT '{}',
    -- The metric whose revenue the cost is set against; the card's own when NULL.
    attributed_metric_id        UUID REFERENCES billable_metric (id),
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((reported_amount_property IS NULL) = (reported_default_currency IS NULL))
);
CREATE INDEX cost_card_settings_tenant ON cost_card_settings (tenant_id);

-- Every cost card made before this table is an AI card: costed from `cost_reported` by model.
INSERT INTO cost_card_settings (rate_card_id, tenant_id, cost_currency, reported_amount_property,
                                reported_currency_property, reported_default_currency,
                                reported_group_by)
SELECT rc.id,
       rc.tenant_id,
       COALESCE((SELECT min(e.currency) FROM rate_card_entry e WHERE e.rate_card_id = rc.id), 'USD'),
       'cost_reported',
       'cost_currency',
       'USD',
       ARRAY ['model']
FROM rate_card rc
WHERE rc.purpose = 'COST';

-- Cost as a share of revenue, for revenue whose metric no cost card costs that day. A NULL
-- metric is the tenant default.
CREATE TABLE cost_ratio
(
    id             UUID PRIMARY KEY,
    tenant_id      UUID           NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    metric_id      UUID REFERENCES billable_metric (id) ON DELETE CASCADE,
    ratio          NUMERIC(10, 6) NOT NULL CHECK (ratio >= 0 AND ratio <= 10),
    effective_from DATE           NOT NULL,
    created_at     TIMESTAMPTZ    NOT NULL DEFAULT now(),
    archived_at    TIMESTAMPTZ
);
CREATE UNIQUE INDEX cost_ratio_live ON cost_ratio (tenant_id,
                                                  COALESCE(metric_id, '00000000-0000-0000-0000-000000000000'),
                                                  effective_from) WHERE archived_at IS NULL;

-- An estimated payment fee: basis points of the amount plus a fixed fee, for one connector or
-- all of them. Without a rule a fee is unknown, never a default.
CREATE TABLE payment_fee_rule
(
    id             UUID PRIMARY KEY,
    tenant_id      UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    connector_id   UUID REFERENCES connector (id) ON DELETE CASCADE,
    bps            INTEGER     NOT NULL CHECK (bps BETWEEN 0 AND 10000),
    fixed_minor    BIGINT      NOT NULL DEFAULT 0 CHECK (fixed_minor >= 0),
    fixed_currency TEXT,
    effective_from DATE        NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    archived_at    TIMESTAMPTZ,
    CHECK (fixed_minor = 0 OR fixed_currency IS NOT NULL)
);
CREATE UNIQUE INDEX payment_fee_rule_live ON payment_fee_rule (tenant_id,
                                                              COALESCE(connector_id, '00000000-0000-0000-0000-000000000000'),
                                                              effective_from) WHERE archived_at IS NULL;

-- Through which day a card's costs are projected, and from which day a backdated rate change
-- asks them to be projected again.
CREATE TABLE cost_projection_cursor
(
    rate_card_id      UUID PRIMARY KEY REFERENCES rate_card (id) ON DELETE CASCADE,
    tenant_id         UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    projected_through DATE,
    dirty_from        DATE,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
