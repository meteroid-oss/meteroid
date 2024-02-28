BEGIN;

INSERT INTO public.organization
VALUES ('018c2c82-3def-7fa0-bf6f-a5f8fe341549', 'Local Org', '91ny4j5j4j01', '2023-12-02 21:49:42.255', NULL);

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
VALUES ('97cacc4d-64fd-43d6-9cb5-9a00e3226cda', '2020-01-01', '{"AUD": 1.424502,"BRL": 4.019699,"CAD": 1.29765,"CHF": 0.967795,"CNY": 6.9632,"COP": 3286.8887,"EUR": 0.891348,"GBP": 0.754603,"HKD": 7.79267,"JPY": 108.72525,"KRW": 1154.969938,"MXN": 18.914,"NZD": 1.484656,"SEK": 9.346581,"USD": 1}');

COMMIT;


