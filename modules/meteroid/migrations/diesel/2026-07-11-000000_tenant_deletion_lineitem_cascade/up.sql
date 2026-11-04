-- Let a whole-tenant/organization delete cascade cleanly.
--
-- quote_* and subscription_* line-item tables have no tenant_id of their own —
-- they are reached only through their quote/subscription parent. Their FKs to
-- catalog rows (coupon/product/price/price_component/add_on) were RESTRICT, which
-- is checked immediately (unlike NO ACTION), so the tenant cascade could delete a
-- catalog row before the line item and be blocked. A line item cannot outlive the
-- catalog row it references, so these become ON DELETE CASCADE.

ALTER TABLE public.quote_coupon DROP CONSTRAINT quote_coupon_coupon_id_fkey;
ALTER TABLE public.quote_coupon ADD CONSTRAINT quote_coupon_coupon_id_fkey FOREIGN KEY (coupon_id) REFERENCES coupon(id) ON DELETE CASCADE;

ALTER TABLE public.quote_component DROP CONSTRAINT quote_component_product_id_fkey;
ALTER TABLE public.quote_component ADD CONSTRAINT quote_component_product_id_fkey FOREIGN KEY (product_id) REFERENCES product(id) ON DELETE CASCADE;
ALTER TABLE public.quote_component DROP CONSTRAINT quote_component_price_component_id_fkey;
ALTER TABLE public.quote_component ADD CONSTRAINT quote_component_price_component_id_fkey FOREIGN KEY (price_component_id) REFERENCES price_component(id) ON DELETE CASCADE;
ALTER TABLE public.quote_component DROP CONSTRAINT quote_component_price_id_fkey;
ALTER TABLE public.quote_component ADD CONSTRAINT quote_component_price_id_fkey FOREIGN KEY (price_id) REFERENCES price(id) ON DELETE CASCADE;

ALTER TABLE public.quote_add_on DROP CONSTRAINT quote_add_on_add_on_id_fkey;
ALTER TABLE public.quote_add_on ADD CONSTRAINT quote_add_on_add_on_id_fkey FOREIGN KEY (add_on_id) REFERENCES add_on(id) ON DELETE CASCADE;
ALTER TABLE public.quote_add_on DROP CONSTRAINT quote_add_on_product_id_fkey;
ALTER TABLE public.quote_add_on ADD CONSTRAINT quote_add_on_product_id_fkey FOREIGN KEY (product_id) REFERENCES product(id) ON DELETE CASCADE;
ALTER TABLE public.quote_add_on DROP CONSTRAINT quote_add_on_price_id_fkey;
ALTER TABLE public.quote_add_on ADD CONSTRAINT quote_add_on_price_id_fkey FOREIGN KEY (price_id) REFERENCES price(id) ON DELETE CASCADE;

ALTER TABLE public.subscription_component DROP CONSTRAINT subscription_component_product_item_id_fkey;
ALTER TABLE public.subscription_component ADD CONSTRAINT subscription_component_product_item_id_fkey FOREIGN KEY (product_id) REFERENCES product(id) ON DELETE CASCADE;
ALTER TABLE public.subscription_component DROP CONSTRAINT subscription_component_price_component_id_fkey;
ALTER TABLE public.subscription_component ADD CONSTRAINT subscription_component_price_component_id_fkey FOREIGN KEY (price_component_id) REFERENCES price_component(id) ON DELETE CASCADE;
ALTER TABLE public.subscription_component DROP CONSTRAINT subscription_component_price_id_fkey;
ALTER TABLE public.subscription_component ADD CONSTRAINT subscription_component_price_id_fkey FOREIGN KEY (price_id) REFERENCES price(id) ON DELETE CASCADE;

ALTER TABLE public.subscription_add_on DROP CONSTRAINT subscription_add_on_add_on_id_fkey;
ALTER TABLE public.subscription_add_on ADD CONSTRAINT subscription_add_on_add_on_id_fkey FOREIGN KEY (add_on_id) REFERENCES add_on(id) ON DELETE CASCADE;
ALTER TABLE public.subscription_add_on DROP CONSTRAINT subscription_add_on_product_id_fkey;
ALTER TABLE public.subscription_add_on ADD CONSTRAINT subscription_add_on_product_id_fkey FOREIGN KEY (product_id) REFERENCES product(id) ON DELETE CASCADE;
ALTER TABLE public.subscription_add_on DROP CONSTRAINT subscription_add_on_price_id_fkey;
ALTER TABLE public.subscription_add_on ADD CONSTRAINT subscription_add_on_price_id_fkey FOREIGN KEY (price_id) REFERENCES price(id) ON DELETE CASCADE;

-- Join row cannot outlive its price.
ALTER TABLE public.plan_component_price DROP CONSTRAINT plan_component_price_price_id_fkey;
ALTER TABLE public.plan_component_price ADD CONSTRAINT plan_component_price_price_id_fkey FOREIGN KEY (price_id) REFERENCES price(id) ON DELETE CASCADE;

-- Organization-scoped connect rows may reference a tenant's customer / invoicing
-- entity. They must survive a tenant delete, so drop the dangling reference.
ALTER TABLE public.connect_connected_account DROP CONSTRAINT connect_connected_account_platform_customer_id_fkey;
ALTER TABLE public.connect_connected_account ADD CONSTRAINT connect_connected_account_platform_customer_id_fkey FOREIGN KEY (platform_customer_id) REFERENCES customer(id) ON DELETE SET NULL;
ALTER TABLE public.connect_platform_settings DROP CONSTRAINT connect_platform_settings_branding_invoicing_entity_id_fkey;
ALTER TABLE public.connect_platform_settings ADD CONSTRAINT connect_platform_settings_branding_invoicing_entity_id_fkey FOREIGN KEY (branding_invoicing_entity_id) REFERENCES invoicing_entity(id) ON DELETE SET NULL;
