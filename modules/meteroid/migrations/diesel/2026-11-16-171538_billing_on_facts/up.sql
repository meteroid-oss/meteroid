-- The release billing from dated facts, its additions and backfills: nothing here is undone by a
-- later migration. The changes still pending as scheduled events are written as facts after it,
-- and `billing_legacy_dropped` retires what the facts replace; the migrations run all three in one
-- transaction, under one lock of the tables the backfills read.
SET LOCAL lock_timeout = '5s';

-- ── Paused subscriptions of live tenants ─────────────────────────────────────────────────────
-- No fact gives PAUSED or SUSPENDED: `billing_legacy_dropped` makes an activated one ACTIVE, billed
-- again. A live tenant's (ACTIVE, in PRODUCTION or DEVELOPMENT) is never resumed by a deploy: the
-- release fails, listing them, to be cancelled or resumed first.
DO $$
DECLARE
    paused TEXT;
BEGIN
    SELECT string_agg(s.id::text || ' (tenant ' || s.tenant_id || ', ' || s.status || ')', ', '
                      ORDER BY s.tenant_id, s.id)
    INTO paused
    FROM subscription s
    JOIN tenant t ON t.id = s.tenant_id
    WHERE s.status IN ('PAUSED', 'SUSPENDED')
      AND t.status = 'ACTIVE' AND t.environment IN ('PRODUCTION', 'DEVELOPMENT');
    IF paused IS NOT NULL THEN
        RAISE EXCEPTION 'Paused or suspended subscriptions of live tenants (active, in production or development) would be resumed by this version: %. Nothing was changed: on your previous version, cancel or resume each of them, then upgrade again', paused;
    END IF;
END $$;

-- ── Credit note origin: corrections ──────────────────────────────────────────────────────────
-- An operator's credit note that corrects what was billed (days refunded by hand): reconcile counts
-- it like the engine's, so a later change in the period never refunds those days again.
ALTER TYPE "CreditNoteOriginEnum" ADD VALUE IF NOT EXISTS 'CORRECTION';

-- ── Billing controller ───────────────────────────────────────────────────────────────────────
-- One writer per customer: the controller runs a customer's billing steps when its
-- next wake is due or an input changed (dirty), under a lease so replicas never run it twice.
CREATE TABLE billing_controller (
    tenant_id    UUID        NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    customer_id  UUID        NOT NULL REFERENCES customer (id) ON DELETE CASCADE,
    next_wake_at TIMESTAMPTZ,
    dirty_since  TIMESTAMPTZ,
    lease_until  TIMESTAMPTZ,
    attempts     INTEGER     NOT NULL DEFAULT 0,
    last_error   TEXT,
    -- The first and the latest of the failed runs in a row, cleared with them.
    first_failed_at TIMESTAMPTZ,
    last_failed_at  TIMESTAMPTZ,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, customer_id)
);

CREATE INDEX billing_controller_wake ON billing_controller (next_wake_at)
    WHERE next_wake_at IS NOT NULL;
CREATE INDEX billing_controller_dirty ON billing_controller (dirty_since)
    WHERE dirty_since IS NOT NULL;
CREATE INDEX billing_controller_customer ON billing_controller (customer_id);
CREATE INDEX billing_controller_due ON billing_controller (LEAST(next_wake_at, dirty_since));

-- Tenants the billing controller runs when the release is held to an allowlist
-- (BILLING_CONTROLLER_RELEASE=allowlist): a row with no tenant releases every tenant. Unused in
-- the default mode, where every tenant runs.
CREATE TABLE billing_controller_release (
    tenant_id   UUID UNIQUE REFERENCES tenant (id) ON DELETE CASCADE,
    released_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    released_by TEXT
);
CREATE UNIQUE INDEX billing_controller_release_all ON billing_controller_release ((1))
    WHERE tenant_id IS NULL;

-- Every customer with a subscription or credit still holding value runs once, which computes its
-- first wake.
INSERT INTO billing_controller (tenant_id, customer_id, dirty_since)
SELECT tenant_id, customer_id, now() FROM subscription
UNION
SELECT tenant_id, customer_id, now() FROM customer_credit_grant WHERE remaining > 0
ON CONFLICT DO NOTHING;

-- ── Subscription facts ───────────────────────────────────────────────────────────────────────
-- A subscription's lifecycle recorded as dated facts, written by the paths that act
-- next to the events and state that still drive billing. Nothing bills from them yet.

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

