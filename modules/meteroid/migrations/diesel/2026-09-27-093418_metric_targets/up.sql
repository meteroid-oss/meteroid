-- Targets: the planned value of a catalog metric over a month or a quarter, its
-- owner, and the history of every change to it.
CREATE TYPE "MetricTargetPeriodGrainEnum" AS ENUM ('MONTH', 'QUARTER');

CREATE TABLE metric_target (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    -- A catalog metric key, as the explorer and the monitors use it.
    metric_key TEXT NOT NULL,
    -- Optional scope: the customers it counts, as a cohort (the segment builder's predicates),
    -- and how that reads. NULL = the whole business. Kept as set: editing a saved segment later
    -- does not move a target.
    cohort JSONB,
    scope_label TEXT,
    -- Who owns the number. A departing user never removes the target.
    owner_user_id UUID REFERENCES "user" (id) ON DELETE SET NULL,
    period_grain "MetricTargetPeriodGrainEnum" NOT NULL,
    period_start DATE NOT NULL,
    -- In the metric's own unit: minor units of `currency` for money, the plain number otherwise.
    target_value NUMERIC NOT NULL,
    -- The reporting currency a money target was set in; NULL for other units.
    currency TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT now(),
    updated_at TIMESTAMP
);

CREATE INDEX idx_metric_target_tenant_period ON metric_target (tenant_id, period_start);

-- One target per metric, scope and period. COALESCE keeps a tenant-wide target apart from a
-- scoped one; jsonb's text is canonical, so the same cohort hashes the same.
CREATE UNIQUE INDEX uq_metric_target_scope ON metric_target (
    tenant_id, metric_key, period_grain, period_start, COALESCE(md5(cohort::text), '')
);

-- Every change to a target, kept after the target is deleted: the number the board saw can be
-- traced. `before` / `after` hold the target's value, owner and currency.
CREATE TYPE "MetricTargetChangeKindEnum" AS ENUM ('CREATED', 'UPDATED', 'DELETED');

CREATE TABLE metric_target_change (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    metric_target_id UUID NOT NULL,
    kind "MetricTargetChangeKindEnum" NOT NULL,
    -- Set when the change came from a plan import.
    imported BOOLEAN NOT NULL DEFAULT FALSE,
    actor_user_id UUID REFERENCES "user" (id) ON DELETE SET NULL,
    before JSONB,
    after JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT now()
);

CREATE INDEX idx_metric_target_change_target
    ON metric_target_change (tenant_id, metric_target_id, created_at DESC);

-- Who owns a monitor's number. Distinct from account ownership.
ALTER TABLE alert_definition
    ADD COLUMN owner_user_id UUID REFERENCES "user" (id) ON DELETE SET NULL;

-- A monitor muted until a moment: it keeps evaluating and recording its firings, but sends
-- nothing until then.
ALTER TABLE alert_definition ADD COLUMN muted_until TIMESTAMP;

-- A running target's assessment once a day: what changed since Monday, since when a status holds,
-- how close "if nothing changes" came to the final value.
CREATE TABLE metric_target_snapshot (
    metric_target_id UUID NOT NULL REFERENCES metric_target (id) ON DELETE CASCADE,
    snapshot_date DATE NOT NULL,
    tenant_id UUID NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    actual DOUBLE PRECISION,
    projected DOUBLE PRECISION,
    expected DOUBLE PRECISION,
    status TEXT NOT NULL,
    scheduled_count INT NOT NULL DEFAULT 0,
    -- A day rebuilt from billing records rather than read that day: the actual is exact, "if
    -- nothing changes" counts only what billing had on its calendar then, priced as today for what
    -- is still ahead. A reading taken on the day replaces it.
    reconstructed BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT now(),
    PRIMARY KEY (metric_target_id, snapshot_date)
);

CREATE INDEX idx_metric_target_snapshot_tenant_date
    ON metric_target_snapshot (tenant_id, snapshot_date);
