DO
$$
  DECLARE

    var_user_id                    UUID := 'ae35bbb9-65da-477d-b856-7dbd87546441';
    var_tenant_id                  UUID := '018c2c82-3df1-7e84-9e05-6e141d0e751a';

    -- customers

    var_cust_spotify_id            UUID := '018c345f-7324-7cd2-a692-78e5ab9158e0';
    var_cust_uber_id               UUID := '018c345f-dff1-7857-b988-6c792ed6fa3f';
    var_cust_comodo_id             UUID := '018c3463-05f3-7c1f-92b1-ddb1f70905a2';

    -- plan_versions

    var_plan_version_1_leetcode_id UUID := '018c344a-78a9-7e2b-af90-5748672711f8';
    var_plan_version_notion_id     UUID := '018c344b-da87-7392-bbae-c5c8780adb1b';
    var_plan_version_supabase_id   UUID := '018c35cc-3f41-7551-b7b6-f8bbcd62b784';

    -- components

    var_comp_leetcode_rate_id      UUID := '018c344b-6050-7ec8-bd8c-d2e9c41ab711';
    var_comp_notion_seats_id       UUID := '018c344c-9ec9-7608-b115-1537b6985e73';
    var_comp_supabase_org_slots_id UUID := '3b083801-c77c-4488-848e-a185f0f0a8be';

    -- subscriptions

    var_sub_spotify_notion_id      UUID := '018c3475-bdc5-77dd-9e26-e9a7fdd60426';
    var_sub_spotify_supabase_id    UUID := '018c3762-d554-7339-b13d-6fff8c9b76a0';
    var_sub_uber_notion_id         UUID := '018c3477-2274-7029-9743-b3a4eb779399';
    var_sub_uber_leetcode_id       UUID := '018c3479-fa9d-713f-b74f-6d9cc22cf110';
    var_sub_comodo_leetcode_id     UUID := '018c347a-b42b-709f-8e70-b0b63029aa35';
    var_sub_comodo_supabase_id     UUID := '018c3763-070e-709d-8413-f42828e71943';


  BEGIN


    --
