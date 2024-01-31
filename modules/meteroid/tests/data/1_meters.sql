--
-- Data for Name: product_family; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--

INSERT INTO public.product_family VALUES ('018c2c82-3df2-71a4-b45c-86cb8604b75c', 'Default', 'default', '2023-12-02 21:49:42.255', NULL, NULL, '018c2c82-3df1-7e84-9e05-6e141d0e751a');


--
-- Data for Name: billable_metric; Type: TABLE DATA; Schema: public; Owner: meteroidbilling
--

INSERT INTO public.billable_metric VALUES ('018c3452-129f-702c-93f4-9c15095b0ef4', 'Database size (GB)', '', 'db_size', 'LATEST', 'size_gb', 1, NULL, '{"matrix": null}', '', '2023-12-04 10:14:03.168', '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', NULL, NULL, '018c2c82-3df1-7e84-9e05-6e141d0e751a', '018c2c82-3df2-71a4-b45c-86cb8604b75c');
INSERT INTO public.billable_metric VALUES ('018c3453-1f11-76a8-8d69-f74921b2646d', 'Bandwidth (GB)', '', 'bandwidth', 'SUM', 'value', 1, NULL, '{"matrix": null}', NULL, '2023-12-04 10:15:11.89', '378d66e2-ea89-4d6b-9fe0-7970a99eb03e', NULL, NULL, '018c2c82-3df1-7e84-9e05-6e141d0e751a', '018c2c82-3df2-71a4-b45c-86cb8604b75c');