-- A switch's details name plan versions as UUIDs or, since typed ids, in their public form:
-- `plv_` and the base62 of the UUID's bits rotated left by 67. div() and mod() stay exact where
-- `/` rounds a 128-bit quotient.
CREATE FUNCTION pg_temp.plan_version_of(v TEXT) RETURNS UUID LANGUAGE sql IMMUTABLE AS $$
SELECT CASE
    WHEN v IS NULL THEN NULL
    WHEN v ~* '^[0-9a-f]{8}-?[0-9a-f]{4}-?[0-9a-f]{4}-?[0-9a-f]{4}-?[0-9a-f]{12}$' THEN v::uuid
    ELSE (SELECT string_agg(substr('0123456789abcdef',
                                   mod(div(r, power(16::numeric, 31 - j)), 16)::int + 1, 1),
                            '' ORDER BY j)
          FROM generate_series(0, 31) j,
               (SELECT mod(x, power(2::numeric, 67)) * power(2::numeric, 61)
                           + div(x, power(2::numeric, 67)) AS r
                FROM (SELECT sum((strpos('0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz',
                                         substr(split_part(v, '_', 2), i, 1)) - 1)::numeric
                                 * power(62::numeric, length(split_part(v, '_', 2)) - i)) AS x
                      FROM generate_series(1, length(split_part(v, '_', 2))) i) n) rotated)::uuid
END
$$;

-- The plan version a subscription started on: its first switch's old one, else, for a switch that
-- recorded only its new version, the version of the components it closed other than that day's new
-- versions, else its current one.
CREATE TEMP TABLE first_plan_switch ON COMMIT DROP AS
SELECT DISTINCT ON (e.subscription_id)
       e.subscription_id,
       e.applies_to,
       COALESCE(
           pg_temp.plan_version_of(e.details ->> 'old_plan_version_id'),
           (SELECT pc.plan_version_id
            FROM subscription_component c
            JOIN price_component pc ON pc.id = c.price_component_id
            WHERE c.subscription_id = e.subscription_id
              AND c.effective_to = e.applies_to
              AND pc.plan_version_id NOT IN (
                  SELECT pg_temp.plan_version_of(o.details ->> 'new_plan_version_id')
                  FROM subscription_event o
                  WHERE o.subscription_id = e.subscription_id AND o.event_type = 'SWITCH'
                    AND o.applies_to = e.applies_to
                    AND o.details ->> 'new_plan_version_id' IS NOT NULL)
            ORDER BY c.effective_from
            LIMIT 1)) AS old_version
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
-- What the subscription did decides freeness, never the version's flag edited since (R-100): a
-- change moving the billing start ended a free trial; an invoice on the trial's first day billed a
-- paid one; billing first from its end, or waiting to end it, is free.
CROSS JOIN LATERAL (
    SELECT CASE
               WHEN s.billing_start_date = f.applies_to AND s.billing_start_date > s.start_date
                   THEN TRUE
               WHEN EXISTS (SELECT 1 FROM invoice i
                            WHERE i.subscription_id = s.id
                              AND i.invoice_type = 'RECURRING'
                              AND i.invoice_date = COALESCE(s.billing_start_date, s.start_date))
                   THEN FALSE
               WHEN EXISTS (SELECT 1 FROM invoice i
                            WHERE i.subscription_id = s.id
                              AND i.invoice_type = 'RECURRING'
                              AND i.invoice_date >= COALESCE(s.billing_start_date, s.start_date)
                                                    + s.trial_duration)
                   THEN TRUE
               WHEN s.next_cycle_action = 'END_TRIAL' THEN TRUE
               ELSE COALESCE(pv.trial_is_free, TRUE) END AS free
) d
CROSS JOIN LATERAL (
    SELECT d.free,
           CASE WHEN d.free
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
    -- When the ending was decided: a cancellation requested after its day is backdated (R-016),
    -- whenever its row was written.
    requested_at    TIMESTAMP        NOT NULL,
    PRIMARY KEY (subscription_id, kind),
    CONSTRAINT subscription_ending_kind CHECK (kind IN ('CANCEL', 'COMPLETE'))
);

-- The cancellation still to apply (pending, on a live subscription), else the one a cancelled
-- subscription ended by. A failed or cancelled event was dead on the previous release: no ending.
-- Requested when its event was created (never null on main), never when this migration runs.
INSERT INTO subscription_ending (subscription_id, tenant_id, kind, at, reason, actor_type,
                                 actor_uuid, actor_alias, created_at, requested_at)
