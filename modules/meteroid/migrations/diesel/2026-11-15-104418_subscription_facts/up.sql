-- Phase 7, PR 1: a subscription's lifecycle recorded as dated facts, written by the paths that act
-- next to the events and state that still drive billing. Nothing bills from them yet; the
-- backfills below are checked by design/pre-release-checks.sql (checks 9 to 14).
SET LOCAL lock_timeout = '5s';

-- ── Trials ────────────────────────────────────────────────────────────────────────────────────
CREATE TABLE subscription_trial (
    subscription_id UUID      PRIMARY KEY REFERENCES subscription (id) ON DELETE CASCADE,
    tenant_id       UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    from_date       DATE      NOT NULL,
    until_date      DATE      NOT NULL,
    -- Decided by the plan version in force at the start, never by a later one.
    free            BOOLEAN   NOT NULL,
    created_at      TIMESTAMP NOT NULL,
    CONSTRAINT subscription_trial_ends_after_it_starts CHECK (until_date >= from_date)
);

-- The plan version a subscription started on: its first switch's old one, else its current one.
CREATE TEMP TABLE first_plan_switch ON COMMIT DROP AS
SELECT DISTINCT ON (e.subscription_id)
       e.subscription_id,
       e.applies_to,
       (e.details ->> 'old_plan_version_id')::uuid AS old_version
FROM subscription_event e
WHERE e.event_type = 'SWITCH'
  AND e.details ->> 'old_plan_version_id' IS DISTINCT FROM e.details ->> 'new_plan_version_id'
ORDER BY e.subscription_id, e.applies_to, e.created_at;

-- A free trial ended by a plan change moved the billing start onto the change: it began at the
-- start date then. Any plan change ends a trial (as the synthesized trial did).
INSERT INTO subscription_trial (subscription_id, tenant_id, from_date, until_date, free, created_at)
SELECT s.id, s.tenant_id, t.from_date,
       GREATEST(t.from_date,
                LEAST(t.from_date + s.trial_duration, COALESCE(f.applies_to, 'infinity'::date))),
       t.free,
       now() AT TIME ZONE 'UTC'
FROM subscription s
LEFT JOIN first_plan_switch f ON f.subscription_id = s.id
LEFT JOIN plan_version pv ON pv.id = COALESCE(f.old_version, s.plan_version_id)
CROSS JOIN LATERAL (
    SELECT COALESCE(pv.trial_is_free, TRUE) AS free,
           CASE WHEN COALESCE(pv.trial_is_free, TRUE)
                     AND s.billing_start_date = f.applies_to
                     AND s.billing_start_date > s.start_date
                THEN s.start_date
                ELSE COALESCE(s.billing_start_date, s.start_date) END AS from_date
) t
WHERE s.trial_duration > 0;

-- ── Settled starts ────────────────────────────────────────────────────────────────────────────
CREATE TABLE subscription_start_settled (
    subscription_id UUID      PRIMARY KEY REFERENCES subscription (id) ON DELETE CASCADE,
    tenant_id       UUID      NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    at              TIMESTAMP NOT NULL,
    settled_by      TEXT      NOT NULL,
    created_at      TIMESTAMP NOT NULL,
    CONSTRAINT subscription_start_settled_by CHECK (
        settled_by IN ('START', 'QUOTE', 'CHECKOUT', 'PAYMENT', 'MANUAL'))
);

-- Every start is stamped on activated_at (trg_subscription_set_activated_at); who settled it is
-- read from the activation condition, a checkout after an expired trial counting as its checkout.
INSERT INTO subscription_start_settled (subscription_id, tenant_id, at, settled_by, created_at)
SELECT s.id, s.tenant_id, s.activated_at,
       CASE s.activation_condition
           WHEN 'ON_START' THEN CASE WHEN s.quote_id IS NULL THEN 'START' ELSE 'QUOTE' END
           WHEN 'ON_CHECKOUT' THEN 'CHECKOUT'
           ELSE 'MANUAL' END,
       now() AT TIME ZONE 'UTC'
FROM subscription s
WHERE s.activated_at IS NOT NULL;

-- An On Start subscription not activated yet starts billing on its start date.
INSERT INTO subscription_start_settled (subscription_id, tenant_id, at, settled_by, created_at)
SELECT s.id, s.tenant_id, s.start_date::timestamp,
       CASE WHEN s.quote_id IS NULL THEN 'START' ELSE 'QUOTE' END,
       now() AT TIME ZONE 'UTC'
