DO
$$
    DECLARE
        var_user_id              UUID := 'ae35bbb9-65da-477d-b856-7dbd87546441';
        var_tenant_id            UUID := '018c2c82-3df1-7e84-9e05-6e141d0e751a';
        var_product_family_id    UUID := '018c2c82-3df2-71a4-b45c-86cb8604b75c';
        var_metric_database_size UUID := '018c3452-129f-702c-93f4-9c15095b0ef4';
        var_metric_bandwidth     UUID := '018c3453-1f11-76a8-8d69-f74921b2646d';
    BEGIN


        --
-- Data for Name: product_family; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--

        INSERT INTO public.product_family
            (id, name, external_id, created_at, updated_at, archived_at, tenant_id)
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


        COMMIT;
    END
$$;