SELECT DISTINCT ON (e.subscription_id)
       e.subscription_id, e.tenant_id, 'CANCEL',
       COALESCE((e.event_data -> 'CancelSubscription' ->> 'usage_cutoff')::timestamp,
                e.scheduled_time),
       e.event_data -> 'CancelSubscription' ->> 'reason',
       e.actor_type, e.actor_uuid, e.actor_alias,
       now() AT TIME ZONE 'UTC', e.created_at
FROM scheduled_event e
JOIN subscription s ON s.id = e.subscription_id
WHERE e.event_type = 'CANCEL_SUBSCRIPTION'
  AND ((e.status IN ('PENDING', 'PROCESSING')
        AND s.status::text NOT IN ('CANCELLED', 'COMPLETED', 'ABORTED', 'SUPERSEDED'))
       OR (e.status = 'COMPLETED' AND s.status = 'CANCELLED'))
ORDER BY e.subscription_id, (e.status <> 'COMPLETED') DESC, e.created_at DESC;

INSERT INTO subscription_ending (subscription_id, tenant_id, kind, at, reason, actor_type,
                                 actor_uuid, actor_alias, created_at, requested_at)
SELECT s.id, s.tenant_id, 'COMPLETE', s.end_date::timestamp, NULL, 'SYSTEM', NULL, NULL,
       now() AT TIME ZONE 'UTC', LEAST(s.created_at, s.end_date::timestamp)
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
           pg_temp.plan_version_of(e.details ->> 'new_plan_version_id') AS new_version,
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
-- The FinalizeInvoice and RetryPayment events still pending, carried by their invoice while it
-- still waits for them: a draft its grace, an unpaid finalized invoice its attempt. A failed,
-- completed or cancelled event was dead on the previous release and stays so.
ALTER TABLE invoice
    ADD COLUMN finalize_after          TIMESTAMP,
    ADD COLUMN next_payment_attempt_at TIMESTAMP;

UPDATE invoice i
SET finalize_after = e.scheduled_time
FROM (SELECT DISTINCT ON ((event_data -> 'FinalizeInvoice' ->> 'invoice_id')::uuid)
             (event_data -> 'FinalizeInvoice' ->> 'invoice_id')::uuid AS invoice_id,
             COALESCE(next_attempt_at, scheduled_time) AS scheduled_time
      FROM scheduled_event
      WHERE event_type = 'FINALIZE_INVOICE' AND status IN ('PENDING', 'PROCESSING')
      ORDER BY (event_data -> 'FinalizeInvoice' ->> 'invoice_id')::uuid, scheduled_time DESC) e
WHERE i.id = e.invoice_id AND i.status = 'DRAFT';

UPDATE invoice i
SET next_payment_attempt_at = e.scheduled_time
FROM (SELECT DISTINCT ON ((event_data -> 'RetryPayment' ->> 'invoice_id')::uuid)
             (event_data -> 'RetryPayment' ->> 'invoice_id')::uuid AS invoice_id,
             COALESCE(next_attempt_at, scheduled_time) AS scheduled_time
      FROM scheduled_event
      WHERE event_type = 'RETRY_PAYMENT' AND status IN ('PENDING', 'PROCESSING')
      ORDER BY (event_data -> 'RetryPayment' ->> 'invoice_id')::uuid, scheduled_time DESC) e
WHERE i.id = e.invoice_id AND i.status = 'FINALIZED' AND i.payment_status <> 'PAID';

-- ── Reconcile watermark ──────────────────────────────────────────────────────────────────────
-- The reconcile watermark. A period ending on or before `reconcile_from` was settled by the
-- per-operation code: billing from facts never evaluates it again, and an issued line starting
-- before it is never billed against. NULL: every period is billed from facts.

ALTER TABLE subscription ADD COLUMN reconcile_from DATE;

