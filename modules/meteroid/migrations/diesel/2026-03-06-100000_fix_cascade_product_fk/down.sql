ALTER TABLE subscription_component DROP CONSTRAINT subscription_component_product_item_id_fkey;
ALTER TABLE subscription_component ADD CONSTRAINT subscription_component_product_item_id_fkey
    FOREIGN KEY (product_id) REFERENCES product(id) ON DELETE CASCADE;

ALTER TABLE subscription_component DROP CONSTRAINT subscription_component_price_component_id_fkey;
ALTER TABLE subscription_component ADD CONSTRAINT subscription_component_price_component_id_fkey
    FOREIGN KEY (price_component_id) REFERENCES price_component(id) ON DELETE CASCADE;

ALTER TABLE quote_component DROP CONSTRAINT quote_component_product_id_fkey;
ALTER TABLE quote_component ADD CONSTRAINT quote_component_product_id_fkey
    FOREIGN KEY (product_id) REFERENCES product(id) ON DELETE CASCADE;

ALTER TABLE quote_component DROP CONSTRAINT quote_component_price_component_id_fkey;
ALTER TABLE quote_component ADD CONSTRAINT quote_component_price_component_id_fkey
    FOREIGN KEY (price_component_id) REFERENCES price_component(id) ON DELETE CASCADE;

ALTER TABLE subscription_add_on DROP CONSTRAINT subscription_add_on_add_on_id_fkey;
ALTER TABLE subscription_add_on ADD CONSTRAINT subscription_add_on_add_on_id_fkey
    FOREIGN KEY (add_on_id) REFERENCES add_on(id) ON DELETE CASCADE;

ALTER TABLE quote_add_on DROP CONSTRAINT quote_add_on_add_on_id_fkey;
ALTER TABLE quote_add_on ADD CONSTRAINT quote_add_on_add_on_id_fkey
    FOREIGN KEY (add_on_id) REFERENCES add_on(id) ON DELETE CASCADE;

ALTER TABLE quote_coupon DROP CONSTRAINT quote_coupon_coupon_id_fkey;
ALTER TABLE quote_coupon ADD CONSTRAINT quote_coupon_coupon_id_fkey
    FOREIGN KEY (coupon_id) REFERENCES coupon(id) ON DELETE CASCADE;

ALTER TABLE subscription_event DROP CONSTRAINT subscription_event_bi_mrr_movement_log_id_fkey;
ALTER TABLE subscription_event ADD CONSTRAINT subscription_event_bi_mrr_movement_log_id_fkey
    FOREIGN KEY (bi_mrr_movement_log_id) REFERENCES bi_mrr_movement_log(id) ON DELETE CASCADE;
