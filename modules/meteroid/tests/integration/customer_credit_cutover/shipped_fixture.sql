-- Shipped (main) shapes only: scalar balances, their history, and the credit notes the carry-over
-- reads (one credited to the scalar, one debt cancellation). Main's credit purchases were never used
-- in production: the release refuses to upgrade over any.
CREATE FUNCTION pg_temp.line(local_id TEXT, amount BIGINT) RETURNS jsonb
    LANGUAGE sql AS
$$
SELECT jsonb_build_object(
               'local_id', local_id, 'name', 'Seats', 'charge_type', 'one_time',
               'amount_subtotal', amount, 'tax_rate', '0', 'taxable_amount', amount, 'tax_amount', 0,
               'amount_total', amount, 'quantity', '1', 'unit_price', (amount / 100.0)::text,
               'start_date', '2026-06-01', 'end_date', '2026-06-01', 'sub_lines', '[]'::jsonb,
               'is_prorated', false, 'price_component_id', NULL, 'sub_component_id', NULL,
               'sub_add_on_id', NULL, 'product_id', NULL, 'metric_id', NULL, 'description', NULL,
               'kind', 'Regular')
$$;

CREATE FUNCTION pg_temp.invoice(id UUID, customer UUID, currency TEXT, status TEXT, payment TEXT, lines jsonb,
                                amount BIGINT) RETURNS void
    LANGUAGE sql AS
$$
INSERT INTO invoice (id, tenant_id, customer_id, currency, line_items, invoice_date, total, net_terms,
                     invoice_number, tax_amount, subtotal_recurring, customer_details, amount_due, subtotal,
                     seller_details, invoicing_entity_id, billing_period_start, status, payment_status,
                     invoice_type)
VALUES (id, '00000000-0000-0000-0000-00000000a001', customer, currency, lines, '2026-06-01', amount, 0,
        'INV-' || id::text, 0, 0, '{}', CASE WHEN payment = 'PAID' THEN 0 ELSE amount END, amount,
        '{}', '00000000-0000-0000-0000-00000000a003', '2026-06-01', status::"InvoiceStatusEnum",
        payment::"InvoicePaymentStatus", 'ONE_OFF')
$$;

CREATE FUNCTION pg_temp.credit_note(id UUID, invoice UUID, customer UUID, status TEXT, kind TEXT, line jsonb,
                                    amount BIGINT) RETURNS void
    LANGUAGE sql AS
$$
INSERT INTO credit_note (id, credit_note_number, status, tenant_id, customer_id, invoice_id, invoice_number,
                         currency, subtotal, tax_amount, total, customer_details, seller_details,
                         invoicing_entity_id, credit_type, line_items)
VALUES (id, 'CN-' || id::text, status::"CreditNoteStatus", '00000000-0000-0000-0000-00000000a001',
        customer, invoice, 'INV', 'USD', -amount, 0, -amount, '{}', '{}',
        '00000000-0000-0000-0000-00000000a003', kind::"CreditTypeEnum", jsonb_build_array(line))
$$;

INSERT INTO organization (id, trade_name, slug, default_country)
VALUES ('00000000-0000-0000-0000-00000000a000', 'Cutover Org', 'cutover', 'FR');
INSERT INTO "user" (id, email) VALUES ('00000000-0000-0000-0000-00000000a004', 'cutover@meteroid.dev');
INSERT INTO tenant (id, name, slug, organization_id, reporting_currency)
VALUES ('00000000-0000-0000-0000-00000000a001', 'Cutover', 'cutover', '00000000-0000-0000-0000-00000000a000', 'USD');
INSERT INTO invoicing_entity (id, is_default, legal_name, invoice_number_pattern, next_invoice_number,
                              next_credit_note_number, grace_period_hours, net_terms, country,
                              accounting_currency, tenant_id)
VALUES ('00000000-0000-0000-0000-00000000a003', true, 'Cutover SAS', 'INV-{number}', 1, 1, 0, 0, 'FR', 'USD',
        '00000000-0000-0000-0000-00000000a001');

-- c1: USD balance, invoiced in USD. c2: EUR balance, invoiced only in USD. c3: JPY balance, never
-- invoiced. c4: BHD (3 decimals), no balance.
INSERT INTO customer (id, name, tenant_id, currency, invoicing_entity_id, balance_value_cents)
VALUES ('00000000-0000-0000-0000-0000000000c1', 'Plain', '00000000-0000-0000-0000-00000000a001', 'USD',
        '00000000-0000-0000-0000-00000000a003', 1234),
       ('00000000-0000-0000-0000-0000000000c2', 'Other currency', '00000000-0000-0000-0000-00000000a001', 'EUR',
        '00000000-0000-0000-0000-00000000a003', 500),
       ('00000000-0000-0000-0000-0000000000c3', 'Yen', '00000000-0000-0000-0000-00000000a001', 'JPY',
        '00000000-0000-0000-0000-00000000a003', 42),
       ('00000000-0000-0000-0000-0000000000c4', 'Dinar', '00000000-0000-0000-0000-00000000a001', 'BHD',
        '00000000-0000-0000-0000-00000000a003', 0);