-- The start of the latest period whose recurring invoice is issued (closed included) with no draft
-- before it: every period ending by then was billed. A draft still open is billed from facts. A
-- consolidated child is issued with its parent, and is no draft once merged.
UPDATE subscription s
SET reconcile_from = w.settled
FROM (SELECT i.subscription_id, max(i.billing_period_start) AS settled
      FROM invoice i
      LEFT JOIN invoice p ON p.id = i.consolidated_into_invoice_id
      WHERE i.invoice_type = 'RECURRING'
        AND i.subscription_id IS NOT NULL
        AND CASE WHEN i.consolidated_into_invoice_id IS NULL
                 THEN i.status IN ('FINALIZED', 'UNCOLLECTIBLE', 'CLOSED')
                 ELSE p.status IN ('FINALIZED', 'UNCOLLECTIBLE', 'CLOSED') END
        AND NOT EXISTS (SELECT 1 FROM invoice d
                        WHERE d.subscription_id = i.subscription_id
                          AND d.invoice_type = 'RECURRING'
                          AND d.status = 'DRAFT'
                          AND d.consolidated_into_invoice_id IS NULL
                          AND d.billing_period_start <= i.billing_period_start)
      GROUP BY i.subscription_id) w
WHERE w.subscription_id = s.id;

-- ── Price lineage ────────────────────────────────────────────────────────────────────────────
-- A price's rates change only from a date, as a successor in its lineage (R-097): the row a
-- component, add-on or subscription references is the lineage's root, never rewritten; each
-- successor names it and the day its rates take over.

ALTER TABLE price
    ADD COLUMN effective_from DATE,
    ADD COLUMN supersedes     UUID REFERENCES price (id) ON DELETE CASCADE,
    ADD CONSTRAINT price_successor_is_dated CHECK ((supersedes IS NULL) = (effective_from IS NULL));

CREATE INDEX price_supersedes_idx ON price (supersedes) WHERE supersedes IS NOT NULL;

-- Every writer goes through a successor: a rate rewritten in place would re-rate every day billed
-- from that price, issued ones included, so the database refuses it.
CREATE FUNCTION price_rates_are_dated() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'price % rates cannot be edited in place: add a new price effective from a date instead', OLD.id
        USING ERRCODE = 'check_violation';
END $$;

CREATE TRIGGER trg_price_rates_are_dated
    BEFORE UPDATE OF pricing, cadence, currency, credit_unit_id, credit_unit_rate ON price
    FOR EACH ROW
    WHEN (OLD.pricing IS DISTINCT FROM NEW.pricing
          OR OLD.cadence IS DISTINCT FROM NEW.cadence
          OR OLD.currency IS DISTINCT FROM NEW.currency
          OR OLD.credit_unit_id IS DISTINCT FROM NEW.credit_unit_id
          OR OLD.credit_unit_rate IS DISTINCT FROM NEW.credit_unit_rate)
    EXECUTE FUNCTION price_rates_are_dated();

-- ── Next wake ────────────────────────────────────────────────────────────────────────────────
-- When the facts next change a subscription's derived state (a plan window
-- opening, a boundary, a trial end, an ending), written by `lifecycle::derived::derive_state_tx`.
-- The billing controller wakes the customer then. NULL: nothing to derive until a fact changes.

ALTER TABLE subscription ADD COLUMN next_wake_at TIMESTAMPTZ;

CREATE INDEX subscription_next_wake_at_idx ON subscription (tenant_id, customer_id, next_wake_at)
    WHERE next_wake_at IS NOT NULL;

-- ── Imports' watermark ───────────────────────────────────────────────────────────────────────
-- An import was billed elsewhere up to its import period, that period's advance included, and
-- owes that period's arrears and usage at its first renewal. `reconcile_arrears_from` is the
-- watermark of what a period bills at its end, earlier than `reconcile_from` for an import.
-- An import with no issued recurring invoice settles through its current period, or through
-- its first renewal's period when that is still a draft; one renewed on main keeps the
-- watermark of its issued invoices.

ALTER TABLE subscription ADD COLUMN reconcile_arrears_from DATE;

UPDATE subscription s
SET reconcile_from         = COALESCE(d.first_renewal, s.current_period_end),
    reconcile_arrears_from = CASE WHEN d.first_renewal IS NULL THEN s.current_period_start
                                  ELSE (s.imported_at - interval '1 day')::date END
FROM (SELECT s2.id,
             (SELECT min(i.billing_period_start) FROM invoice i
              WHERE i.subscription_id = s2.id AND i.invoice_type = 'RECURRING'
                AND i.status <> 'VOID') AS first_renewal
      FROM subscription s2
      WHERE s2.imported_at IS NOT NULL AND s2.reconcile_from IS NULL) d
WHERE d.id = s.id
  AND (d.first_renewal IS NOT NULL OR s.current_period_end IS NOT NULL);

