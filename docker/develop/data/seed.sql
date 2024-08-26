--
-- PostgreSQL database dump
--

-- Dumped from database version 15.2 (Debian 15.2-1.pgdg110+1)
-- Dumped by pg_dump version 15.4

--
-- Data for Name: organization; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--

INSERT INTO public.organization
VALUES ('018c2c82-3def-7fa0-bf6f-a5f8fe341549', 'Local Org', 'TESTORG', '2023-12-02 21:49:42.255', NULL);


--
-- Data for Name: tenant; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--

INSERT INTO public.tenant
VALUES ('018c2c82-3df1-7e84-9e05-6e141d0e751a', 'Sandbox', 'testslug', '2023-12-02 21:49:42.255', NULL, NULL,
        '018c2c82-3def-7fa0-bf6f-a5f8fe341549', 'EUR');


--
-- Data for Name: api_token; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--
INSERT INTO public.api_token
VALUES ('018ce957-b628-7355-a460-f0d71e01335e', 'token-pD_', '2024-01-08 13:51:29.151',
        'ae35bbb9-65da-477d-b856-7dbd87546441', '018c2c82-3df1-7e84-9e05-6e141d0e751a',
        '"$argon2id$v=19$m=19456,t=2,p=1$98CkbdqB8KNdlqryCBIx+g$nhTanF/4QsVnpPFvPHzshLPOGd7btYxXfq2UWB0xkiU"',
        'pv_sand_9XzH...AbBG');


--
-- Data for Name: product_family; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--

INSERT INTO public.product_family
VALUES ('018c2c82-3df2-71a4-b45c-86cb8604b75c', 'Default', 'default', '2023-12-02 21:49:42.255', NULL, NULL,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a');


-- --
-- -- Data for Name: historical_rates_from_usd; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
-- -- A default historical value that will be used as fallback for usd calculation until rates gets updated
-- --
INSERT INTO public.historical_rates_from_usd
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
       ('018df083-46df-767d-8ba3-2e42d8ad0a88', '2020-01-01', '{
         "AUD": 1.424502,
         "BRL": 4.019699,
         "CAD": 1.29765,
         "CHF": 0.967795,
         "CNY": 6.9632,
         "COP": 3286.8887,
         "EUR": 0.891348,
         "GBP": 0.754603,
         "HKD": 7.79267,
         "JPY": 108.72525,
         "KRW": 1154.969938,
         "MXN": 18.914,
         "NZD": 1.484656,
         "SEK": 9.346581,
         "USD": 1
       }'),
       ('018df083-46df-71b6-ba23-3ebc51265c70', '2021-01-01', '{
         "AUD": 1.29985,
         "BRL": 5.1934,
         "CAD": 1.272993,
         "CHF": 0.890075,
         "CNY": 6.533,
         "COP": 3461.475266,
         "EUR": 0.822681,
         "GBP": 0.73135,
         "HKD": 7.75325,
         "JPY": 103.23998054,
         "KRW": 1085.73,
         "MXN": 19.8822,
         "NZD": 1.412085,
         "SEK": 8.26929,
         "USD": 1
       }'),
       ('018df083-46df-7b64-886a-7a7a4bada7c0', '2022-01-01', '{
         "AUD": 1.376558,
         "BRL": 5.5713,
         "CAD": 1.26405,
         "CHF": 0.911704,
         "CNY": 6.3559,
         "COP": 4052.013259,
         "EUR": 0.879202,
         "GBP": 0.739016,
         "HKD": 7.7961,
         "JPY": 115.108,
         "KRW": 1188.88,
         "MXN": 20.4973,
         "NZD": 1.461562,
         "SEK": 9.05005,
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
-- Data for Name: billable_metric; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--

INSERT INTO public.billable_metric
VALUES ('018c3452-129f-702c-93f4-9c15095b0ef4', 'Database size (GB)', '', 'db_size', 'LATEST', 'size_gb', 1, NULL, '{
  "matrix": null
}', '', '2023-12-04 10:14:03.168', '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', NULL, NULL,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', '018c2c82-3df2-71a4-b45c-86cb8604b75c');
INSERT INTO public.billable_metric
VALUES ('018c3453-1f11-76a8-8d69-f74921b2646d', 'Bandwidth (GB)', '', 'bandwidth', 'SUM', 'value', 1, NULL, '{
  "matrix": null
}', NULL, '2023-12-04 10:15:11.89', '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', NULL, NULL,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', '018c2c82-3df2-71a4-b45c-86cb8604b75c');