FROM subscription s
WHERE s.activated_at IS NULL
  AND s.activation_condition = 'ON_START'
  AND s.status NOT IN ('CANCELLED', 'ABORTED', 'COMPLETED', 'SUPERSEDED');

-- ── Endings ───────────────────────────────────────────────────────────────────────────────────
CREATE TABLE subscription_ending (
    subscription_id UUID             NOT NULL REFERENCES subscription (id) ON DELETE CASCADE,
    tenant_id       UUID             NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    -- CANCEL: a cancellation, scheduled or carried out. COMPLETE: a fixed term's end date.
    kind            TEXT             NOT NULL,
    at              TIMESTAMP        NOT NULL,
    reason          TEXT,
    actor_type      "ActorTypeEnum"  NOT NULL,
    actor_uuid      UUID,
    actor_alias     TEXT,
    created_at      TIMESTAMP        NOT NULL,
    PRIMARY KEY (subscription_id, kind),
    CONSTRAINT subscription_ending_kind CHECK (kind IN ('CANCEL', 'COMPLETE'))
);

-- The cancellation still to apply, else the one a cancelled subscription ended by.
INSERT INTO subscription_ending (subscription_id, tenant_id, kind, at, reason, actor_type,
                                 actor_uuid, actor_alias, created_at)
SELECT DISTINCT ON (e.subscription_id)
       e.subscription_id, e.tenant_id, 'CANCEL',
       COALESCE((e.event_data -> 'CancelSubscription' ->> 'usage_cutoff')::timestamp,
                e.scheduled_time),
       e.event_data -> 'CancelSubscription' ->> 'reason',
       e.actor_type, e.actor_uuid, e.actor_alias,
       now() AT TIME ZONE 'UTC'
FROM scheduled_event e
JOIN subscription s ON s.id = e.subscription_id
WHERE e.event_type = 'CANCEL_SUBSCRIPTION'
  AND (e.status IN ('PENDING', 'PROCESSING', 'FAILED')
       OR (e.status = 'COMPLETED' AND s.status = 'CANCELLED'))
ORDER BY e.subscription_id, (e.status <> 'COMPLETED') DESC, e.created_at DESC;

INSERT INTO subscription_ending (subscription_id, tenant_id, kind, at, reason, actor_type,
                                 actor_uuid, actor_alias, created_at)
SELECT s.id, s.tenant_id, 'COMPLETE', s.end_date::timestamp, NULL, 'SYSTEM', NULL, NULL,
       now() AT TIME ZONE 'UTC'
FROM subscription s
WHERE s.end_date IS NOT NULL;

-- ── Plan windows ──────────────────────────────────────────────────────────────────────────────
CREATE TABLE subscription_plan_window (
    id              UUID                PRIMARY KEY,
    tenant_id       UUID                NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    subscription_id UUID                NOT NULL REFERENCES subscription (id) ON DELETE CASCADE,
    plan_version_id UUID                NOT NULL,
    from_date       DATE                NOT NULL,
    to_date         DATE,
    cadence         "BillingPeriodEnum" NOT NULL,
    -- Where the grid of this window's cadence starts (R-084).
    cadence_origin  DATE                NOT NULL,
    created_at      TIMESTAMP           NOT NULL,
    CONSTRAINT subscription_plan_window_ends_after_it_starts
        CHECK (to_date IS NULL OR to_date > from_date)
);

CREATE UNIQUE INDEX subscription_plan_window_open
    ON subscription_plan_window (subscription_id) WHERE to_date IS NULL;
CREATE INDEX subscription_plan_window_subscription
    ON subscription_plan_window (subscription_id, from_date);

-- Windows whose origin the rule decides: a switch that changed the cadence and recorded none.
-- The controller's sweep decides them (facts::decide_pending_origins) and empties this table.
CREATE TABLE subscription_plan_window_origin_pending (
    window_id UUID PRIMARY KEY REFERENCES subscription_plan_window (id) ON DELETE CASCADE
);