-- ── Gaps at the cursor ───────────────────────────────────────────────────────────────────────
-- The previous release billed period after period and never went back. A live subscription whose
-- current period has no recurring invoice nor any open draft (its draft deleted, a period skipped)
-- owes nothing before that period nor its advance, and that period's arrears and usage at its end.
UPDATE subscription s
SET reconcile_from         = s.current_period_end,
    reconcile_arrears_from = s.current_period_start
WHERE s.imported_at IS NULL
  AND s.status = 'ACTIVE'
  AND s.current_period_end IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM invoice i
                  WHERE i.subscription_id = s.id
                    AND i.invoice_type = 'RECURRING'
                    AND i.billing_period_start = s.current_period_start)
  AND NOT EXISTS (SELECT 1 FROM invoice d
                  WHERE d.subscription_id = s.id
                    AND d.invoice_type = 'RECURRING'
                    AND d.status = 'DRAFT'
                    AND d.consolidated_into_invoice_id IS NULL);

-- ── Drafts never issued ──────────────────────────────────────────────────────────────────────
-- A live subscription the previous release only left empty drafts, on one cadence and with no
-- add-on, is billed from its first draft: no period of its grid before it was billed. A mix of
-- cadences keeps its history, a longer fee straddling that draft.
UPDATE subscription s
SET reconcile_from         = f.first_draft,
    reconcile_arrears_from = f.first_draft
FROM (SELECT d.subscription_id, min(d.billing_period_start) AS first_draft
      FROM invoice d
      WHERE d.invoice_type = 'RECURRING' AND d.status <> 'VOID'
      GROUP BY d.subscription_id) f
WHERE f.subscription_id = s.id
  AND s.reconcile_from IS NULL
  AND s.imported_at IS NULL
  AND s.status = 'ACTIVE'
  AND NOT EXISTS (SELECT 1 FROM invoice i
                  WHERE i.subscription_id = s.id
                    AND i.invoice_type = 'RECURRING'
                    AND i.status NOT IN ('DRAFT', 'VOID'))
  AND NOT EXISTS (SELECT 1 FROM invoice d
                  WHERE d.subscription_id = s.id
                    AND d.invoice_type = 'RECURRING'
                    AND d.status = 'DRAFT'
                    AND (d.total <> 0 OR jsonb_array_length(d.line_items) > 0
                         OR d.consolidated_into_invoice_id IS NOT NULL))
  AND NOT EXISTS (SELECT 1 FROM subscription_component c
                  WHERE c.subscription_id = s.id
                    AND c.period::text NOT IN (s.period::text, 'ONE_TIME'))
  AND NOT EXISTS (SELECT 1 FROM subscription_add_on a WHERE a.subscription_id = s.id);

-- ── Billing failures ─────────────────────────────────────────────────────────────────────────
-- A subscription whose billing step fails, backed off on its own (its customer's other work goes
-- on): the failures in a row, the first and the latest, the latest error and the retry. Cleared
-- by the next step that takes. Never a status: billing is retried forever.
ALTER TABLE subscription
    ADD COLUMN billing_failures        INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN billing_first_failed_at TIMESTAMPTZ,
    ADD COLUMN billing_last_failed_at  TIMESTAMPTZ,
    ADD COLUMN billing_last_error      TEXT,
    ADD COLUMN billing_retry_at        TIMESTAMPTZ;

CREATE INDEX subscription_billing_failing_idx ON subscription (tenant_id, billing_last_failed_at)
    WHERE billing_failures > 0;