--
-- Data for Name: checkpoint_draft_subscription; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--


--
-- Data for Name: customer; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--

INSERT INTO public.customer
VALUES ('018c345f-7324-7cd2-a692-78e5ab9158e0', 'Sportify', '2023-12-04 10:28:39.845',
        '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', NULL, NULL, NULL, '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL,
        'spotify');
INSERT INTO public.customer
VALUES ('018c345f-dff1-7857-b988-6c792ed6fa3f', 'Uber', '2023-12-04 10:29:07.699',
        '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', NULL, NULL, NULL, '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, 'uber');
INSERT INTO public.customer
VALUES ('018c3463-05f3-7c1f-92b1-ddb1f70905a2', 'Comodo', '2023-12-04 10:32:34.036',
        '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', NULL, NULL, NULL, '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL,
        'comodo');


--
-- Data for Name: fang_tasks; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--


--
-- Data for Name: fang_tasks_archive; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--


--
-- Data for Name: invoice; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--


--
-- Data for Name: invoicing_config; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--


--
-- Data for Name: user; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--

INSERT INTO public."user"
VALUES ('ae35bbb9-65da-477d-b856-7dbd87546441', 'demo-user@meteroid.dev', '2023-12-02 21:49:08.805', NULL,
        '$argon2id$v=19$m=19456,t=2,p=1$rOoOLTawiBwSD8YNQN0vQw$76WWtJJm5Yjdl51LWHSalU9a/FQKbg/Bm82RmJCgMy4');


--
-- Data for Name: organization_member; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--

INSERT INTO public.organization_member
VALUES ('ae35bbb9-65da-477d-b856-7dbd87546441', '018c2c82-3def-7fa0-bf6f-a5f8fe341549', 'ADMIN');


--
-- Data for Name: plan; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--

INSERT INTO public.plan
VALUES ('018c344a-78a8-79bc-aefd-09113eaf5cb3', 'LeetCode', '', '2023-12-04 10:05:45',
        '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', NULL, NULL, '018c2c82-3df1-7e84-9e05-6e141d0e751a',
        '018c2c82-3df2-71a4-b45c-86cb8604b75c', 'default_leet-code', 'STANDARD', 'ACTIVE');
INSERT INTO public.plan
VALUES ('018c344b-da85-70dc-ae6f-5b919847dbbf', 'Notion', '', '2023-12-04 10:07:15.589',
        '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', NULL, NULL, '018c2c82-3df1-7e84-9e05-6e141d0e751a',
        '018c2c82-3df2-71a4-b45c-86cb8604b75c', 'default_notion', 'STANDARD', 'ACTIVE');
INSERT INTO public.plan
VALUES ('018c344d-5957-72cf-816b-938dea2f5c76', 'Supabase', '', '2023-12-04 10:08:53.591',
        '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', NULL, NULL, '018c2c82-3df1-7e84-9e05-6e141d0e751a',
        '018c2c82-3df2-71a4-b45c-86cb8604b75c', 'default_supabase', 'STANDARD', 'ACTIVE');


--
-- Data for Name: plan_version; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--

INSERT INTO public.plan_version
VALUES ('018c344a-78a9-7e2b-af90-5748672711f8', false, '018c344a-78a8-79bc-aefd-09113eaf5cb3', 1, NULL, NULL,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, 0, 'EUR', NULL, '2023-12-04 10:05:45',
        '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', '{MONTHLY,ANNUAL}');
INSERT INTO public.plan_version
VALUES ('018c344b-da87-7392-bbae-c5c8780adb1b', false, '018c344b-da85-70dc-ae6f-5b919847dbbf', 1, NULL, NULL,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, 0, 'EUR', NULL, '2023-12-04 10:07:15.589',
        '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', '{MONTHLY,ANNUAL}');
INSERT INTO public.plan_version
VALUES ('018c35cc-3f41-7551-b7b6-f8bbcd62b784', false, '018c344d-5957-72cf-816b-938dea2f5c76', 3, NULL, NULL,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, 0, 'EUR', NULL, '2023-12-04 17:07:07.2',
        'ae35bbb9-65da-477d-b856-7dbd87546441', '{}');


--
-- Data for Name: product; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--


--
-- Data for Name: price_component; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--

INSERT INTO public.price_component
VALUES ('018c344b-6050-7ec8-bd8c-d2e9c41ab711', 'Subscription Rate', '{
  "binary": "Ch0KGxIZCgkSBwoFMzUuMDAKDAgCEggKBjE1OS4wMA==",
  "_introspect": {
    "fee": {
      "Rate": {
        "pricing": {
          "pricing": {
            "TermBased": {
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
      }
    }
  }
}', '018c344a-78a9-7e2b-af90-5748672711f8', NULL);
INSERT INTO public.price_component
VALUES ('018c344c-9ec9-7608-b115-1537b6985e73', 'Seats', '{
  "binary": "EicKGhIYCgkSBwoFMTAuMDAKCwgCEgcKBTk2LjAwEgcSBVNlYXRzKAE=",
  "_introspect": {
    "fee": {
      "SlotBased": {
        "quota": null,
        "pricing": {
          "pricing": {
            "TermBased": {
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
    }
  }
}', '018c344b-da87-7392-bbae-c5c8780adb1b', NULL);
INSERT INTO public.price_component
VALUES ('3b083801-c77c-4488-848e-a185f0f0a8be', 'Organization Slots', '{
  "binary": "Eh8KCwoJCgcKBTI1LjAwEg4SDE9yZ2FuaXphdGlvbigB",
  "_introspect": {
    "fee": {
      "SlotBased": {
        "quota": null,
        "pricing": {
          "pricing": {
            "Single": {
              "price": {
                "value": "25.00"
              },
              "cadence": 0
            }
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
    }
  }
}', '018c35cc-3f41-7551-b7b6-f8bbcd62b784', NULL);
INSERT INTO public.price_component
VALUES ('705265c8-6069-4b84-a815-73bc7bd773bd', 'Bandwidth (GB)', '{
  "binary": "IksKKwokMDE4YzM0NTMtMWYxMS03NmE4LThkNjktZjc0OTIxYjI2NDZkEgNOL0ESHBIaCgsQ+gEaBgoEMC4wMAoLCPoBGgYKBDAuMDk=",
  "_introspect": {
    "fee": {
      "UsageBased": {
        "model": {
          "model": {
            "Tiered": {
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
          }
        },
        "metric": {
          "id": "018c3453-1f11-76a8-8d69-f74921b2646d",
          "name": "N/A"
        }
      }
    }
  }
}', '018c35cc-3f41-7551-b7b6-f8bbcd62b784', NULL);
INSERT INTO public.price_component
VALUES ('331810d4-05b1-4d8e-bf9b-d61cedaec117', 'Database size (GB)', '{
  "binary": "IkoKKwokMDE4YzM0NTItMTI5Zi03MDJjLTkzZjQtOWMxNTA5NWIwZWY0EgNOL0ESGxIZCgoQCBoGCgQwLjAwCgsICBoHCgUwLjEyNQ==",
  "_introspect": {
    "fee": {
      "UsageBased": {
        "model": {
          "model": {
            "Tiered": {
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
          }
        },
        "metric": {
          "id": "018c3452-129f-702c-93f4-9c15095b0ef4",
          "name": "N/A"
        }
      }
    }
  }
}', '018c35cc-3f41-7551-b7b6-f8bbcd62b784', NULL);


--
-- Data for Name: provider_config; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--


--
-- Data for Name: schedule; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--


--
-- Data for Name: subscription; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--

INSERT INTO public.subscription
VALUES ('018c3475-bdc5-77dd-9e26-e9a7fdd60426', '018c345f-7324-7cd2-a692-78e5ab9158e0', 1,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, '2023-11-04', NULL, '018c344b-da87-7392-bbae-c5c8780adb1b',
        '2023-12-04 10:53:00.742', '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', '{
    "binary": "CigKJDAxOGMzNDRjLTllYzktNzYwOC1iMTE1LTE1MzdiNjk4NWU3MxADEAA=",
    "_introspect": {
      "parameters": [
        {
          "value": 3,
          "component_id": "018c344c-9ec9-7608-b115-1537b6985e73"
        }
      ],
      "committed_billing_period": 0
    }
  }', 'MONTHLY', 0, NULL, NULL);
INSERT INTO public.subscription
VALUES ('018c347a-b42b-709f-8e70-b0b63029aa35', '018c3463-05f3-7c1f-92b1-ddb1f70905a2', 31,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, '2023-11-04', NULL, '018c344a-78a9-7e2b-af90-5748672711f8',
        '2023-12-04 10:58:25.964', '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', '{
    "binary": "EAA=",
    "_introspect": {
      "parameters": [],
      "committed_billing_period": 0
    }
  }', 'MONTHLY', 30, NULL, NULL);
INSERT INTO public.subscription
VALUES ('018c3477-2274-7029-9743-b3a4eb779399', '018c345f-dff1-7857-b988-6c792ed6fa3f', 1,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, '2023-11-04', NULL, '018c344b-da87-7392-bbae-c5c8780adb1b',
        '2023-12-04 10:54:32.056', '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', '{
    "binary": "CigKJDAxOGMzNDRjLTllYzktNzYwOC1iMTE1LTE1MzdiNjk4NWU3MxAZEAI=",
    "_introspect": {
      "parameters": [
        {
          "value": 25,
          "component_id": "018c344c-9ec9-7608-b115-1537b6985e73"
        }
      ],
      "committed_billing_period": 2
    }
  }', 'ANNUAL', 0, NULL, NULL);
INSERT INTO public.subscription
VALUES ('018c3479-fa9d-713f-b74f-6d9cc22cf110', '018c345f-dff1-7857-b988-6c792ed6fa3f', 15,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, '2023-11-04', NULL, '018c344a-78a9-7e2b-af90-5748672711f8',
        '2023-12-04 10:57:38.462', '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', '{
    "binary": "EAI=",
    "_introspect": {
      "parameters": [],
      "committed_billing_period": 2
    }
  }', 'ANNUAL', 30, NULL, NULL);
INSERT INTO public.subscription
VALUES ('018c3762-d554-7339-b13d-6fff8c9b76a0', '018c345f-7324-7cd2-a692-78e5ab9158e0', 1,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, '2023-11-05', NULL, '018c35cc-3f41-7551-b7b6-f8bbcd62b784',
        '2023-12-05 00:31:13.237', 'ae35bbb9-65da-477d-b856-7dbd87546441', '{
    "binary": "CigKJDNiMDgzODAxLWM3N2MtNDQ4OC04NDhlLWExODVmMGYwYThiZRAPEAA=",
    "_introspect": {
      "parameters": [
        {
          "value": 15,
          "component_id": "3b083801-c77c-4488-848e-a185f0f0a8be"
        }
      ],
      "committed_billing_period": 0
    }
  }', 'MONTHLY', 0, NULL, NULL);
INSERT INTO public.subscription
VALUES ('018c3763-070e-709d-8413-f42828e71943', '018c3463-05f3-7c1f-92b1-ddb1f70905a2', 1,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, '2023-11-05', NULL, '018c35cc-3f41-7551-b7b6-f8bbcd62b784',
        '2023-12-05 00:31:25.967', 'ae35bbb9-65da-477d-b856-7dbd87546441', '{
    "binary": "CigKJDNiMDgzODAxLWM3N2MtNDQ4OC04NDhlLWExODVmMGYwYThiZRADEAA=",
    "_introspect": {
      "parameters": [
        {
          "value": 3,
          "component_id": "3b083801-c77c-4488-848e-a185f0f0a8be"
        }
      ],
      "committed_billing_period": 0
    }
  }', 'MONTHLY', 0, NULL, NULL);


--
-- Data for Name: invoice
--

INSERT INTO public."invoice" (id, status, external_status, created_at, updated_at, tenant_id, customer_id,
                              subscription_id, currency, invoicing_provider, line_items, issued, issue_attempts,
                              last_issue_attempt_at, last_issue_error, data_updated_at, invoice_date)
VALUES ('123e4567-e89b-12d3-a456-426614174000',
           -- status
        'VOID',
           -- external_status
        'VOID',
           -- dates
        '2023-12-02 21:49:08.805', '2024-12-02 21:49:08.805',
           -- tenant_id
        '018c2c82-3df1-7e84-9e05-6e141d0e751a',
           -- customer_id
        '018c3463-05f3-7c1f-92b1-ddb1f70905a2',
           -- subscription_id
        '018c347a-b42b-709f-8e70-b0b63029aa35',
           -- currency
        'USD',
           -- invoicing_provider
        'STRIPE',
           -- line_items
        '[]',
           -- issued
        FALSE,
           -- attempts
        2,
           -- last_issue_attempt_at
        '2023-12-04 10:28:39.845',
           -- last_issue_error
        NULL,
           -- data_updated_at
        '2023-12-04 10:28:39.845',
           -- invoice_date
        '2023-12-04 10:00:00.000');

--
-- Data for Name: tenant_invite; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--


--
-- Data for Name: tenant_invite_link; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--


--
-- Data for Name: webhook_event; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--


--
-- PostgreSQL database dump complete
--