-- Data for Name: subscription; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--
    INSERT INTO public.subscription (id, customer_id, billing_day_anchor, tenant_id, trial_duration, start_date,
                                     end_date, plan_version_id, created_at, created_by, net_terms,
                                     invoice_memo,
                                     invoice_threshold, activated_at, canceled_at, cancellation_reason, currency,
                                     mrr_cents,
                                     period)
    VALUES (var_sub_spotify_notion_id, var_cust_spotify_id, 1,
            var_tenant_id, NULL, '2023-11-04', NULL,
            var_plan_version_notion_id,
            '2023-12-04 10:53:00.742', var_user_id, 0, null, null, null, null, null,
            'EUR', 0,
            'MONTHLY');


    INSERT INTO public.subscription_component
    (id, name, subscription_id, price_component_id, product_id, period, fee)
    VALUES ('018f0a4f-7bb5-78f4-b239-dece81ee4585', 'Seats', var_sub_spotify_notion_id,
            var_comp_notion_seats_id, null, 'MONTHLY', '{
        "Slot": {
          "unit": "Seats",
          "max_slots": null,
          "min_slots": 1,
          "unit_rate": "10.00",
          "initial_slots": 12
        }
      }');


    INSERT INTO public.subscription (id, customer_id, billing_day_anchor, tenant_id, trial_duration, start_date,
                                     end_date, plan_version_id, created_at, created_by, net_terms,
                                     invoice_memo,
                                     invoice_threshold, activated_at, canceled_at, cancellation_reason, currency,
                                     mrr_cents,
                                     period)
    VALUES (var_sub_comodo_leetcode_id, var_cust_comodo_id, 31,
            var_tenant_id, NULL, '2023-11-04', NULL,
            var_plan_version_1_leetcode_id,
            '2023-12-04 10:58:25.964', var_user_id, 30, null, null, null, null, null,
            'EUR', 0,
            'MONTHLY');

    INSERT INTO public.subscription_component
    (id, name, subscription_id, price_component_id, product_id, period, fee)
    VALUES ('018f0a4f-9f81-7b70-871f-8efcf61f284c', 'Seats', var_sub_comodo_leetcode_id,
            var_comp_leetcode_rate_id, null, 'MONTHLY', '{
        "Rate": {
          "rate": "35.00"
        }
      }');


    INSERT INTO public.subscription (id, customer_id, billing_day_anchor, tenant_id, trial_duration, start_date,
                                     end_date, plan_version_id, created_at, created_by, net_terms,
                                     invoice_memo,
                                     invoice_threshold, activated_at, canceled_at, cancellation_reason, currency,
                                     mrr_cents,
                                     period)
    VALUES (var_sub_uber_notion_id, var_cust_uber_id, 1,
            var_tenant_id, NULL, '2023-11-04', NULL,
            var_plan_version_notion_id,
            '2023-12-04 10:54:32.056', var_user_id, 0, null, null, null, null, null,
            'EUR', 0,
            'ANNUAL');

    INSERT INTO public.subscription_component
    (id, name, subscription_id, price_component_id, product_id, period, fee)
    VALUES ('018f0a50-0053-7c41-bd4b-f7bdcca609e7', 'Seats', var_sub_uber_notion_id,
            var_comp_notion_seats_id, null, 'ANNUAL', '{
        "Slot": {
          "unit": "Seats",
          "max_slots": null,
          "min_slots": 1,
          "unit_rate": "96.00",
          "initial_slots": 25
        }
      }');


    INSERT INTO public.subscription(id, customer_id, billing_day_anchor, tenant_id, trial_duration, start_date,
                                    end_date, plan_version_id, created_at, created_by, net_terms,
                                    invoice_memo,
                                    invoice_threshold, activated_at, canceled_at, cancellation_reason, currency,
                                    mrr_cents,
                                    period)
    VALUES (var_sub_uber_leetcode_id, var_cust_uber_id, 15,
            var_tenant_id, NULL, '2023-11-04', NULL,
            var_plan_version_1_leetcode_id,
            '2023-12-04 10:57:38.462', var_user_id, 30, null, null, null, null, null,
            'EUR', 0,
            'ANNUAL');


    INSERT INTO public.subscription_component
    (id, name, subscription_id, price_component_id, product_id, period, fee)
    VALUES ('018f0a50-3a67-7448-8235-6ca5a4c75b41', 'Seats', var_sub_uber_leetcode_id,
            var_comp_leetcode_rate_id, null, 'ANNUAL', '{
        "Rate": {
          "rate": "159.00"
        }
      }');

    INSERT INTO public.subscription (id, customer_id, billing_day_anchor, tenant_id, trial_duration, start_date,
                                     end_date, plan_version_id, created_at, created_by, net_terms,
                                     invoice_memo,
                                     invoice_threshold, activated_at, canceled_at, cancellation_reason, currency,
                                     mrr_cents,
                                     period)
    VALUES (var_sub_spotify_supabase_id, var_cust_spotify_id, 1,
            var_tenant_id, NULL, '2023-11-04', NULL,
            var_plan_version_supabase_id,
            '2023-12-05 00:31:13.237', var_user_id, 0, null, null, null, null, null,
            'EUR', 0,
            'MONTHLY');

    INSERT INTO public.subscription_component
    (id, name, subscription_id, price_component_id, product_id, period, fee)
    VALUES ('018f0a50-50f7-779e-9255-cbbad34f5a88', 'Organization Slots', var_sub_spotify_supabase_id,
            var_comp_supabase_org_slots_id, null, 'MONTHLY', '{
        "Slot": {
          "unit": "Organization",
          "max_slots": null,
          "min_slots": 1,
          "unit_rate": "96.00",
          "initial_slots": 15
        }
      }');


    INSERT INTO public.subscription (id, customer_id, billing_day_anchor, tenant_id, trial_duration, start_date,
                                     end_date, plan_version_id, created_at, created_by, net_terms,
                                     invoice_memo,
                                     invoice_threshold, activated_at, canceled_at, cancellation_reason, currency,
                                     mrr_cents,
                                     period)
    VALUES (var_sub_comodo_supabase_id, var_cust_comodo_id, 1,
            var_tenant_id, NULL, '2023-11-04', NULL,
            var_plan_version_supabase_id,
            '2023-12-05 00:31:25.967', var_user_id, 0, null, null, null, null, null,
            'EUR', 0,
            'MONTHLY');

    INSERT INTO public.subscription_component
    (id, name, subscription_id, price_component_id, product_id, period, fee)
    VALUES ('018f0a50-9bcc-73c8-a3ca-25e2439c1dbd', 'Organization Slots', var_sub_comodo_supabase_id,
            var_comp_supabase_org_slots_id, null, 'MONTHLY', '{
        "Slot": {
          "unit": "Organization",
          "max_slots": null,
          "min_slots": 1,
          "unit_rate": "96.00",
          "initial_slots": 3
        }
      }');


  END
$$;
