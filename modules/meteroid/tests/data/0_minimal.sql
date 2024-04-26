BEGIN;

INSERT INTO public.organization
VALUES ('018c2c82-3def-7fa0-bf6f-a5f8fe341549', 'Local Org', '91ny4j5j4j01', '2023-12-02 21:49:42.255', NULL, 'fake-invite-link');

INSERT INTO public."user"
VALUES ('ae35bbb9-65da-477d-b856-7dbd87546441', 'demo-user@meteroid.dev', '2023-12-02 21:49:08.805', NULL,
        '$argon2id$v=19$m=19456,t=2,p=1$dawIX5+sybNHqfFoNvHFhw$uhtWJd50wiFDV8nR10RNZI4OCrOAJ1kiNZQF0OUSoGE');


INSERT INTO public.organization_member
VALUES ('ae35bbb9-65da-477d-b856-7dbd87546441', '018c2c82-3def-7fa0-bf6f-a5f8fe341549', 'ADMIN');

-- --
-- -- Data for Name: tenant; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
-- --
INSERT INTO public.tenant
VALUES ('018c2c82-3df1-7e84-9e05-6e141d0e751a', 'Sandbox', 'a712afi5lzhk', '2023-12-02 21:49:42.255', NULL, NULL,
        '018c2c82-3def-7fa0-bf6f-a5f8fe341549', 'EUR');
--
--
-- --
-- -- Data for Name: api_token; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
-- --
INSERT INTO public.api_token
VALUES ('018ce957-b628-7355-a460-f0d71e01335e', 'token-pD_', '2024-01-08 13:51:29.151',
        'ae35bbb9-65da-477d-b856-7dbd87546441', '018c2c82-3df1-7e84-9e05-6e141d0e751a',
        '$argon2id$v=19$m=19456,t=2,p=1$98CkbdqB8KNdlqryCBIx+g$nhTanF/4QsVnpPFvPHzshLPOGd7btYxXfq2UWB0xkiU',
        'pv_sand_9XzH...AbBG');


-- TODO need to merge with tenant, or to make sure that this is created on tenant creation (or default in sql queries)
INSERT INTO public.invoicing_config
VALUES ('0a356cd7-d0fa-4be8-87b0-098fb0943579', '018c2c82-3df1-7e84-9e05-6e141d0e751a', 1);


-- --
-- -- Data for Name: historical_rates_from_usd; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
-- -- A default historical value that will be used as fallback until rates gets updated
-- --
INSERT INTO public.historical_rates_from_usd
VALUES ('018df083-46df-7326-a3ca-fb98888e1196', '2010-01-01', '{"AUD": 1.108609,"BRL": 1.741616,"CAD": 1.048367,"CHF": 1.0338,"CNY": 6.828759,"COP": 2044.171135,"EUR": 0.697253,"GBP": 0.618224,"HKD": 7.754729,"JPY": 92.910732,"KRW": 1160.640163,"MXN": 13.108757,"NZD": 1.377768,"SEK": 7.138645,"USD": 1}');
VALUES ('018df083-46df-767d-8ba3-2e42d8ad0a88', '2020-01-01', '{"AUD": 1.424502,"BRL": 4.019699,"CAD": 1.29765,"CHF": 0.967795,"CNY": 6.9632,"COP": 3286.8887,"EUR": 0.891348,"GBP": 0.754603,"HKD": 7.79267,"JPY": 108.72525,"KRW": 1154.969938,"MXN": 18.914,"NZD": 1.484656,"SEK": 9.346581,"USD": 1}');
VALUES ('018df083-46df-71b6-ba23-3ebc51265c70', '2021-01-01', '{"AUD": 1.29985,"BRL": 5.1934,"CAD": 1.272993,"CHF": 0.890075,"CNY": 6.533,"COP": 3461.475266,"EUR": 0.822681,"GBP": 0.73135,"HKD": 7.75325,"JPY": 103.23998054,"KRW": 1085.73,"MXN": 19.8822,"NZD": 1.412085,"SEK": 8.26929,"USD": 1}');
VALUES ('018df083-46df-7b64-886a-7a7a4bada7c0', '2022-01-01', '{"AUD": 1.376558,"BRL": 5.5713,"CAD": 1.26405,"CHF": 0.911704,"CNY": 6.3559,"COP": 4052.013259,"EUR": 0.879202,"GBP": 0.739016,"HKD": 7.7961,"JPY": 115.108,"KRW": 1188.88,"MXN": 20.4973,"NZD": 1.461562,"SEK": 9.05005,"USD": 1}');
VALUES ('018df083-46df-7f80-86da-f8c878b120f9', '2023-01-01', '{"AUD": 1.466361,"BRL": 5.286471,"CAD": 1.35339,"CHF": 0.924587,"CNY": 6.89814,"COP": 4837.794852,"EUR": 0.934096,"GBP": 0.826651,"HKD": 7.80261,"JPY": 130.926,"KRW": 1261.764305,"MXN": 19.497266,"NZD": 1.573642,"SEK": 10.421755,"USD": 1}');
VALUES ('018df083-b921-7e28-8824-3a7a6ae2733e', '2024-01-01', '{"AUD": 1.468645,"BRL": 4.8539,"CAD": 1.324436,"CHF": 0.841915,"CNY": 7.0786,"COP": 3887.87175,"EUR": 0.906074,"GBP": 0.78569,"HKD": 7.81035,"JPY": 141.115,"KRW": 1280.64,"MXN": 16.9664,"NZD": 1.583713,"SEK": 10.074633,"USD": 1}');

COMMIT;