-- ── Seat upgrade lines ───────────────────────────────────────────────────────────────────────
-- Seat purchases the previous release invoiced name their price component but no fee row, so
-- reconcile reads their seats as unbilled. In periods still open, each such line names the seat
-- row of its subscription in force on its start.
WITH attached AS (
    SELECT DISTINCT ON (i.id, e.ord) i.id AS invoice_id, e.ord, c.id AS component_id
    FROM invoice i
    CROSS JOIN LATERAL jsonb_array_elements(i.line_items) WITH ORDINALITY AS e(l, ord)
    JOIN subscription_component c
      ON c.subscription_id = coalesce((e.l ->> 'subscription_id')::uuid, i.subscription_id)
     AND c.price_component_id = (e.l ->> 'price_component_id')::uuid
     AND c.effective_from <= (e.l ->> 'start_date')::date
     AND (c.effective_to IS NULL OR c.effective_to > (e.l ->> 'start_date')::date)
    JOIN product p ON p.id = c.product_id AND p.fee_type = 'SLOT'
    WHERE e.l ->> 'sub_component_id' IS NULL
      AND e.l ->> 'sub_add_on_id' IS NULL
      AND coalesce(e.l ->> 'kind', 'Regular') = 'Regular'
      AND (e.l ->> 'end_date')::date >= CURRENT_DATE
    ORDER BY i.id, e.ord, c.effective_from DESC
)
UPDATE invoice i
SET line_items = (SELECT jsonb_agg(CASE WHEN a.component_id IS NULL THEN e.l
                                        ELSE e.l || jsonb_build_object('sub_component_id', a.component_id)
                                   END ORDER BY e.ord)
                  FROM jsonb_array_elements(i.line_items) WITH ORDINALITY AS e(l, ord)
                  LEFT JOIN attached a ON a.invoice_id = i.id AND a.ord = e.ord)
WHERE i.id IN (SELECT invoice_id FROM attached);

-- ── Checkout session coupons before a payment ────────────────────────────────────────────────
-- Named for what they hold: a session's coupons before a payment attempt changed them. A pod of the
-- release before reads and writes the session by the old names, so its checkout fails at its
-- first read of the session, before charging a card for a subscription it could no longer record.
ALTER TABLE checkout_session RENAME COLUMN pre_payment_coupon_code TO coupon_code_before_payment;
ALTER TABLE checkout_session RENAME COLUMN pre_payment_coupon_ids TO coupon_ids_before_payment;

-- ── Subscription changes ─────────────────────────────────────────────────────────────────────
-- A plan change or amendment still to come: its identity (the API withdraws it by id), its
-- request and the rows it wrote as facts (withdrawing it removes them; a Switch row naming it marks
-- it applied). The changes the previous release left pending are written here, ids kept.
CREATE TYPE "SubscriptionChangeKindEnum" AS ENUM ('PLAN_CHANGE', 'AMENDMENT');

CREATE TABLE subscription_change (
    id                  UUID                         PRIMARY KEY,
    tenant_id           UUID                         NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    subscription_id     UUID                         NOT NULL REFERENCES subscription (id) ON DELETE CASCADE,
    kind                "SubscriptionChangeKindEnum" NOT NULL,
    effective_at        TIMESTAMP                    NOT NULL,
    -- The request, in its ScheduledEventData form ({"ApplyPlanChange": ...}).
    change              JSONB                        NOT NULL,
    -- The rows it wrote as facts (`facts::pending::WrittenFacts`).
    written_facts       JSONB,
    withdrawn_at        TIMESTAMP,
    withdrawn_reason    TEXT,
    source              TEXT                         NOT NULL,
    created_by_customer BOOLEAN                      NOT NULL DEFAULT FALSE,
    actor_type          "ActorTypeEnum"              NOT NULL DEFAULT 'SYSTEM',
    actor_uuid          UUID,
    actor_alias         TEXT,
    created_at          TIMESTAMP                    NOT NULL,
    updated_at          TIMESTAMP                    NOT NULL
);
CREATE INDEX idx_subscription_change_subscription_id ON subscription_change (subscription_id);
CREATE INDEX idx_subscription_change_tenant_id ON subscription_change (tenant_id);

-- A change whose facts are written and whose documents are not: the billing controller settles
-- it (its adjustment invoice and notes) when the facts make that day's adjustment due, then
-- deletes the row. Written for a change the previous release left pending inside a period.
CREATE TABLE subscription_settlement_due (
    subscription_id UUID                         NOT NULL REFERENCES subscription (id) ON DELETE CASCADE,
    effective_on    DATE                         NOT NULL,
    tenant_id       UUID                         NOT NULL REFERENCES tenant (id) ON DELETE CASCADE,
    kind            "SubscriptionChangeKindEnum" NOT NULL,
    created_at      TIMESTAMPTZ                  NOT NULL DEFAULT now(),
    PRIMARY KEY (subscription_id, effective_on)
);
CREATE INDEX idx_subscription_settlement_due_tenant_id ON subscription_settlement_due (tenant_id);

-- A pending cancellation has an identity of its own.
ALTER TABLE subscription_ending ADD COLUMN id UUID NOT NULL DEFAULT gen_random_uuid();
ALTER TABLE subscription_ending ADD CONSTRAINT subscription_ending_id_key UNIQUE (id);
