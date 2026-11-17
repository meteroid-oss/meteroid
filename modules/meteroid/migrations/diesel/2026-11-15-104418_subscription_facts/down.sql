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