CREATE TEMP TABLE plan_window_draft ON COMMIT DROP AS
WITH switches AS (
    SELECT e.subscription_id, e.applies_to, e.created_at,
           (e.details ->> 'new_plan_version_id')::uuid AS new_version,
           (e.details ->> 'cadence_origin')::date      AS recorded
    FROM subscription_event e
    WHERE e.event_type = 'SWITCH'
      AND e.details ->> 'new_plan_version_id' IS NOT NULL
      AND e.details ->> 'old_plan_version_id' IS DISTINCT FROM e.details ->> 'new_plan_version_id'
),
bounds AS (
    SELECT s.id AS subscription_id, 0::bigint AS n, s.start_date AS from_date,
           COALESCE(f.old_version, s.plan_version_id) AS plan_version_id, NULL::date AS recorded
    FROM subscription s
    LEFT JOIN first_plan_switch f ON f.subscription_id = s.id
    UNION ALL
    SELECT w.subscription_id,
           row_number() OVER (PARTITION BY w.subscription_id ORDER BY w.applies_to, w.created_at),
           GREATEST(w.applies_to, s.start_date), w.new_version, w.recorded
    FROM switches w
    JOIN subscription s ON s.id = w.subscription_id
),
windows AS (
    SELECT b.*, lead(b.from_date) OVER (PARTITION BY b.subscription_id ORDER BY b.n) AS to_date
    FROM bounds b
)
SELECT gen_random_uuid() AS id, s.tenant_id, w.subscription_id, w.n,
       -- The open window is the subscription's plan; a switch naming another is not trusted.
       CASE WHEN w.to_date IS NULL THEN s.plan_version_id ELSE w.plan_version_id END
           AS plan_version_id,
       w.from_date, w.to_date,
       CASE WHEN w.to_date IS NULL THEN s.period
            ELSE COALESCE(
                (SELECT c.period::text::"BillingPeriodEnum"
                 FROM subscription_component c
                 WHERE c.subscription_id = w.subscription_id
                   AND c.period <> 'ONE_TIME'
                   AND c.effective_from <= w.from_date
                   AND (c.effective_to IS NULL OR c.effective_to > w.from_date)
                 ORDER BY CASE c.period WHEN 'MONTHLY' THEN 1 WHEN 'QUARTERLY' THEN 3
                                        WHEN 'SEMIANNUAL' THEN 6 ELSE 12 END
                 LIMIT 1),
                s.period) END AS cadence,
       -- The first window's grid starts at the billing start, after a free trial (R-084).
       CASE WHEN w.n = 0 THEN
                CASE WHEN t.free
                     THEN GREATEST(t.until_date, COALESCE(s.billing_start_date, s.start_date))
                     ELSE COALESCE(s.billing_start_date, s.start_date) END
            ELSE w.recorded END AS cadence_origin
FROM windows w
JOIN subscription s ON s.id = w.subscription_id
LEFT JOIN subscription_trial t ON t.subscription_id = w.subscription_id
WHERE w.to_date IS NULL OR w.to_date > w.from_date;

-- A switch keeping the cadence keeps its grid.
DO $$
BEGIN
    LOOP
        UPDATE plan_window_draft w
        SET cadence_origin = p.cadence_origin
        FROM plan_window_draft p
        WHERE w.cadence_origin IS NULL
          AND p.subscription_id = w.subscription_id
          AND p.to_date = w.from_date
          AND p.cadence = w.cadence
          AND p.cadence_origin IS NOT NULL;
        EXIT WHEN NOT FOUND;
    END LOOP;
END $$;

INSERT INTO subscription_plan_window (id, tenant_id, subscription_id, plan_version_id, from_date,
                                      to_date, cadence, cadence_origin, created_at)
SELECT id, tenant_id, subscription_id, plan_version_id, from_date, to_date, cadence,
       COALESCE(cadence_origin, from_date), now() AT TIME ZONE 'UTC'
FROM plan_window_draft;

INSERT INTO subscription_plan_window_origin_pending (window_id)
SELECT id FROM plan_window_draft WHERE cadence_origin IS NULL;

-- ── Coupons ───────────────────────────────────────────────────────────────────────────────────
-- The default serves the previous release during a rolling deploy; drop it after.
ALTER TABLE applied_coupon ADD COLUMN effective_from DATE;

-- A coupon applied with the subscription applies from its start, a later one from its day.
UPDATE applied_coupon ac
SET effective_from = CASE WHEN ac.created_at <= s.created_at + interval '1 minute'
                          THEN s.start_date
                          ELSE ac.created_at::date END
FROM subscription s
WHERE s.id = ac.subscription_id;

UPDATE applied_coupon SET effective_from = created_at::date WHERE effective_from IS NULL;

ALTER TABLE applied_coupon
    ALTER COLUMN effective_from SET NOT NULL,
    ALTER COLUMN effective_from SET DEFAULT CURRENT_DATE;

-- ── Dated minimums ────────────────────────────────────────────────────────────────────────────
-- A minimum is closed on a change instead of replaced; the open row is the one in force.
ALTER TABLE subscription_adjustment_component
    DROP CONSTRAINT subscription_adjustment_component_subscription_id_kind_fkey;

