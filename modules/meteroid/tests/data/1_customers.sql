DO
$$
    DECLARE
        var_user_id             UUID := 'ae35bbb9-65da-477d-b856-7dbd87546441';
        var_tenant_id           UUID := '018c2c82-3df1-7e84-9e05-6e141d0e751a';
        var_invoicing_entity_id UUID := 'cf144094-ab72-441c-8c8a-54e18bfba0ef';
        var_cust_spotify_id     UUID := '018c345f-7324-7cd2-a692-78e5ab9158e0';
        var_cust_uber_id        UUID := '018c345f-dff1-7857-b988-6c792ed6fa3f';
        var_cust_comodo_id      UUID := '018c3463-05f3-7c1f-92b1-ddb1f70905a2';

    BEGIN


        --
-- Data for Name: customer; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--


        INSERT INTO public.customer
        (id, name, created_at, created_by, updated_at, updated_by, archived_at, tenant_id,
         billing_config, alias, email, invoicing_email, phone, balance_value_cents, currency,
         billing_address, shipping_address, invoicing_entity_id)
        VALUES (var_cust_spotify_id, 'Sportify', '2023-12-04 10:28:39.845',
                var_user_id, NULL, NULL, NULL, var_tenant_id, '{
            "Stripe": {
              "customer_id": "spotify",
              "collection_method": 0
            }
          }', 'spotify', NULL, NULL, NULL, 0, 'EUR', NULL, NULL, var_invoicing_entity_id),
               (var_cust_uber_id, 'Uber', '2023-12-04 10:29:07.699',
                var_user_id, NULL, NULL, NULL, var_tenant_id, '{
                 "Stripe": {
                   "customer_id": "uber",
                   "collection_method": 0
                 }
               }', 'uber', NULL, NULL, NULL, 0, 'EUR', NULL, NULL, var_invoicing_entity_id),
               (var_cust_comodo_id, 'Comodo', '2023-12-04 10:32:34.036',
                var_user_id, NULL, NULL, NULL, var_tenant_id, '{
                 "Stripe": {
                   "customer_id": "comodo",
                   "collection_method": 0
                 }
               }', 'comodo', NULL, NULL, NULL, 0, 'EUR', NULL, NULL, var_invoicing_entity_id);


        COMMIT;
    END
$$;