SELECT pg_temp.invoice('00000000-0000-0000-0000-0000000002c2', '00000000-0000-0000-0000-0000000000c2', 'USD',
                       'FINALIZED', 'PAID', jsonb_build_array(pg_temp.line('x', 100)), 100);
SELECT pg_temp.invoice('00000000-0000-0000-0000-000000000101', '00000000-0000-0000-0000-0000000000c1', 'USD',
                       'FINALIZED', 'PAID', jsonb_build_array(pg_temp.line('l1', 5000)), 5000);
-- 102: 3000 refunded by a finalized note; a voided note does not count.
SELECT pg_temp.invoice('00000000-0000-0000-0000-000000000102', '00000000-0000-0000-0000-0000000000c1', 'USD',
                       'FINALIZED', 'PAID', jsonb_build_array(pg_temp.line('l2', 8000)), 8000);
SELECT pg_temp.credit_note('00000000-0000-0000-0000-000000000302', '00000000-0000-0000-0000-000000000102',
                           '00000000-0000-0000-0000-0000000000c1', 'FINALIZED', 'REFUND',
                           pg_temp.line('l2', -3000), 3000);
SELECT pg_temp.credit_note('00000000-0000-0000-0000-000000000312', '00000000-0000-0000-0000-000000000102',
                           '00000000-0000-0000-0000-0000000000c1', 'VOIDED', 'REFUND',
                           pg_temp.line('l2', -1000), 1000);
-- 103: fully credited back to the (scalar) balance.
SELECT pg_temp.invoice('00000000-0000-0000-0000-000000000103', '00000000-0000-0000-0000-0000000000c1', 'USD',
                       'FINALIZED', 'PAID', jsonb_build_array(pg_temp.line('l3', 2000)), 2000);
SELECT pg_temp.credit_note('00000000-0000-0000-0000-000000000303', '00000000-0000-0000-0000-000000000103',
                           '00000000-0000-0000-0000-0000000000c1', 'FINALIZED', 'CREDIT_TO_BALANCE',
                           pg_temp.line('l3', -2000), 2000);
UPDATE credit_note SET credited_amount_cents = 2000 WHERE id = '00000000-0000-0000-0000-000000000303';
-- 109: partially paid, 1000 cancelled as debt.
SELECT pg_temp.invoice('00000000-0000-0000-0000-000000000109', '00000000-0000-0000-0000-0000000000c1', 'USD',
                       'FINALIZED', 'PARTIALLY_PAID', jsonb_build_array(pg_temp.line('l9', 6000)), 6000);
SELECT pg_temp.credit_note('00000000-0000-0000-0000-000000000309', '00000000-0000-0000-0000-000000000109',
                           '00000000-0000-0000-0000-0000000000c1', 'FINALIZED', 'DEBT_CANCELLATION',
                           pg_temp.line('l9', -1000), 1000);

-- The scalar's history: c1's two credits make its 1234.
INSERT INTO customer_balance_tx (id, amount_cents, balance_cents_after, tenant_id, customer_id)
VALUES ('00000000-0000-0000-0000-000000000407', 1000, 1000, '00000000-0000-0000-0000-00000000a001',
        '00000000-0000-0000-0000-0000000000c1'),
       ('00000000-0000-0000-0000-000000000408', 234, 1234, '00000000-0000-0000-0000-00000000a001',
        '00000000-0000-0000-0000-0000000000c1');
-- c2 is EUR but invoiced in USD: the currency of its deduction cannot be proven. c3 never invoiced.
INSERT INTO customer_balance_tx (id, amount_cents, balance_cents_after, invoice_id, tenant_id, customer_id)
VALUES ('00000000-0000-0000-0000-000000000409', -300, 200, '00000000-0000-0000-0000-0000000002c2',
        '00000000-0000-0000-0000-00000000a001', '00000000-0000-0000-0000-0000000000c2'),
       ('00000000-0000-0000-0000-00000000040a', 42, 42, NULL, '00000000-0000-0000-0000-00000000a001',
        '00000000-0000-0000-0000-0000000000c3');