ALTER TABLE subscription_adjustment
    ADD COLUMN effective_from DATE,
    ADD COLUMN effective_to   DATE;

UPDATE subscription_adjustment a
SET effective_from = CASE WHEN a.created_at <= s.created_at + interval '1 minute'
                          THEN s.start_date
                          ELSE a.created_at::date END
FROM subscription s
WHERE s.id = a.subscription_id;

-- The default serves the previous release during a rolling deploy; drop it after.
ALTER TABLE subscription_adjustment
    ALTER COLUMN effective_from SET NOT NULL,
    ALTER COLUMN effective_from SET DEFAULT CURRENT_DATE,
    DROP CONSTRAINT subscription_adjustment_pkey,
    ADD PRIMARY KEY (subscription_id, kind, effective_from),
    ADD CONSTRAINT subscription_adjustment_ends_after_it_starts
        CHECK (effective_to IS NULL OR effective_to > effective_from);

CREATE UNIQUE INDEX subscription_adjustment_open
    ON subscription_adjustment (subscription_id, kind) WHERE effective_to IS NULL;

ALTER TABLE subscription_adjustment_component ADD COLUMN effective_from DATE;

UPDATE subscription_adjustment_component c
SET effective_from = a.effective_from
FROM subscription_adjustment a
WHERE a.subscription_id = c.subscription_id AND a.kind = c.kind;

DELETE FROM subscription_adjustment_component WHERE effective_from IS NULL;

ALTER TABLE subscription_adjustment_component
    ALTER COLUMN effective_from SET NOT NULL,
    DROP CONSTRAINT subscription_adjustment_component_pkey,
    ADD PRIMARY KEY (subscription_id, kind, effective_from, product_id),
    ADD CONSTRAINT subscription_adjustment_component_of_its_minimum
        FOREIGN KEY (subscription_id, kind, effective_from)
            REFERENCES subscription_adjustment (subscription_id, kind, effective_from)
            ON DELETE CASCADE;

-- The previous release inserts a minimum's scope without its date during a rolling deploy: it
-- belongs to the open minimum. Drop it with the defaults above.
CREATE FUNCTION subscription_adjustment_component_of_open_minimum() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    SELECT a.effective_from INTO NEW.effective_from
    FROM subscription_adjustment a
    WHERE a.subscription_id = NEW.subscription_id AND a.kind = NEW.kind
      AND a.effective_to IS NULL;
    RETURN NEW;
END $$;

CREATE TRIGGER trg_subscription_adjustment_component_of_open_minimum
    BEFORE INSERT ON subscription_adjustment_component
    FOR EACH ROW
    WHEN (NEW.effective_from IS NULL)
    EXECUTE FUNCTION subscription_adjustment_component_of_open_minimum();

-- ── Invoice deadlines ─────────────────────────────────────────────────────────────────────────
-- Written next to the FinalizeInvoice and RetryPayment events they replace; nothing reads them yet.
ALTER TABLE invoice
    ADD COLUMN finalize_after          TIMESTAMP,
    ADD COLUMN next_payment_attempt_at TIMESTAMP;

UPDATE invoice i
SET finalize_after = e.scheduled_time
FROM (SELECT DISTINCT ON ((event_data -> 'FinalizeInvoice' ->> 'invoice_id')::uuid)
             (event_data -> 'FinalizeInvoice' ->> 'invoice_id')::uuid AS invoice_id,
             scheduled_time
      FROM scheduled_event
      WHERE event_type = 'FINALIZE_INVOICE' AND status IN ('PENDING', 'PROCESSING', 'FAILED')
      ORDER BY (event_data -> 'FinalizeInvoice' ->> 'invoice_id')::uuid, scheduled_time DESC) e
WHERE i.id = e.invoice_id;

UPDATE invoice i
SET next_payment_attempt_at = e.scheduled_time
FROM (SELECT DISTINCT ON ((event_data -> 'RetryPayment' ->> 'invoice_id')::uuid)
             (event_data -> 'RetryPayment' ->> 'invoice_id')::uuid AS invoice_id,
             scheduled_time
      FROM scheduled_event
      WHERE event_type = 'RETRY_PAYMENT' AND status IN ('PENDING', 'PROCESSING', 'FAILED')
      ORDER BY (event_data -> 'RetryPayment' ->> 'invoice_id')::uuid, scheduled_time DESC) e
WHERE i.id = e.invoice_id;
