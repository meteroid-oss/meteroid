DO
$$
    DECLARE
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


    END
$$;
