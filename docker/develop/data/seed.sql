DO
$$
  DECLARE
    var_org_id                     UUID := '018c2c82-3def-7fa0-bf6f-a5f8fe341549';
    var_user_id                    UUID := 'ae35bbb9-65da-477d-b856-7dbd87546441';
    var_tenant_id                  UUID := '018c2c82-3df1-7e84-9e05-6e141d0e751a';
    var_invoicing_entity_id        UUID := 'cf144094-ab72-441c-8c8a-54e18bfba0ef';
    var_product_family_id          UUID := '018c2c82-3df2-71a4-b45c-86cb8604b75c';

    -- customers

    var_cust_spotify_id            UUID := '018c345f-7324-7cd2-a692-78e5ab9158e0';
    var_cust_uber_id               UUID := '018c345f-dff1-7857-b988-6c792ed6fa3f';
    var_cust_comodo_id             UUID := '018c3463-05f3-7c1f-92b1-ddb1f70905a2';

    -- metrics

    var_metric_database_size       UUID := '018c3452-129f-702c-93f4-9c15095b0ef4';
    var_metric_bandwidth           UUID := '018c3453-1f11-76a8-8d69-f74921b2646d';

    -- plans
    var_plan_leetcode_id           UUID := '018c344a-78a8-79bc-aefd-09113eaf5cb3';
    var_plan_notion_id             UUID := '018c344b-da85-70dc-ae6f-5b919847dbbf';
    var_plan_supabase_id           UUID := '018c344d-5957-72cf-816b-938dea2f5c76';

    -- plan_versions
    var_plan_version_1_leetcode_id UUID := '018c344a-78a9-7e2b-af90-5748672711f8';
    var_plan_version_2_leetcode_id UUID := '018c344a-78a9-7e2b-af90-5748672711f9';
    var_plan_version_notion_id     UUID := '018c344b-da87-7392-bbae-c5c8780adb1b';
    var_plan_version_supabase_id   UUID := '018c35cc-3f41-7551-b7b6-f8bbcd62b784';

    -- components

    var_comp_leetcode_rate_id      UUID := '018c344b-6050-7ec8-bd8c-d2e9c41ab711';
    var_comp_notion_seats_id       UUID := '018c344c-9ec9-7608-b115-1537b6985e73';
    var_comp_supabase_org_slots_id UUID := '3b083801-c77c-4488-848e-a185f0f0a8be';
    var_comp_supabase_bandwidth_id UUID := '705265c8-6069-4b84-a815-73bc7bd773bd';
    var_comp_supabase_db_size_id   UUID := '331810d4-05b1-4d8e-bf9b-d61cedaec117';

    -- subscriptions

    var_sub_spotify_notion_id      UUID := '018c3475-bdc5-77dd-9e26-e9a7fdd60426';
    var_sub_spotify_supabase_id    UUID := '018c3762-d554-7339-b13d-6fff8c9b76a0';
    var_sub_uber_notion_id         UUID := '018c3477-2274-7029-9743-b3a4eb779399';
    var_sub_uber_leetcode_id       UUID := '018c3479-fa9d-713f-b74f-6d9cc22cf110';
    var_sub_comodo_leetcode_id     UUID := '018c347a-b42b-709f-8e70-b0b63029aa35';
    var_sub_comodo_supabase_id     UUID := '018c3763-070e-709d-8413-f42828e71943';

  BEGIN

    INSERT INTO public.organization
    (id, trade_name, slug, created_at, archived_at, invite_link_hash, default_country)
    VALUES (var_org_id, 'Local Org', 'TESTORG', '2023-12-02 21:49:42.255', NULL,
            'fake-invite-link', 'FR');

    INSERT INTO public."user"
    (id, email, created_at, archived_at, password_hash, onboarded, first_name, last_name, department)
    VALUES (var_user_id, 'demo-user@meteroid.dev', '2023-12-02 21:49:08.805', NULL,
            '$argon2id$v=19$m=19456,t=2,p=1$dawIX5+sybNHqfFoNvHFhw$uhtWJd50wiFDV8nR10RNZI4OCrOAJ1kiNZQF0OUSoGE',
            true,
            'John', 'Doe', 'Engineering');


    INSERT INTO public.organization_member
      (user_id, organization_id, role)
    VALUES (var_user_id, var_org_id, 'ADMIN');

    -- --
