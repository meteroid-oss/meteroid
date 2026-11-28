-- Volume agreements (ADR-0021): tiers of the members' pooled metrics counted on the volume of all
-- of them over each window of the agreement's grid, each member invoiced on its own.
ALTER TABLE subscription ADD CONSTRAINT subscription_id_tenant_key UNIQUE (id, tenant_id);
ALTER TABLE billable_metric ADD CONSTRAINT billable_metric_id_tenant_key UNIQUE (id, tenant_id);

CREATE TABLE volume_agreement (
    id                 UUID PRIMARY KEY,
    tenant_id          UUID                NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    name               TEXT                NOT NULL,
    currency           TEXT                NOT NULL,
    cadence            "BillingPeriodEnum" NOT NULL,
    billing_day_anchor SMALLINT            NOT NULL CHECK (billing_day_anchor BETWEEN 1 AND 31),
    start_date         DATE                NOT NULL,
    end_date           DATE CHECK (end_date > start_date),
    owner_customer_id  UUID REFERENCES customer (id) ON DELETE SET NULL,
    archived_at        TIMESTAMPTZ,
    created_at         TIMESTAMPTZ         NOT NULL DEFAULT now(),
    created_by         UUID                NOT NULL,
    updated_at         TIMESTAMPTZ,
    CONSTRAINT volume_agreement_id_tenant_key UNIQUE (id, tenant_id)
);
CREATE INDEX volume_agreement_tenant_idx ON volume_agreement (tenant_id) WHERE archived_at IS NULL;

CREATE TABLE volume_agreement_metric (
    agreement_id      UUID NOT NULL,
    tenant_id         UUID NOT NULL,
    billable_metric_id UUID NOT NULL,
    PRIMARY KEY (agreement_id, billable_metric_id),
    FOREIGN KEY (agreement_id, tenant_id) REFERENCES volume_agreement (id, tenant_id) ON DELETE CASCADE,
    FOREIGN KEY (billable_metric_id, tenant_id) REFERENCES billable_metric (id, tenant_id) ON DELETE CASCADE
);

CREATE TABLE volume_agreement_member (
    id              UUID PRIMARY KEY,
    tenant_id       UUID        NOT NULL,
    agreement_id    UUID        NOT NULL,
    subscription_id UUID        NOT NULL,
    from_date       DATE        NOT NULL,
    until_date      DATE CHECK (until_date > from_date),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (agreement_id, tenant_id) REFERENCES volume_agreement (id, tenant_id) ON DELETE CASCADE,
    FOREIGN KEY (subscription_id, tenant_id) REFERENCES subscription (id, tenant_id) ON DELETE CASCADE
);
CREATE INDEX volume_agreement_member_agreement_idx ON volume_agreement_member (agreement_id, from_date);
CREATE INDEX volume_agreement_member_subscription_idx ON volume_agreement_member (subscription_id);

-- A window's volume of one pooled metric, written once by the window's close (ADR-0021 §4).
CREATE TABLE volume_agreement_basis (
    tenant_id          UUID        NOT NULL,
    agreement_id       UUID        NOT NULL,
    billable_metric_id UUID        NOT NULL,
    period_start       DATE        NOT NULL,
    period_end         DATE        NOT NULL CHECK (period_end > period_start),
    quantity           NUMERIC     NOT NULL CHECK (quantity >= 0),
    closed_at          TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (agreement_id, billable_metric_id, period_start),
    FOREIGN KEY (agreement_id, tenant_id) REFERENCES volume_agreement (id, tenant_id) ON DELETE CASCADE
);
