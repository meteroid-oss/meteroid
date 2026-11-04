-- A price in credits has no column in the shipped schema: refused rather than repriced in money.
DO
$$
    BEGIN
        IF EXISTS (SELECT 1 FROM price WHERE credit_unit_id IS NOT NULL) THEN
            RAISE EXCEPTION 'prices in credit units exist; the shipped schema cannot represent them';
        END IF;
    END
$$;

-- The entitlement limit terms removed on upgrade did nothing: there is nothing to restore.
DROP INDEX entitlement_metered_limit_tenant_idx;
DROP INDEX subscription_live_customer_idx;

DROP TABLE promo_code_attempt;
ALTER TABLE customer_payment_method DROP COLUMN saved_for_payment_at;
ALTER TABLE checkout_session
    DROP COLUMN pre_payment_coupon_ids,
    DROP COLUMN pre_payment_coupon_code;

ALTER TABLE invoice DROP COLUMN delivery_held_at;

DROP TABLE payment_reversal_movement;
ALTER TABLE invoice DROP COLUMN payment_disputed_at;
ALTER TABLE payment_transaction
    DROP CONSTRAINT payment_transaction_reversal_split,
    DROP COLUMN refund_excess,
    DROP COLUMN amount_reversed_excess,
    DROP COLUMN amount_disputed;

DROP TABLE subscription_usage_filter;
ALTER TABLE subscription
    DROP COLUMN usage_filter_since,
    DROP COLUMN usage_filter_value,
    DROP COLUMN usage_filter_property;

ALTER TABLE scheduled_event DROP COLUMN next_attempt_at;

DROP INDEX idx_subscription_event_incomplete_cancellation;
DROP INDEX idx_subscription_event_incomplete_switch;
ALTER TABLE subscription_event
    DROP COLUMN outgoing_next_check_at,
    DROP COLUMN outgoing_checked_at;

DROP INDEX idx_credit_note_store_xid;
DROP INDEX idx_invoice_store_xid;
CREATE OR REPLACE FUNCTION set_store_committed_at() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    NEW.store_committed_at := clock_timestamp();
    RETURN NEW;
END $$;
DROP TRIGGER trg_credit_note_store_xid ON credit_note;
DROP TRIGGER trg_invoice_store_xid ON invoice;
DROP FUNCTION set_store_xid();
ALTER TABLE credit_note DROP COLUMN store_xid;
ALTER TABLE invoice DROP COLUMN store_xid;

-- The backfilled `voided_at` moments are kept: they record when the notes were voided.
ALTER TABLE credit_note DROP COLUMN voids_invoice;
ALTER TABLE invoice DROP COLUMN promotional_credits;

DROP TABLE sandbox_seed;
DROP TYPE "SandboxSeedStatusEnum";

ALTER TABLE billable_metric
    DROP COLUMN unit_plural,
    DROP COLUMN unit_singular;

ALTER TABLE price
    DROP CONSTRAINT price_credit_unit_fk,
    DROP CONSTRAINT price_credit_unit_rate_positive_check,
    DROP COLUMN credit_unit_rate,
    DROP COLUMN credit_unit_id;