-- -- Data for Name: tenant; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
-- --
    INSERT INTO public.tenant
    (id, name, slug, created_at, updated_at, archived_at, organization_id, currency, environment)
    VALUES (var_tenant_id, 'Sandbox', 'testslug', '2023-12-02 21:49:42.255', NULL,
            NULL,
            var_org_id, 'EUR', 'DEVELOPMENT'::"TenantEnvironmentEnum");
    --
--
-- --
-- -- Data for Name: api_token; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
-- --
    INSERT INTO public.api_token
      (id, name, created_at, created_by, tenant_id, hash, hint)
    VALUES ('018ce957-b628-7355-a460-f0d71e01335e', 'token-pD_', '2024-01-08 13:51:29.151',
            var_user_id, var_tenant_id,
            '$argon2id$v=19$m=19456,t=2,p=1$98CkbdqB8KNdlqryCBIx+g$nhTanF/4QsVnpPFvPHzshLPOGd7btYxXfq2UWB0xkiU',
            'pv_sand_9XzH...AbBG');


    INSERT INTO public.invoicing_entity
    (id, local_id, is_default, legal_name, invoice_number_pattern, next_invoice_number, next_credit_note_number,
     grace_period_hours, net_terms, invoice_footer_info, invoice_footer_legal, logo_attachment_id, brand_color,
     address_line1, address_line2, zip_code, state, city, vat_number, country, accounting_currency, tenant_id)
    VALUES (var_invoicing_entity_id, 'ive_O0sddA9FDlfeq', true, 'ACME_UK', 'INV-{number}', 1, 1, 23, 30, 'hello',
            'world', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 'FE', 'EUR', var_tenant_id);


    -- --
-- -- Data for Name: historical_rates_from_usd; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
-- -- A default historical value that will be used as fallback until rates gets updated
-- --
    INSERT INTO public.historical_rates_from_usd
      (id, date, rates)
    VALUES ('018df083-46df-7326-a3ca-fb98888e1196', '2010-01-01', '{
      "AUD": 1.108609,
      "BRL": 1.741616,
      "CAD": 1.048367,
      "CHF": 1.0338,
      "CNY": 6.828759,
      "COP": 2044.171135,
      "EUR": 0.697253,
      "GBP": 0.618224,
      "HKD": 7.754729,
      "JPY": 92.910732,
      "KRW": 1160.640163,
      "MXN": 13.108757,
      "NZD": 1.377768,
      "SEK": 7.138645,
      "USD": 1
    }'),
           ('018df083-46df-7f80-86da-f8c878b120f9', '2023-01-01', '{
             "AUD": 1.466361,
             "BRL": 5.286471,
             "CAD": 1.35339,
             "CHF": 0.924587,
             "CNY": 6.89814,
             "COP": 4837.794852,
             "EUR": 0.934096,
             "GBP": 0.826651,
             "HKD": 7.80261,
             "JPY": 130.926,
             "KRW": 1261.764305,
             "MXN": 19.497266,
             "NZD": 1.573642,
             "SEK": 10.421755,
             "USD": 1
           }'),
           ('018df083-b921-7e28-8824-3a7a6ae2733e', '2024-01-01', '{
             "AUD": 1.468645,
             "BRL": 4.8539,
             "CAD": 1.324436,
             "CHF": 0.841915,
             "CNY": 7.0786,
             "COP": 3887.87175,
             "EUR": 0.906074,
             "GBP": 0.78569,
             "HKD": 7.81035,
             "JPY": 141.115,
             "KRW": 1280.64,
             "MXN": 16.9664,
             "NZD": 1.583713,
             "SEK": 10.074633,
             "USD": 1
           }');


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


    --
