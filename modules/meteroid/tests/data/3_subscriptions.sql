--
-- Data for Name: subscription; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--
INSERT INTO public.subscription (id, customer_id, billing_day, tenant_id, trial_start_date, billing_start_date,
                                 billing_end_date, plan_version_id, created_at, created_by, net_terms, invoice_memo,
                                 invoice_threshold, activated_at, canceled_at, cancellation_reason, currency, mrr_cents)
VALUES ('018c3475-bdc5-77dd-9e26-e9a7fdd60426', '018c345f-7324-7cd2-a692-78e5ab9158e0', 1,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, '2023-11-04', NULL, '018c344b-da87-7392-bbae-c5c8780adb1b',
        '2023-12-04 10:53:00.742', '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', 0, null, null, null, null, null, 'EUR', 0);


INSERT INTO public.subscription_component
(id, name, subscription_id, price_component_id, product_item_id, period, fee)
VALUES ('018f0a4f-7bb5-78f4-b239-dece81ee4585', 'Seats', '018c3475-bdc5-77dd-9e26-e9a7fdd60426',
        '018c344c-9ec9-7608-b115-1537b6985e73', null, 'MONTHLY', '{
    "Slot": {
      "unit": "Seats",
      "max_slots": null,
      "min_slots": 1,
      "unit_rate": "10.00",
      "initial_slots": 12
    }
  }');


INSERT INTO public.subscription
VALUES ('018c347a-b42b-709f-8e70-b0b63029aa35', '018c3463-05f3-7c1f-92b1-ddb1f70905a2', 31,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, '2023-11-04', NULL, '018c344a-78a9-7e2b-af90-5748672711f8',
        '2023-12-04 10:58:25.964', '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', 30, null, null, null, null, null, 'EUR', 0);

INSERT INTO public.subscription_component
(id, name, subscription_id, price_component_id, product_item_id, period, fee)
VALUES ('018f0a4f-9f81-7b70-871f-8efcf61f284c', 'Seats', '018c347a-b42b-709f-8e70-b0b63029aa35',
        '018c344b-6050-7ec8-bd8c-d2e9c41ab711', null, 'MONTHLY', '{
    "Rate": {
      "rate": "35.00"
    }
  }');



INSERT INTO public.subscription (id, customer_id, billing_day, tenant_id, trial_start_date, billing_start_date,
                                 billing_end_date, plan_version_id, created_at, created_by, net_terms, invoice_memo,
                                 invoice_threshold, activated_at, canceled_at, cancellation_reason, currency, mrr_cents)
VALUES ('018c3477-2274-7029-9743-b3a4eb779399', '018c345f-dff1-7857-b988-6c792ed6fa3f', 1,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, '2023-11-04', NULL, '018c344b-da87-7392-bbae-c5c8780adb1b',
        '2023-12-04 10:54:32.056', '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', 0, null, null, null, null, null, 'EUR', 0);

INSERT INTO public.subscription_component
(id, name, subscription_id, price_component_id, product_item_id, period, fee)
VALUES ('018f0a50-0053-7c41-bd4b-f7bdcca609e7', 'Seats', '018c3477-2274-7029-9743-b3a4eb779399',
        '018c344c-9ec9-7608-b115-1537b6985e73', null, 'ANNUAL', '{
    "Slot": {
      "unit": "Seats",
      "max_slots": null,
      "min_slots": 1,
      "unit_rate": "96.00",
      "initial_slots": 25
    }
  }');



INSERT INTO public.subscription
VALUES ('018c3479-fa9d-713f-b74f-6d9cc22cf110', '018c345f-dff1-7857-b988-6c792ed6fa3f', 15,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, '2023-11-04', NULL, '018c344a-78a9-7e2b-af90-5748672711f8',
        '2023-12-04 10:57:38.462', '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', 30, null, null, null, null, null, 'EUR', 0);


INSERT INTO public.subscription_component
(id, name, subscription_id, price_component_id, product_item_id, period, fee)
VALUES ('018f0a50-3a67-7448-8235-6ca5a4c75b41', 'Seats', '018c3479-fa9d-713f-b74f-6d9cc22cf110',
        '018c344b-6050-7ec8-bd8c-d2e9c41ab711', null, 'ANNUAL', '{
    "Rate": {
      "rate": "159.00"
    }
  }');

INSERT INTO public.subscription
VALUES ('018c3762-d554-7339-b13d-6fff8c9b76a0', '018c345f-7324-7cd2-a692-78e5ab9158e0', 1,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, '2023-11-04', NULL, '018c35cc-3f41-7551-b7b6-f8bbcd62b784',
        '2023-12-05 00:31:13.237', 'ae35bbb9-65da-477d-b856-7dbd87546441', 0, null, null, null, null, null, 'EUR', 0);

INSERT INTO public.subscription_component
(id, name, subscription_id, price_component_id, product_item_id, period, fee)
VALUES ('018f0a50-50f7-779e-9255-cbbad34f5a88', 'Organization Slots', '018c3762-d554-7339-b13d-6fff8c9b76a0',
        '3b083801-c77c-4488-848e-a185f0f0a8be', null, 'MONTHLY', '{
    "Slot": {
      "unit": "Organization",
      "max_slots": null,
      "min_slots": 1,
      "unit_rate": "96.00",
      "initial_slots": 15
    }
  }');


INSERT INTO public.subscription
VALUES ('018c3763-070e-709d-8413-f42828e71943', '018c3463-05f3-7c1f-92b1-ddb1f70905a2', 1,
        '018c2c82-3df1-7e84-9e05-6e141d0e751a', NULL, '2023-11-04', NULL, '018c35cc-3f41-7551-b7b6-f8bbcd62b784',
        '2023-12-05 00:31:25.967', 'ae35bbb9-65da-477d-b856-7dbd87546441', 0, null, null, null, null, null, 'EUR', 0);

INSERT INTO public.subscription_component
(id, name, subscription_id, price_component_id, product_item_id, period, fee)
VALUES ('018f0a50-9bcc-73c8-a3ca-25e2439c1dbd', 'Organization Slots', '018c3763-070e-709d-8413-f42828e71943',
        '3b083801-c77c-4488-848e-a185f0f0a8be', null, 'MONTHLY', '{
    "Slot": {
      "unit": "Organization",
      "max_slots": null,
      "min_slots": 1,
      "unit_rate": "96.00",
      "initial_slots": 3
    }
  }');
