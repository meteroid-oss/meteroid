--
-- LeetCode plan
-- Rate only (monthly, annual)
--

INSERT INTO public.plan
VALUES ('018c344a-78a8-79bc-aefd-09113eaf5cb3', 'LeetCode', '', '2023-12-04 10:05:45',
        '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', NULL, NULL, '018c2c82-3df1-7e84-9e05-6e141d0e751a',
        '018c2c82-3df2-71a4-b45c-86cb8604b75c', 'default_leet-code', 'STANDARD', 'ACTIVE');

INSERT INTO public.plan_version
VALUES ('018c344a-78a9-7e2b-af90-5748672711f8', false, '018c344a-78a8-79bc-aefd-09113eaf5cb3', 1, NULL, NULL,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, 0, 'EUR', NULL, '2023-12-04 10:05:45',
        '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', '{MONTHLY,ANNUAL}');

INSERT INTO public.plan_version
VALUES ('018c344a-78a9-7e2b-af90-5748672711f9', true, '018c344a-78a8-79bc-aefd-09113eaf5cb3', 2, NULL, NULL,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, 0, 'EUR', NULL, '2023-12-04 10:05:45',
        '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', '{MONTHLY,ANNUAL}');

INSERT INTO public.price_component
VALUES ('018c344b-6050-7ec8-bd8c-d2e9c41ab711', 'Subscription Rate', '{
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
}', '018c344a-78a9-7e2b-af90-5748672711f8', NULL);


------------------------------------------------------------

--
-- Notion plan
-- Seat based (monthly :10, annual : 96)
--
INSERT INTO public.plan
VALUES ('018c344b-da85-70dc-ae6f-5b919847dbbf', 'Notion', '', '2023-12-04 10:07:15.589',
        '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', NULL, NULL, '018c2c82-3df1-7e84-9e05-6e141d0e751a',
        '018c2c82-3df2-71a4-b45c-86cb8604b75c', 'default_notion', 'STANDARD', 'ACTIVE');

INSERT INTO public.plan_version
VALUES ('018c344b-da87-7392-bbae-c5c8780adb1b', false, '018c344b-da85-70dc-ae6f-5b919847dbbf', 1, NULL, NULL,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, 0, 'EUR', NULL, '2023-12-04 10:07:15.589',
        '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', '{MONTHLY,ANNUAL}');

INSERT INTO public.price_component
VALUES ('018c344c-9ec9-7608-b115-1537b6985e73', 'Seats', '{
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
}', '018c344b-da87-7392-bbae-c5c8780adb1b', NULL);

------------------------------------------------------------

--
-- Supabase plan
-- Usage based (DB size, bandwith) + Slot-based (Organizations, monthly)
--
INSERT INTO public.plan
VALUES ('018c344d-5957-72cf-816b-938dea2f5c76', 'Supabase', '', '2023-12-04 10:08:53.591',
        '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', NULL, NULL, '018c2c82-3df1-7e84-9e05-6e141d0e751a',
        '018c2c82-3df2-71a4-b45c-86cb8604b75c', 'default_supabase', 'STANDARD', 'ACTIVE');

INSERT INTO public.plan_version
VALUES ('018c35cc-3f41-7551-b7b6-f8bbcd62b784', false, '018c344d-5957-72cf-816b-938dea2f5c76', 3, NULL, NULL,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, 0, 'EUR', NULL, '2023-12-04 17:07:07.2',
        'ae35bbb9-65da-477d-b856-7dbd87546441', '{}');


INSERT INTO public.price_component
VALUES ('3b083801-c77c-4488-848e-a185f0f0a8be', 'Organization Slots', '{
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
}', '018c35cc-3f41-7551-b7b6-f8bbcd62b784', NULL);
INSERT INTO public.price_component
VALUES ('705265c8-6069-4b84-a815-73bc7bd773bd', 'Bandwidth (GB)', '{
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
}', '018c35cc-3f41-7551-b7b6-f8bbcd62b784', NULL);
INSERT INTO public.price_component
VALUES ('331810d4-05b1-4d8e-bf9b-d61cedaec117', 'Database size (GB)', '{
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
}', '018c35cc-3f41-7551-b7b6-f8bbcd62b784', NULL);
