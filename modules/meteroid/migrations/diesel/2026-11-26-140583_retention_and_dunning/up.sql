-- Retention (EE module): the cancel flow a tenant offers in its portal, the reasons a customer picks
-- from, the offer each reason leads to, and one session per cancellation attempt with its outcome.
CREATE TYPE "RetentionOfferKindEnum" AS ENUM ('COUPON', 'PLAN_CHANGE', 'CONTACT');
CREATE TYPE "RetentionOutcomeEnum" AS ENUM ('PENDING', 'SAVED', 'CONTACT_REQUESTED', 'CANCELLED');

CREATE TABLE retention_settings (
    tenant_id           UUID PRIMARY KEY REFERENCES tenant (id) ON DELETE CASCADE,
    enabled             BOOLEAN     NOT NULL DEFAULT FALSE,
    offer_cooldown_days INTEGER     NOT NULL DEFAULT 365 CHECK (offer_cooldown_days BETWEEN 0 AND 3650),
    -- Days before an annual or semiannual renewal, or a term end, when the tenant is notified.
    renewal_checkpoints INTEGER[]   NOT NULL DEFAULT '{90,60,30}' CHECK (
        cardinality(renewal_checkpoints) <= 6
        AND array_position(renewal_checkpoints, NULL) IS NULL
        AND 1 <= ALL (renewal_checkpoints) AND 365 >= ALL (renewal_checkpoints)
    ),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE retention_offer (
    id          UUID PRIMARY KEY,
    tenant_id   UUID                     NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    kind        "RetentionOfferKindEnum" NOT NULL,
    title       TEXT                     NOT NULL CHECK (length(title) BETWEEN 1 AND 120),
    description TEXT                     NOT NULL DEFAULT '' CHECK (length(description) <= 1000),
    coupon_id   UUID REFERENCES coupon (id) ON DELETE CASCADE,
    plan_id     UUID REFERENCES plan (id) ON DELETE CASCADE,
    archived_at TIMESTAMPTZ,
    created_at  TIMESTAMPTZ              NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ              NOT NULL DEFAULT now(),
    CONSTRAINT retention_offer_coupon_kind CHECK ((kind = 'COUPON') = (coupon_id IS NOT NULL)),
    CONSTRAINT retention_offer_plan_kind CHECK ((kind = 'PLAN_CHANGE') = (plan_id IS NOT NULL))
);
CREATE INDEX retention_offer_tenant_idx ON retention_offer (tenant_id);

CREATE TABLE retention_reason (
    id          UUID PRIMARY KEY,
    tenant_id   UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    label       TEXT        NOT NULL CHECK (length(label) BETWEEN 1 AND 120),
    position    INTEGER     NOT NULL DEFAULT 0,
    offer_id    UUID REFERENCES retention_offer (id) ON DELETE SET NULL,
    archived_at TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX retention_reason_tenant_idx ON retention_reason (tenant_id, position);

-- The reason label and the offer kind are copied, so a session reads the same after its reason or
-- offer is edited or removed.
CREATE TABLE retention_session (
    id              UUID PRIMARY KEY,
    tenant_id       UUID                   NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    customer_id     UUID                   NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    subscription_id UUID                   NOT NULL REFERENCES subscription (id) ON DELETE CASCADE,
    reason_id       UUID REFERENCES retention_reason (id) ON DELETE SET NULL,
    reason_label    TEXT,
    comment         TEXT CHECK (length(comment) <= 2000),
    offer_id        UUID REFERENCES retention_offer (id) ON DELETE SET NULL,
    offer_kind      "RetentionOfferKindEnum",
    -- The coupon or plan version shown, so an offer changed since is not accepted in its place.
    offer_target    UUID,
    outcome         "RetentionOutcomeEnum" NOT NULL DEFAULT 'PENDING',
    created_at      TIMESTAMPTZ            NOT NULL DEFAULT now(),
    resolved_at     TIMESTAMPTZ,
    -- Set while an accept or a confirmation applies its effect, so only one of them runs.
    claimed_at      TIMESTAMPTZ,
    -- When the tenant marked a contact request as answered.
    contact_handled_at TIMESTAMPTZ,
    CONSTRAINT retention_session_resolved CHECK ((outcome = 'PENDING') = (resolved_at IS NULL)),
    CONSTRAINT retention_session_contact_handled
        CHECK (contact_handled_at IS NULL OR outcome = 'CONTACT_REQUESTED')
);
CREATE INDEX retention_session_tenant_idx ON retention_session (tenant_id, created_at DESC);
CREATE INDEX retention_session_subscription_idx ON retention_session (subscription_id, created_at DESC);
CREATE UNIQUE INDEX retention_session_one_pending ON retention_session (subscription_id) WHERE outcome = 'PENDING';

-- One row per checkpoint notified, so each fires once per renewal date whichever worker runs it.
CREATE TABLE retention_renewal_notice (
    subscription_id UUID        NOT NULL REFERENCES subscription (id) ON DELETE CASCADE,
    tenant_id       UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    renewal_date    DATE        NOT NULL,
    days_before     INTEGER     NOT NULL,
    -- A renewal noticed at 30 days and then cancelled is noticed again as a cancellation.
    mode            TEXT        NOT NULL,
    notified_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (subscription_id, renewal_date, days_before, mode)
);

-- A tenant's automatic payment retries (EE dunning module): the delays between failed attempts and
-- what happens to the subscription once they run out. Without a row, core keeps 3, 5 and 7 days
-- and leaves the invoice in collection.
CREATE TYPE "DunningExhaustedActionEnum" AS ENUM ('NONE', 'CANCEL_SUBSCRIPTION');

CREATE TABLE dunning_policy (
    tenant_id        UUID PRIMARY KEY REFERENCES tenant (id) ON DELETE CASCADE,
    retry_days       INTEGER[]                    NOT NULL,
    exhausted_action "DunningExhaustedActionEnum" NOT NULL DEFAULT 'NONE',
    updated_at       TIMESTAMPTZ                  NOT NULL DEFAULT now(),
    CONSTRAINT dunning_policy_retry_days CHECK (
        cardinality(retry_days) <= 8
        AND array_position(retry_days, NULL) IS NULL
        AND 1 <= ALL (retry_days) AND 60 >= ALL (retry_days)
    ),
    -- Cancelling on the first decline, with no retry, would cut off a transient failure.
    CONSTRAINT dunning_policy_cancel_needs_retries CHECK (
        exhausted_action <> 'CANCEL_SUBSCRIPTION' OR cardinality(retry_days) > 0
    )
);

-- When a hard decline stopped an invoice's retries, the time the tenant's cancel policy ends its
-- subscription unless the invoice is paid or a new card revives the retries.
CREATE TABLE invoice_dunning_cancel (
    invoice_id      UUID PRIMARY KEY REFERENCES invoice (id) ON DELETE CASCADE,
    tenant_id       UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    customer_id     UUID      NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    subscription_id UUID      NOT NULL REFERENCES subscription (id) ON DELETE CASCADE,
    cancel_at       TIMESTAMP NOT NULL
);
CREATE INDEX invoice_dunning_cancel_customer_idx
    ON invoice_dunning_cancel (tenant_id, customer_id, cancel_at);
CREATE INDEX invoice_dunning_cancel_subscription_idx ON invoice_dunning_cancel (subscription_id);

-- The first service period a coupon added to a running subscription discounts: the lines of earlier
-- periods, billed in arrears after it, keep their price. NULL: every line it is applied to.
ALTER TABLE applied_coupon ADD COLUMN discounts_from DATE;
