DO
$$
    DECLARE
        var_org_id              UUID := '018c2c82-3def-7fa0-bf6f-a5f8fe341549';
        var_user_id             UUID := 'ae35bbb9-65da-477d-b856-7dbd87546441';
        var_tenant_id           UUID := '018c2c82-3df1-7e84-9e05-6e141d0e751a';
        var_invoicing_entity_id UUID := 'cf144094-ab72-441c-8c8a-54e18bfba0ef';

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

        COMMIT;
    END
$$;
