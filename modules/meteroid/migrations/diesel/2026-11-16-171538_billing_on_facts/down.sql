-- Reverses billing_on_facts. A change written as facts is withdrawn first: dropping its record
-- would leave the rows it wrote.
SET LOCAL lock_timeout = '5s';

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM subscription_change WHERE withdrawn_at IS NULL) THEN
        RAISE EXCEPTION 'subscription changes still to come exist; withdraw them first';
    END IF;
END $$;

ALTER TABLE subscription_ending DROP CONSTRAINT subscription_ending_id_key;
ALTER TABLE subscription_ending DROP COLUMN id;
DROP TABLE subscription_settlement_due;
DROP TABLE subscription_change;
DROP TYPE "SubscriptionChangeKindEnum";

-- ── Checkout session coupons before a payment ────────────────────────────────────────────────
ALTER TABLE checkout_session RENAME COLUMN coupon_code_before_payment TO pre_payment_coupon_code;
ALTER TABLE checkout_session RENAME COLUMN coupon_ids_before_payment TO pre_payment_coupon_ids;

-- ── Next wake ────────────────────────────────────────────────────────────────────────────────
DROP INDEX IF EXISTS subscription_next_wake_at_idx;
ALTER TABLE subscription DROP COLUMN next_wake_at;
ALTER TABLE subscription DROP COLUMN reconcile_arrears_from;
DROP INDEX IF EXISTS subscription_billing_failing_idx;
ALTER TABLE subscription
    DROP COLUMN billing_failures,
    DROP COLUMN billing_first_failed_at,
    DROP COLUMN billing_last_failed_at,
    DROP COLUMN billing_last_error,
    DROP COLUMN billing_retry_at;

-- ── Price lineage ────────────────────────────────────────────────────────────────────────────
-- A dated successor holds the rates subscriptions bill from its date: refuse while one exists.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM price WHERE supersedes IS NOT NULL) THEN
        RAISE EXCEPTION 'dated price successors exist; the previous release cannot bill them';
    END IF;
END $$;

DROP TRIGGER IF EXISTS trg_price_rates_are_dated ON price;
DROP FUNCTION IF EXISTS price_rates_are_dated();
DROP INDEX IF EXISTS price_supersedes_idx;
ALTER TABLE price
    DROP CONSTRAINT price_successor_is_dated,
    DROP COLUMN supersedes,
    DROP COLUMN effective_from;

-- ── Reconcile watermark ──────────────────────────────────────────────────────────────────────
ALTER TABLE subscription DROP COLUMN reconcile_from;

-- ── Subscription facts ───────────────────────────────────────────────────────────────────────
ALTER TABLE invoice
    DROP COLUMN next_payment_attempt_at,
    DROP COLUMN finalize_after;

DROP TRIGGER trg_subscription_adjustment_component_of_open_minimum ON subscription_adjustment_component;
DROP FUNCTION subscription_adjustment_component_of_open_minimum();

-- Only the minimum in force is kept.
DELETE FROM subscription_adjustment WHERE effective_to IS NOT NULL;

ALTER TABLE subscription_adjustment_component
    DROP CONSTRAINT subscription_adjustment_component_of_its_minimum,
    DROP CONSTRAINT subscription_adjustment_component_pkey,
    DROP COLUMN effective_from,
    ADD PRIMARY KEY (subscription_id, kind, product_id);

DROP INDEX subscription_adjustment_open;

ALTER TABLE subscription_adjustment
    DROP CONSTRAINT subscription_adjustment_ends_after_it_starts,
    DROP CONSTRAINT subscription_adjustment_pkey,
    DROP COLUMN effective_to,
    DROP COLUMN effective_from,
    ADD PRIMARY KEY (subscription_id, kind);

ALTER TABLE subscription_adjustment_component
    ADD CONSTRAINT subscription_adjustment_component_subscription_id_kind_fkey
        FOREIGN KEY (subscription_id, kind)
            REFERENCES subscription_adjustment (subscription_id, kind) ON DELETE CASCADE;

ALTER TABLE applied_coupon DROP COLUMN effective_from;

DROP TABLE subscription_plan_window_origin_pending;
DROP TABLE subscription_plan_window;
DROP TABLE subscription_ending;
DROP TABLE subscription_start_settled;
DROP TABLE subscription_trial;

-- ── Billing controller ───────────────────────────────────────────────────────────────────────
DROP TABLE billing_controller_release;
DROP TABLE billing_controller;

-- ── Credit note origin: corrections ──────────────────────────────────────────────────────────
-- Postgres cannot drop an enum value in place: notes marked as corrections become goodwill again.
UPDATE credit_note SET origin = 'MANUAL' WHERE origin::text = 'CORRECTION';