-- Data for Name: product_family; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--

    INSERT INTO public.product_family
      (id, name, local_id, created_at, updated_at, archived_at, tenant_id)
    VALUES (var_product_family_id, 'Default', 'default', '2023-12-02 21:49:42.255', NULL, NULL, var_tenant_id);


    --
-- Data for Name: billable_metric; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--

    INSERT INTO public.billable_metric
    (id, name, description, code, aggregation_type, aggregation_key,
     unit_conversion_factor, unit_conversion_rounding, segmentation_matrix,
     usage_group_key, created_at, created_by, updated_at, archived_at, tenant_id,
     product_family_id)
    VALUES (var_metric_database_size, 'Database size (GB)', '', 'db_size', 'LATEST', 'size_gb', 1,
            NULL, '{
        "matrix": null
      }', '', '2023-12-04 10:14:03.168', var_user_id, NULL, NULL,
            var_tenant_id, var_product_family_id),
           (var_metric_bandwidth, 'Bandwidth (GB)', '', 'bandwidth', 'SUM', 'value', 1, NULL, '{
             "matrix": null
           }', NULL, '2023-12-04 10:15:11.89', var_user_id, NULL, NULL,
            var_tenant_id, var_product_family_id);


    INSERT INTO public.plan
    (id, name, description, created_at, created_by, updated_at, archived_at, tenant_id,
     product_family_id, local_id, plan_type, status)
    VALUES (var_plan_leetcode_id, 'LeetCode', '', '2023-12-04 10:05:45',
            var_user_id, NULL, NULL, var_tenant_id,
            var_product_family_id, 'default_leet-code', 'STANDARD', 'ACTIVE');

    INSERT INTO public.plan_version
    VALUES (var_plan_version_1_leetcode_id, false, var_plan_leetcode_id, 1, NULL, NULL,
            var_tenant_id, NULL, 0, 'EUR', NULL, '2023-12-04 10:05:45',
            var_user_id, '{MONTHLY,ANNUAL}');

    INSERT INTO public.plan_version
    VALUES ('018c344a-78a9-7e2b-af90-5748672711f9', true, var_plan_leetcode_id, 2, NULL, NULL,
            var_tenant_id, NULL, 0, 'EUR', NULL, '2023-12-04 10:05:45',
            var_user_id, '{MONTHLY,ANNUAL}');

    INSERT INTO public.price_component
    VALUES (var_comp_leetcode_rate_id, 'Subscription Rate', '{
      "rate": {
        "pricing": {
          "term_based": {
            "rates": [
              {
                "term": 0,
                "price": {
                  "value": "35.00"
                }
              },
              {
                "term": 2,
                "price": {
                  "value": "159.00"
                }
              }
            ]
          }
        }
      }
    }', var_plan_version_1_leetcode_id, NULL);


    ------------------------------------------------------------

--
-- Notion plan
-- Seat based (monthly :10, annual : 96)
--
    INSERT INTO public.plan
    VALUES (var_plan_notion_id, 'Notion', '', '2023-12-04 10:07:15.589',
            var_user_id, NULL, NULL, var_tenant_id,
            var_product_family_id, 'default_notion', 'STANDARD', 'ACTIVE');

    INSERT INTO public.plan_version
    VALUES (var_plan_version_notion_id, false, var_plan_notion_id, 1, NULL, NULL,
            var_tenant_id, NULL, 0, 'EUR', NULL, '2023-12-04 10:07:15.589',
            var_user_id, '{MONTHLY,ANNUAL}');

    INSERT INTO public.price_component
    VALUES (var_comp_notion_seats_id, 'Seats', '{
      "slot_based": {
        "quota": null,
        "pricing": {
          "term_based": {
            "rates": [
              {
                "term": 0,
                "price": {
                  "value": "10.00"
                }
              },
              {
                "term": 2,
                "price": {
                  "value": "96.00"
                }
              }
            ]
          }
        },
        "slot_unit": {
          "id": null,
          "name": "Seats"
        },
        "minimum_count": 1,
        "upgrade_policy": 0,
        "downgrade_policy": 0
      }
    }', var_plan_version_notion_id, NULL);

    ------------------------------------------------------------

