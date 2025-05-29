DO
$$
  DECLARE
    var_user_id                    UUID := 'ae35bbb9-65da-477d-b856-7dbd87546441';
    var_tenant_id                  UUID := '018c2c82-3df1-7e84-9e05-6e141d0e751a';
    var_product_family_id          UUID := '018c2c82-3df2-71a4-b45c-86cb8604b75c';

    -- plans
    var_plan_leetcode_id           UUID := '018c344a-78a8-79bc-aefd-09113eaf5cb3';
    var_plan_notion_id             UUID := '018c344b-da85-70dc-ae6f-5b919847dbbf';
    var_plan_supabase_id           UUID := '018c344d-5957-72cf-816b-938dea2f5c76';

    -- plan_versions
    var_plan_version_1_leetcode_id UUID := '018c344a-78a9-7e2b-af90-5748672711f8';
    var_plan_version_notion_id     UUID := '018c344b-da87-7392-bbae-c5c8780adb1b';
    var_plan_version_supabase_id   UUID := '018c35cc-3f41-7551-b7b6-f8bbcd62b784';

    -- components

    var_comp_leetcode_rate_id      UUID := '018c344b-6050-7ec8-bd8c-d2e9c41ab711';
    var_comp_notion_seats_id       UUID := '018c344c-9ec9-7608-b115-1537b6985e73';
    var_comp_supabase_org_slots_id UUID := '3b083801-c77c-4488-848e-a185f0f0a8be';
    var_comp_supabase_bandwidth_id UUID := '705265c8-6069-4b84-a815-73bc7bd773bd';
    var_comp_supabase_db_size_id   UUID := '331810d4-05b1-4d8e-bf9b-d61cedaec117';


  BEGIN


    --
-- LeetCode plan
-- Rate only (monthly, annual)
--

    INSERT INTO public.plan
    (id, name, description, created_at, created_by, updated_at, archived_at, tenant_id,
     product_family_id, plan_type, status, active_version_id, draft_version_id)
    VALUES (var_plan_leetcode_id, 'LeetCode', '', '2023-12-04 10:05:45',
            var_user_id, NULL, NULL, var_tenant_id,
            var_product_family_id, 'STANDARD', 'ACTIVE', NULL, NULL);

    INSERT INTO public.plan_version
    VALUES (var_plan_version_1_leetcode_id, false, var_plan_leetcode_id, 1, NULL, NULL,
            var_tenant_id, NULL, 0, 'EUR', NULL, '2023-12-04 10:05:45',
            var_user_id);

    INSERT INTO public.plan_version
    VALUES ('018c344a-78a9-7e2b-af90-5748672711f9', true, var_plan_leetcode_id, 2, NULL, NULL,
            var_tenant_id, NULL, 0, 'EUR', NULL, '2023-12-04 10:05:45',
            var_user_id);

    UPDATE public.plan
    SET active_version_id = var_plan_version_1_leetcode_id,
        draft_version_id  = '018c344a-78a9-7e2b-af90-5748672711f9'
    WHERE id = var_plan_leetcode_id;

    INSERT INTO public.price_component
    VALUES (var_comp_leetcode_rate_id, 'Subscription Rate', '{
      "Rate": {
        "rates": [
          {
            "price": "35.00",
            "term": 0
          },
          {
            "price": "159.00",
            "term": 2
          }
        ]
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
            var_product_family_id, 'STANDARD', 'ACTIVE');

    INSERT INTO public.plan_version
    VALUES (var_plan_version_notion_id, false, var_plan_notion_id, 1, NULL, NULL,
            var_tenant_id, NULL, 0, 'EUR', NULL, '2023-12-04 10:07:15.589',
            var_user_id);

    UPDATE public.plan
    SET active_version_id = var_plan_version_notion_id
    WHERE id = var_plan_notion_id;

    INSERT INTO public.price_component
    VALUES (var_comp_notion_seats_id, 'Seats', '{
      "Slot": {
        "quota": null,
        "rates": [
          {
            "term": "Monthly",
            "price": "10.00"
          },
          {
            "term": "Annual",
            "price": "96.00"
          }
        ],
        "slot_unit_name": "Seats",
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
            var_product_family_id, 'STANDARD', 'ACTIVE');

    INSERT INTO public.plan_version
    VALUES (var_plan_version_supabase_id, false, var_plan_supabase_id, 3, NULL, NULL,
            var_tenant_id, NULL, 0, 'EUR', NULL, '2023-12-04 17:07:07.2',
            var_user_id);

    UPDATE public.plan
    SET active_version_id = var_plan_version_supabase_id
    WHERE id = var_plan_supabase_id;

    INSERT INTO public.price_component
    VALUES (var_comp_supabase_org_slots_id, 'Organization Slots', '{
      "Slot": {
        "quota": null,
        "rates": [
          {
            "term": "Monthly",
            "price": "25.00"
          }
        ],
        "slot_unit_name": "Organization",
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

  END
$$;
