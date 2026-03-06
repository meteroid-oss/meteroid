-- Fix ON DELETE CASCADE → RESTRICT on billing history tables.
-- CASCADE was wrong: parent deletion must not destroy billing/quote snapshots.
-- Use soft-deletion at the application layer instead of hard deletes.

-- subscription_component: protect product_id and price_component_id references
ALTER TABLE subscription_component DROP CONSTRAINT subscription_component_product_item_id_fkey;
ALTER TABLE subscription_component ADD CONSTRAINT subscription_component_product_item_id_fkey
    FOREIGN KEY (product_id) REFERENCES product(id) ON DELETE RESTRICT;

ALTER TABLE subscription_component DROP CONSTRAINT subscription_component_price_component_id_fkey;
ALTER TABLE subscription_component ADD CONSTRAINT subscription_component_price_component_id_fkey
    FOREIGN KEY (price_component_id) REFERENCES price_component(id) ON DELETE RESTRICT;

-- quote_component: protect product_id and price_component_id references
ALTER TABLE quote_component DROP CONSTRAINT quote_component_product_id_fkey;
ALTER TABLE quote_component ADD CONSTRAINT quote_component_product_id_fkey
    FOREIGN KEY (product_id) REFERENCES product(id) ON DELETE RESTRICT;

ALTER TABLE quote_component DROP CONSTRAINT quote_component_price_component_id_fkey;
ALTER TABLE quote_component ADD CONSTRAINT quote_component_price_component_id_fkey
    FOREIGN KEY (price_component_id) REFERENCES price_component(id) ON DELETE RESTRICT;

-- subscription_add_on: protect add_on_id reference (subscription_id CASCADE is fine)
ALTER TABLE subscription_add_on DROP CONSTRAINT subscription_add_on_add_on_id_fkey;
ALTER TABLE subscription_add_on ADD CONSTRAINT subscription_add_on_add_on_id_fkey
    FOREIGN KEY (add_on_id) REFERENCES add_on(id) ON DELETE RESTRICT;

-- quote_add_on: protect add_on_id reference (quote_id CASCADE is fine)
ALTER TABLE quote_add_on DROP CONSTRAINT quote_add_on_add_on_id_fkey;
ALTER TABLE quote_add_on ADD CONSTRAINT quote_add_on_add_on_id_fkey
    FOREIGN KEY (add_on_id) REFERENCES add_on(id) ON DELETE RESTRICT;

-- quote_coupon: protect coupon_id reference (quote_id CASCADE is fine)
ALTER TABLE quote_coupon DROP CONSTRAINT quote_coupon_coupon_id_fkey;
ALTER TABLE quote_coupon ADD CONSTRAINT quote_coupon_coupon_id_fkey
    FOREIGN KEY (coupon_id) REFERENCES coupon(id) ON DELETE RESTRICT;

-- subscription_event: protect bi_mrr_movement_log_id (SET NULL, not CASCADE)
ALTER TABLE subscription_event DROP CONSTRAINT subscription_event_bi_mrr_movement_log_id_fkey;
ALTER TABLE subscription_event ADD CONSTRAINT subscription_event_bi_mrr_movement_log_id_fkey
    FOREIGN KEY (bi_mrr_movement_log_id) REFERENCES bi_mrr_movement_log(id) ON DELETE SET NULL;