--
-- Supabase plan
-- Usage based (DB size, bandwith) + Slot-based (Organizations, monthly)
--
    INSERT INTO public.plan
    VALUES (var_plan_supabase_id, 'Supabase', '', '2023-12-04 10:08:53.591',
            var_user_id, NULL, NULL, var_tenant_id,
            var_product_family_id, 'default_supabase', 'STANDARD', 'ACTIVE');

    INSERT INTO public.plan_version
    VALUES (var_plan_version_supabase_id, false, var_plan_supabase_id, 3, NULL, NULL,
            var_tenant_id, NULL, 0, 'EUR', NULL, '2023-12-04 17:07:07.2',
            var_user_id, '{}');


    INSERT INTO public.price_component
    VALUES (var_comp_supabase_org_slots_id, 'Organization Slots', '{
      "slot_based": {
        "quota": null,
        "pricing": {
          "single": {
            "price": {
              "value": "25.00"
            },
            "cadence": 0
          }
        },
        "slot_unit": {
          "id": null,
          "name": "Organization"
        },
        "minimum_count": 1,
        "upgrade_policy": 0,
        "downgrade_policy": 0
      }
    }', var_plan_version_supabase_id, NULL);
    INSERT INTO public.price_component
    VALUES (var_comp_supabase_bandwidth_id, 'Bandwidth (GB)', '{
      "usage_based": {
        "model": {
          "tiered": {
            "rows": [
              {
                "flat_cap": null,
                "flat_fee": null,
                "last_unit": 250,
                "first_unit": 0,
                "unit_price": {
                  "value": "0.00"
                }
              },
              {
                "flat_cap": null,
                "flat_fee": null,
                "last_unit": null,
                "first_unit": 250,
                "unit_price": {
                  "value": "0.09"
                }
              }
            ],
            "block_size": null
          }
        },
        "metric": {
          "id": "018c3453-1f11-76a8-8d69-f74921b2646d",
          "name": "N/A"
        }
      }
    }', var_plan_version_supabase_id, NULL);
    INSERT INTO public.price_component
    VALUES (var_comp_supabase_db_size_id, 'Database size (GB)', '{
      "usage_based": {
        "model": {
          "tiered": {
            "rows": [
              {
                "flat_cap": null,
                "flat_fee": null,
                "last_unit": 8,
                "first_unit": 0,
                "unit_price": {
                  "value": "0.00"
                }
              },
              {
                "flat_cap": null,
                "flat_fee": null,
                "last_unit": null,
                "first_unit": 8,
                "unit_price": {
                  "value": "0.125"
                }
              }
            ],
            "block_size": null
          }
        },
        "metric": {
          "id": "018c3452-129f-702c-93f4-9c15095b0ef4",
          "name": "N/A"
        }
      }
    }', var_plan_version_supabase_id, NULL);


    --
-- Data for Name: subscription; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--
    INSERT INTO public.subscription (id, customer_id, billing_day, tenant_id, trial_start_date, billing_start_date,
                                     billing_end_date, plan_version_id, created_at, created_by, net_terms,
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


    INSERT INTO public.subscription
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


    INSERT INTO public.subscription (id, customer_id, billing_day, tenant_id, trial_start_date, billing_start_date,
                                     billing_end_date, plan_version_id, created_at, created_by, net_terms,
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


    INSERT INTO public.subscription
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

    INSERT INTO public.subscription
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


    INSERT INTO public.subscription
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
